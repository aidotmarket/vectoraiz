//! Native handoffs are seller attestations. The importer independently verifies
//! the selected destination inventory and ownership.

use anyhow::{bail, ensure, Result};
use russh::keys::ssh_encoding::base64::{Base64Unpadded, Encoding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256 as Hasher};
use std::{
    collections::BTreeMap,
    io::{Cursor, Write},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use crate::{
    listing_commands::{ListingFailure, ListingReference},
    listing_policy::{ListingError, ListingPolicy},
    review::{ListingReview, Sha256, StagedArtifact},
    review_store::{check_cancel, CurrentReview, ReviewStore},
};

pub const MAX_ENCODED_BYTES: usize = 48 * 1024;
pub const MAX_EXPANDED_BYTES: usize = 64 * 1024;
pub const MAX_GATEWAY_FILES: usize = 64;

/// Reject before decoding, and check expanded length while decompressing, before
/// allocating/writing each output chunk. This helper does not decompress input.
/// Lengths are bytes, not Unicode character counts; nothing is truncated.
/// The backend bounds decompression when importing the handoff.
pub fn validate_lengths(encoded_bytes: usize, expanded_bytes: usize) -> Result<()> {
    if encoded_bytes > MAX_ENCODED_BYTES {
        bail!("handoff exceeds 48 KB encoded limit");
    }
    if expanded_bytes > MAX_EXPANDED_BYTES {
        bail!("handoff exceeds 64 KB expanded limit");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub enum HandoffVersion {
    V1,
}

impl TryFrom<u32> for HandoffVersion {
    type Error = &'static str;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::V1),
            _ => Err("unsupported handoff version"),
        }
    }
}

impl From<HandoffVersion> for u32 {
    fn from(_: HandoffVersion) -> Self {
        1
    }
}

/// Only the five design draft text fields. Limits match ListingDraftContent at
/// backend pin d352f625; tags is its text field, not an array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftText {
    pub title: String,
    pub brief: String,
    pub description: String,
    pub category: String,
    pub tags: String,
}

