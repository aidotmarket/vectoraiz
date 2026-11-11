//! Private, append-only review generations. This storage is not a sanitizer or
//! a listing entry point: production transfers remain gated until C1 is complete.
//! No private binding, receipt, or source name belongs in delivered payloads.
use crate::{
    listing_policy::{ListingError, ListingPolicy, PolicyLease},
    review::{ListingReview, Sha256, StagedArtifact},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256 as Hasher};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[cfg(windows)]
mod windows;

const MAX_FILES: usize = 50_000;
const MAX_BYTES: u64 = 2_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileHash {
    size: u64,
    sha256: Sha256,
}

/// Complete tree commitments include names and additions/deletions, not mtimes.
/// Capture while the caller holds its index read/write fence. The store rechecks
/// on acceptance and transfer; it never mutates or recovers the source/index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewBinding {
    source: PathBuf,
    index: PathBuf,
    source_files: BTreeMap<String, FileHash>,
    index_files: BTreeMap<String, FileHash>,
    recipe: PathBuf,
    recipe_file: FileHash,
    recipe_sha256: Sha256,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection: Option<NativeSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeSelection {
    library: PathBuf,
    index: PathBuf,
    pointer: Option<FileHash>,
}

impl NativeSelection {
    fn capture(library: &Path) -> Result<Self> {
        ensure!(
            !vz_core::active_index::needs_pointer_repair(library),
            ListingError::StaleGeneration
        );
        let pointer = library.join("active-index.json");
        Ok(Self {
            library: plain_directory(library)?,
            index: plain_directory(&vz_core::active_index::resolve_state(library))?,
            pointer: if pointer.try_exists()? {
                Some(hash_file(&pointer)?)
            } else {
                None
            },
        })
    }
}

/// Read-only live native state fence. Does not open SQLite, run recovery or
/// load models. Library selection and the selected writer are fenced across
/// capture/accept/transfer. External source changes are still hash checked.
pub struct NativeIndexFence {
    selection: NativeSelection,
    _selection: vz_core::active_index::SelectionFence,
    writer: File,
}

impl NativeIndexFence {
    fn acquire(library: &Path) -> Result<Self> {
        plain_directory(library)?;
        let selection = vz_core::active_index::selection_fence(library, false)?;
        let snapshot = NativeSelection::capture(library)?;
        let lock_path = snapshot.index.join("writer.lock");
        check_regular(&lock_path)?;
        let writer = OpenOptions::new().read(true).open(lock_path)?;
        fs2::FileExt::try_lock_shared(&writer)
            .context("native index is in use; retry after indexing/search")?;
        ensure!(
            NativeSelection::capture(library)? == snapshot,
            ListingError::StaleGeneration
        );
        Ok(Self {
            selection: snapshot,
            _selection: selection,
            writer,
        })
    }
}

impl Drop for NativeIndexFence {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.writer);
    }
}

impl ReviewBinding {
    pub fn capture(source: &Path, index: &Path, recipe: &Path) -> Result<Self> {
        Ok(Self {
            source: plain_directory(source)?,
            index: plain_directory(index)?,
            source_files: tree(source)?,
            index_files: tree(index)?,
            recipe: fs::canonicalize(recipe)?,
            recipe_file: hash_file(recipe)?,
            recipe_sha256: digest(&read_input_json::<serde_json::Value>(recipe)?)?,
            selection: None,
        })
    }

    fn revisions(&self) -> Result<(String, String)> {
        Ok((
            digest(&self.source_files)?.into(),
            digest(&self.index_files)?.into(),
        ))
    }

    fn check(&self) -> Result<()> {
        let mut captured = Self::capture(&self.source, &self.index, &self.recipe)?;
        if let Some(selection) = &self.selection {
            let current = NativeSelection::capture(&selection.library)?;
            ensure!(
                current == *selection && current.index == self.index,
                ListingError::StaleGeneration
            );
            captured.selection = Some(current);
        }
        ensure!(captured == *self, ListingError::StaleGeneration);
        Ok(())
    }
}

/// Required on acceptance and every future reviewed write/upload/export call.
/// The commitment covers the immutable review, payload and private binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentReview {
    pub pack_id: Uuid,
    pub generation: Uuid,
    pub review_hash: Sha256,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Generation {
    current: CurrentReview,
    binding: ReviewBinding,
    review: ListingReview,
}

/// Durable job evidence is outside the payload. An intent without a completed
/// receipt is uncertain after restart and must be reconciled, never retried as
/// a new promotion or cleaned by deleting completed destination data.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferIntent {
    pub job_id: Uuid,
    pub current: CurrentReview,
    pub destination_commitment: Sha256,
}

/// A caller-selected application-private directory outside all collection and
/// index trees. Owner-only Unix modes and protected Windows DACLs are enforced.
pub struct ReviewStore {
    root: PathBuf,
}

impl ReviewStore {
    pub fn open(root: &Path) -> Result<Self> {
        private_directory(root)?;
        Ok(Self {
            root: fs::canonicalize(root)?,
        })
    }

