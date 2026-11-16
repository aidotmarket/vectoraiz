//! Default listing build: projection text only, with fresh embeddings.
use super::*;
use std::time::{Duration, Instant};
use vz_core::office::projection::{Projection, VERSION};

pub(super) fn project(
    name: &str,
    kind: &str,
    bytes: &[u8],
    cancel: Option<tokio_util::sync::CancellationToken>,
    started: Instant,
    spent: Duration,
) -> Result<Projection> {
    let remaining = Duration::from_secs(1200).saturating_sub(started.elapsed().max(spent));
    ensure!(
        !remaining.is_zero(),
        "PROJECTION_PACK_DEADLINE_EXCEEDED: 1200 seconds"
    );
    let root = std::env::temp_dir()
        .canonicalize()?
        .join(format!("vz-projection-{}", Uuid::new_v4()));
    private_directory(&root)?;
    let result = (|| {
        let input = root.join("input.bin");
        let mut file = private_file(&input)?;
        file.write_all(bytes)?;
        drop(file);
        let request = vz_core::worker::Request {
            path: input,
            pdfium: None,
            office: None,
            readback: None,
            projection: Some(vz_core::worker::ProjectionJob {
                kind: kind.into(),
                output: root.clone(),
            }),
        };
        let document_start = Instant::now();
        let result = vz_core::worker::run_cancellable(
            &vz_core::worker::executable()?,
            &request,
            remaining.min(Duration::from_secs(120)),
            || {
                ensure!(
                    !cancel.as_ref().is_some_and(|c| c.is_cancelled()),
                    "projection cancelled"
                );
                ensure!(
                    started.elapsed().max(spent + document_start.elapsed())
                        < Duration::from_secs(1200),
                    "PROJECTION_PACK_DEADLINE_EXCEEDED at {name}: 1200 seconds"
                );
                Ok(())
            },
        );
        result.map_err(|e| {
            anyhow::anyhow!(
                "PROJECTION_WORKER_REFUSAL at {name}: {}",
                format!("{e:#}").replace("OFFICE_", "PROJECTION_")
            )
        })?;
        let projection: Projection =
            serde_json::from_slice(&std::fs::read(root.join("projection.json"))?)?;
        ensure!(
            projection.version == VERSION,
            "projection worker capability/version mismatch"
        );
        Ok(projection)
    })();
    let cleanup = std::fs::remove_dir_all(&root);
    if result.is_ok() {
        cleanup?;
    }
    result
}

fn readback(
    name: &str,
    bytes: &[u8],
    terms: &[String],
    pack: &NativePackReview<'_>,
    started: Instant,
    spent: Duration,
) -> Result<Vec<sanitize::Entry>> {
    let remaining = Duration::from_secs(1200).saturating_sub(started.elapsed().max(spent));
    ensure!(
        !remaining.is_zero(),
        "PROJECTION_PACK_DEADLINE_EXCEEDED: 1200 seconds"
    );
    let root = std::env::temp_dir()
        .canonicalize()?
        .join(format!("vz-readback-{}", Uuid::new_v4()));
    private_directory(&root)?;
    let result = (|| {
        let input = root.join("input.bin");
        let mut file = private_file(&input)?;
        file.write_all(bytes)?;
        drop(file);
        let request = vz_core::worker::Request {
            path: input,
            pdfium: None,
            office: None,
            projection: None,
            readback: Some(vz_core::worker::ReadbackJob {
                name: name.into(),
                output: root.clone(),
                identity_terms: terms.to_vec(),
            }),
        };
        vz_core::worker::run_cancellable(
            &vz_core::worker::executable()?,
            &request,
            remaining.min(Duration::from_secs(120)),
            || {
                pack.check_policy()?;
                ensure!(
                    started.elapsed() < Duration::from_secs(1200),
                    "PROJECTION_PACK_DEADLINE_EXCEEDED: 1200 seconds"
                );
                Ok(())
            },
        )?;
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("inventory.json"))?)?;
        ensure!(
            report["version"] == 1,
            "generated readback worker version mismatch"
        );
        Ok(serde_json::from_value(report["inventory"].clone())?)
    })();
    let cleanup = std::fs::remove_dir_all(&root);
    if result.is_ok() {
        cleanup?;
    }
    result
}