impl DraftText {
    pub fn validate(&self) -> Result<()> {
        for (value, limit) in [
            (&self.title, 255),
            (&self.brief, 4000),
            (&self.description, 10000),
            (&self.category, 200),
            (&self.tags, 1000),
        ] {
            if value.chars().count() > limit {
                bail!("handoff draft text exceeds field limit");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BucketProvider {
    Aws,
    R2,
}

/// Bucket metadata only: never the bucket's per-file inventory. `region` is
/// present for AWS and `account_id` for R2. The design fixes the connection folder
/// to `vz` and the pack prefix to `vz/<pack-id>/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BucketHandoff {
    pub version: HandoffVersion,
    pub pack_id: String,
    pub provider: BucketProvider,
    pub bucket: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub connection_folder: String,
    pub pack_prefix: String,
    pub file_count: u64,
    /// Native selection commitment; ETags are not independently verified SHA256s.
    pub selection_sha256: Sha256,
    pub draft: DraftText,
}

fn validate_pack_id(pack_id: &str) -> Result<()> {
    ensure!(
        Uuid::parse_str(pack_id).is_ok_and(|uuid| uuid.to_string() == pack_id),
        "invalid handoff pack id"
    );
    Ok(())
}

fn identifier(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && !matches!(value, "." | "..")
}

impl BucketHandoff {
    pub fn validate(&self) -> Result<()> {
        validate_pack_id(&self.pack_id)?;
        if self.connection_folder != "vz" || self.pack_prefix != format!("vz/{}/", self.pack_id) {
            bail!("bucket handoff must select its exact vz pack prefix");
        }
        let nonempty = |value: &Option<String>| value.as_ref().is_some_and(|s| !s.is_empty());
        if !identifier(&self.bucket, 255)
            || self
                .region
                .as_ref()
                .is_some_and(|value| !identifier(value, 80))
            || self
                .account_id
                .as_ref()
                .is_some_and(|value| !identifier(value, 128))
            || !match self.provider {
                BucketProvider::Aws => nonempty(&self.region) && self.account_id.is_none(),
                BucketProvider::R2 => nonempty(&self.account_id) && self.region.is_none(),
            }
        {
            bail!("bucket handoff requires the provider's region or account id");
        }
        if self.file_count == 0 {
            bail!("bucket handoff requires at least one file");
        }
        self.draft.validate()
    }

    /// Compare a separately obtained post-upload listing with this commitment.
    /// A match alone establishes neither ownership nor metadata safety.
    pub fn validate_selection(&self, tuples: &[(&str, &str, u64)]) -> Result<()> {
        self.validate()?;
        if u64::try_from(tuples.len())? != self.file_count
            || bucket_selection_digest(&self.pack_id, tuples)? != self.selection_sha256
        {
            bail!("bucket selection count or digest mismatch");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayFile {
    pub size: u64,
    pub sha256: Sha256,
}

/// No gateway ids, paths, URLs, identity terms or file names. These are native
/// commitments to match against independently discovered gateway files later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayHandoff {
    pub version: HandoffVersion,
    pub pack_id: String,
    pub files: Vec<GatewayFile>,
    pub draft: DraftText,
}

impl GatewayHandoff {
    pub fn validate(&self) -> Result<()> {
        validate_pack_id(&self.pack_id)?;
        if self.files.is_empty() {
            bail!("gateway handoff requires at least one size/SHA256 pair");
        }
        if self.files.len() > MAX_GATEWAY_FILES {
            bail!("gateway handoff exceeds 64 size/SHA256 pairs");
        }
        for (index, file) in self.files.iter().enumerate() {
            if self.files[..index].contains(file) {
                bail!("gateway handoff requires unique size/SHA256 pairs");
            }
        }
        self.draft.validate()
    }
}

/// The two existing designs have disjoint strict fields; no extra routing field
/// is needed on the wire. Call `validate` after deserialization and before use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Handoff {
    Bucket(BucketHandoff),
    Gateway(GatewayHandoff),
}

impl Handoff {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Bucket(value) => value.validate(),
            Self::Gateway(value) => value.validate(),
        }
    }
}

/// Validate local post-upload listing tuples and compute the selection digest.
/// Tuple representation is compact JSON `[[key,etag,size],...]`, without a
/// newline, sorted by key's UTF-8 bytes. Preserve ETags verbatim, including quotes
/// and multipart suffixes. Do not sort, normalize or truncate invalid input.
/// This canonicalization supplies the previously unspecified wire spelling of
/// design 7.2's tuple digest; the browser must use exactly the same bytes.
/// The tuples themselves are never part of a handoff payload.
pub fn bucket_selection_digest(pack_id: &str, tuples: &[(&str, &str, u64)]) -> Result<Sha256> {
    validate_pack_id(pack_id)?;
    let prefix = format!("vz/{pack_id}/");
    let mut previous: Option<&str> = None;
    for &(key, etag, _) in tuples {
        if !key.starts_with(&prefix)
            || key.len() == prefix.len()
            || key.split('/').any(|part| matches!(part, "" | "." | ".."))
            || key.contains('\\')
            || etag.is_empty()
            || previous.is_some_and(|prev| prev >= key)
        {
            bail!("expected unique sorted pack key-ETag-size tuples");
        }
        previous = Some(key);
    }
    let bytes = serde_json::to_vec(tuples)?;
    Ok(format!("{:x}", Hasher::digest(bytes))
        .try_into()
        .expect("SHA256 hex"))
}

/// Input metadata stays local. A bucket caller supplies provider readback tuples
/// (quoted ETags, exact keys and sizes); gateway commitments come from the review.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum HandoffTarget {
    Bucket {
        provider: BucketProvider,
        bucket: String,
        region: Option<String>,
        account_id: Option<String>,
        objects: Vec<BucketObject>,
    },
    Gateway {
        names: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BucketObject {
    pub key: String,
    pub etag: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffRequest {
    pub draft: DraftText,
    pub target: HandoffTarget,
}

/// Only `handoff` is the encoded 7.2/7.3 payload. The generation UUID maps to
/// ImportRequest.idempotency_key. The other fields describe the local review;
/// the backend independently derives its own source revision and commitment.
#[derive(Debug, Serialize)]
pub struct ListingHandoff {
    pub handoff: String,
    pub idempotency_key: Uuid,
    pub source_revision: String,
    pub native_commitment: Sha256,
}

pub fn get_listing_handoff(
    policy: &ListingPolicy,
    reference: &ListingReference,
    request: HandoffRequest,
) -> Result<ListingHandoff, ListingFailure> {
    get_listing_handoff_inner(policy, reference, request, None)
}

pub fn get_listing_handoff_with_cancel(
    policy: &ListingPolicy,
    reference: &ListingReference,
    request: HandoffRequest,
    cancel: &CancellationToken,
) -> Result<ListingHandoff, ListingFailure> {
    get_listing_handoff_inner(policy, reference, request, Some(cancel))
}

fn get_listing_handoff_inner(
    policy: &ListingPolicy,
    reference: &ListingReference,
    request: HandoffRequest,
    cancel: Option<&CancellationToken>,
) -> Result<ListingHandoff, ListingFailure> {
    let run = || {
        policy.require_preview()?;
        get_reviewed_handoff(policy, reference, request, cancel)
    };
    run().map_err(Into::into)
}

fn get_reviewed_handoff(
    policy: &ListingPolicy,
    reference: &ListingReference,
    request: HandoffRequest,
    cancel: Option<&CancellationToken>,
) -> Result<ListingHandoff> {
    check_cancel(cancel)?;
    let store = ReviewStore::open_existing(&reference.store)?;
    // Reuse the same published-index selection, binding verification (including
    // old index_binding formats), policy lease and lock order as transfer.
    let pack = store.lock_existing_native_current(
        policy,
        &reference.current,
        &reference.library,
        cancel,
    )?;
    let review = pack.check_accepted(&reference.current)?;
    let artifacts = vector_artifacts(&review)?;
    let (staged, terms) = pack.reviewed_payload(&reference.current)?;
    #[cfg(feature = "listing-originals")]
    {
        let manifest = crate::verify_checked(&staged, &|| check_cancel(cancel))?;
        ensure!(
            manifest["originals"] == false,
            ListingError::StaleGeneration
        );
    }
    #[cfg(not(feature = "listing-originals"))]
    let _ = staged;
    check_cancel(cancel)?;
    let payload = build_payload(&review, &artifacts, &terms, request)?;
    let result = encode_listing(&reference.current, review.source_revision, &payload)?;
    check_cancel(cancel)?;
    pack.check_policy()?;
    Ok(result)
}

fn vector_artifacts(review: &ListingReview) -> Result<BTreeMap<&str, &StagedArtifact>> {
    let artifacts: BTreeMap<_, _> = review
        .staged_artifacts
        .iter()
        .map(|file| (file.artifact.as_str(), file))
        .collect();
    ensure!(
        review.staged_artifacts.len() == crate::vector_payload::FILES.len()
            && artifacts.len() == crate::vector_payload::FILES.len()
            && crate::vector_payload::FILES
                .iter()
                .all(|name| artifacts.contains_key(name)),
        ListingError::StaleGeneration
    );
    Ok(artifacts)
}

fn build_payload(
    review: &ListingReview,
    artifacts: &BTreeMap<&str, &StagedArtifact>,
    terms: &[String],
    request: HandoffRequest,
) -> Result<Handoff> {
    let HandoffRequest { draft, target } = request;
    let pack_id = review.pack_id.to_string();
    crate::sanitize::check_identifier(&pack_id, terms)?;
    let payload = match target {
        HandoffTarget::Bucket {
            provider,
            bucket,
            region,
            account_id,
            mut objects,
        } => {
            for identifier in std::iter::once(&bucket)
                .chain(region.iter())
                .chain(account_id.iter())
            {
                crate::sanitize::check_identifier(identifier, terms)?;
            }
            let prefix = format!("vz/{pack_id}/");
            ensure!(
                objects.len() == artifacts.len(),
                "bucket selection count mismatch"
            );
            objects.sort_unstable_by(|a, b| a.key.cmp(&b.key));
            for object in &objects {
                let found = object
                    .key
                    .strip_prefix(&prefix)
                    .and_then(|name| artifacts.get(name));
                ensure!(
                    found.is_some_and(|file| file.size == object.size),
                    "bucket selection names or sizes differ from review"
                );
                ensure!(
                    object
                        .etag
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .is_some_and(|s| !s.is_empty() && !s.contains(['"', '\r', '\n'])),
                    "bucket selection requires quoted provider ETags"
                );
            }
            let tuples: Vec<_> = objects
                .iter()
                .map(|o| (o.key.as_str(), o.etag.as_str(), o.size))
                .collect();
            let selection_sha256 = bucket_selection_digest(&pack_id, &tuples)?;
            Handoff::Bucket(BucketHandoff {
                version: HandoffVersion::V1,
                pack_id,
                provider,
                bucket,
                region,
                account_id,
                connection_folder: "vz".into(),
                pack_prefix: prefix,
                file_count: tuples.len() as u64,
                selection_sha256,
                draft,
            })
        }
        HandoffTarget::Gateway { mut names } => {
            let mut expected: Vec<_> = artifacts
                .keys()
                .map(|name| name.rsplit('/').next().unwrap())
                .collect();
            expected.sort_unstable();
            names.sort_unstable();
            ensure!(
                names == expected,
                "gateway selection names differ from review"
            );
            let files = artifacts
                .values()
                .map(|file| GatewayFile {
                    size: file.size,
                    sha256: file.sha256.clone(),
                })
                .collect();
            Handoff::Gateway(GatewayHandoff {
                version: HandoffVersion::V1,
                pack_id,
                files,
                draft,
            })
        }
    };
    Ok(payload)
}

fn encode_listing(
    current: &CurrentReview,
    source_revision: String,
    payload: &Handoff,
) -> Result<ListingHandoff> {
    payload.validate()?;
    let raw = serde_json::to_vec(payload)?;
    validate_lengths(0, raw.len())?;
    let handoff = encode_deflate(&raw)?;
    validate_lengths(handoff.len(), raw.len())?;
    Ok(ListingHandoff {
        handoff,
        idempotency_key: current.generation,
        source_revision,
        native_commitment: current.review_hash.clone(),
    })
}

fn encode_deflate(raw: &[u8]) -> Result<String> {
    // ZIP method 8 stores a raw RFC 1951 stream. Use the existing compressor
    // and extract exactly that member's compressed bytes for the wire format.
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file(
        "handoff",
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(6)),
    )?;
    writer.write_all(raw)?;
    let archive = writer.finish()?.into_inner();
    let mut zip = ZipArchive::new(Cursor::new(&archive))?;
    let entry = zip.by_index_raw(0)?;
    let start = usize::try_from(entry.data_start())?;
    let end = start + usize::try_from(entry.compressed_size())?;
    // The backend requires unpadded URL-safe base64.
    Ok(Base64Unpadded::encode_string(&archive[start..end])
        .replace('+', "-")
        .replace('/', "_"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn draft() -> Value {
        json!({"title": "Pack", "brief": "", "description": "", "category": "", "tags": ""})
    }

    fn gateway() -> Value {
        json!({"version": 1, "pack_id": "550e8400-e29b-41d4-a716-446655440000", "files": [{"size": 42, "sha256": "a".repeat(64)}], "draft": draft()})
    }

    fn bucket() -> Value {
        json!({"version": 1, "pack_id": "550e8400-e29b-41d4-a716-446655440000", "provider": "aws", "bucket": "seller-bucket", "region": "eu-west-1", "connection_folder": "vz", "pack_prefix": "vz/550e8400-e29b-41d4-a716-446655440000/", "file_count": 1, "selection_sha256": "b".repeat(64), "draft": draft()})
    }

    struct Allowed;
    impl crate::listing_policy::PolicySource for Allowed {
        fn read(&self) -> crate::listing_policy::ManagedPolicy {
            crate::listing_policy::ManagedPolicy::Allowed
        }
    }

    fn gateway_request() -> HandoffRequest {
        HandoffRequest {
            target: HandoffTarget::Gateway {
                names: crate::vector_payload::FILES
                    .iter()
                    .map(|name| name.rsplit('/').next().unwrap().to_string())
                    .collect(),
            },
            draft: serde_json::from_value(draft()).unwrap(),
        }
    }

    fn freeze_fixture(
        dir: &std::path::Path,
        policy: &ListingPolicy,
        originals: bool,
    ) -> ListingReference {
        use std::fs;
        let root = dir.canonicalize().unwrap();
        let library = root.join("index");
        let source = root.join("source");
        let payload = root.join("fixture-payload");
        let store_path = root.join("reviews");
        fs::create_dir(&library).unwrap();
        fs::create_dir(&source).unwrap();
        fs::create_dir_all(payload.join("examples")).unwrap();
        fs::write(library.join("writer.lock"), b"").unwrap();
        fs::write(library.join("recipe.json"), b"{}").unwrap();
        fs::write(source.join("source.txt"), b"original content").unwrap();
        let mut files = serde_json::Map::new();
        for name in crate::vector_payload::FILES
            .into_iter()
            .filter(|name| *name != "manifest.json")
        {
            fs::write(payload.join(name), name).unwrap();
            files.insert(name.into(), json!(format!("{:x}", Hasher::digest(name))));
        }
        if originals {
            fs::write(payload.join("originals-0001.zip"), b"legacy original").unwrap();
        }
        fs::write(
            payload.join("manifest.json"),
            json!({"originals": originals, "files": files}).to_string(),
        )
        .unwrap();

        let pack_id = Uuid::new_v4();
        let review = serde_json::from_value(json!({
            "schema_version":"listing-review-v1", "pack_id": pack_id,
            "staged_artifacts": [], "metadata_inventory_schema_version":1,
            "metadata_inventory":[{"location":"fixture","kind":"text","value":"staged","warning":false}],
            "source_revision":"", "index_revision":"", "embedding_recipe_sha256":"0".repeat(64),
            "warnings":[], "exclusions":[]
        })).unwrap();
        let store = ReviewStore::open(&store_path).unwrap();
        let guard = store.lock_native(policy, pack_id, &library).unwrap();
        let current = guard
            .freeze_sanitized(
                guard.capture(&source).unwrap(),
                review,
                &payload,
                json!({"identity_terms":["Sensitive Corp"]}),
            )
            .unwrap();
        guard.accept(&current).unwrap();
        ListingReference {
            store: store_path,
            library,
            current,
        }
    }

    #[test]
    #[cfg(not(any(
        feature = "listing-preview-harness",
        feature = "listing-preview-test-switch"
    )))]
    fn allowed_policy_cannot_get_handoff_in_default_build() {
        let dir = tempfile::tempdir().unwrap();
        let policy = ListingPolicy::new(std::sync::Arc::new(Allowed));
        let reference = freeze_fixture(dir.path(), &policy, false);
        assert_eq!(
            get_listing_handoff(&policy, &reference, gateway_request())
                .unwrap_err()
                .code,
            ListingError::PreviewDisabled
        );
    }

    #[test]
    fn unknown_pack_leaves_store_untouched_and_stale_pack_is_invalidated() {
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let policy = ListingPolicy::new(std::sync::Arc::new(Allowed));
        let reference = freeze_fixture(dir.path(), &policy, false);
        let snapshot = || {
            walkdir::WalkDir::new(&reference.store)
                .sort_by_file_name()
                .into_iter()
                .map(|entry| {
                    let entry = entry.unwrap();
                    let bytes = entry
                        .file_type()
                        .is_file()
                        .then(|| fs::read(entry.path()).unwrap());
                    (entry.into_path(), bytes)
                })
                .collect::<Vec<_>>()
        };
        let before = snapshot();
        let mut unknown = reference.clone();
        unknown.current.pack_id = Uuid::new_v4();
        let error = get_reviewed_handoff(&policy, &unknown, gateway_request(), None).unwrap_err();
        assert_eq!(error.downcast_ref(), Some(&ListingError::StaleGeneration));
        assert_eq!(snapshot(), before);

        unknown.store = reference.store.with_file_name("missing-reviews");
        let error = get_reviewed_handoff(&policy, &unknown, gateway_request(), None).unwrap_err();
        assert_eq!(error.downcast_ref(), Some(&ListingError::StaleGeneration));
        assert!(!unknown.store.exists());

        let source = reference
            .library
            .with_file_name("source")
            .join("source.txt");
        let original = fs::read(&source).unwrap();
        fs::write(&source, b"changed source").unwrap();
        let error = get_reviewed_handoff(&policy, &reference, gateway_request(), None).unwrap_err();
        assert_eq!(error.downcast_ref(), Some(&ListingError::StaleGeneration));
        let invalidation = reference
            .store
            .join(reference.current.pack_id.to_string())
            .join(reference.current.generation.to_string())
            .join("invalidated.json");
        assert!(invalidation.is_file());
        fs::write(&source, original).unwrap();
        let error = get_reviewed_handoff(&policy, &reference, gateway_request(), None).unwrap_err();
        assert_eq!(error.downcast_ref(), Some(&ListingError::StaleGeneration));
    }

    #[test]
    fn cancelled_handoff_worker_exits_while_pack_is_locked() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let policy = std::sync::Arc::new(ListingPolicy::new(std::sync::Arc::new(Allowed)));
        let reference = freeze_fixture(dir.path(), &policy, false);
        let store = ReviewStore::open_existing(&reference.store).unwrap();
        let lock = store.lock(&policy, reference.current.pack_id).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let worker_policy = policy.clone();
        let (started, waiting) = std::sync::mpsc::channel();
        let mut worker = runtime.spawn_blocking(move || {
            started.send(()).unwrap();
            get_reviewed_handoff(
                &worker_policy,
                &reference,
                gateway_request(),
                Some(&worker_cancel),
            )
        });
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        let (blocked, outcome) = runtime.block_on(async {
            let blocked = tokio::time::timeout(Duration::from_millis(100), &mut worker).await;
            cancel.cancel();
            let outcome = if blocked.is_err() {
                Some(tokio::time::timeout(Duration::from_secs(2), &mut worker).await)
            } else {
                None
            };
            (blocked, outcome)
        });
        drop(lock);
        assert!(blocked.is_err(), "worker did not wait for the held lock");
        let error = outcome
            .unwrap()
            .expect("worker must exit before the lock is released")
            .unwrap()
            .unwrap_err();
        assert_eq!(error.to_string(), "listing handoff cancelled");
    }