    /// Acquire a policy lease BEFORE the cross-process per-pack mutex. Policy
    /// refresh never takes this mutex. Holding this guard serializes rebuilding,
    /// accepting and future transfer promotion for the same pack UUID.
    pub fn lock<'a>(&'a self, policy: &'a ListingPolicy, pack_id: Uuid) -> Result<PackReview<'a>> {
        let lease = policy.policy_lease()?;
        let path = self.root.join(pack_id.to_string());
        private_directory(&path)?;
        let lock_path = path.join("pack.lock");
        let lock = match private_file(&lock_path) {
            Ok(file) => file,
            Err(error) if lock_path.try_exists()? => {
                check_regular(&lock_path)?;
                check_private(&lock_path)?;
                OpenOptions::new()
                    .write(true)
                    .open(&lock_path)
                    .context(error)?
            }
            Err(error) => return Err(error),
        };
        loop {
            lease.check(policy)?;
            match fs2::FileExt::try_lock_exclusive(&lock) {
                Ok(()) => break,
                Err(error)
                    if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                {
                    // A waiter must observe policy cancellation even while a
                    // different process still owns the pack's file lock.
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(error) => return Err(error.into()),
            }
        }
        lease.check(policy)?;
        Ok(PackReview {
            store: self,
            path,
            pack_id,
            policy,
            lease,
            _lock: lock,
        })
    }

    /// Native commands acquire policy → pack → library selection → index.
    /// Re-index never acquires a pack mutex, so policy updates and indexing
    /// cannot take these locks in reverse order.
    pub fn lock_native<'a>(
        &'a self,
        policy: &'a ListingPolicy,
        pack_id: Uuid,
        library: &Path,
    ) -> Result<NativePackReview<'a>> {
        let pack = self.lock(policy, pack_id)?;
        let index = NativeIndexFence::acquire(library)?;
        Ok(NativePackReview { pack, index })
    }

    /// Existing-generation entries validate and durably invalidate before
    /// attempting the live fence. A damaged pointer must not revive an old
    /// acceptance merely because it is restored before the next command.
    pub fn lock_native_current<'a>(
        &'a self,
        policy: &'a ListingPolicy,
        current: &CurrentReview,
        library: &Path,
    ) -> Result<NativePackReview<'a>> {
        let pack = self.lock(policy, current.pack_id)?;
        let generation = pack.validate(current, false)?;
        let selection = NativeSelection::capture(library);
        if selection.as_ref().ok() != generation.binding.selection.as_ref() {
            pack.invalidate(current, "native index selection changed or binding absent")?;
            return Err(ListingError::StaleGeneration.into());
        }
        let index = NativeIndexFence::acquire(library)?;
        let native = NativePackReview { pack, index };
        native.inspect(current)?;
        Ok(native)
    }
}

pub struct NativePackReview<'a> {
    pub(crate) pack: PackReview<'a>,
    index: NativeIndexFence,
}

impl NativePackReview<'_> {
    /// Only a sanitizer rebuild may call freeze with this captured binding.
    pub fn capture(&self, source: &Path) -> Result<ReviewBinding> {
        self.pack.lease.check(self.pack.policy)?;
        let index = &self.index.selection.index;
        let mut binding = ReviewBinding::capture(source, index, &index.join("recipe.json"))?;
        binding.selection = Some(self.index.selection.clone());
        binding.check()?;
        self.pack.lease.check(self.pack.policy)?;
        Ok(binding)
    }

    fn check_native(&self, current: &CurrentReview, accepted: bool) -> Result<ListingReview> {
        let generation = self.pack.validate(current, accepted)?;
        if generation.binding.selection.as_ref() != Some(&self.index.selection) {
            self.pack
                .invalidate(current, "native index selection changed or binding absent")?;
            return Err(ListingError::StaleGeneration.into());
        }
        if accepted
            || self
                .pack
                .generation_path(current)
                .join("accepted.json")
                .try_exists()?
        {
            self.pack.accepted_review(generation)
        } else {
            Ok(generation.review)
        }
    }

    pub fn inspect(&self, current: &CurrentReview) -> Result<ListingReview> {
        self.check_native(current, false)
    }
    pub fn accept(&self, current: &CurrentReview) -> Result<ListingReview> {
        self.check_native(current, false)?;
        self.pack.accept(current)
    }
    pub fn check_accepted(&self, current: &CurrentReview) -> Result<ListingReview> {
        self.check_native(current, true)
    }
}

pub struct PackReview<'a> {
    store: &'a ReviewStore,
    path: PathBuf,
    pack_id: Uuid,
    policy: &'a ListingPolicy,
    lease: PolicyLease,
    _lock: File,
}

impl Drop for PackReview<'_> {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self._lock);
    }
}