pub(super) fn build(
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
        .map(|c| pack.edit_origin(c))
        .transpose();
    pack.begin_rebuild()?;
    changes.validate()?;
    let origin = origin?;
    let binding = pack.capture(source)?;
    let recipe: vz_core::engine::Recipe =
        serde_json::from_slice(&std::fs::read(pack.index_path().join("recipe.json"))?)?;
    ensure!(
        recipe == *engines.recipe(),
        "embedding recipe mismatch; re-index required"
    );
    let indexed = State::review_documents(pack.index_path())?;
    let identity_key = pack.identity_key()?;
    let work = pack.work()?;
    let result = (|| {
        let started = Instant::now();
        let mut spent = Duration::ZERO;
        let mut expanded = 0u64;
        let mut parts = 0usize;
        let mut entries = 0usize;
        let mut docs = Vec::new();
        let mut mapping = BTreeMap::new();
        let mut document_ids = BTreeMap::new();
        let mut projections = BTreeMap::new();
        let mut projection_warnings = Vec::new();
        let mut projection_inventory = Vec::new();
        let mut budget = Budget::default();
        let source = source.canonicalize()?;
        let mut applied = std::collections::BTreeSet::new();
        let mut excluded_targets = std::collections::BTreeSet::new();
        for mut doc in indexed {
            pack.check_policy()?;
            let relative = Path::new(&doc.path)
                .strip_prefix(&source)
                .context("indexed document outside reviewed source")?
                .to_str()
                .context("UTF-8 source")?
                .replace('\\', "/");
            if changes.exclusions.contains(&relative) {
                excluded_targets.insert(relative);
                continue;
            }
            let mut identity = Hmac::<Sha256>::new_from_slice(&identity_key)?;
            identity.update(doc.doc_id.as_bytes());
            let public_id = format!("{:x}", identity.finalize().into_bytes());
            let location = format!("projection/{public_id}");
            if changes.exclusions.contains(&location) {
                excluded_targets.insert(location);
                continue;
            }
            let bytes = source_bytes(&doc, 512_000_000)?;
            let document_start = Instant::now();
            let mut projection = project(
                &location,
                &doc.kind,
                &bytes,
                Some(pack.cancellation_token()),
                started,
                spent,
            )?;
            spent += document_start.elapsed();
            expanded += projection.expanded_bytes;
            parts += projection.parts;
            entries += projection.nodes.len() + projection.omissions.len();
            ensure!(
                expanded <= 2 * 1024 * 1024 * 1024,
                "PROJECTION_PACK_BYTES_EXCEEDED: {expanded} / 2147483648"
            );
            ensure!(
                parts <= 100_000,
                "PROJECTION_PACK_PARTS_EXCEEDED: {parts} / 100000"
            );
            ensure!(
                entries <= 20_000_000,
                "PROJECTION_PACK_INVENTORY_EXCEEDED: {entries} / 20000000"
            );
            let mut text = String::new();
            let mut previous_flow: Option<String> = None;
            let mut nodes = BTreeMap::new();
            for node in &mut projection.nodes {
                let key = format!("{location}/{}", node.location);
                let source_sha256 = sha256(node.text.as_bytes());
                if let Some((_, private)) = &origin {
                    let previous = &private["projections"][&public_id]["nodes"][&key];
                    if !previous.is_null() {
                        ensure!(
                            previous["source_sha256"].as_str() == Some(source_sha256.as_str()),
                            "projection source node changed; fresh review required"
                        );
                        node.text = previous["text"]
                            .as_str()
                            .context("prior projection text")?
                            .into();
                    }
                }
                let prior_sha256 = sha256(node.text.as_bytes());
                let excluded = changes.exclusions.contains(&key);
                if excluded {
                    let (_, private) = origin
                        .as_ref()
                        .context("projection exclusions require originating generation/hash")?;
                    ensure!(
                        private["projections"][&public_id]["nodes"][&key]["prior_sha256"].as_str()
                            == Some(prior_sha256.as_str()),
                        "stale/ambiguous projection exclusion target"
                    );
                    excluded_targets.insert(key.clone());
                }
                if let Some(value) = changes.edits.get(&key) {
                    let (_, private) = origin
                        .as_ref()
                        .context("projection edits require originating generation/hash")?;
                    ensure!(
                        private["projection_version"] == VERSION
                            && private["document_ids"][&doc.doc_id].as_str()
                                == Some(public_id.as_str()),
                        "projection document identity mismatch"
                    );
                    ensure!(
                        private["projections"][&public_id]["nodes"][&key]["prior_sha256"].as_str()
                            == Some(prior_sha256.as_str()),
                        "stale/ambiguous projection node or prior value"
                    );
                    node.text = value.clone();
                    applied.insert(key.clone());
                }
                let included = !excluded && (!node.hidden || changes.include_hidden);
                nodes.insert(key.clone(), json!({"source_sha256":source_sha256,"prior_sha256":sha256(node.text.as_bytes()),"text":node.text,"kind":node.kind,"hidden":node.hidden,"included":included}));
                if node.hidden {
                    projection_warnings.push(format!(
                        "{key}: hidden {}",
                        if included { "included" } else { "omitted" }
                    ));
                }
                projection_inventory.push(sanitize::Entry {
                    location: key.clone(),
                    kind: format!(
                        "projection {}{}: {}",
                        node.kind,
                        if node.hidden { " (hidden)" } else { "" },
                        if included { "included" } else { "omitted" }
                    ),
                    value: Some(node.text.clone()),
                    warning: node.hidden
                        || node.kind.starts_with("notes")
                        || node.kind.ends_with("alt text")
                        || changes.identity_terms.iter().any(|t| {
                            !t.trim().is_empty()
                                && node.text.to_lowercase().contains(&t.trim().to_lowercase())
                        }),
                });
                if included {
                    let flow = node
                        .location
                        .rsplit_once("/flow[")
                        .map(|(path, _)| path.to_string());
                    if !text.is_empty() && (flow.is_none() || flow != previous_flow) {
                        text.push('\n');
                    }
                    text.push_str(&node.text);
                    previous_flow = flow;
                }
            }
            for (node, reason) in &projection.omissions {
                projection_inventory.push(sanitize::Entry {
                    location: format!("{location}/{node}"),
                    kind: format!("mandatory omission: {reason}"),
                    value: None,
                    warning: false,
                });
            }
            for (category, count) in &projection.drops {
                projection_warnings.push(format!("{location}: mandatory drop {category}: {count}"));
            }
            projection_warnings.push(format!(
                "{location}: hidden nodes: {}",
                projection.nodes.iter().filter(|n| n.hidden).count()
            ));
            projection_warnings.extend(
                projection
                    .warnings
                    .iter()
                    .map(|w| format!("{location}/{w}")),
            );
            projections.insert(
                public_id.clone(),
                json!({"nodes":nodes,"omissions":projection.omissions,"drops":projection.drops}),
            );
            mapping.insert(relative, location.clone());
            document_ids.insert(doc.doc_id.clone(), public_id.clone());
            doc.doc_id = public_id;
            doc.sha256 = sha256(text.as_bytes());
            doc.bytes = text.len() as u64;
            doc.chunks = chunk(
                &doc.doc_id,
                &[Page {
                    page: 1,
                    text,
                    needs_ocr: false,
                }],
            );
            if doc.chunks.is_empty() {
                projection_warnings.push(format!("{location}: all projected text omitted by the hidden-content choice or explicit node exclusions; no chunks delivered"));
            }
            for c in &mut doc.chunks {
                pack.check_policy()?;
                c.embedding = engines.vector(&c.text, false)?;
            }
            // Source commitment remains in the private fence. Delivered metadata
            // describes the projection with neutral names and no source hash.
            doc.pages = 1;
            doc.path = format!("document-{:04}", docs.len() + 1);
            doc.kind = "projection-v1".into();
            docs.push(doc);
            ensure!(
                started.elapsed().max(spent) < Duration::from_secs(1200),
                "PROJECTION_PACK_DEADLINE_EXCEEDED: 1200 seconds"
            );
        }
        for key in &changes.exclusions {
            ensure!(
                excluded_targets.contains(key),
                "exclusion target missing or ambiguous: {key}"
            );
        }
        for key in changes.edits.keys().filter(|key| !applied.contains(*key)) {
            ensure!(
                !key.starts_with("projection/"),
                "projection edit target missing/excluded"
            );
            ensure!(
                key == "README.md"
                    || key == "examples/query.py"
                    || key == "examples/query.json/query"
                    || ((key.starts_with("documents.parquet/column/")
                        || key.starts_with("chunks.parquet/column/"))
                        && key.ends_with("/name"))
                    || (key.starts_with("documents.parquet/id/") && key.ends_with("/column/2"))
                    || (key.starts_with("chunks.parquet/id/") && key.ends_with("/column/4")),
                "edit location has no projection rebuild handler"
            );
            if key.starts_with("documents.parquet/id/") || key.starts_with("chunks.parquet/id/") {
                let (review, _) = origin
                    .as_ref()
                    .context("derived edits require originating generation/hash")?;
                ensure!(
                    review.metadata_inventory.iter().any(|e| &e.location == key),
                    "derived edit absent from originating generation"
                );
                if key.starts_with("chunks.parquet/id/") {
                    let chunk_id = key
                        .strip_prefix("chunks.parquet/id/")
                        .and_then(|s| s.strip_suffix("/column/4"))
                        .context("chunk heading selector")?;
                    let current = docs
                        .iter()
                        .flat_map(|d| &d.chunks)
                        .find(|c| c.chunk_id == chunk_id)
                        .context("chunk identity no longer exists")?;
                    let text_key = format!("chunks.parquet/id/{chunk_id}/column/2");
                    ensure!(review.metadata_inventory.iter().any(|e| e.location==text_key && e.value.as_deref()==Some(current.text.as_str())), "chunk identity changed after rechunking; review again before editing headings");
                }
            }
        }
        ensure!(!docs.is_empty(), "no supported reviewed documents remain");
        let chunks = docs
            .iter()
            .flat_map(|d| d.chunks.clone())
            .collect::<Vec<_>>();
        ensure!(!chunks.is_empty(), "no included sale text; include supported hidden content or explicitly exclude these documents");
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
        let recipe_description = format!("\nParquet schema (column order; editable display names may differ):\ndocuments: neutral document ID, neutral projection reference, reviewed title, projection type, projection-text SHA256, language (und means unknown), projected pages, projected UTF-8 bytes.\nchunks: chunk ID, neutral document ID, reviewed text, projected page, heading path (JSON array), token count, float32 embedding vector.\nJoin on doc_id. References identify projections, not downloadable source files. Projection text is flattened to one page.\n\nEmbedding recipe (including model, normalization and dimensions):\n```json\n{}\n```\n", serde_json::to_string_pretty(&recipe)?);
        for (name, bytes) in [("README.md", b"# vectorAIz vector pack\n\nLoad the Parquet files with Apache Arrow. The seller attests: chunks, embeddings and metadata; no source files are delivered. Text is customer data; vectors are not anonymization. Original names and file properties are absent. Source projections omit deletions, instructions, comments, properties and formulas. Spreadsheet cells use saved cached values only; formulas are never evaluated. Numeric dates retain Excel serial numbers; cell number formats are not applied. Worksheet header/footer text is dropped and counted privately. Hidden content is omitted unless the seller includes it. No OCR or resource fetching. PDF/HTML are unsupported in V1. Language and date ranges are unknown.\nRun examples/query.py with Python, numpy and pyarrow.\n".as_slice()), ("examples/query.py", include_bytes!("../../../scripts/pack-query.py").as_slice())] {
            let bytes = if name == "README.md" { [bytes, recipe_description.as_bytes()].concat() } else { bytes.to_vec() };
            let rebuilt = sanitize::rewrite(name, &bytes, changes, &mut mapping, &mut budget)?;
            if name=="README.md" {
                let readme=std::str::from_utf8(&rebuilt)?;
                ensure!(readme.contains(crate::vector_payload::ATTESTATION) && readme.contains(&recipe_description) && readme.contains("Hidden content is omitted") && readme.contains("cached values only"), "README edit must retain the fixed vector attestation, embedding recipe and extraction limitations");
            }
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
        let mut manifest = json!({"format_version":1,"pack_id":pack_id,"created_at":chrono::Utc::now().to_rfc3339(),"counts":{"documents":docs.len(),"chunks":chunks.len()},"languages":null,"date_ranges":null,"embedding_recipe":recipe,"files":hashes,"hash_scope":"all payload files; manifest.json excludes itself","originals":false});
        manifest.sort_all_objects();
        put(
            &work,
            "manifest.json",
            &serde_json::to_vec_pretty(&manifest)?,
        )?;
        crate::vector_payload::verify(&work)?;
        let mut inventory = Vec::new();
        for entry in walkdir::WalkDir::new(&work).sort_by_file_name() {
            let entry = entry?;
            if entry.file_type().is_file() {
                pack.check_policy()?;
                ensure!(
                    started.elapsed().max(spent) < Duration::from_secs(1200),
                    "PROJECTION_PACK_DEADLINE_EXCEEDED: 1200 seconds"
                );
                let name = entry
                    .path()
                    .strip_prefix(&work)?
                    .to_str()
                    .context("UTF-8 readback path")?
                    .replace('\\', "/");
                let read_start = Instant::now();
                let bytes = std::fs::read(entry.path())?;
                expanded += bytes.len() as u64;
                ensure!(
                    expanded <= 2 * 1024 * 1024 * 1024,
                    "PROJECTION_PACK_BYTES_EXCEEDED: {expanded} / 2147483648"
                );
                let values =
                    readback(&name, &bytes, &changes.identity_terms, pack, started, spent)?;
                spent += read_start.elapsed();
                entries += values.len();
                ensure!(
                    entries <= 20_000_000,
                    "PROJECTION_PACK_INVENTORY_EXCEEDED: {entries} / 20000000"
                );
                inventory.extend(values);
            }
        }
        for (location, edit) in changes
            .edits
            .iter()
            .filter(|(k, _)| !k.starts_with("projection/"))
        {
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
        warnings.push(crate::vector_payload::ATTESTATION.into());
        warnings.extend(projection_warnings);
        warnings.sort();
        warnings.dedup();
        let review = ListingReview {
            schema_version: ReviewVersion::V1,
            pack_id,
            staged_artifacts: Vec::new(),
            metadata_inventory_schema_version: 1,
            metadata_inventory: inventory,
            projection_inventory,
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
            json!({"mapping":mapping,"document_ids":document_ids,"identity_terms":changes.identity_terms,"projection_version":vz_core::office::projection::VERSION,"projections":projections,"include_hidden":changes.include_hidden}),
        )
    })();
    let cleanup = std::fs::remove_dir_all(&work);
    match result {
        Ok(current) => {
            cleanup?;
            Ok(current)
        }
        Err(error) => {
            let _ = cleanup;
            Err(error)
        }
    }
}