    #[test]
    fn import_marker_idempotency_for_accepted_generation() {
        let dir = tempfile::tempdir().unwrap();
        let policy = ListingPolicy::new(std::sync::Arc::new(Allowed));
        let reference = freeze_fixture(dir.path(), &policy, false);
        let first = get_reviewed_handoff(&policy, &reference, gateway_request(), None).unwrap();
        let second = get_reviewed_handoff(&policy, &reference, gateway_request(), None).unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(first.idempotency_key, reference.current.generation);
        assert_eq!(first.native_commitment, reference.current.review_hash);
        assert_eq!(first.source_revision.len(), 64);

        let store = ReviewStore::open(&reference.store).unwrap();
        let guard = store
            .lock_native_current(&policy, &reference.current, &reference.library)
            .unwrap();
        let review = guard.check_accepted(&reference.current).unwrap();
        let artifacts = vector_artifacts(&review).unwrap();
        let bucket_request = |etag: &str| HandoffRequest {
            target: HandoffTarget::Bucket {
                provider: BucketProvider::Aws,
                bucket: "seller-bucket".into(),
                region: Some("us-east-1".into()),
                account_id: None,
                objects: artifacts
                    .values()
                    .rev()
                    .map(|file| BucketObject {
                        key: format!("vz/{}/{}", review.pack_id, file.artifact),
                        etag: format!("\"{etag}\""),
                        size: file.size,
                    })
                    .collect(),
            },
            draft: serde_json::from_value(draft()).unwrap(),
        };
        let first = build_payload(&review, &artifacts, &[], bucket_request("abc")).unwrap();
        let mut swapped = bucket_request("abc");
        if let HandoffTarget::Bucket { objects, .. } = &mut swapped.target {
            objects[0].etag = "\"def\"".into();
        }
        let swapped = build_payload(&review, &artifacts, &[], swapped).unwrap();
        assert_ne!(first, swapped);
        for case in ["missing", "extra", "duplicate", "wrong name", "wrong size"] {
            let mut request = bucket_request("abc");
            if let HandoffTarget::Bucket { objects, .. } = &mut request.target {
                match case {
                    "missing" => {
                        objects.pop();
                    }
                    "extra" => objects.push(BucketObject {
                        key: "extra".into(),
                        etag: "\"extra\"".into(),
                        size: 1,
                    }),
                    "duplicate" => objects[0].key = objects[1].key.clone(),
                    "wrong name" => objects[0].key = "originals-0001.zip".into(),
                    _ => objects[0].size += 1,
                }
            }
            assert!(
                build_payload(&review, &artifacts, &[], request).is_err(),
                "{case}"
            );
        }
    }