impl PackReview<'_> {
    fn invalidate(&self, current: &CurrentReview, reason: &str) -> Result<()> {
        let path = self.generation_path(current).join("invalidated.json");
        if !path.try_exists()? {
            write_json(&path, &reason)?;
        }
        Ok(())
    }
    /// Persist a rebuilt payload supplied by the future sanitizer. No source
    /// mappings or terms are copied from private state into this payload.
    /// This method does not certify metadata safety or enable delivery.
    pub fn freeze(
        &self,
        binding: ReviewBinding,
        mut review: ListingReview,
        payload: &Path,
    ) -> Result<CurrentReview> {
        self.lease.check(self.policy)?;
        for protected in [&binding.source, &binding.index] {
            ensure!(
                !self.store.root.starts_with(protected) && !protected.starts_with(&self.store.root),
                "review store must be outside source and index trees"
            );
        }
        ensure!(
            review.pack_id == self.pack_id && review.pack_id.get_version_num() == 4,
            "new reviews require the pack's UUID v4"
        );
        ensure!(
            review.accepted_review_hash.is_none()
                && review.completed_transfer_fingerprint.is_none(),
            "new generation cannot inherit acceptance or transfer"
        );
        binding.check()?;
        let generation = Uuid::new_v4();
        let partial = self.path.join(format!("{generation}.partial"));
        private_directory(&partial)?;
        let result = (|| {
            let staged = partial.join("payload");
            private_directory(&staged)?;
            let files = tree(payload)?;
            ensure!(!files.is_empty(), "empty reviewed payload");
            for (name, expected) in &files {
                self.lease.check(self.policy)?;
                let target = staged.join(name);
                private_parents(&staged, Path::new(name))?;
                let mut input = File::open(payload.join(name))?.take(expected.size + 1);
                let mut output = private_file(&target)?;
                let cancel = self.lease.cancellation_token();
                let mut copied = 0;
                let mut buffer = [0; 65536];
                loop {
                    ensure!(!cancel.is_cancelled(), ListingError::StaleGeneration);
                    let count = input.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    output.write_all(&buffer[..count])?;
                    copied += count as u64;
                }
                ensure!(copied == expected.size, "payload changed while staging");
                output.sync_all()?;
                ensure!(
                    hash_file(&target)? == *expected,
                    "payload changed while staging"
                );
            }
            ensure!(
                tree(payload)? == files && tree(&staged)? == files,
                "payload changed while staging"
            );
            review.staged_artifacts = files
                .into_iter()
                .map(|(artifact, hash)| StagedArtifact {
                    artifact,
                    size: hash.size,
                    sha256: hash.sha256,
                })
                .collect();
            (review.source_revision, review.index_revision) = binding.revisions()?;
            review.embedding_recipe_sha256 = binding.recipe_sha256.clone();
            let review_hash = digest(&(&binding, &review))?;
            let current = CurrentReview {
                pack_id: self.pack_id,
                generation,
                review_hash,
            };
            write_json(
                &partial.join("generation.json"),
                &Generation {
                    current: current.clone(),
                    binding: binding.clone(),
                    review,
                },
            )?;
            binding.check()?;
            self.lease.check(self.policy)?;
            // Persist nested directory entries as well as file data before
            // exposing the immutable generation after a power loss.
            for entry in walkdir::WalkDir::new(&partial).contents_first(true) {
                let entry = entry?;
                if entry.file_type().is_dir() {
                    sync_directory(entry.path())?;
                }
            }
            fs::rename(&partial, self.path.join(generation.to_string()))?;
            sync_directory(&self.path)?;
            // Append a monotonically ordered head instead of overwriting a
            // pointer. This also works on Windows without rename-overwrite.
            let sequence = self.head()?.map_or(Ok(0), |(n, _)| {
                n.checked_add(1).context("generation overflow")
            })?;
            write_json(
                &self.path.join(format!("head-{sequence:020}.json")),
                &current,
            )?;
            Ok(current)
        })();
        if result.is_err() {
            // Only this call's UUID-owned incomplete stage is removable.
            let _ = fs::remove_dir_all(&partial);
        }
        result
    }

    pub fn accept(&self, current: &CurrentReview) -> Result<ListingReview> {
        let generation = self.validate(current, false)?;
        let receipt = self.generation_path(current).join("accepted.json");
        if !receipt.try_exists()? {
            write_json(&receipt, &current.review_hash)?;
        }
        self.accepted_review(generation)
    }

    /// Validate before EACH destination write and final promotion while holding
    /// this guard. No payload path is returned by this storage API: a future
    /// reviewed transfer must additionally enforce the preview/sanitizer gate.
    pub fn check_accepted(&self, current: &CurrentReview) -> Result<ListingReview> {
        let generation = self.validate(current, true)?;
        self.accepted_review(generation)
    }

    pub fn record_transfer_intent(
        &self,
        current: &CurrentReview,
        destination_commitment: Sha256,
    ) -> Result<TransferIntent> {
        self.check_accepted(current)?;
        let intent = TransferIntent {
            job_id: Uuid::new_v4(),
            current: current.clone(),
            destination_commitment,
        };
        write_json(
            &self
                .generation_path(current)
                .join(format!("transfer-{}.json", intent.job_id)),
            &intent,
        )?;
        Ok(intent)
    }

    /// Call only after destination readback and successful promotion. Storage
    /// records evidence, not proof of provider success; C1 transfer adapters must
    /// compute the fingerprint from the actual destination inventory.
    pub fn record_completed_transfer(
        &self,
        intent: &TransferIntent,
        fingerprint: Sha256,
    ) -> Result<ListingReview> {
        let mut review = self.check_accepted(&intent.current)?;
        let path = self.generation_path(&intent.current);
        let stored: TransferIntent =
            read_json(&path.join(format!("transfer-{}.json", intent.job_id)))?;
        ensure!(
            stored.current == intent.current
                && stored.destination_commitment == intent.destination_commitment,
            ListingError::StaleGeneration
        );
        let receipt = path.join(format!("completed-{}.json", intent.job_id));
        if receipt.try_exists()? {
            ensure!(
                read_json::<Sha256>(&receipt)? == fingerprint,
                "transfer receipt conflict"
            );
        } else {
            write_json(&receipt, &fingerprint)?;
        }
        review.completed_transfer_fingerprint = Some(fingerprint);
        Ok(review)
    }

    /// Recover the exact completed transfer binding after restart. Do not use
    /// an arbitrary latest destination when building a later handoff.
    pub fn completed_transfer(&self, intent: &TransferIntent) -> Result<ListingReview> {
        let mut review = self.check_accepted(&intent.current)?;
        let path = self.generation_path(&intent.current);
        let stored: TransferIntent =
            read_json(&path.join(format!("transfer-{}.json", intent.job_id)))?;
        ensure!(
            stored.current == intent.current
                && stored.destination_commitment == intent.destination_commitment,
            ListingError::StaleGeneration
        );
        review.completed_transfer_fingerprint = Some(read_json(
            &path.join(format!("completed-{}.json", intent.job_id)),
        )?);
        Ok(review)
    }

    pub fn uncertain_transfers(&self, current: &CurrentReview) -> Result<Vec<TransferIntent>> {
        ensure!(
            current.pack_id == self.pack_id,
            ListingError::StaleGeneration
        );
        let path = self.generation_path(current);
        let mut intents = Vec::new();
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            if let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("transfer-"))
                .and_then(|n| n.strip_suffix(".json"))
            {
                let id = Uuid::parse_str(id)?;
                if !path.join(format!("completed-{id}.json")).try_exists()? {
                    let intent: TransferIntent = read_json(&entry.path())?;
                    ensure!(
                        intent.job_id == id && intent.current == *current,
                        "transfer intent binding mismatch"
                    );
                    intents.push(intent);
                }
            }
        }
        Ok(intents)
    }

    fn accepted_review(&self, generation: Generation) -> Result<ListingReview> {
        let mut review = generation.review;
        let accepted: Sha256 = read_json(
            &self
                .generation_path(&generation.current)
                .join("accepted.json"),
        )
        .map_err(|_| ListingError::StaleGeneration)?;
        ensure!(
            accepted == generation.current.review_hash,
            ListingError::StaleGeneration
        );
        review.accepted_review_hash = Some(accepted);
        Ok(review)
    }

    fn generation_path(&self, current: &CurrentReview) -> PathBuf {
        self.path.join(current.generation.to_string())
    }

    fn validate(&self, current: &CurrentReview, require_accepted: bool) -> Result<Generation> {
        self.lease.check(self.policy)?;
        ensure!(
            current.pack_id == self.pack_id
                && self
                    .head()
                    .map_err(|_| ListingError::StaleGeneration)?
                    .map(|(_, h)| h)
                    == Some(current.clone()),
            ListingError::StaleGeneration
        );
        let path = self.generation_path(current);
        ensure!(
            !path.join("invalidated.json").try_exists()?,
            ListingError::StaleGeneration
        );
        let generation: Generation =
            read_json(&path.join("generation.json")).map_err(|_| ListingError::StaleGeneration)?;
        ensure!(
            generation.current == *current
                && digest(&(&generation.binding, &generation.review))? == current.review_hash,
            ListingError::StaleGeneration
        );
        let expected = generation
            .review
            .staged_artifacts
            .iter()
            .map(|f| {
                (
                    f.artifact.clone(),
                    FileHash {
                        size: f.size,
                        sha256: f.sha256.clone(),
                    },
                )
            })
            .collect();
        if generation.binding.check().is_err()
            || tree(&path.join("payload")).ok().as_ref() != Some(&expected)
        {
            // Durable invalidation: reverting changed source/staged bytes must
            // never revive previously accepted work after restart.
            self.invalidate(
                current,
                "source/index/recipe/staged bytes changed or unreadable",
            )?;
            return Err(ListingError::StaleGeneration.into());
        }
        if require_accepted {
            ensure!(
                path.join("accepted.json").try_exists()?,
                ListingError::ReviewRequired
            );
        }
        // Hashing can be long-running. A policy change during verification must
        // cancel the operation before it returns authority to its caller.
        self.lease.check(self.policy)?;
        Ok(generation)
    }

    fn head(&self) -> Result<Option<(u64, CurrentReview)>> {
        let mut latest = None;
        for entry in fs::read_dir(&self.path)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(n) = name
                .strip_prefix("head-")
                .and_then(|s| s.strip_suffix(".json"))
            {
                let sequence: u64 = n.parse().context("invalid review head")?;
                if latest.as_ref().is_none_or(|(old, _)| sequence > *old) {
                    latest = Some((sequence, read_json(&entry.path())?));
                }
            }
        }
        Ok(latest)
    }
}

