//! Native-only private rebuild. No source/index writer or delivered mapping.
use crate::{
    review::{ListingReview, ReviewVersion},
    review_store::{private_directory, private_file, CurrentReview, NativePackReview, ReviewStore},
    sanitize::{self, Budget, Changes},
};
use anyhow::{ensure, Context, Result};
use parquet::arrow::ArrowWriter;
use serde_json::json;
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
    engines: &Engines,
    query: &str,
    changes: &Changes,
) -> Result<CurrentReview> {
    let binding = pack.capture(source)?;
    let index = pack.index_path();
    let recipe: vz_core::engine::Recipe =
        serde_json::from_slice(&std::fs::read(index.join("recipe.json"))?)?;
    ensure!(
        recipe == engines.recipe,
        "embedding recipe mismatch; re-index required"
    );
    let indexed = State::review_documents(index)?;
    let work = pack.work()?;
    let result = (|| {
        let mut docs = Vec::new();
        let mut originals = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let mut mapping = BTreeMap::new();
        let mut budget = Budget::default();
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
                || (key.starts_with("documents.parquet/row/") && key.ends_with("/column/2"))
                || (key.starts_with("chunks.parquet/row/") && key.ends_with("/column/4"));
            ensure!(allowed, "edit location has no proven rebuild handler");
        }
        let source = source.canonicalize()?;
        for mut doc in indexed {
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
            // Current proven original parsers preserve all UTF-8 text. Office,
            // PDF and HTML must be excluded or edited/re-ingested as text.
            ensure!(
                ["txt", "md", "markdown"].contains(&doc.kind.as_str()),
                "original format has no proven rewrite handler; exclude or re-ingest as text"
            );
            let name = format!("document-{:04}.{}", docs.len() + 1, doc.kind);
            let location = format!("originals-0001.zip!{name}");
            if changes.exclusions.contains(&location) {
                continue;
            }
            let mut bytes = Vec::new();
            std::fs::File::open(&doc.path)?
                .take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= 64 * 1024 * 1024 && sha256(&bytes) == doc.sha256,
                "source changed since indexing"
            );
            let bytes = sanitize::rewrite(&location, &bytes, changes, &mut mapping, &mut budget)?;
            mapping.insert(relative, location);
            let text = std::str::from_utf8(&bytes)?;
            doc.chunks = chunk(
                &doc.doc_id,
                &[Page {
                    page: 1,
                    text: text.into(),
                    needs_ocr: false,
                }],
            );
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
            originals.start_file(name, zip::write::SimpleFileOptions::default())?;
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
        for (name, batch) in [
            ("documents.parquet", documents_batch(&docs)?),
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
        let warnings = inventory
            .iter()
            .filter(|e| e.warning)
            .map(|e| format!("{}: {}", e.location, e.kind))
            .collect();
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
            json!({"mapping":mapping,"identity_terms":changes.identity_terms}),
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
