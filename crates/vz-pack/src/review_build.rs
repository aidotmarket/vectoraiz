//! Native-only private rebuild. No source/index writer or delivered mapping.
use crate::{
    review::{ListingReview, ReviewVersion},
    review_store::{private_directory, private_file, CurrentReview, NativePackReview, ReviewStore},
    sanitize::{self, Budget, Changes},
};
use anyhow::{ensure, Context, Result};
use hmac::{Hmac, Mac};
use parquet::arrow::ArrowWriter;
use serde_json::json;
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
    path::Path,
};
use uuid::Uuid;
use vz_core::{
    engine::Engines,
    pipeline::{chunk, sha256, Page},
    storage::{chunks_batch, documents_batch, State},
};

#[derive(serde::Serialize)]
pub struct Candidate {
    pub source: String,
    pub artifact: String,
    pub format: String,
    pub supported: bool,
}

pub fn candidates(library: &Path, source: &Path) -> Result<Vec<Candidate>> {
    let policy = crate::listing_policy::system();
    policy.require_preview()?;
    let lease = policy.policy_lease()?;
    let source = source.canonicalize()?;
    let candidates = crate::review_store::NativeIndexFence::candidate_documents(library, &source)?
        .into_iter()
        .enumerate()
        .map(|(position, doc)| {
            Ok(Candidate {
                source: Path::new(&doc.path)
                    .strip_prefix(&source)?
                    .to_str()
                    .context("source path must be UTF-8")?
                    .replace('\\', "/"),
                artifact: format!(
                    "originals-0001.zip!document-{:04}.{}",
                    position + 1,
                    doc.kind
                ),
                supported: ["txt", "md", "markdown", "docx", "xlsx", "pptx"]
                    .contains(&doc.kind.as_str()),
                format: doc.kind,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    lease.check(policy)?;
    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
pub fn rebuild(
    store: &Path,
    library: &Path,
    source: &Path,
    pack_id: Uuid,
    engines: &Engines,
    query: &str,
    changes: &Changes,
) -> Result<CurrentReview> {
    let policy = crate::listing_policy::system();
    policy.require_preview()?;
    let store = ReviewStore::open(store)?;
    let pack = store.lock_native(policy, pack_id, library)?;
    build(&pack, source, pack_id, engines, query, changes)
}

fn put(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = Path::new(name)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        private_directory(&root.join(parent))?;
    }
    let mut file = private_file(&root.join(name))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn build(
    pack: &NativePackReview<'_>,
    source: &Path,
    pack_id: Uuid,
    engines: &impl BuildEngine,
    query: &str,
    changes: &Changes,
) -> Result<CurrentReview> {
    let origin = changes
        .origin
        .as_ref()
        .map(|current| pack.edit_origin(current))
        .transpose();
    pack.begin_rebuild()?;
    changes.validate()?;
    let origin = origin?;
    let binding = pack.capture(source)?;
    let index = pack.index_path();
    let recipe: vz_core::engine::Recipe =
        serde_json::from_slice(&std::fs::read(index.join("recipe.json"))?)?;
    ensure!(
        recipe == *engines.recipe(),
        "embedding recipe mismatch; re-index required"
    );
    let indexed = State::review_documents(index)?;
    let identity_key = pack.identity_key()?;
    let work = pack.work()?;
    let result = (|| {
        let mut docs = Vec::new();
        let mut originals = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let mut mapping = BTreeMap::new();
        let mut document_ids = BTreeMap::new();
        let mut office_document_ids = BTreeMap::new();
        let mut budget = Budget::default();
        let has_office = indexed
            .iter()
            .any(|d| ["docx", "xlsx", "pptx"].contains(&d.kind.as_str()));
        if has_office {
            budget.enable_office();
            budget.cancel = Some(pack.cancellation_token());
        }
        for (key, value) in &changes.edits {
            if key.contains(".docx!") || key.contains(".xlsx!") || key.contains(".pptx!") {
                let (review, private) = origin
                    .as_ref()
                    .context("Office edits require the originating review generation/hash")?;
                let previous = review
                    .metadata_inventory
                    .iter()
                    .find(|entry| &entry.location == key)
                    .context("Office edit selector absent from originating generation")?;
                let old = previous
                    .value
                    .as_ref()
                    .context("Office edit target has no prior value")?;
                ensure!(
                    private["office_nodes"][key]["prior_sha256"].as_str()
                        == Some(sha256(old.as_bytes()).as_str()),
                    "Office stable node binding absent or mismatched"
                );
                budget.edit_bindings.insert(
                    key.clone(),
                    vz_core::office::Edit {
                        expected_sha256: sha256(old.as_bytes()),
                        value: value.clone(),
                    },
                );
            }
        }
        // Refuse edits which a handler cannot apply; never accept a cosmetic
        // preview change while silently delivering the previous value.
        for key in changes.edits.keys() {
            let allowed = key == "README.md"
                || key == "examples/query.py"
                || key == "examples/query.json/query"
                || (key.starts_with("originals-0001.zip!document-") && !key.contains('/'))
                || ((key.starts_with("documents.parquet/column/")
                    || key.starts_with("chunks.parquet/column/"))
                    && key.ends_with("/name"))
                || (key.starts_with("documents.parquet/id/") && key.ends_with("/column/2"))
                || (key.starts_with("chunks.parquet/id/") && key.ends_with("/column/4"));
            let allowed = allowed || budget.edit_bindings.contains_key(key);
            ensure!(allowed, "edit location has no proven rebuild handler");
        }
        let source = source.canonicalize()?;
        for (position, mut doc) in indexed.into_iter().enumerate() {
            pack.check_policy()?;
            let relative = Path::new(&doc.path)
                .strip_prefix(&source)
                .context("indexed document is outside reviewed source")?
                .to_str()
                .context("source path must be UTF-8")?
                .replace('\\', "/");
            if changes.exclusions.contains(&relative) {
                continue;
            }
            // Only originals with a staged rewrite and readback handler qualify.
            // PDF and HTML still require explicit seller exclusion.
            ensure!(
                ["txt", "md", "markdown", "docx", "xlsx", "pptx"].contains(&doc.kind.as_str()),
                "original format has no proven rewrite handler; exclude or re-ingest as text"
            );
            let name = format!("document-{:04}.{}", position + 1, doc.kind);
            let location = format!("originals-0001.zip!{name}");
            if changes.exclusions.contains(&location) {
                continue;
            }
            let mut bytes = Vec::new();
            std::fs::File::open(&doc.path)?
                .take(if has_office {
                    512 * 1024 * 1024 + 1
                } else {
                    64 * 1024 * 1024 + 1
                })
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len()
                    <= if has_office {
                        512 * 1024 * 1024
                    } else {
                        64 * 1024 * 1024
                    }
                    && sha256(&bytes) == doc.sha256,
                "source changed since indexing"
            );
            let office = ["docx", "xlsx", "pptx"].contains(&doc.kind.as_str());
            // A stable per-pack pseudonym preserves identity-bound edits without
            // exposing a path digest or linking independent packs to one index.
            let mut identity = Hmac::<Sha256>::new_from_slice(&identity_key)?;
            identity.update(doc.doc_id.as_bytes());
            let public_id = format!("{:x}", identity.finalize().into_bytes());
            if office {
                office_document_ids.insert(location.clone(), public_id.clone());
            }
            let derived_title = format!("documents.parquet/id/{public_id}/column/2");
            if office
                && (changes.edits.contains_key(&derived_title)
                    || changes
                        .edits
                        .keys()
                        .any(|key| key.starts_with(&format!("chunks.parquet/id/{public_id}-"))))
            {
                let (review, private) = origin
                    .as_ref()
                    .context("derived Office edits require the originating generation/hash")?;
                ensure!(
                    private["document_ids"][&doc.doc_id].as_str() == Some(public_id.as_str()),
                    "derived Office edit document identity mismatch"
                );
                for key in changes.edits.keys().filter(|key| {
                    **key == derived_title
                        || key.starts_with(&format!("chunks.parquet/id/{public_id}-"))
                }) {
                    ensure!(
                        review.metadata_inventory.iter().any(|e| &e.location == key),
                        "derived Office edit absent from originating generation"
                    );
                }
            }
            if office
                && budget
                    .edit_bindings
                    .keys()
                    .any(|key| key.starts_with(&format!("{location}!")))
            {
                let (_, private) = origin.as_ref().context("Office edit origin missing")?;
                ensure!(
                    private["document_ids"][&doc.doc_id].as_str() == Some(public_id.as_str())
                        && private["mapping"][&relative].as_str() == Some(location.as_str()),
                    "Office document identity mismatch"
                );
                for key in budget
                    .edit_bindings
                    .keys()
                    .filter(|key| key.starts_with(&format!("{location}!")))
                {
                    ensure!(
                        private["office_nodes"][key]["document_id"].as_str()
                            == Some(public_id.as_str()),
                        "Office stable node document mismatch"
                    );
                    let selector = &key[location.len() + 1..];
                    let mut identity = Hmac::<Sha256>::new_from_slice(&identity_key)?;
                    identity.update(b"office-node-v1\0");
                    identity.update(public_id.as_bytes());
                    identity.update(selector.as_bytes());
                    let node_id = format!("{:x}", identity.finalize().into_bytes());
                    ensure!(
                        private["office_nodes"][key]["selector"].as_str() == Some(selector)
                            && private["office_nodes"][key]["node_id"].as_str()
                                == Some(node_id.as_str()),
                        "Office stable node identity mismatch"
                    );
                }
            }
            if office
                && origin.as_ref().is_some_and(|(_, private)| {
                    private["mapping"][&relative].as_str() == Some(location.as_str())
                })
            {
                bytes = pack.origin_original(
                    changes
                        .origin
                        .as_ref()
                        .context("Office edit origin missing")?,
                    &name,
                )?;
                budget.from_origin.insert(location.clone());
                if let Some((_, private)) = &origin {
                    if let Some(records) = private.get("office_removals") {
                        budget.removals.extend(serde_json::from_value::<
                            Vec<vz_core::office::Removal>,
                        >(records.clone())?);
                    }
                }
            }
            let bytes = sanitize::rewrite(&location, &bytes, changes, &mut mapping, &mut budget)?;
            mapping.insert(relative.clone(), location.clone());
            let text = if office {
                let (_, report) = sanitize::office_worker(&location, &bytes, &mut budget, true)?;
                report.text()
            } else {
                std::str::from_utf8(&bytes)?.to_owned()
            };
            document_ids.insert(doc.doc_id.clone(), public_id.clone());
            doc.doc_id = public_id;
            doc.chunks = chunk(
                &doc.doc_id,
                &[Page {
                    page: 1,
                    text,
                    needs_ocr: false,
                }],
            );
            if office {
                for chunk in &doc.chunks {
                    let heading = format!("chunks.parquet/id/{}/column/4", chunk.chunk_id);
                    if changes.edits.contains_key(&heading) {
                        let (review, _) = origin
                            .as_ref()
                            .context("Office heading edit lacks generation binding")?;
                        let text = format!("chunks.parquet/id/{}/column/2", chunk.chunk_id);
                        ensure!(review.metadata_inventory.iter().any(|entry|entry.location==text&&entry.value.as_deref()==Some(chunk.text.as_str())),"Office chunk identity changed after rechunking; review again before editing headings");
                    }
                }
            }
            ensure!(
                !doc.chunks.is_empty(),
                "edited document has no text; exclude it instead"
            );
            for chunk in &mut doc.chunks {
                pack.check_policy()?;
                chunk.embedding = engines.vector(&chunk.text, false)?;
            }
            doc.sha256 = sha256(&bytes);
            doc.bytes = bytes.len() as u64;
            doc.pages = 1;
            doc.path = name.clone();
            originals.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )?;
            originals.write_all(&bytes)?;
            docs.push(doc);
        }
        ensure!(!docs.is_empty(), "no supported reviewed documents remain");
        put(
            &work,
            "originals-0001.zip",
            &originals.finish()?.into_inner(),
        )?;
        let chunks = docs
            .iter()
            .flat_map(|d| d.chunks.clone())
            .collect::<Vec<_>>();
        let documents = documents_batch(&docs)?;
        let mut columns = documents.columns().to_vec();
        columns[1] = std::sync::Arc::new(arrow_array::StringArray::from_iter_values(
            docs.iter().map(|doc| &doc.path),
        ));
        let documents = arrow_array::RecordBatch::try_new(documents.schema(), columns)?;
        for (name, batch) in [
            ("documents.parquet", documents),
            ("chunks.parquet", chunks_batch(&chunks)?),
        ] {
            let mut writer = ArrowWriter::try_new(Vec::new(), batch.schema(), None)?;
            writer.write(&batch)?;
            let bytes = sanitize::rewrite(
                name,
                &writer.into_inner()?,
                changes,
                &mut mapping,
                &mut budget,
            )?;
            put(&work, name, &bytes)?;
        }
        for (name, bytes) in [("README.md", b"# vectorAIz vector pack\n\nLoad the Parquet files with Apache Arrow. Original names and file properties are absent. Language and date ranges are unknown.\nRun examples/query.py with Python, numpy and pyarrow.\n".as_slice()), ("examples/query.py", include_bytes!("../../../scripts/pack-query.py").as_slice())] {
            let rebuilt = sanitize::rewrite(name, bytes, changes, &mut mapping, &mut budget)?;
            put(&work, name, &rebuilt)?;
        }
        let query = changes
            .edits
            .get("examples/query.json/query")
            .map_or(query, String::as_str);
        let vector = engines.vector(query, true)?;
        let mut scores = chunks
            .iter()
            .map(|c| {
                (
                    c.chunk_id.clone(),
                    c.embedding
                        .iter()
                        .zip(&vector)
                        .map(|(a, b)| a * b)
                        .sum::<f32>(),
                )
            })
            .collect::<Vec<_>>();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut example = json!({"query":query,"query_embedding":vector,"expected_top":scores.iter().take(3).map(|v| &v.0).collect::<Vec<_>>(),"method":"cosine on L2-normalized float32 embeddings; ties by chunk_id","embedding_recipe_sha256":crate::embedding_recipe_sha256(&recipe)?});
        example.sort_all_objects();
        put(
            &work,
            "examples/query.json",
            &serde_json::to_vec_pretty(&example)?,
        )?;
        let mut hashes = BTreeMap::new();
        for entry in walkdir::WalkDir::new(&work) {
            let entry = entry?;
            if entry.file_type().is_file() {
                hashes.insert(
                    entry
                        .path()
                        .strip_prefix(&work)?
                        .to_str()
                        .context("UTF-8 staged path")?
                        .replace('\\', "/"),
                    sha256(&std::fs::read(entry.path())?),
                );
            }
        }
        let mut manifest = json!({"format_version":1,"pack_id":pack_id,"created_at":chrono::Utc::now().to_rfc3339(),"counts":{"documents":docs.len(),"chunks":chunks.len()},"languages":null,"date_ranges":null,"embedding_recipe":recipe,"files":hashes,"hash_scope":"all payload files; manifest.json excludes itself","originals":true});
        manifest.sort_all_objects();
        put(
            &work,
            "manifest.json",
            &serde_json::to_vec_pretty(&manifest)?,
        )?;
        crate::verify(&work)?;
        let mut inventory = Vec::new();
        let mut read_budget = Budget::default();
        if has_office {
            read_budget.enable_office();
            read_budget.cancel = Some(pack.cancellation_token());
            read_budget.started = budget.started;
            read_budget.worker_time = budget.worker_time.clone();
        }
        for entry in walkdir::WalkDir::new(&work).sort_by_file_name() {
            let entry = entry?;
            if entry.file_type().is_file() {
                pack.check_policy()?;
                let name = entry
                    .path()
                    .strip_prefix(&work)?
                    .to_str()
                    .context("UTF-8 readback path")?
                    .replace('\\', "/");
                sanitize::inspect(
                    &name,
                    &std::fs::read(entry.path())?,
                    changes,
                    &mut inventory,
                    &mut read_budget,
                )?;
            }
        }
        for (location, edit) in &changes.edits {
            ensure!(
                inventory
                    .iter()
                    .any(|entry| &entry.location == location && entry.value.as_ref() == Some(edit)),
                "edit did not reach delivered readback"
            );
        }
        let mut warnings: Vec<String> = inventory
            .iter()
            .filter(|e| e.warning)
            .map(|e| format!("{}: {}", e.location, e.kind))
            .collect();
        warnings.extend(
            budget
                .removals
                .iter()
                .map(|r| format!("{}: {}", r.location, r.reason)),
        );
        warnings.extend(budget.office_warnings.iter().cloned());
        let mut office_nodes = BTreeMap::new();
        for entry in &inventory {
            if let Some((document, doc_id)) = office_document_ids
                .iter()
                .find(|(location, _)| entry.location.starts_with(&format!("{location}!")))
            {
                if let Some(value) = &entry.value {
                    let node = &entry.location[document.len() + 1..];
                    let mut identity = Hmac::<Sha256>::new_from_slice(&identity_key)?;
                    identity.update(b"office-node-v1\0");
                    identity.update(doc_id.as_bytes());
                    identity.update(node.as_bytes());
                    office_nodes.insert(entry.location.clone(),json!({"document_id":doc_id,"node_id":format!("{:x}",identity.finalize().into_bytes()),"selector":node,"prior_sha256":sha256(value.as_bytes())}));
                }
            }
        }
        let review = ListingReview {
            schema_version: ReviewVersion::V1,
            pack_id,
            staged_artifacts: Vec::new(),
            metadata_inventory_schema_version: 1,
            metadata_inventory: inventory,
            source_revision: String::new(),
            index_revision: String::new(),
            embedding_recipe_sha256: "0".repeat(64).try_into().map_err(anyhow::Error::msg)?,
            warnings,
            exclusions: changes.exclusions.iter().cloned().collect(),
            accepted_review_hash: None,
            completed_transfer_fingerprint: None,
        };
        pack.freeze_sanitized(
            binding,
            review,
            &work,
            json!({"mapping":mapping,"document_ids":document_ids,"identity_terms":changes.identity_terms,"office_removals":budget.removals,"office_origin":changes.origin,"office_nodes":office_nodes,"office_allowlist_version":vz_core::office::ALLOWLIST_VERSION}),
        )
    })();
    // The UUID-owned private scratch is never a seller destination.
    let cleanup = std::fs::remove_dir_all(&work);
    match result {
        Ok(current) => {
            cleanup?;
            Ok(current)
        }
        Err(error) => Err(error),
    }
}

trait BuildEngine {
    fn recipe(&self) -> &vz_core::engine::Recipe;
    fn vector(&self, text: &str, query: bool) -> Result<Vec<f32>>;
}
impl BuildEngine for Engines {
    fn recipe(&self) -> &vz_core::engine::Recipe {
        &self.recipe
    }
    fn vector(&self, text: &str, query: bool) -> Result<Vec<f32>> {
        self.vector(text, query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing_policy::{ListingPolicy, ManagedPolicy, PolicySource};
    use std::sync::{Arc, Mutex};
    struct Source(Mutex<ManagedPolicy>);
    impl PolicySource for Source {
        fn read(&self) -> ManagedPolicy {
            *self.0.lock().unwrap()
        }
    }
    struct FakeEngine {
        recipe: vz_core::engine::Recipe,
        texts: Mutex<Vec<String>>,
    }
    impl BuildEngine for FakeEngine {
        fn recipe(&self) -> &vz_core::engine::Recipe {
            &self.recipe
        }
        fn vector(&self, text: &str, _: bool) -> Result<Vec<f32>> {
            self.texts.lock().unwrap().push(text.into());
            Ok(if text.contains("Reviewed") {
                vec![0., 1., 0.]
            } else {
                vec![1., 0., 0.]
            })
        }
    }
    struct Fixture {
        _temp: tempfile::TempDir,
        root: std::path::PathBuf,
        engine: FakeEngine,
        policy: ListingPolicy,
        policy_source: Arc<Source>,
    }
    impl Fixture {
        fn new() -> Self {
            Self::with_path_ids(false)
        }
        fn with_path_ids(path_ids: bool) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let pins =
                serde_json::from_str(include_str!("../../../spike/fixtures/models.json")).unwrap();
            let recipe = vz_core::engine::Recipe::from_pins(&pins, 3).unwrap();
            let source = root.join("source");
            let index = root.join("index");
            std::fs::create_dir(&source).unwrap();
            std::fs::create_dir(&index).unwrap();
            std::fs::write(
                index.join("recipe.json"),
                serde_json::to_vec(&recipe).unwrap(),
            )
            .unwrap();
            let mut state = State::open(&index).unwrap();
            for (id, name, bytes, kind) in [
                (
                    "a",
                    "client-original.md",
                    b"# Project ORION\nConfidential fixture-identity client notes.\n".as_slice(),
                    "md",
                ),
                (
                    "b",
                    "unsupported.pdf",
                    b"%PDF-unsupported".as_slice(),
                    "pdf",
                ),
            ] {
                let path = source.join(name);
                std::fs::write(&path, bytes).unwrap();
                let doc = vz_core::pipeline::Document {
                    doc_id: if path_ids {
                        sha256(path.to_str().unwrap().as_bytes())
                    } else {
                        id.into()
                    },
                    path: path.to_str().unwrap().into(),
                    sha256: sha256(bytes),
                    bytes: bytes.len() as u64,
                    pages: 1,
                    kind: kind.into(),
                    needs_ocr_pages: vec![],
                    chunks: vec![],
                };
                let generation = Uuid::new_v4().to_string();
                let directory = index.join("generations").join(&generation);
                std::fs::create_dir(&directory).unwrap();
                std::fs::write(
                    directory.join("document.json"),
                    serde_json::to_vec(&doc).unwrap(),
                )
                .unwrap();
                state.commit(&doc, "fixture", &generation).unwrap();
            }
            drop(state);
            let policy_source = Arc::new(Source(Mutex::new(ManagedPolicy::Allowed)));
            Self {
                _temp: temp,
                root,
                engine: FakeEngine {
                    recipe,
                    texts: Mutex::new(vec![]),
                },
                policy: ListingPolicy::new(policy_source.clone()),
                policy_source,
            }
        }
        fn changes() -> Changes {
            Changes {
                exclusions: ["unsupported.pdf".into()].into(),
                identity_terms: vec!["fixture-identity".into()],
                ..Changes::default()
            }
        }
    }

    #[test]
    fn path_ids_are_private_and_independent_packs_cannot_be_linked_by_id() {
        let f = Fixture::with_path_ids(true);
        let source_id = sha256(
            f.root
                .join("source/client-original.md")
                .to_str()
                .unwrap()
                .as_bytes(),
        );
        let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let mut public_ids = Vec::new();
        for _ in 0..2 {
            let id = Uuid::new_v4();
            let guard = store
                .lock_native(&f.policy, id, &f.root.join("index"))
                .unwrap();
            let current = build(
                &guard,
                &f.root.join("source"),
                id,
                &f.engine,
                "query",
                &Fixture::changes(),
            )
            .unwrap();
            let review = guard.inspect(&current).unwrap();
            assert!(review.metadata_inventory.iter().all(|entry| {
                !entry.location.contains(&source_id)
                    && !entry
                        .value
                        .as_deref()
                        .unwrap_or_default()
                        .contains(&source_id)
            }));
            guard.accept(&current).unwrap();
            let (payload, _) = guard.reviewed_payload(&current).unwrap();
            for entry in walkdir::WalkDir::new(&payload) {
                let entry = entry.unwrap();
                if entry.file_type().is_file() {
                    let bytes = std::fs::read(entry.path()).unwrap();
                    assert!(!bytes
                        .windows(source_id.len())
                        .any(|w| w == source_id.as_bytes()));
                }
            }
            assert!(!payload.join("identity-key.json").exists());
            let private: serde_json::Value = serde_json::from_slice(
                &std::fs::read(payload.parent().unwrap().join("private.json")).unwrap(),
            )
            .unwrap();
            let public_id = private["document_ids"][&source_id].as_str().unwrap();
            assert_ne!(public_id, source_id);
            assert!(review
                .metadata_inventory
                .iter()
                .any(|e| e.value.as_deref() == Some(public_id)));
            public_ids.push(public_id.to_owned());
        }
        assert_ne!(public_ids[0], public_ids[1]);
    }

    #[test]
    fn title_edit_follows_document_after_exclusion_and_heading_retarget_refuses() {
        let f = Fixture::new();
        let mut state = State::open(&f.root.join("index")).unwrap();
        let bytes = b"# Second heading\nSecond document text.\n";
        let path = f.root.join("source/second.md");
        std::fs::write(&path, bytes).unwrap();
        let doc = vz_core::pipeline::Document {
            doc_id: "c".into(),
            path: path.to_str().unwrap().into(),
            sha256: sha256(bytes),
            bytes: bytes.len() as u64,
            pages: 1,
            kind: "md".into(),
            needs_ocr_pages: vec![],
            chunks: vec![],
        };
        let generation = Uuid::new_v4().to_string();
        let directory = f.root.join("index/generations").join(&generation);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("document.json"),
            serde_json::to_vec(&doc).unwrap(),
        )
        .unwrap();
        state.commit(&doc, "fixture", &generation).unwrap();
        drop(state);
        let id = Uuid::new_v4();
        let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let guard = store
            .lock_native(&f.policy, id, &f.root.join("index"))
            .unwrap();
        let mut changes = Fixture::changes();
        let first = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap();
        let inventory = guard.inspect(&first).unwrap().metadata_inventory;
        let title = inventory
            .iter()
            .find(|e| e.kind == "title" && e.value.as_deref() == Some("Document 2"))
            .unwrap()
            .location
            .clone();
        let heading = inventory
            .iter()
            .rfind(|e| e.kind == "heading_path")
            .unwrap()
            .location
            .clone();
        changes
            .edits
            .insert(title.clone(), "Reviewed second title".into());
        changes.exclusions.insert("client-original.md".into());
        let next = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap();
        assert!(guard
            .inspect(&next)
            .unwrap()
            .metadata_inventory
            .iter()
            .any(|e| e.location == title && e.value.as_deref() == Some("Reviewed second title")));
        changes
            .edits
            .insert(heading, r#"["Reviewed heading"]"#.into());
        let heading_review = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap();
        assert!(guard
            .inspect(&heading_review)
            .unwrap()
            .metadata_inventory
            .iter()
            .any(|e| e.kind == "heading_path"
                && e.value.as_deref() == Some(r#"["Reviewed heading"]"#)));
        // Replacing the original causes rechunking; the retained heading edit
        // must not apply to a different chunk at the same ordinal.
        changes.edits.insert(
            "originals-0001.zip!document-0003.md".into(),
            "# Replacement heading\nCompletely different content.".into(),
        );
        let error = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("edit did not reach delivered readback"),
            "{error:#}"
        );
    }

    #[test]
    fn delivered_inventory_roundtrip_native_generation_and_changed_embeddings() {
        let f = Fixture::new();
        let id = Uuid::new_v4();
        let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let guard = store
            .lock_native(&f.policy, id, &f.root.join("index"))
            .unwrap();
        assert!(build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &Changes::default()
        )
        .is_err());
        let current = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &Fixture::changes(),
        )
        .unwrap();
        let review = guard.inspect(&current).unwrap();
        assert!(review
            .metadata_inventory
            .iter()
            .any(|e| e.warning && e.location.contains("originals-0001.zip!document-0001.md")));
        assert!(guard.reviewed_payload(&current).is_err());
        guard.accept(&current).unwrap();
        let (payload, terms) = guard.reviewed_payload(&current).unwrap();
        assert_eq!(terms, ["fixture-identity"]);
        let output = f.root.join("delivered");
        let manifest =
            crate::review_transfer::package(&guard, &current, &payload, &output).unwrap();
        assert_eq!(manifest["pack_id"], id.to_string());
        for artifact in &review.staged_artifacts {
            let bytes = std::fs::read(output.join(&artifact.artifact)).unwrap();
            assert_eq!(bytes.len() as u64, artifact.size);
            assert_eq!(sha256(&bytes), String::from(artifact.sha256.clone()));
            assert!(!bytes
                .windows(b"client-original.md".len())
                .any(|w| w == b"client-original.md"));
        }
        let mut changes = Fixture::changes();
        changes.edits.insert(
            "originals-0001.zip!document-0001.md".into(),
            "# Reviewed title\nReviewed delivered content".into(),
        );
        let next = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap();
        assert!(guard.check_accepted(&current).is_err());
        let next_review = guard.inspect(&next).unwrap();
        assert!(next_review.accepted_review_hash.is_none());
        assert!(next_review
            .metadata_inventory
            .iter()
            .any(|e| e.value.as_deref() == Some("# Reviewed title\nReviewed delivered content")));
        assert!(f
            .engine
            .texts
            .lock()
            .unwrap()
            .iter()
            .any(|text| text.contains("Reviewed delivered content")));
        assert!(
            std::fs::read_to_string(f.root.join("source/client-original.md"))
                .unwrap()
                .contains("Confidential")
        );
        // A failed rebuild still invalidates the previous review permanently.
        guard.accept(&next).unwrap();
        changes
            .edits
            .insert("unhandled/location".into(), "edit".into());
        assert!(build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes
        )
        .is_err());
        assert!(guard.check_accepted(&next).is_err());
    }

    #[test]
    fn office_edit_exclusion_rechunk_and_generation_binding() {
        let f = Fixture::new();
        let bytes = include_bytes!("../../../spike/fixtures/documents/library.docx");
        let path = f.root.join("source/z-office.docx");
        std::fs::write(&path, bytes).unwrap();
        let index = f.root.join("index");
        let mut state = State::open(&index).unwrap();
        let doc = vz_core::pipeline::Document {
            doc_id: "office-stable-source".into(),
            path: path.to_str().unwrap().into(),
            sha256: sha256(bytes),
            bytes: bytes.len() as u64,
            pages: 1,
            kind: "docx".into(),
            needs_ocr_pages: vec![],
            chunks: vec![],
        };
        let generation = Uuid::new_v4().to_string();
        let directory = index.join("generations").join(&generation);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("document.json"),
            serde_json::to_vec(&doc).unwrap(),
        )
        .unwrap();
        state.commit(&doc, "fixture", &generation).unwrap();
        drop(state);
        let id = Uuid::new_v4();
        let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let guard = store.lock_native(&f.policy, id, &index).unwrap();
        let source = f.root.join("source");
        let mut changes = Fixture::changes();
        let first = build(&guard, &source, id, &f.engine, "query", &changes).unwrap();
        let review = guard.inspect(&first).unwrap();
        let nodes: Vec<_> = review
            .metadata_inventory
            .iter()
            .filter(|entry| entry.kind == "Office text" && entry.value.is_some())
            .collect();
        assert_eq!(nodes.len(), 2);
        changes.origin = Some(first.clone());
        changes.exclusions.insert("client-original.md".into());
        changes
            .edits
            .insert(nodes[0].location.clone(), "Reviewed Office title".into());
        changes.edits.insert(
            nodes[1].location.clone(),
            "Reviewed changed Office body. ".repeat(500),
        );
        let second = build(&guard, &source, id, &f.engine, "query", &changes).unwrap();
        let second_review = guard.inspect(&second).unwrap();
        for (location, value) in &changes.edits {
            assert!(second_review
                .metadata_inventory
                .iter()
                .any(|e| &e.location == location && e.value.as_ref() == Some(value)));
        }
        assert!(f
            .engine
            .texts
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.contains("Reviewed changed Office body")));
        assert!(guard.accept(&first).is_err());
        assert!(guard.accept(&second).is_ok());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // The old generation cannot authorize another mutation, even if the
        // location and expected text happen to match a current node.
        assert!(build(&guard, &source, id, &f.engine, "query", &changes).is_err());
        changes.origin = Some(second);
        std::fs::write(&path, b"observed source swap").unwrap();
        assert!(build(&guard, &source, id, &f.engine, "query", &changes).is_err());
        std::fs::write(&path, bytes).unwrap();
        assert!(
            build(&guard, &source, id, &f.engine, "query", &changes).is_err(),
            "restoring bytes cannot revive a revoked Office edit origin"
        );
    }

    #[test]
    fn managed_policy_cancels_reviewed_writes_and_keeps_completed_artifacts() {
        let f = Fixture::new();
        let id = Uuid::new_v4();
        let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let guard = store
            .lock_native(&f.policy, id, &f.root.join("index"))
            .unwrap();
        let current = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &Fixture::changes(),
        )
        .unwrap();
        guard.accept(&current).unwrap();
        let (payload, _) = guard.reviewed_payload(&current).unwrap();
        let completed = f.root.join("completed");
        crate::review_transfer::package(&guard, &current, &payload, &completed).unwrap();
        let bytes = std::fs::read(completed.join("manifest.json")).unwrap();
        *f.policy_source.0.lock().unwrap() = ManagedPolicy::Denied;
        f.policy.refresh();
        assert!(crate::review_transfer::package(
            &guard,
            &current,
            &payload,
            &f.root.join("denied")
        )
        .is_err());
        assert!(!f.root.join("denied").exists());
        assert_eq!(
            std::fs::read(completed.join("manifest.json")).unwrap(),
            bytes
        );
        *f.policy_source.0.lock().unwrap() = ManagedPolicy::Allowed;
        f.policy.refresh();
        assert!(guard.check_accepted(&current).is_err());
    }

    #[test]
    fn reviewed_transfer_source_swaps_refuse_before_writes_and_before_promotion() {
        use crate::review_transfer::{package_with_checks, Point};
        for cutoff in [Point::BeforeWrite, Point::BeforePromotion] {
            let f = Fixture::new();
            let id = Uuid::new_v4();
            let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
            let guard = store
                .lock_native(&f.policy, id, &f.root.join("index"))
                .unwrap();
            let current = build(
                &guard,
                &f.root.join("source"),
                id,
                &f.engine,
                "query",
                &Fixture::changes(),
            )
            .unwrap();
            guard.accept(&current).unwrap();
            let (payload, _) = guard.reviewed_payload(&current).unwrap();
            let source = f.root.join("source/client-original.md");
            let original = std::fs::read(&source).unwrap();
            let sentinel = f.root.join(".vz-sentinel.partial");
            std::fs::create_dir(&sentinel).unwrap();
            std::fs::write(sentinel.join("keep"), b"seller data").unwrap();
            let output = f.root.join("swapped");
            assert!(
                package_with_checks(&guard, &current, &payload, &output, |point| {
                    if point == cutoff {
                        std::fs::write(&source, b"changed source")?;
                    }
                    Ok(())
                })
                .is_err()
            );
            assert!(!output.exists());
            assert_eq!(
                std::fs::read(sentinel.join("keep")).unwrap(),
                b"seller data"
            );
            assert_eq!(
                std::fs::read_dir(&f.root)
                    .unwrap()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().starts_with(".vz-"))
                    .count(),
                1
            );
            std::fs::write(source, original).unwrap();
            drop(guard);
            let restarted = store.lock_native_current(&f.policy, &current, &f.root.join("index"));
            assert!(
                restarted.is_err(),
                "restoring source bytes must not revive a cancelled generation"
            );
        }
    }

    #[test]
    fn managed_policy_cancellation_at_reviewed_promotion_barrier() {
        use crate::review_transfer::{package_with_checks, Point};
        let f = Fixture::new();
        let id = Uuid::new_v4();
        let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let guard = store
            .lock_native(&f.policy, id, &f.root.join("index"))
            .unwrap();
        let current = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &Fixture::changes(),
        )
        .unwrap();
        guard.accept(&current).unwrap();
        let (payload, _) = guard.reviewed_payload(&current).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let output = f.root.join("cancelled");
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                *f.policy_source.0.lock().unwrap() = ManagedPolicy::Denied;
                f.policy.refresh();
                barrier.wait();
            });
            assert!(
                package_with_checks(&guard, &current, &payload, &output, |point| {
                    if point == Point::BeforePromotion {
                        barrier.wait();
                        barrier.wait();
                    }
                    Ok(())
                })
                .is_err()
            );
        });
        assert!(!output.exists());
        assert!(!std::fs::read_dir(&f.root).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".vz-")));
        *f.policy_source.0.lock().unwrap() = ManagedPolicy::Allowed;
        f.policy.refresh();
        assert!(guard.check_accepted(&current).is_err());
    }

    #[test]
    fn reviewed_promotion_lost_receipt_retains_destination_evidence_after_restart() {
        use crate::review_transfer::{package_with_checks, Point, PromotionOutcomeUnknown};
        let f = Fixture::new();
        let id = Uuid::new_v4();
        let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let guard = store
            .lock_native(&f.policy, id, &f.root.join("index"))
            .unwrap();
        let current = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &Fixture::changes(),
        )
        .unwrap();
        guard.accept(&current).unwrap();
        let (payload, _) = guard.reviewed_payload(&current).unwrap();
        let output = f.root.join("possibly-promoted");
        let error = package_with_checks(&guard, &current, &payload, &output, |point| {
            anyhow::ensure!(
                point != Point::AfterPromotion,
                "lost durable completion receipt"
            );
            Ok(())
        })
        .unwrap_err();
        assert!(error.is::<PromotionOutcomeUnknown>());
        assert_eq!(crate::export::exit_code(&error), 3);
        assert!(output.join("manifest.json").is_file());
        drop(guard);
        drop(store);
        let reopened = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let pack = reopened.lock(&f.policy, id).unwrap();
        let uncertain = pack.uncertain_transfers(&current).unwrap();
        assert_eq!(uncertain.len(), 1);
        assert_eq!(
            uncertain[0].destination.as_deref(),
            Some(format!("package:{}", output.display()).as_str())
        );
        assert!(output.join("manifest.json").is_file());
    }
}