fn digest(value: &impl Serialize) -> Result<Sha256> {
    let mut value = serde_json::to_value(value)?;
    value.sort_all_objects();
    format!("{:x}", Hasher::digest(serde_json::to_vec(&value)?))
        .try_into()
        .map_err(anyhow::Error::msg)
}

fn check_regular(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(metadata.is_file(), "review input must be a regular file");
    #[cfg(windows)]
    windows::check_node(path, true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.nlink() == 1,
            "hard-linked review input is forbidden"
        );
    }
    Ok(metadata)
}

fn hash_file(path: &Path) -> Result<FileHash> {
    let metadata = check_regular(path)?;
    ensure!(metadata.len() <= MAX_BYTES, "review file byte limit");
    let mut reader = File::open(path)?.take(MAX_BYTES + 1);
    let mut hasher = Hasher::new();
    let size = std::io::copy(&mut reader, &mut hasher)?;
    ensure!(
        size == metadata.len() && size <= MAX_BYTES,
        "review input changed while hashing"
    );
    Ok(FileHash {
        size,
        sha256: format!("{:x}", hasher.finalize())
            .try_into()
            .map_err(anyhow::Error::msg)?,
    })
}

fn plain_directory(path: &Path) -> Result<PathBuf> {
    // Reject links in all ancestors as well as the leaf.
    for ancestor in path.ancestors() {
        if !ancestor.as_os_str().is_empty() {
            #[cfg(windows)]
            windows::check_node(ancestor, false)?;
            ensure!(
                !fs::symlink_metadata(ancestor)?.file_type().is_symlink(),
                "linked review directory"
            );
        }
    }
    ensure!(
        fs::symlink_metadata(path)?.is_dir(),
        "review directory required"
    );
    Ok(fs::canonicalize(path)?)
}