    #[test]
    fn handoff_boundary_rejects_invalid_current_and_oversized_json() {
        let dir = tempfile::tempdir().unwrap();
        let policy = ListingPolicy::new(std::sync::Arc::new(Allowed));
        let reference = freeze_fixture(dir.path(), &policy, false);
        let mut forged = reference.clone();
        forged.current.review_hash = "f".repeat(64).try_into().unwrap();
        assert!(get_reviewed_handoff(&policy, &forged, gateway_request(), None).is_err());
        let mut request = gateway_request();
        request.draft.brief = "\0".repeat(4000);
        request.draft.description = "\0".repeat(10000);
        assert!(get_reviewed_handoff(&policy, &reference, request, None)
            .unwrap_err()
            .to_string()
            .contains("64 KB"));

        std::fs::write(reference.library.join("active-index.json"), b"invalid").unwrap();
        let stale = get_reviewed_handoff(&policy, &reference, gateway_request(), None).unwrap_err();
        assert_eq!(stale.downcast_ref(), Some(&ListingError::StaleGeneration));
        std::fs::remove_file(reference.library.join("active-index.json")).unwrap();
        assert!(get_reviewed_handoff(&policy, &reference, gateway_request(), None).is_err());

        let legacy = tempfile::tempdir().unwrap();
        let legacy = freeze_fixture(legacy.path(), &policy, true);
        let stale = get_reviewed_handoff(&policy, &legacy, gateway_request(), None).unwrap_err();
        assert_eq!(stale.downcast_ref(), Some(&ListingError::StaleGeneration));
    }

