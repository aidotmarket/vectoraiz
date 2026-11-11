//! Untrusted native desktop design 7.2/7.3 handoff contracts, without a codec,
//! browser launcher, network client, or integration with export. Gate 2 C2 keeps
//! identity terms client-only, superseding their mention in design 7.3.
//!
//! Neither hashes nor successful validation prove an honest native client. A
//! server must independently verify source ownership/selection; native review is
//! a seller attestation. No local purchaser inventory is transmitted here.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256 as Hasher};

use crate::review::Sha256;

pub const MAX_ENCODED_BYTES: usize = 48 * 1024;
pub const MAX_EXPANDED_BYTES: usize = 64 * 1024;
pub const MAX_GATEWAY_FILES: usize = 64;

/// Reject before decoding, and check expanded length while decompressing, before
/// allocating/writing each output chunk. This helper does not decompress input.
/// Lengths are bytes, not Unicode character counts; nothing is truncated.
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
    // Supports existing native pack-<hash> ids and future review UUIDs. A pack
    // id is one neutral identifier, never a path, URL, or navigation component.
    if pack_id.is_empty()
        || !pack_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        bail!("invalid handoff pack id");
    }
    Ok(())
}

impl BucketHandoff {
    pub fn validate(&self) -> Result<()> {
        validate_pack_id(&self.pack_id)?;
        if self.connection_folder != "vz" || self.pack_prefix != format!("vz/{}/", self.pack_id) {
            bail!("bucket handoff must select its exact vz pack prefix");
        }
        let nonempty = |value: &Option<String>| value.as_ref().is_some_and(|s| !s.is_empty());
        if self.bucket.is_empty()
            || !match self.provider {
                BucketProvider::Aws => nonempty(&self.region) && self.account_id.is_none(),
                BucketProvider::R2 => nonempty(&self.account_id) && self.region.is_none(),
            }
        {
            bail!("bucket handoff requires the provider's region or account id");
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
        if self.files.len() > MAX_GATEWAY_FILES {
            bail!("gateway handoff exceeds 64 size/SHA256 pairs");
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn draft() -> Value {
        json!({"title": "Pack", "brief": "", "description": "", "category": "", "tags": ""})
    }

    fn gateway() -> Value {
        json!({"version": 1, "pack_id": "pack-123", "files": [{"size": 42, "sha256": "a".repeat(64)}], "draft": draft()})
    }

    fn bucket() -> Value {
        json!({"version": 1, "pack_id": "pack-123", "provider": "aws", "bucket": "seller-bucket", "region": "eu-west-1", "connection_folder": "vz", "pack_prefix": "vz/pack-123/", "file_count": 1, "selection_sha256": "b".repeat(64), "draft": draft()})
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
    fn encoded_and_expanded_boundaries_reject_without_truncating() {
        validate_lengths(MAX_ENCODED_BYTES, MAX_EXPANDED_BYTES).unwrap();
        assert!(validate_lengths(MAX_ENCODED_BYTES + 1, 0).is_err());
        assert!(validate_lengths(0, MAX_EXPANDED_BYTES + 1).is_err());
    }

    #[test]
    fn gateway_pair_limit() {
        let mut value: GatewayHandoff = serde_json::from_value(gateway()).unwrap();
        value.files = vec![value.files[0].clone(); 64];
        value.validate().unwrap();
        value.files.push(value.files[0].clone());
        assert!(value.validate().is_err());
        assert_eq!(value.files.len(), 65);
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
            ("vz/pack-123/a", "\"multipart-2\"", 42),
            ("vz/pack-123/b", "etag", 100),
        ];
        let expected: Sha256 = format!(
            "{:x}",
            Hasher::digest(
                br#"[["vz/pack-123/a","\"multipart-2\"",42],["vz/pack-123/b","etag",100]]"#
            )
        )
        .try_into()
        .unwrap();
        assert_eq!(
            bucket_selection_digest("pack-123", &tuples).unwrap(),
            expected
        );
        assert!(bucket_selection_digest("pack-123", &[tuples[1], tuples[0]]).is_err());
        assert!(bucket_selection_digest("pack-123", &[tuples[0], tuples[0]]).is_err());
        for key in [
            "other/a",
            "vz/pack-123/",
            "vz/pack-123/../a",
            "vz/pack-123//a",
            "vz/pack-123/a\\b",
        ] {
            assert!(bucket_selection_digest("pack-123", &[(key, "etag", 1)]).is_err());
        }
        assert!(bucket_selection_digest("pack-123", &[("vz/pack-123/a", "", 1)]).is_err());
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
        let tuples = [("vz/pack-123/a", "etag", 42)];
        let mut payload: BucketHandoff = serde_json::from_value(bucket()).unwrap();
        payload.selection_sha256 = bucket_selection_digest(&payload.pack_id, &tuples).unwrap();
        payload.validate_selection(&tuples).unwrap();
        assert!(payload.validate_selection(&[]).is_err());
        assert!(payload
            .validate_selection(&[("vz/pack-123/a", "changed", 42)])
            .is_err());
        assert!(payload
            .validate_selection(&[("vz/pack-123/a", "etag", 43)])
            .is_err());
    }
}