fn tree(root: &Path) -> Result<BTreeMap<String, FileHash>> {
    plain_directory(root)?;
    let mut files = BTreeMap::new();
    let mut bytes = 0_u64;
    for entry in walkdir::WalkDir::new(root).max_depth(64) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            ensure!(entry.depth() < 64, "review directory depth limit");
            continue;
        }
        ensure!(files.len() < MAX_FILES, "review file count limit");
        let hash = hash_file(entry.path())?;
        bytes = bytes
            .checked_add(hash.size)
            .context("review byte overflow")?;
        ensure!(bytes <= MAX_BYTES, "review total byte limit");
        let name = entry
            .path()
            .strip_prefix(root)?
            .to_str()
            .context("review paths must be UTF-8")?
            .replace('\\', "/");
        ensure!(files.insert(name, hash).is_none(), "duplicate review path");
    }
    Ok(files)
}

fn private_directory(path: &Path) -> Result<()> {
    if !path.try_exists()? {
        #[cfg(windows)]
        windows::create_directory(path)?;
        #[cfg(not(windows))]
        {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path)?;
        }
    }
    plain_directory(path)?;
    check_private(path)
}

fn check_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            fs::metadata(path)?.permissions().mode() & 0o077 == 0,
            "review storage must be owner-only"
        );
    }
    #[cfg(windows)]
    windows::check_private(path)?;
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("private review directory ACL enforcement is not implemented on this platform");
    #[allow(unreachable_code)]
    Ok(())
}

fn private_parents(root: &Path, name: &Path) -> Result<()> {
    let mut path = root.to_path_buf();
    if let Some(parent) = name.parent() {
        for component in parent.components() {
            ensure!(
                matches!(component, std::path::Component::Normal(_)),
                "unsafe staged path"
            );
            path.push(component);
            private_directory(&path)?;
        }
    }
    Ok(())
}

