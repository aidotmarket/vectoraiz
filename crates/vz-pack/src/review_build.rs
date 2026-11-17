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
#[cfg(any(test, feature = "listing-originals"))]
use std::io::Cursor;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
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
    pub refusal: Option<String>,
    /// Private source-only summary, available even when all text is hidden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_summary: Option<serde_json::Value>,
}

#[cfg(not(feature = "listing-originals"))]
#[path = "review_vector.rs"]
mod vector;
#[cfg(not(feature = "listing-originals"))]
use vector::build;

pub fn candidates(library: &Path, source: &Path) -> Result<Vec<Candidate>> {
    candidates_with_pdfium(library, source, None)
}

pub fn candidates_with_pdfium(
    library: &Path,
    source: &Path,
    pdfium: Option<&Path>,
) -> Result<Vec<Candidate>> {
    let policy = crate::listing_policy::system();
    policy.require_preview()?;
    let lease = policy.policy_lease()?;
    let source = source.canonicalize()?;
    let started = std::time::Instant::now();
    let candidates = crate::review_store::NativeIndexFence::candidate_documents(library, &source)?
        .into_iter()
        .enumerate()
        .map(|(position, doc)| {
            let artifact = candidate_location(position, &doc.kind);
            lease.check(policy)?;
            let mut budget = Budget::default();
            budget.enable_office();
            budget.started = Some(started);
            budget.cancel = Some(lease.cancellation_token());
            let refusal = preparation_refusal_with_pdfium(&doc, &artifact, &mut budget, pdfium);
            #[cfg(not(feature = "listing-originals"))]
            let projection_summary = budget.projection.as_ref().map(|projection| json!({"mandatory_drops":projection.drops,"hidden_locations":projection.nodes.iter().filter(|n|n.hidden).map(|n|&n.location).collect::<Vec<_>>()}));
            #[cfg(feature = "listing-originals")]
            let projection_summary = None;
            Ok(Candidate {
                projection_summary,
                source: Path::new(&doc.path)
                    .strip_prefix(&source)?
                    .to_str()
                    .context("source path must be UTF-8")?
                    .replace('\\', "/"),
                artifact,
                supported: refusal.is_none(),
                refusal,
                format: doc.kind,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    lease.check(policy)?;
    Ok(candidates)
}

fn candidate_location(position: usize, kind: &str) -> String {
    #[cfg(feature = "listing-originals")]
    {
        format!("originals-0001.zip!document-{:04}.{kind}", position + 1)
    }
    #[cfg(not(feature = "listing-originals"))]
    {
        let _ = kind;
        format!("projection/document-{:04}", position + 1)
    }
}

// Run the production rebuild worker, including its independent readback and
// resource limits. Parser success or an extension alone cannot certify a file.
#[cfg(all(test, feature = "listing-originals"))]
fn preparation_refusal(
    doc: &vz_core::pipeline::Document,
    artifact: &str,
    budget: &mut Budget,
) -> Option<String> {
    preparation_refusal_with_pdfium(doc, artifact, budget, None)
}

fn preparation_refusal_with_pdfium(
    doc: &vz_core::pipeline::Document,
    artifact: &str,
    budget: &mut Budget,
    pdfium: Option<&Path>,
) -> Option<String> {
    let result = (|| -> Result<()> {
        let supported =
            ["txt", "md", "markdown", "docx", "xlsx", "pptx"].contains(&doc.kind.as_str());
        #[cfg(not(feature = "listing-originals"))]
        let supported = supported || ["pdf", "html", "htm"].contains(&doc.kind.as_str());
        #[cfg(feature = "listing-originals")]
        let _ = pdfium;
        ensure!(
            supported,
            "original format has no proven rewrite handler; exclude or re-ingest as text"
        );
        let office = ["docx", "xlsx", "pptx"].contains(&doc.kind.as_str());
        let bytes = source_bytes(
            doc,
            if office {
                512_000_000
            } else if ["pdf", "html", "htm"].contains(&doc.kind.as_str()) {
                256_000_000
            } else {
                64 * 1024 * 1024
            },
        )?;
        #[cfg(feature = "listing-originals")]
        if office {
            sanitize::office_worker(artifact, &bytes, budget, false)?;
        }
        #[cfg(not(feature = "listing-originals"))]
        {
            budget.projection = Some(vector::project(
                artifact,
                &doc.kind,
                &bytes,
                pdfium,
                budget.cancel.clone(),
                budget.started.unwrap_or_else(std::time::Instant::now),
                std::time::Duration::ZERO,
            )?);
        }
        Ok(())
    })();
    result.err().map(|error| format!("{error:#}"))
}

fn source_bytes(doc: &vz_core::pipeline::Document, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(&doc.path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit && sha256(&bytes) == doc.sha256,
        "source changed since indexing"
    );
    Ok(bytes)
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

#[cfg(feature = "listing-originals")]
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
        let origin_removals = origin
            .as_ref()
            .and_then(|(_, private)| private.get("office_removals"))
            .map(|records| serde_json::from_value::<Vec<vz_core::office::Removal>>(records.clone()))
            .transpose()?
            .unwrap_or_default();
        let mut carried_legacy_office_removals = false;
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
            let mut bytes = source_bytes(
                &doc,
                if has_office {
                    512 * 1024 * 1024
                } else {
                    64 * 1024 * 1024
                },
            )?;
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
                if let Some(sources) = origin
                    .as_ref()
                    .and_then(|(_, private)| private.get("office_chart_sources"))
                    .and_then(|m| m.get(&location))
                {
                    budget
                        .chart_sources
                        .insert(location.clone(), serde_json::from_value(sources.clone())?);
                }
                budget.from_origin.insert(location.clone());
                budget.removals.extend(
                    origin_removals
                        .iter()
                        .filter(|r| {
                            r.location.starts_with(&format!("{location}!"))
                                || (!carried_legacy_office_removals && !r.location.contains('!'))
                        })
                        .cloned(),
                );
                carried_legacy_office_removals = true;
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
        warnings.sort();
        warnings.dedup();
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
            projection_inventory: Vec::new(),
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
            json!({"mapping":mapping,"document_ids":document_ids,"identity_terms":changes.identity_terms,"office_removals":budget.removals,"office_chart_sources":budget.chart_sources,"office_origin":changes.origin,"office_nodes":office_nodes,"office_allowlist_version":vz_core::office::ALLOWLIST_VERSION}),
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
    #[cfg(not(feature = "listing-originals"))]
    fn pdfium(&self) -> Option<std::path::PathBuf> {
        None
    }
    fn recipe(&self) -> &vz_core::engine::Recipe;
    fn vector(&self, text: &str, query: bool) -> Result<Vec<f32>>;
}
impl BuildEngine for Engines {
    #[cfg(not(feature = "listing-originals"))]
    fn pdfium(&self) -> Option<std::path::PathBuf> {
        Some(self.pdfium_library())
    }
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
    #[test]
    #[cfg(feature = "listing-originals")]
    fn preparation_reports_every_refusal_with_production_worker_reasons() {
        let temp = tempfile::tempdir().unwrap();
        let cases: &[(&str, &[u8])] = &[
            (
                "ole.pptx",
                include_bytes!("../tests/fixtures/office-public/45545_Comment.pptx"),
            ),
            ("bad.docx", b"not an Office package"),
            ("bad.xlsx", b"not an Office package either"),
            (
                "comments.docx",
                include_bytes!("../tests/fixtures/office-public/testComment.docx"),
            ),
        ];
        let refusals: Vec<_> = cases
            .iter()
            .map(|(name, bytes)| {
                let path = temp.path().join(name);
                std::fs::write(&path, bytes).unwrap();
                let doc = vz_core::pipeline::Document {
                    doc_id: name.to_string(),
                    path: path.to_str().unwrap().into(),
                    sha256: sha256(bytes),
                    bytes: bytes.len() as u64,
                    pages: 1,
                    kind: name.rsplit('.').next().unwrap().into(),
                    needs_ocr_pages: vec![],
                    chunks: vec![],
                };
                let mut budget = Budget::default();
                budget.enable_office();
                let refusal = preparation_refusal(&doc, name, &mut budget);
                let expected = vz_core::office::rebuild(name, bytes, &BTreeMap::new())
                    .err()
                    .map(|e| e.to_string());
                assert_eq!(
                    refusal, expected,
                    "preparation must use the rebuild refusal for {name}"
                );
                // A source swap cannot be marked supported even for a supported extension.
                std::fs::write(&path, b"changed since indexing").unwrap();
                assert!(preparation_refusal(&doc, name, &mut budget)
                    .unwrap()
                    .contains("source changed since indexing"));
                refusal
            })
            .collect();
        assert!(refusals[..3].iter().all(Option::is_some));
        assert!(refusals[3].is_none());
    }

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
        #[cfg(not(feature = "listing-originals"))]
        fn pdfium(&self) -> Option<std::path::PathBuf> {
            Some(
                std::env::var_os("VZ_TEST_PDFIUM")
                    .map(Into::into)
                    .unwrap_or_else(|| {
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("../../.assets")
                            .join(if cfg!(windows) {
                                "pdfium/bin/pdfium.dll"
                            } else {
                                "pdfium/lib/libpdfium.dylib"
                            })
                    }),
            )
        }
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
    #[cfg(feature = "listing-originals")]
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
    #[cfg(feature = "listing-originals")]
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
    #[cfg(feature = "listing-originals")]
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
    #[cfg(feature = "listing-originals")]
    fn chart_source_bindings_survive_private_review_generations() {
        let f = Fixture::new();
        let marker = "private-S1934-charted-cell-marker";
        let original = include_bytes!("../tests/fixtures/office-public/57362.xlsx");
        let mut input = zip::ZipArchive::new(Cursor::new(original)).unwrap();
        let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..input.len() {
            let mut part = input.by_index(i).unwrap();
            let name = part.name().to_owned();
            let mut bytes = Vec::new();
            part.read_to_end(&mut bytes).unwrap();
            if name.ends_with(".xml") {
                bytes = String::from_utf8(bytes)
                    .unwrap()
                    .replace("Column1", marker)
                    .into_bytes();
            }
            output
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            output.write_all(&bytes).unwrap();
        }
        let bytes = output.finish().unwrap().into_inner();
        let path = f.root.join("source/chart.xlsx");
        std::fs::write(&path, &bytes).unwrap();
        let index = f.root.join("index");
        let mut state = State::open(&index).unwrap();
        let doc = vz_core::pipeline::Document {
            doc_id: "charted-source".into(),
            path: path.to_str().unwrap().into(),
            sha256: sha256(&bytes),
            bytes: bytes.len() as u64,
            pages: 1,
            kind: "xlsx".into(),
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
        let mut changes = Fixture::changes();
        let source = f.root.join("source");
        let mut current = build(&guard, &source, id, &f.engine, "query", &changes).unwrap();
        for round in 0..2 {
            let review = guard.inspect(&current).unwrap();
            let node = review
                .metadata_inventory
                .iter()
                .find(|e| {
                    e.location.contains(".xlsx!")
                        && e.location.contains("/cell/A1/")
                        && e.location.ends_with("/text")
                })
                .unwrap();
            let value = format!("Reviewed chart category {round}");
            changes.origin = Some(current.clone());
            changes.edits = BTreeMap::from([(node.location.clone(), value.clone())]);
            let next = build(&guard, &source, id, &f.engine, "query", &changes).unwrap();
            let (review, private) = guard.edit_origin(&next).unwrap();
            let key = node
                .location
                .split('!')
                .take(2)
                .collect::<Vec<_>>()
                .join("!");
            assert!(!private["office_chart_sources"][&key]["points"]
                .as_array()
                .unwrap()
                .is_empty());
            assert_eq!(
                review
                    .metadata_inventory
                    .iter()
                    .filter(|e| e.location.contains(".xlsx!")
                        && e.location.ends_with("/text")
                        && e.value.as_deref() == Some(value.as_str()))
                    .count(),
                3
            );
            assert!(review
                .metadata_inventory
                .iter()
                .filter_map(|e| e.value.as_ref())
                .all(|v| !v.contains(marker)));
            let delivered = guard
                .origin_original(&next, key.split('!').nth(1).unwrap())
                .unwrap();
            assert!(!delivered
                .windows(marker.len())
                .any(|b| b == marker.as_bytes()));
            let mut archive = zip::ZipArchive::new(Cursor::new(delivered)).unwrap();
            for i in 0..archive.len() {
                let mut bytes = Vec::new();
                archive
                    .by_index(i)
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                assert!(!bytes.windows(marker.len()).any(|b| b == marker.as_bytes()));
            }
            current = next;
        }
        assert_eq!(sha256(&std::fs::read(path).unwrap()), doc.sha256);
    }

    #[test]
    #[cfg(feature = "listing-originals")]
    fn office_removals_stay_linear_across_edit_generations() {
        let f = Fixture::new();
        let source = f.root.join("source");
        let index = f.root.join("index");
        let mut state = State::open(&index).unwrap();
        for i in 0..3 {
            let original = include_bytes!("../../../spike/fixtures/documents/library.docx");
            let mut input = zip::ZipArchive::new(Cursor::new(original)).unwrap();
            let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for j in 0..input.len() {
                let mut part = input.by_index(j).unwrap();
                let name = part.name().to_owned();
                let mut bytes = Vec::new();
                part.read_to_end(&mut bytes).unwrap();
                if name == "word/document.xml" {
                    bytes = String::from_utf8(bytes)
                        .unwrap()
                        .replace("<w:body>", "<w:body w:rsidR=\"planted-removal-1927\">")
                        .into_bytes();
                }
                output
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                output.write_all(&bytes).unwrap();
            }
            let bytes = output.finish().unwrap().into_inner();
            let path = source.join(format!("office-{i}.docx"));
            std::fs::write(&path, &bytes).unwrap();
            let doc = vz_core::pipeline::Document {
                doc_id: format!("office-{i}"),
                path: path.to_str().unwrap().into(),
                sha256: sha256(&bytes),
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
        }
        drop(state);
        let id = Uuid::new_v4();
        let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let guard = store.lock_native(&f.policy, id, &index).unwrap();
        let mut changes = Fixture::changes();
        let mut current = build(&guard, &source, id, &f.engine, "query", &changes).unwrap();
        for round in 0..4 {
            let (_, private) = guard.edit_origin(&current).unwrap();
            let records = private["office_removals"].as_array().unwrap();
            assert_eq!(records.len(), 3, "round {round}");
            assert!(records.iter().all(|r| r["location"]
                .as_str()
                .unwrap()
                .starts_with("originals-0001.zip!document-")));
            let review = guard.inspect(&current).unwrap();
            let node = review
                .metadata_inventory
                .iter()
                .find(|e| e.kind == "Office text" && e.value.is_some())
                .unwrap();
            for record in records {
                let warning = format!(
                    "{}: {}",
                    record["location"].as_str().unwrap(),
                    record["reason"].as_str().unwrap()
                );
                assert_eq!(review.warnings.iter().filter(|w| **w == warning).count(), 1);
            }
            if round < 3 {
                changes.origin = Some(current.clone());
                changes.edits.insert(
                    node.location.clone(),
                    format!("Edited Office text round {round}"),
                );
                current = build(&guard, &source, id, &f.engine, "query", &changes).unwrap();
            }
        }
    }

    #[test]
    #[cfg(feature = "listing-originals")]
    fn build_reports_hidden_sheet_at_preserved_attribute_without_private_values() {
        use std::io::{Read, Write};
        let f = Fixture::new();
        for state_value in ["hidden", "veryHidden"] {
            let source = include_bytes!("../tests/fixtures/office-public/WithVariousData.xlsx");
            let mut input = zip::ZipArchive::new(Cursor::new(source)).unwrap();
            let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for i in 0..input.len() {
                let mut part = input.by_index(i).unwrap();
                let mut bytes = Vec::new();
                part.read_to_end(&mut bytes).unwrap();
                if part.name() == "xl/workbook.xml" {
                    bytes = String::from_utf8(bytes)
                        .unwrap()
                        .replace(
                            "sheetId=\"2\"",
                            &format!("sheetId=\"2\" state=\"{state_value}\""),
                        )
                        .into_bytes();
                }
                output
                    .start_file(part.name(), zip::write::SimpleFileOptions::default())
                    .unwrap();
                output.write_all(&bytes).unwrap();
            }
            let bytes = output.finish().unwrap().into_inner();
            let path = f.root.join("source/private-workbook.xlsx");
            std::fs::write(&path, &bytes).unwrap();
            let index = f.root.join("index");
            let mut state = State::open(&index).unwrap();
            let doc = vz_core::pipeline::Document {
                doc_id: "hidden-sheet-source".into(),
                path: path.to_str().unwrap().into(),
                sha256: sha256(&bytes),
                bytes: bytes.len() as u64,
                pages: 1,
                kind: "xlsx".into(),
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
            let suffix = "!xl/workbook.xml/{http://schemas.openxmlformats.org/spreadsheetml/2006/main}sheets[1]/{http://schemas.openxmlformats.org/spreadsheetml/2006/main}sheet[2]/@{}state: hidden sheet kept hidden";
            assert_eq!(
                review
                    .warnings
                    .iter()
                    .filter(|w| w.ends_with(suffix))
                    .count(),
                1
            );
            assert!(review.warnings.iter().all(|w| !w.contains("Sheet2")
                && !w.contains("private-workbook")
                && !w.contains(path.to_str().unwrap())));
        }
    }

    #[test]
    #[cfg(feature = "listing-originals")]
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

    #[cfg(not(feature = "listing-originals"))]
    fn projection_fixture(f: &Fixture) -> Vec<u8> {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:t>DISPLAYED_RESULT</w:t></w:r><w:r><w:t> con</w:t></w:r><w:r><w:t>fidential agree</w:t></w:r><w:r><w:t>ment Ac</w:t></w:r><w:r><w:t>me</w:t></w:r><w:del><w:r><w:delText>DELETION_CANARY</w:delText></w:r></w:del><w:moveFrom><w:r><w:t>MOVED_CANARY</w:t></w:r></w:moveFrom><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:sdt><w:sdtContent><w:r><w:t>INSTRUCTION_CANARY</w:t></w:r></w:sdtContent></w:sdt><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>FIELD_RESULT</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:rPr><w:vanish/></w:rPr><w:t>HIDDEN_CANARY</w:t></w:r></w:p><w:tbl><w:tr><w:trPr><w:del/></w:trPr><w:tc><w:p><w:r><w:t>DELETED_ROW_CANARY</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:cellDel/></w:tcPr><w:p><w:r><w:t>DELETED_CELL_CANARY</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:drawing><pic:pic><pic:nvPicPr><pic:cNvPr descr="Acme PICTURE_ALT_CANARY" title="PICTURE_TITLE_CANARY"/></pic:nvPicPr></pic:pic><dgm:relIds r:dm="model"/></w:drawing></w:r></w:p></w:body></w:document>"#;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in [
            ("word/document.xml", xml),
            (
                "word/_rels/document.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="model" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="diagrams/data1.xml"/></Relationships>"#,
            ),
            (
                "word/diagrams/data1.xml",
                r#"<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram"><dgm:ptLst><dgm:pt modelId="node"><dgm:t><a:p><a:r><a:t>PAIRED_TEXT</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst><dsp:dataModelExt relId="cache"/></dgm:dataModel>"#,
            ),
            (
                "word/diagrams/_rels/data1.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="cache" Type="http://schemas.microsoft.com/office/2007/relationships/diagramDrawing" Target="drawing1.xml"/></Relationships>"#,
            ),
            (
                "word/diagrams/drawing1.xml",
                r#"<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:sp modelId="node"><dsp:nvSpPr><dsp:cNvPr descr="Acme SMARTART_ALT_CANARY" title="SMARTART_TITLE_CANARY"/></dsp:nvSpPr><a:p><a:r><a:t>PAIRED_TEXT</a:t></a:r></a:p></dsp:sp></dsp:drawing>"#,
            ),
            ("word/comments.xml", "<comments>COMMENT_CANARY</comments>"),
            (
                "docProps/core.xml",
                "<properties>PROPERTY_CANARY</properties>",
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="root" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        index_poisoned_projection(f, "docx", &bytes);
        bytes
    }

    #[cfg(not(feature = "listing-originals"))]
    fn index_poisoned_projection(f: &Fixture, kind: &str, bytes: &[u8]) {
        let path = f.root.join(format!("source/private-source.{kind}"));
        std::fs::write(&path, bytes).unwrap();
        let id = format!("poisoned-{kind}");
        let mut doc = vz_core::pipeline::Document {
            doc_id: id.clone(),
            path: path.to_str().unwrap().into(),
            sha256: sha256(bytes),
            bytes: bytes.len() as u64,
            pages: 1,
            kind: kind.into(),
            needs_ocr_pages: vec![],
            chunks: chunk(&id, &[Page {
                page: 1,
                text: "DELETION_CANARY MOVED_CANARY INSTRUCTION_CANARY COMMENT_CANARY PROPERTY_CANARY FORMULA_CANARY INDEX_POISON".into(),
                needs_ocr: false,
            }]),
        };
        for c in &mut doc.chunks {
            c.embedding = vec![999.; 3];
        }
        let index = f.root.join("index");
        let mut state = State::open(&index).unwrap();
        let generation = Uuid::new_v4().to_string();
        let directory = index.join("generations").join(&generation);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("document.json"),
            serde_json::to_vec(&doc).unwrap(),
        )
        .unwrap();
        state.commit(&doc, "poisoned", &generation).unwrap();
    }

    #[cfg(not(feature = "listing-originals"))]
    fn other_office_projection_fixtures(f: &Fixture) {
        for (kind, main, parts) in [
            (
                "xlsx",
                "xl/workbook.xml",
                vec![
                    (
                        "xl/workbook.xml",
                        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Reviewed sheet" sheetId="1" r:id="sheet"/></sheets></workbook>"#,
                    ),
                    (
                        "xl/_rels/workbook.xml.rels",
                        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="sheet" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
                    ),
                    (
                        "xl/worksheets/sheet1.xml",
                        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="str"><f>FORMULA_CANARY</f><v>CACHED_RESULT</v></c><c r="B1" t="e"><f>ERROR_FORMULA_CANARY</f><v>#DIV/0!</v></c></row></sheetData></worksheet>"#,
                    ),
                    ("xl/comments1.xml", "<comments>COMMENT_CANARY</comments>"),
                ],
            ),
            (
                "pptx",
                "ppt/presentation.xml",
                vec![
                    (
                        "ppt/presentation.xml",
                        r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="1" r:id="slide"/></p:sldIdLst></p:presentation>"#,
                    ),
                    (
                        "ppt/_rels/presentation.xml.rels",
                        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="slide" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#,
                    ),
                    (
                        "ppt/charts/chart1.xml",
                        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>Annual</a:t></a:r><a:br/><a:r><a:t>Rev</a:t></a:r><a:r><a:t>enue</a:t></a:r></a:p></c:rich></c:tx></c:title></c:chart></c:chartSpace>"#,
                    ),
                    (
                        "ppt/slides/slide1.xml",
                        r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="chart"/><p:sp><p:txBody><a:p><a:r><a:t>SLIDE_RESULT con</a:t></a:r><a:r><a:t>fidential agree</a:t></a:r><a:r><a:t>ment Ac</a:t></a:r><a:r><a:t>me</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
                    ),
                    (
                        "ppt/slides/_rels/slide1.xml.rels",
                        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="chart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/><Relationship Id="notes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notes1.xml"/></Relationships>"#,
                    ),
                    (
                        "ppt/notesSlides/notes1.xml",
                        r#"<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:p><a:r><a:t>NOTES_RESULT</a:t></a:r></a:p></p:notes>"#,
                    ),
                    (
                        "ppt/comments/comment1.xml",
                        "<comments>COMMENT_CANARY</comments>",
                    ),
                ],
            ),
        ] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let root = format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="root" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="{main}"/></Relationships>"#
            );
            for (name, text) in parts.into_iter().chain([
                ("_rels/.rels", root.as_str()),
                (
                    "docProps/core.xml",
                    "<properties>PROPERTY_CANARY</properties>",
                ),
            ]) {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(text.as_bytes()).unwrap();
            }
            index_poisoned_projection(f, kind, &zip.finish().unwrap().into_inner());
        }
    }

    #[test]
    #[cfg(not(feature = "listing-originals"))]
    fn vector_only_payload_contract_and_projection_exclusion_canaries() {
        let f = Fixture::new();
        let source_bytes = projection_fixture(&f);
        other_office_projection_fixtures(&f);
        let id = Uuid::new_v4();
        let store = ReviewStore::open(&f.root.join("reviews")).unwrap();
        let guard = store
            .lock_native(&f.policy, id, &f.root.join("index"))
            .unwrap();
        let mut changes = Fixture::changes();
        changes.identity_terms = vec!["Acme".into()];
        let current = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap();
        let review = guard.inspect(&current).unwrap();
        for marker in [
            "DELETION_CANARY",
            "DELETED_ROW_CANARY",
            "DELETED_CELL_CANARY",
            "MOVED_CANARY",
            "INSTRUCTION_CANARY",
            "COMMENT_CANARY",
            "PROPERTY_CANARY",
            "FORMULA_CANARY",
            "HIDDEN_CANARY",
            "INDEX_POISON",
        ] {
            assert!(
                f.engine
                    .texts
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|s| !s.contains(marker)),
                "embedding input: {marker}"
            );
            assert!(
                review
                    .metadata_inventory
                    .iter()
                    .filter_map(|e| e.value.as_ref())
                    .all(|s| !s.contains(marker)),
                "delivered inventory: {marker}"
            );
        }
        assert!(f
            .engine
            .texts
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.contains("FIELD_RESULT")));
        for retained in ["CACHED_RESULT", "#DIV/0!", "SLIDE_RESULT", "NOTES_RESULT"] {
            assert!(
                f.engine
                    .texts
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|s| s.contains(retained)),
                "missing embedding input: {retained}"
            );
        }
        assert!(review
            .warnings
            .iter()
            .any(|s| s.contains("mandatory drop deletions: 3")));
        assert!(review
            .projection_inventory
            .iter()
            .any(|e| e.kind.contains("hidden") && e.kind.ends_with("omitted")));
        let inputs = f.engine.texts.lock().unwrap();
        // Chunking normalizes the projected line break to a word separator.
        assert!(inputs.iter().any(|s| s.contains("Annual Revenue")));
        assert!(inputs
            .iter()
            .all(|s| !s.contains("AnnualRevenue") && !s.contains("Rev enue")));
        assert!(
            inputs
                .iter()
                .filter(|s| s.contains("confidential agreement Acme"))
                .count()
                >= 2
        );
        for marker in [
            "PICTURE_ALT_CANARY",
            "PICTURE_TITLE_CANARY",
            "SMARTART_ALT_CANARY",
            "SMARTART_TITLE_CANARY",
        ] {
            assert!(
                inputs.iter().any(|s| s.contains(marker)),
                "embedding input: {marker}"
            );
            assert!(
                review
                    .projection_inventory
                    .iter()
                    .any(|e| e.kind.contains("alt text")
                        && e.value.as_ref().is_some_and(|s| s.contains(marker))
                        && e.warning),
                "private inventory: {marker}"
            );
        }
        assert!(review
            .projection_inventory
            .iter()
            .filter(|e| e
                .value
                .as_ref()
                .is_some_and(|s| s.contains("confidential agreement Acme")))
            .all(|e| e.warning));
        assert!(review.metadata_inventory.iter().any(|e| e.warning
            && e.value
                .as_ref()
                .is_some_and(|s| s.contains("confidential agreement Acme"))));
        let mapping = guard.source_mapping(&current).unwrap();
        assert!(review
            .projection_inventory
            .iter()
            .any(|e| e.location.starts_with(&mapping["private-source.docx"])));
        drop(inputs);
        guard.accept(&current).unwrap();
        let (payload, _) = guard.reviewed_payload(&current).unwrap();
        crate::vector_payload::verify(&payload).unwrap();
        assert_eq!(crate::verify(&payload).unwrap()["originals"], false);
        assert!(!payload.join("originals-0001.zip").exists());
        crate::review_transfer::package(&guard, &current, &payload, &f.root.join("delivery"))
            .unwrap();
        assert_eq!(
            std::fs::read(f.root.join("source/private-source.docx")).unwrap(),
            source_bytes
        );
        assert!(review
            .metadata_inventory
            .iter()
            .filter_map(|e| e.value.as_ref())
            .all(|s| !s.contains("private-source")));
    }

    #[test]
    #[cfg(not(feature = "listing-originals"))]
    fn projection_v2_embedding_inputs_and_seller_hidden_choice() {
        let f = Fixture::new();
        index_poisoned_projection(
            &f,
            "html",
            include_bytes!("../../vz-core/tests/fixtures/projection-v2.html"),
        );
        index_poisoned_projection(
            &f,
            "pdf",
            include_bytes!("../../vz-core/tests/fixtures/projection-v2.pdf"),
        );
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
        let review = guard.inspect(&first).unwrap();
        let inputs = f.engine.texts.lock().unwrap().join("\n");
        assert!(
            inputs.contains("VISIBLE_PDF")
                && inputs.contains("INVISIBLE_PDF")
                && inputs.contains("OFFPAGE_PDF")
        );
        assert!(inputs.contains("STYLESHEET_CONTENT"));
        assert!(
            !inputs.contains("CANARY")
                && !inputs.contains("_HIDDEN")
                && !inputs.contains("INDEX_POISON")
        );
        assert!(review
            .projection_inventory
            .iter()
            .any(|e| e.kind.contains("off-page")));
        assert!(review
            .warnings
            .iter()
            .any(|s| s.contains("no-text-pages: 1")));
        guard.accept(&first).unwrap();
        changes.origin = Some(first.clone());
        changes.include_hidden = true;
        f.engine.texts.lock().unwrap().clear();
        let second = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap();
        assert_ne!(first.generation, second.generation);
        assert!(guard.check_accepted(&first).is_err());
        let inputs = f.engine.texts.lock().unwrap().join("\n");
        for hidden in [
            "ATTR_HIDDEN",
            "DISPLAY_HIDDEN",
            "VISIBILITY_HIDDEN",
            "TEMPLATE_HIDDEN",
            "INHERITED_HIDDEN",
            "ESCAPED_HIDDEN",
        ] {
            assert!(inputs.contains(hidden), "{hidden}");
        }
        assert!(!inputs.contains("CANARY") && !inputs.contains("INDEX_POISON"));
        guard.accept(&second).unwrap();
        let (payload, _) = guard.reviewed_payload(&second).unwrap();
        crate::vector_payload::verify(&payload).unwrap();
        assert_eq!(crate::verify(&payload).unwrap()["originals"], false);
        for entry in guard.inspect(&second).unwrap().metadata_inventory {
            assert!(!entry.value.unwrap_or_default().contains("CANARY"));
        }
    }

    #[test]
    #[cfg(not(feature = "listing-originals"))]
    fn projection_hidden_seller_choice_and_edit_generation_binding() {
        let f = Fixture::new();
        projection_fixture(&f);
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
        guard.accept(&first).unwrap();
        f.engine.texts.lock().unwrap().clear();
        changes.include_hidden = true;
        let second = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap();
        assert!(guard.check_accepted(&first).is_err());
        assert!(f
            .engine
            .texts
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.contains("HIDDEN_CANARY")));
        let review = guard.inspect(&second).unwrap();
        for marker in ["DELETED_ROW_CANARY", "DELETED_CELL_CANARY"] {
            assert!(f
                .engine
                .texts
                .lock()
                .unwrap()
                .iter()
                .all(|s| !s.contains(marker)));
            assert!(review
                .metadata_inventory
                .iter()
                .filter_map(|e| e.value.as_ref())
                .all(|s| !s.contains(marker)));
        }

        let hidden = review
            .projection_inventory
            .iter()
            .find(|e| e.value.as_deref() == Some("HIDDEN_CANARY"))
            .unwrap();
        let key = hidden.location.clone();
        changes.origin = Some(second.clone());
        changes
            .edits
            .insert(key.clone(), "Reviewed hidden replacement ".repeat(600));
        f.engine.texts.lock().unwrap().clear();
        let third = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap();
        assert!(f
            .engine
            .texts
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.contains("Reviewed hidden replacement")));
        assert!(guard
            .inspect(&third)
            .unwrap()
            .projection_inventory
            .iter()
            .any(|e| e.location == key && e.value.as_ref() == changes.edits.get(&key)));
        assert!(
            build(
                &guard,
                &f.root.join("source"),
                id,
                &f.engine,
                "query",
                &changes
            )
            .is_err(),
            "stale origin cannot edit another generation"
        );
        changes.origin = Some(third);
        changes.edits.clear();
        changes.exclusions.insert(key);
        f.engine.texts.lock().unwrap().clear();
        let fourth = build(
            &guard,
            &f.root.join("source"),
            id,
            &f.engine,
            "query",
            &changes,
        )
        .unwrap();
        assert!(f
            .engine
            .texts
            .lock()
            .unwrap()
            .iter()
            .all(|s| !s.contains("HIDDEN_CANARY")));
        guard.accept(&fourth).unwrap();
    }

    #[test]
    #[cfg(not(feature = "listing-originals"))]
    fn legacy_originals_generation_refused_at_package_and_transfer() {
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
        let legacy = f.root.join("legacy");
        std::fs::create_dir(&legacy).unwrap();
        for name in crate::vector_payload::FILES {
            put(&legacy, name, &std::fs::read(payload.join(name)).unwrap()).unwrap();
        }
        put(&legacy, "originals-0001.zip", b"legacy source bytes").unwrap();
        let mut manifest = crate::verify(&legacy).unwrap();
        manifest["originals"] = json!(true);
        manifest["files"]["originals-0001.zip"] = json!(sha256(b"legacy source bytes"));
        std::fs::write(
            legacy.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let mut review = guard.inspect(&current).unwrap();
        review.accepted_review_hash = None;
        review.completed_transfer_fingerprint = None;
        let legacy_current = guard
            .freeze_sanitized(
                guard.capture(&f.root.join("source")).unwrap(),
                review,
                &legacy,
                json!({"identity_terms":[]}),
            )
            .unwrap();
        guard.accept(&legacy_current).unwrap(); // simulate a locally accepted pre-upgrade generation
        assert!(guard.reviewed_payload(&legacy_current).is_err());
        assert!(crate::review_transfer::package(
            &guard,
            &legacy_current,
            &legacy,
            &f.root.join("refused")
        )
        .is_err());
        assert!(
            crate::review_transfer::check_input(&guard, &legacy_current, &legacy, &legacy).is_err()
        );
        assert!(!f.root.join("refused").exists());
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