    #[test]
    fn payload_roundtrips_both_providers_and_gateway() {
        let mut r2 = bucket();
        r2["provider"] = json!("r2");
        r2.as_object_mut().unwrap().remove("region");
        r2["account_id"] = json!("seller-account");
        for value in [bucket(), r2, gateway()] {
            let payload: Handoff = serde_json::from_value(value.clone()).unwrap();
            payload.validate().unwrap();
            assert_eq!(serde_json::to_value(payload).unwrap(), value);
        }
    }

    #[test]
    fn unknown_versions_and_forbidden_fields_rejected() {
        for original in [bucket(), gateway()] {
            for version in [json!(0), json!(2), json!("1"), json!(null), json!(1.0)] {
                let mut value = original.clone();
                value["version"] = version;
                assert!(serde_json::from_value::<Handoff>(value).is_err());
            }
            for field in [
                "path",
                "url",
                "credentials",
                "draft_id",
                "identity_terms",
                "gateway_id",
                "bucket_files",
            ] {
                let mut value = original.clone();
                value[field] = json!("forbidden");
                assert!(serde_json::from_value::<Handoff>(value).is_err());
            }
            let mut value = original;
            value["draft"]["price"] = json!("1");
            assert!(serde_json::from_value::<Handoff>(value).is_err());
        }
        let mut value = gateway();
        value["files"][0]["path"] = json!("private");
        assert!(serde_json::from_value::<Handoff>(value).is_err());
    }