fn private_file(path: &Path) -> Result<File> {
    #[cfg(windows)]
    return windows::create_file(path);
    #[cfg(not(windows))]
    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(options.open(path)?)
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    // Unique partial prevents readers from accepting a torn final record.
    let partial = path.with_extension(format!("{}.partial", Uuid::new_v4()));
    let result = (|| {
        let mut file = private_file(&partial)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.sync_all()?;
        // Caller holds the pack mutex. Never overwrite committed evidence.
        ensure!(!path.try_exists()?, "review record already exists");
        fs::rename(&partial, path)?;
        sync_directory(path.parent().context("review parent")?)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    check_private(path)?;
    read_input_json(path)
}

fn read_input_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    ensure!(
        check_regular(path)?.len() <= 64 * 1024 * 1024,
        "review record size limit"
    );
    Ok(serde_json::from_reader(File::open(path)?)?)
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use crate::{
        listing_policy::{ManagedPolicy, PolicySource},
        review::ReviewVersion,
    };
    use std::sync::{Arc, Mutex};

    struct Policy(Mutex<ManagedPolicy>);
    impl PolicySource for Policy {
        fn read(&self) -> ManagedPolicy {
            *self.0.lock().unwrap()
        }
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!("vz-review-{}", Uuid::new_v4()));
            private_directory(&path).unwrap();
            for directory in ["source", "index", "rebuilt"] {
                private_directory(&path.join(directory)).unwrap();
            }
            fs::write(path.join("source/original.txt"), "private original").unwrap();
            fs::write(
                path.join("index/recipe.json"),
                r#"{"model":"fixture","dimensions":4}"#,
            )
            .unwrap();
            fs::write(path.join("index/generation.json"), "index-v1").unwrap();
            fs::write(path.join("rebuilt/document-0001.txt"), "reviewed text").unwrap();
            Self(path)
        }
        fn binding(&self) -> ReviewBinding {
            ReviewBinding::capture(
                &self.0.join("source"),
                &self.0.join("index"),
                &self.0.join("index/recipe.json"),
            )
            .unwrap()
        }
        fn freeze(&self, pack: &PackReview<'_>) -> CurrentReview {
            self.freeze_bound(pack, self.binding())
        }
        fn freeze_bound(&self, pack: &PackReview<'_>, binding: ReviewBinding) -> CurrentReview {
            pack.freeze(
                binding,
                ListingReview {
                    schema_version: ReviewVersion::V1,
                    pack_id: pack.pack_id,
                    staged_artifacts: vec![],
                    metadata_inventory_schema_version: 1,
                    source_revision: String::new(),
                    index_revision: String::new(),
                    embedding_recipe_sha256: digest(&"placeholder").unwrap(),
                    warnings: vec!["Unknown language at document 1".into()],
                    exclusions: vec![],
                    accepted_review_hash: None,
                    completed_transfer_fingerprint: None,
                },
                &self.0.join("rebuilt"),
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn policy() -> (Arc<Policy>, ListingPolicy) {
        let source = Arc::new(Policy(Mutex::new(ManagedPolicy::Allowed)));
        (source.clone(), ListingPolicy::new(source))
    }
    fn stale<T>(result: Result<T>) {
        assert!(matches!(
            result.err().unwrap().downcast_ref::<ListingError>(),
            Some(ListingError::StaleGeneration)
        ));
    }

    #[test]
    fn native_review_fences_live_writer_and_selection_without_recovery() {
        let fixture = Fixture::new();
        let index = fixture.0.join("index");
        fs::write(index.join("writer.lock"), b"").unwrap();
        let (_, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let id = Uuid::new_v4();
        let current;
        {
            let native = store.lock_native(&policy, id, &index).unwrap();
            let binding = native.capture(&fixture.0.join("source")).unwrap();
            current = fixture.freeze_bound(&native.pack, binding);
            native.inspect(&current).unwrap();
            native.accept(&current).unwrap();
            native.check_accepted(&current).unwrap();
            let writer = OpenOptions::new()
                .write(true)
                .open(index.join("writer.lock"))
                .unwrap();
            assert!(fs2::FileExt::try_lock_exclusive(&writer).is_err());
            assert!(vz_core::active_index::selection_fence(&index, true).is_err());
            // Neither fence opens SQLite, creates a DB nor recovers the index.
            assert!(!index.join("state.sqlite").exists());
            // An external pointer mutation is detected despite the advisory fence.
            fs::write(index.join("active-index.json"), b"{}").unwrap();
            stale(native.check_accepted(&current));
            fs::remove_file(index.join("active-index.json")).unwrap();
        }
        let native = store.lock_native(&policy, id, &index).unwrap();
        stale(native.accept(&current));
        drop(native);
        let _exclusive = vz_core::active_index::selection_fence(&index, true).unwrap();
        assert!(store.lock_native(&policy, id, &index).is_err());
    }

    #[test]
    fn native_commands_refuse_generations_without_live_native_binding() {
        let fixture = Fixture::new();
        let index = fixture.0.join("index");
        fs::write(index.join("writer.lock"), b"").unwrap();
        let (_, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let id = Uuid::new_v4();
        let current;
        {
            let pack = store.lock(&policy, id).unwrap();
            current = fixture.freeze(&pack);
            pack.accept(&current).unwrap();
        }
        let native = store.lock_native(&policy, id, &index).unwrap();
        stale(native.check_accepted(&current));
        stale(native.accept(&current));
    }

    #[test]
    fn command_entry_invalidates_changed_selection_even_before_live_fence() {
        let fixture = Fixture::new();
        let index = fixture.0.join("index");
        fs::write(index.join("writer.lock"), b"").unwrap();
        let (_, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let id = Uuid::new_v4();
        let current;
        {
            let native = store.lock_native(&policy, id, &index).unwrap();
            current = fixture.freeze_bound(
                &native.pack,
                native.capture(&fixture.0.join("source")).unwrap(),
            );
            native.accept(&current).unwrap();
            assert_eq!(
                native.inspect(&current).unwrap().accepted_review_hash,
                Some(current.review_hash.clone())
            );
        }
        // Resolve to another existing explicit index with byte-identical data.
        // The old index itself has not changed: only the caller's selection did.
        let other = fixture.0.join("other-index");
        private_directory(&other).unwrap();
        stale(store.lock_native_current(&policy, &current, &other));
        stale(store.lock_native_current(&policy, &current, &index));
    }

    #[test]
    fn native_fence_interoperates_with_real_storage_writer_and_recovery() {
        let fixture = Fixture::new();
        let index = fixture.0.join("index");
        let (_, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let id = Uuid::new_v4();
        let writer = vz_core::storage::State::open(&index).unwrap();
        assert!(store.lock_native(&policy, id, &index).is_err());
        drop(writer);
        let current;
        {
            let native = store.lock_native(&policy, id, &index).unwrap();
            current = fixture.freeze_bound(
                &native.pack,
                native.capture(&fixture.0.join("source")).unwrap(),
            );
            native.accept(&current).unwrap();
            let before = tree(&index).unwrap();
            assert!(vz_core::storage::State::open(&index).is_err());
            assert_eq!(tree(&index).unwrap(), before);
            native.check_accepted(&current).unwrap();
        }
        let writer = vz_core::storage::State::open(&index).unwrap();
        writer
            .db
            .execute("INSERT INTO jobs VALUES (1, 'neutral', 'ready', NULL)", [])
            .unwrap();
        drop(writer);
        stale(store.lock_native_current(&policy, &current, &index));
    }

    #[test]
    fn generation_roundtrip_and_explicit_acceptance_survive_restart() {
        let fixture = Fixture::new();
        let (_, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let id = Uuid::new_v4();
        let current;
        let original_record;
        {
            let pack = store.lock(&policy, id).unwrap();
            current = fixture.freeze(&pack);
            original_record =
                fs::read(pack.generation_path(&current).join("generation.json")).unwrap();
            assert_eq!(
                pack.check_accepted(&current)
                    .unwrap_err()
                    .downcast_ref::<ListingError>(),
                Some(&ListingError::ReviewRequired)
            );
            let accepted = pack.accept(&current).unwrap();
            assert_eq!(
                accepted.accepted_review_hash,
                Some(current.review_hash.clone())
            );
            assert_eq!(accepted.staged_artifacts[0].artifact, "document-0001.txt");
            assert_eq!(accepted.staged_artifacts[0].size, 13);
            assert_eq!(
                accepted.embedding_recipe_sha256,
                fixture.binding().recipe_sha256
            );
            assert!(!accepted.source_revision.is_empty());
        }
        drop(store);
        let restarted = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let pack = restarted.lock(&policy, id).unwrap();
        pack.check_accepted(&current).unwrap();
        pack.accept(&current).unwrap();
        assert_eq!(
            fs::read(pack.generation_path(&current).join("generation.json")).unwrap(),
            original_record
        );
        assert_eq!(
            fs::read_to_string(fixture.0.join("source/original.txt")).unwrap(),
            "private original"
        );
    }

    #[test]
    fn recipe_commitment_matches_native_canonical_embedding_recipe() {
        let fixture = Fixture::new();
        let pins =
            serde_json::from_str(include_str!("../../../spike/fixtures/models.json")).unwrap();
        let recipe = vz_core::engine::Recipe::from_pins(&pins, 384).unwrap();
        fs::write(
            fixture.0.join("index/recipe.json"),
            serde_json::to_vec_pretty(&recipe).unwrap(),
        )
        .unwrap();
        let expected: String = fixture.binding().recipe_sha256.into();
        assert_eq!(expected, crate::embedding_recipe_sha256(&recipe).unwrap());
    }

    #[test]
    fn source_index_recipe_and_staged_changes_permanently_invalidate() {
        for relative in [
            "source/original.txt",
            "source/added.txt",
            "index/generation.json",
            "index/recipe.json",
            "staged",
        ] {
            let fixture = Fixture::new();
            let (_, policy) = policy();
            let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
            let id = Uuid::new_v4();
            let pack = store.lock(&policy, id).unwrap();
            let current = fixture.freeze(&pack);
            pack.accept(&current).unwrap();
            let changed = if relative == "staged" {
                pack.generation_path(&current)
                    .join("payload/document-0001.txt")
            } else {
                fixture.0.join(relative)
            };
            let original = fs::read(&changed).ok();
            fs::write(&changed, "changed").unwrap();
            stale(pack.check_accepted(&current));
            match original {
                Some(bytes) => fs::write(&changed, bytes).unwrap(),
                None => fs::remove_file(&changed).unwrap(),
            }
            drop(pack);
            let pack = store.lock(&policy, id).unwrap();
            stale(pack.accept(&current));
            let replacement = fixture.freeze(&pack);
            assert_ne!(current.generation, replacement.generation);
            stale(pack.check_accepted(&current));
            pack.accept(&replacement).unwrap();
        }
    }

    #[test]
    fn rebuilding_never_inherits_acceptance_or_completed_transfer() {
        let fixture = Fixture::new();
        let (_, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let pack = store.lock(&policy, Uuid::new_v4()).unwrap();
        let first = fixture.freeze(&pack);
        pack.accept(&first).unwrap();
        let intent = pack
            .record_transfer_intent(&first, digest(&"neutral-destination").unwrap())
            .unwrap();
        assert_eq!(pack.uncertain_transfers(&first).unwrap().len(), 1);
        let fingerprint = digest(&"actual destination inventory").unwrap();
        let completed = pack
            .record_completed_transfer(&intent, fingerprint.clone())
            .unwrap();
        assert_eq!(
            completed.completed_transfer_fingerprint,
            Some(fingerprint.clone())
        );
        assert_eq!(
            pack.completed_transfer(&intent)
                .unwrap()
                .completed_transfer_fingerprint,
            Some(fingerprint.clone())
        );
        pack.record_completed_transfer(&intent, fingerprint)
            .unwrap();
        assert!(pack
            .record_completed_transfer(&intent, digest(&"conflict").unwrap())
            .is_err());
        assert!(pack.uncertain_transfers(&first).unwrap().is_empty());
        let id = pack.pack_id;
        drop(pack);
        let pack = store.lock(&policy, id).unwrap();
        assert!(pack
            .completed_transfer(&intent)
            .unwrap()
            .completed_transfer_fingerprint
            .is_some());
        let second = fixture.freeze(&pack);
        stale(pack.check_accepted(&first));
        let accepted = pack.accept(&second).unwrap();
        assert!(accepted.completed_transfer_fingerprint.is_none());
    }

    #[test]
    fn uncertain_promotion_evidence_survives_policy_stop_and_restart() {
        let fixture = Fixture::new();
        let (source, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let id = Uuid::new_v4();
        let pack = store.lock(&policy, id).unwrap();
        let current = fixture.freeze(&pack);
        pack.accept(&current).unwrap();
        let intent = pack
            .record_transfer_intent(&current, digest(&"destination").unwrap())
            .unwrap();
        *source.0.lock().unwrap() = ManagedPolicy::Denied;
        policy.refresh();
        stale(pack.record_completed_transfer(&intent, digest(&"promoted?").unwrap()));
        // Evidence reads deliberately remain possible after a policy stop.
        assert_eq!(
            pack.uncertain_transfers(&current).unwrap()[0].job_id,
            intent.job_id
        );
        *source.0.lock().unwrap() = ManagedPolicy::Allowed;
        policy.refresh();
        stale(pack.check_accepted(&current));
        drop(pack);
        let pack = store.lock(&policy, id).unwrap();
        assert_eq!(pack.uncertain_transfers(&current).unwrap().len(), 1);
    }

    #[test]
    fn forged_current_review_and_new_generation_refuse_old_acceptance() {
        let fixture = Fixture::new();
        let (_, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let pack = store.lock(&policy, Uuid::new_v4()).unwrap();
        let current = fixture.freeze(&pack);
        let mut forged = current.clone();
        forged.review_hash = digest(&"forged").unwrap();
        stale(pack.accept(&forged));
        forged = current.clone();
        forged.pack_id = Uuid::new_v4();
        stale(pack.accept(&forged));
        pack.accept(&current).unwrap();
        fixture.freeze(&pack);
        stale(pack.check_accepted(&current));
    }

    #[test]
    #[cfg(unix)]
    fn private_storage_rejects_overlap_links_and_shared_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let fixture = Fixture::new();
        let (_, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("source/reviews")).unwrap();
        let pack = store.lock(&policy, Uuid::new_v4()).unwrap();
        let binding = fixture.binding();
        let review = ListingReview {
            schema_version: ReviewVersion::V1,
            pack_id: pack.pack_id,
            staged_artifacts: vec![],
            metadata_inventory_schema_version: 1,
            source_revision: String::new(),
            index_revision: String::new(),
            embedding_recipe_sha256: digest(&"recipe").unwrap(),
            warnings: vec![],
            exclusions: vec![],
            accepted_review_hash: None,
            completed_transfer_fingerprint: None,
        };
        assert!(pack
            .freeze(binding, review, &fixture.0.join("rebuilt"))
            .is_err());
        symlink(fixture.0.join("source"), fixture.0.join("linked")).unwrap();
        assert!(ReviewStore::open(&fixture.0.join("linked")).is_err());
        symlink(
            fixture.0.join("source/original.txt"),
            fixture.0.join("rebuilt/link.txt"),
        )
        .unwrap();
        assert!(tree(&fixture.0.join("rebuilt")).is_err());
        fs::set_permissions(
            fixture.0.join("source/reviews"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert!(ReviewStore::open(&fixture.0.join("source/reviews")).is_err());
    }

    #[test]
    fn oversized_inputs_and_excessive_depth_block_instead_of_truncating_inventory() {
        let fixture = Fixture::new();
        let large = fixture.0.join("rebuilt/large.bin");
        File::create(&large)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(tree(&fixture.0.join("rebuilt")).is_err());
        fs::remove_file(large).unwrap();
        let mut path = fixture.0.join("rebuilt");
        for _ in 0..64 {
            path.push("nested");
            private_directory(&path).unwrap();
        }
        fs::write(path.join("hidden.txt"), "must not be silently skipped").unwrap();
        assert!(tree(&fixture.0.join("rebuilt")).is_err());
    }

    #[test]
    fn corrupt_committed_records_and_torn_heads_fail_closed() {
        let fixture = Fixture::new();
        let (_, policy) = policy();
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let pack = store.lock(&policy, Uuid::new_v4()).unwrap();
        let current = fixture.freeze(&pack);
        pack.accept(&current).unwrap();
        let record = pack.generation_path(&current).join("generation.json");
        let original = fs::read(&record).unwrap();
        fs::write(&record, "{").unwrap();
        stale(pack.check_accepted(&current));
        fs::write(&record, original).unwrap();
        // A partial head from a crash is invisible; a damaged committed head
        // refuses all work instead of selecting an older accepted generation.
        fs::write(pack.path.join("head-00000000000000000001.partial"), "{").unwrap();
        pack.check_accepted(&current).unwrap();
        fs::write(pack.path.join("head-00000000000000000001.json"), "{").unwrap();
        stale(pack.check_accepted(&current));
    }

    #[test]
    fn per_pack_mutex_serializes_independent_store_instances_but_not_policy_updates() {
        let fixture = Fixture::new();
        let (source, policy) = policy();
        let policy = Arc::new(policy);
        let store = ReviewStore::open(&fixture.0.join("reviews")).unwrap();
        let id = Uuid::new_v4();
        let pack = store.lock(&policy, id).unwrap();
        let path = store.root.clone();
        let other_policy = policy.clone();
        let (started, received) = std::sync::mpsc::channel();
        let (locked, acquired) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let store = ReviewStore::open(&path).unwrap();
            started.send(()).unwrap();
            let result = store.lock(&other_policy, id);
            locked.send(result.is_ok()).unwrap();
        });
        received.recv().unwrap();
        assert!(acquired
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        *source.0.lock().unwrap() = ManagedPolicy::Denied;
        policy.refresh();
        // Cancellation cannot depend on the holder releasing its mutex.
        assert!(!acquired
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap());
        drop(pack);
        worker.join().unwrap();
    }
}