    #[test]
    fn caller_supplied_lengths_compare_at_48_kib_encoded_and_64_kib_expanded_boundaries() {
        validate_lengths(MAX_ENCODED_BYTES, MAX_EXPANDED_BYTES).unwrap();
        assert!(validate_lengths(MAX_ENCODED_BYTES + 1, 0).is_err());
        assert!(validate_lengths(0, MAX_EXPANDED_BYTES + 1).is_err());
    }

    #[test]
    fn gateway_pair_limit() {
        let mut value: GatewayHandoff = serde_json::from_value(gateway()).unwrap();
        value.files = (0..64)
            .map(|size| GatewayFile {
                size,
                sha256: value.files[0].sha256.clone(),
            })
            .collect();
        value.validate().unwrap();
        value.files.push(value.files[0].clone());
        assert!(value.validate().is_err());
        assert_eq!(value.files.len(), 65);
    }

    #[test]
    fn gateway_rejects_empty_and_duplicate_pairs() {
        let mut value: GatewayHandoff = serde_json::from_value(gateway()).unwrap();
        value.files.clear();
        assert!(value.validate().is_err());

        let original: GatewayHandoff = serde_json::from_value(gateway()).unwrap();
        value.files = vec![original.files[0].clone(); 2];
        assert!(value.validate().is_err());

        // Either component may repeat when the pair is distinct.
        value.files[1].size += 1;
        value.validate().unwrap();
        value.files[1].size = value.files[0].size;
        value.files[1].sha256 = "b".repeat(64).try_into().unwrap();
        value.validate().unwrap();
        // Duplicates need not be adjacent.
        value.files.push(value.files[0].clone());
        assert!(value.validate().is_err());
    }

    #[test]
    fn bucket_rejects_zero_file_count() {
        let mut value: BucketHandoff = serde_json::from_value(bucket()).unwrap();
        value.validate().unwrap();
        value.file_count = 0;
        assert!(value.validate().is_err());
    }

    #[test]
    fn pack_id_requires_canonical_uuid() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        validate_pack_id(id).unwrap();
        assert!(validate_pack_id(&id.to_uppercase()).is_err());
        assert!(validate_pack_id(&id.replace('-', "")).is_err());
    }

    #[test]
    fn all_draft_field_limits_use_unicode_characters() {
        let original: DraftText = serde_json::from_value(draft()).unwrap();
        for (field, limit) in [
            ("title", 255),
            ("brief", 4000),
            ("description", 10000),
            ("category", 200),
            ("tags", 1000),
        ] {
            let mut value = serde_json::to_value(&original).unwrap();
            value[field] = json!("é".repeat(limit));
            serde_json::from_value::<DraftText>(value.clone())
                .unwrap()
                .validate()
                .unwrap();
            value[field] = json!("é".repeat(limit + 1));
            assert!(serde_json::from_value::<DraftText>(value)
                .unwrap()
                .validate()
                .is_err());
        }
    }

    #[test]
    fn bucket_digest_is_exact_sorted_tuple_commitment() {
        let tuples = [
            (
                "vz/550e8400-e29b-41d4-a716-446655440000/a",
                "\"multipart-2\"",
                42,
            ),
            ("vz/550e8400-e29b-41d4-a716-446655440000/b", "etag", 100),
        ];
        let expected: Sha256 = format!(
            "{:x}",
            Hasher::digest(
                br#"[["vz/550e8400-e29b-41d4-a716-446655440000/a","\"multipart-2\"",42],["vz/550e8400-e29b-41d4-a716-446655440000/b","etag",100]]"#
            )
        )
        .try_into()
        .unwrap();
        assert_eq!(
            bucket_selection_digest("550e8400-e29b-41d4-a716-446655440000", &tuples).unwrap(),
            expected
        );
        assert!(bucket_selection_digest(
            "550e8400-e29b-41d4-a716-446655440000",
            &[tuples[1], tuples[0]]
        )
        .is_err());
        assert!(bucket_selection_digest(
            "550e8400-e29b-41d4-a716-446655440000",
            &[tuples[0], tuples[0]]
        )
        .is_err());
        for key in [
            "other/a",
            "vz/550e8400-e29b-41d4-a716-446655440000/",
            "vz/550e8400-e29b-41d4-a716-446655440000/../a",
            "vz/550e8400-e29b-41d4-a716-446655440000//a",
            "vz/550e8400-e29b-41d4-a716-446655440000/a\\b",
        ] {
            assert!(bucket_selection_digest(
                "550e8400-e29b-41d4-a716-446655440000",
                &[(key, "etag", 1)]
            )
            .is_err());
        }
        assert!(bucket_selection_digest(
            "550e8400-e29b-41d4-a716-446655440000",
            &[("vz/550e8400-e29b-41d4-a716-446655440000/a", "", 1)]
        )
        .is_err());
    }

    #[test]
    fn bucket_shape_rejections() {
        for (field, invalid) in [
            ("connection_folder", json!("other")),
            ("pack_prefix", json!("vz/other/")),
            ("pack_id", json!("../private")),
            ("region", json!(null)),
            ("account_id", json!("unexpected")),
        ] {
            let mut value = bucket();
            value[field] = invalid;
            assert!(serde_json::from_value::<BucketHandoff>(value)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut value = bucket();
        value["selection_sha256"] = json!("not-a-digest");
        assert!(serde_json::from_value::<BucketHandoff>(value).is_err());
    }

    #[test]
    fn bucket_selection_rejects_count_and_object_changes() {
        let tuples = [("vz/550e8400-e29b-41d4-a716-446655440000/a", "etag", 42)];
        let mut payload: BucketHandoff = serde_json::from_value(bucket()).unwrap();
        payload.selection_sha256 = bucket_selection_digest(&payload.pack_id, &tuples).unwrap();
        payload.validate_selection(&tuples).unwrap();
        assert!(payload.validate_selection(&[]).is_err());
        assert!(payload
            .validate_selection(&[("vz/550e8400-e29b-41d4-a716-446655440000/a", "changed", 42)])
            .is_err());
        assert!(payload
            .validate_selection(&[("vz/550e8400-e29b-41d4-a716-446655440000/a", "etag", 43)])
            .is_err());
    }
}
