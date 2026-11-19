//! Candidate folder scans with explicit per-file outcomes.
use crate::{
    engine::Engines,
    pipeline::{chunk, sha256, Document},
    storage::{Hit, State},
    worker,
};
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::{collections::HashSet, path::Path, time::Duration};

#[derive(Clone, Debug, Serialize, Default)]
pub struct Progress {
    pub completed: usize,
    pub failed: usize,
    pub infrastructure_failed: usize,
    pub skipped: usize,
    pub total: usize,
    pub failure_examples: Vec<String>,
    pub infrastructure_examples: Vec<String>,
    pub needs_ocr_pages: Vec<u32>,
    pub path: String,
    pub reason: Option<String>,
}
impl Progress {
    pub fn ensure_publishable(&self) -> Result<()> {
        // A completed content-only scan is reviewable even with no searchable
        // documents. Refused candidates still require explicit exclusion to build.
        anyhow::ensure!(
            self.infrastructure_failed == 0,
            "Re-index did not finish: {} indexed, {} skipped, {} failed ({} infrastructure), {} total; {}; Re-index to resume",
            self.completed, self.skipped, self.failed, self.infrastructure_failed, self.total, self.infrastructure_examples.join("; ")
        );
        Ok(())
    }
}

/// Write a complete recipe before exposing its name. An interrupted write leaves
/// only an unreferenced temporary file; retries never validate partial JSON.
pub(crate) fn initialize_recipe(
    root: &Path,
    recipe: &crate::engine::Recipe,
    before_rename: impl FnOnce() -> Result<()>,
) -> Result<()> {
    use std::{fs, io::Write};
    let path = root.join("recipe.json");
    if path.exists() {
        let stored: crate::engine::Recipe = serde_json::from_slice(&fs::read(path)?)?;
        anyhow::ensure!(
            stored == *recipe,
            "embedding recipe mismatch; re-index required"
        );
        return Ok(());
    }
    let temp = root.join(format!(".recipe-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(recipe)?)?;
        file.sync_all()?;
        drop(file);
        before_rename()?;
        fs::rename(&temp, path)?;
        #[cfg(unix)]
        fs::File::open(root)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Private retry state. Opening an unchanged scan never creates a workspace.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScanRow {
    pub doc_id: String,
    pub path: String,
    pub size: u64,
    pub mtime: String,
    pub hash: String,
    pub generation: String,
}
impl ScanRow {
    pub(crate) fn read(db: &rusqlite::Connection, table: &str) -> Result<Vec<Self>> {
        Ok(db
            .prepare(&format!(
                "SELECT doc_id,path,size,mtime,sha256,generation FROM {table} ORDER BY doc_id"
            ))?
            .query_map([], |r| {
                Ok(Self {
                    doc_id: r.get(0)?,
                    path: r.get(1)?,
                    size: r.get(2)?,
                    mtime: r.get(3)?,
                    hash: r.get(4)?,
                    generation: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub(crate) fn put(&self, db: &rusqlite::Connection) -> Result<()> {
        db.execute("INSERT INTO documents VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(doc_id) DO UPDATE SET path=excluded.path,size=excluded.size,mtime=excluded.mtime,sha256=excluded.sha256,generation=excluded.generation",
            params![self.doc_id,self.path,self.size,self.mtime,self.hash,self.generation])?;
        Ok(())
    }
}
pub(crate) struct ScanWorkspace {
    pub root: std::path::PathBuf,
    pub published_root: std::path::PathBuf,
    published_by_path: std::collections::HashMap<String, ScanRow>,
    baseline_rows: std::collections::HashMap<String, ScanRow>,
    pub state: Option<State>,
    source: String,
    recipe: String,
    reset_before_write: bool,
}
impl ScanWorkspace {
    pub fn new(
        root: &Path,
        published_root: &Path,
        published: Vec<ScanRow>,
        source: &str,
        recipe: &str,
    ) -> Result<Self> {
        let mut reuse = root.join("scan.sqlite").exists();
        if reuse {
            let matches = (|| -> Result<bool> {
                let db = rusqlite::Connection::open_with_flags(
                    root.join("scan.sqlite"),
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                Ok(db.query_row(
                    "SELECT source_identity,recipe_id FROM scan_state",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )? == (source.into(), recipe.into()))
            })()
            .unwrap_or(false);
            if !matches {
                reuse = false;
            }
        }
        let state = if reuse {
            Some(State::open_workspace(root, source, recipe)?)
        } else {
            None
        };
        Ok(Self {
            root: root.into(),
            published_root: published_root.into(),
            baseline_rows: published
                .iter()
                .map(|r| (r.path.clone(), r.clone()))
                .collect(),
            published_by_path: published
                .iter()
                .map(|r| (r.path.clone(), r.clone()))
                .collect(),
            state,
            reset_before_write: !reuse && root.join("scan.sqlite").exists(),
            source: source.into(),
            recipe: recipe.into(),
        })
    }
    pub fn matches_published(&self, row: &ScanRow) -> bool {
        self.published_by_path.get(&row.path) == Some(row)
    }
    pub fn set_baseline(&mut self, rows: Vec<ScanRow>) {
        self.baseline_rows = rows.into_iter().map(|r| (r.path.clone(), r)).collect();
    }
    fn writable(&mut self) -> Result<&mut State> {
        if self.state.is_none() {
            if self.reset_before_write {
                std::fs::remove_dir_all(&self.root)?;
            }
            self.state = Some(State::open_workspace(
                &self.root,
                &self.source,
                &self.recipe,
            )?);
        }
        Ok(self.state.as_mut().unwrap())
    }
    pub fn staged(&self) -> Result<Vec<ScanRow>> {
        self.state
            .as_ref()
            .map(|s| ScanRow::read(&s.db, "scan_documents"))
            .unwrap_or_else(|| Ok(vec![]))
    }
    pub fn retain(&self, paths: &HashSet<String>) -> Result<()> {
        if let Some(state) = &self.state {
            for row in self.staged()? {
                if !paths.contains(&row.path) {
                    state
                        .db
                        .execute("DELETE FROM scan_documents WHERE path=?1", [&row.path])?;
                }
            }
            state.recover()?;
        }
        Ok(())
    }
    async fn unchanged(&self, path: &str, hash: &str) -> Result<Option<(Document, ScanRow, bool)>> {
        let staged = if let Some(state) = &self.state {
            state.db.query_row("SELECT doc_id,path,size,mtime,sha256,generation FROM scan_documents WHERE path=?1",[path],|r| Ok(ScanRow {doc_id:r.get(0)?,path:r.get(1)?,size:r.get(2)?,mtime:r.get(3)?,hash:r.get(4)?,generation:r.get(5)?})).optional()?
        } else {
            None
        };
        for (row, root, staged) in [
            (staged, &self.root, true),
            (
                self.published_by_path.get(path).cloned(),
                &self.published_root,
                false,
            ),
        ] {
            if let Some(row) = row.filter(|r| r.hash == hash) {
                if let Some(doc) = readable_generation(root, &row, staged).await {
                    return Ok(Some((doc, row, staged)));
                }
            }
        }
        Ok(None)
    }
}

async fn readable_generation(root: &Path, row: &ScanRow, staged: bool) -> Option<Document> {
    if row.generation.is_empty()
        || !row
            .generation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }
    let generation = root.join("generations").join(&row.generation);
    let doc: Document =
        serde_json::from_slice(&std::fs::read(generation.join("document.json")).ok()?).ok()?;
    if doc.path != row.path || doc.sha256 != row.hash || !generation.join("lance").is_dir() {
        return None;
    }
    // Published generations were fully validated before publication. Check the
    // document and manifest nodes cheaply; staged retries still open the table.
    if !staged {
        let table = generation.join("lance/chunks.lance");
        let manifest = std::fs::read_dir(table.join("_versions"))
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|e| {
                Some((
                    e.path().file_stem()?.to_str()?.parse::<u64>().ok()?,
                    e.path(),
                ))
            })
            .max_by_key(|(version, _)| *version)?
            .1;
        if !readable_lance_node(&manifest) {
            return None;
        }
        let mut found_data = false;
        for entry in std::fs::read_dir(table.join("data")).ok()? {
            let path = entry.ok()?.path();
            if path.extension().is_some_and(|x| x == "lance") {
                found_data = true;
                if !readable_lance_node(&path) {
                    return None;
                }
            }
        }
        return found_data.then_some(doc);
    }
    use futures::TryStreamExt;
    use lancedb::query::{ExecutableQuery, QueryBase};
    let db = lancedb::connect(generation.join("lance").to_str()?)
        .execute()
        .await
        .ok()?;
    let table = db.open_table("chunks").execute().await.ok()?;
    let mut batches = table.query().limit(1).execute().await.ok()?;
    batches.try_next().await.ok()??;
    Some(doc)
}

// Local Lance nodes carry a LANC footer. Keep structural damage and missing
// data a cache miss without constructing a database/query for every unchanged
// published document. Staged rows still take the full table-read path below.
fn readable_lance_node(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    (|| -> std::io::Result<bool> {
        if !std::fs::symlink_metadata(path)?.is_file() {
            return Ok(false);
        }
        let mut file = std::fs::File::open(path)?;
        if file.metadata()?.len() < 16 {
            return Ok(false);
        }
        file.seek(SeekFrom::End(-4))?;
        let mut magic = [0; 4];
        file.read_exact(&mut magic)?;
        Ok(&magic == b"LANC")
    })()
    .unwrap_or(false)
}

pub async fn index(
    state: &mut State,
    folder: &Path,
    engines: &Engines,
    assets: &Path,
    progress: impl FnMut(&Progress),
) -> Result<Progress> {
    let location = state.location.clone();
    let result = crate::active_index::scan_candidate(
        &location,
        folder,
        engines,
        assets,
        progress,
        || false,
        crate::active_index::ScanOptions {
            explicit: Some(state),
            flat: true,
            control: None,
        },
    )
    .await;
    let selected = crate::active_index::resolve_state(&location);
    if selected.canonicalize().ok() != state.root.canonicalize().ok() {
        *state = State::open(&location).map_err(|error| {
            error.context(crate::active_index::PublishedFailure("state reopen"))
        })?;
    }
    result
}

/// Everything the publication decision needs; scan writes exist only in a candidate.
#[derive(Default)]
pub(crate) struct ScanOutcome {
    pub completed_walk: bool,
    pub source: Option<std::path::PathBuf>,
    pub seen: HashSet<String>,
    pub succeeded: HashSet<String>,
    pub publishable: HashSet<String>,
    pub failed_content: HashSet<String>,
    pub failed_infrastructure: HashSet<String>,
    pub stats: Progress,
    pub unavailable: bool,
    pub error: Option<anyhow::Error>,
    pub jobs: Vec<(String, String, Option<String>)>,
    pub unchanged: Vec<ScanRow>,
    pub changed: HashSet<String>,
    pub damaged: HashSet<String>,
}

pub(crate) async fn scan(
    state: &mut ScanWorkspace,
    folder: &Path,
    engines: &Engines,
    assets: &Path,
    mut progress: impl FnMut(&Progress),
) -> Result<ScanOutcome> {
    let folder = folder.to_owned();
    if !folder.is_dir() {
        return Ok(ScanOutcome {
            source: Some(folder),
            unavailable: true,
            error: Some(anyhow::anyhow!("source missing")),
            ..Default::default()
        });
    }
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(&folder).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                return Ok(ScanOutcome {
                    source: Some(folder),
                    unavailable: true,
                    error: Some(
                        anyhow::Error::new(error).context("source unavailable during scan"),
                    ),
                    ..Default::default()
                });
            }
        };
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| {
                    matches!(
                        e.to_ascii_lowercase().as_str(),
                        "pdf"
                            | "docx"
                            | "xlsx"
                            | "pptx"
                            | "txt"
                            | "md"
                            | "markdown"
                            | "html"
                            | "htm"
                    )
                })
        {
            files.push(entry.into_path());
        }
    }
    files.sort();
    let mut stats = Progress {
        total: files.len(),
        ..Default::default()
    };
    let mut outcome = ScanOutcome {
        completed_walk: true,
        source: Some(folder.clone()),
        ..Default::default()
    };
    for path in files {
        let path_text = path
            .to_str()
            .context("source path must be UTF-8")?
            .to_owned();
        outcome.seen.insert(path_text.clone());
        stats.path = path_text.clone();
        stats.reason = None;
        stats.needs_ocr_pages.clear();
        let result: Result<bool> = async {
            let metadata =
                std::fs::metadata(&path).map_err(|e| crate::failures::source_io(e.into()))?;
            let mtime = format!("{:?}", metadata.modified()?);
            // Metadata detects candidates; sha256 confirms content, including same-size edits.
            let hash = crate::pipeline::file_sha256(&path)
                .map_err(|e| crate::failures::source_io(e.into()))?;
            if state
                .baseline_rows
                .get(&path_text)
                .is_some_and(|old| old.hash != hash)
            {
                outcome.changed.insert(path_text.clone());
            }
            if let Some((doc, mut row, staged)) = state.unchanged(&path_text, &hash).await? {
                stats.needs_ocr_pages = doc.needs_ocr_pages;
                row.size = metadata.len();
                row.mtime = mtime;
                if staged {
                    outcome.publishable.insert(path_text.clone());
                    let db = &state.state.as_ref().unwrap().db;
                    db.execute(
                        "UPDATE scan_documents SET size=?1,mtime=?2 WHERE path=?3",
                        params![row.size, row.mtime, row.path],
                    )?;
                } else {
                    outcome.unchanged.push(row);
                }
                return Ok(false);
            }
            let library = assets.join(if cfg!(windows) {
                "pdfium/bin/pdfium.dll"
            } else {
                "pdfium/lib/libpdfium.dylib"
            });
            let pages = worker::run(
                &worker::executable()?,
                &worker::Request {
                    path: path.clone(),
                    office: None,
                    projection: None,
                    readback: None,
                    pdfium: if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
                    {
                        Some(library.canonicalize().context("PDFium asset missing")?)
                    } else {
                        None
                    },
                },
                Duration::from_secs(
                    if path.extension().is_some_and(|ext| {
                        ["docx", "xlsx", "pptx"]
                            .iter()
                            .any(|kind| ext.eq_ignore_ascii_case(kind))
                    }) {
                        120
                    } else {
                        60
                    },
                ),
            )?;
            if crate::pipeline::file_sha256(&path)
                .map_err(|e| crate::failures::source_io(e.into()))?
                != hash
            {
                return Err(crate::failures::content(
                    "source changed during parsing; retry indexing",
                ));
            }
            let doc_id = sha256(path_text.as_bytes());
            let mut chunks = chunk(&doc_id, &pages);
            if chunks.is_empty() {
                // A PDF can yield no usable text even after platform OCR ran.
                // Keep Page::needs_ocr's platform contract (OCR not attempted)
                // independent from the seller-facing explanation of this failure.
                let reason = if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
                    || pages.iter().any(|p| p.needs_ocr)
                {
                    "needs text recognition (OCR)"
                } else {
                    "document has no text"
                };
                return Err(crate::failures::content(reason));
            }
            for chunk in &mut chunks {
                chunk.embedding = engines.vector(&chunk.text, false)?;
            }
            let doc = Document {
                doc_id,
                path: path_text.clone(),
                sha256: hash,
                bytes: metadata.len(),
                pages: pages.len() as u32,
                kind: path.extension().unwrap().to_string_lossy().to_lowercase(),
                needs_ocr_pages: pages
                    .iter()
                    .filter(|p| p.needs_ocr)
                    .map(|p| p.page)
                    .collect(),
                chunks,
            };
            let staged = state.writable()?;
            let generation = staged.stage(&doc).await?;
            staged.commit(&doc, &mtime, &generation)?;
            stats.needs_ocr_pages = doc.needs_ocr_pages.clone();
            Ok(true)
        }
        .await;
        match result {
            Ok(changed) => {
                outcome.succeeded.insert(path_text.clone());
                if changed {
                    outcome.publishable.insert(path_text.clone());
                    stats.completed += 1;
                } else {
                    stats.skipped += 1;
                }
                if !stats.needs_ocr_pages.is_empty() {
                    stats.reason = Some(format!(
                        "needs OCR: pages {:?} skipped",
                        stats.needs_ocr_pages
                    ));
                }
                if changed {
                    outcome
                        .jobs
                        .push((path_text.clone(), "done".into(), stats.reason.clone()));
                }
            }
            Err(error) => {
                if let Some(row) = state.baseline_rows.get(&path_text) {
                    if readable_generation(&state.published_root, row, true)
                        .await
                        .is_none()
                    {
                        outcome.damaged.insert(path_text.clone());
                    }
                }
                stats.failed += 1;
                if crate::failures::is_content(&error) {
                    outcome.failed_content.insert(path_text.clone());
                } else {
                    outcome.failed_infrastructure.insert(path_text.clone());
                    stats.infrastructure_failed += 1;
                    if stats.infrastructure_examples.len() < 3 {
                        stats.infrastructure_examples.push(format!(
                            "{}: indexing infrastructure unavailable; re-index to retry",
                            path.strip_prefix(&folder)
                                .context("source outside selected folder")?
                                .display()
                        ));
                    }
                }
                let reason = format!("{error:#}");
                if stats.failure_examples.len() < 3 {
                    let name = path.strip_prefix(&folder).unwrap_or(&path).display();
                    stats.failure_examples.push(format!("{name}: {reason}"));
                }
                stats.reason = Some(reason.clone());
                outcome
                    .jobs
                    .push((path_text.clone(), "failed".into(), Some(reason)));
            }
        }
        progress(&stats);
    }
    state.retain(&outcome.publishable)?;
    outcome.stats = stats;
    Ok(outcome)
}
#[derive(Clone, Debug, Serialize)]
pub struct Citation {
    pub number: usize,
    pub doc_id: String,
    pub page: u32,
    pub chunk_id: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Answer {
    pub text: String,
    pub citations: Vec<Citation>,
}
pub fn ask(
    engines: &Engines,
    question: &str,
    hits: &[Hit],
    mut emit: impl FnMut(&str),
) -> Result<Answer> {
    if hits.is_empty() {
        return Ok(Answer {
            text: "No indexed sources answer this question.".into(),
            citations: vec![],
        });
    }
    let citations = hits
        .iter()
        .enumerate()
        .map(|(i, h)| Citation {
            number: i + 1,
            doc_id: h.chunk.doc_id.clone(),
            page: h.chunk.page,
            chunk_id: h.chunk.chunk_id.clone(),
        })
        .collect::<Vec<_>>();
    let context = hits
        .iter()
        .enumerate()
        .map(|(i, h)| format!("[{}] {}", i + 1, h.chunk.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let text = engines.stream(question, &context, &mut emit)?;
    finish_answer(text, citations, emit)
}

/// Cloud context contains passage labels and text only; no filenames or pages.
/// Drop whole low-ranked passages and keep citation numbering contiguous.
pub fn cloud_passages(hits: &[Hit]) -> (String, Vec<Citation>) {
    let mut context = String::new();
    let mut citations = Vec::new();
    for h in hits.iter().take(3) {
        let number = citations.len() + 1;
        let passage = format!("[{}] {}", number, h.chunk.text);
        let separator = if context.is_empty() { "" } else { "\n\n" };
        if context.len() + separator.len() + passage.len() > 24000 {
            break;
        }
        context.push_str(separator);
        context.push_str(&passage);
        citations.push(Citation {
            number,
            doc_id: h.chunk.doc_id.clone(),
            page: h.chunk.page,
            chunk_id: h.chunk.chunk_id.clone(),
        });
    }
    (context, citations)
}

pub(crate) fn finish_answer(
    mut text: String,
    citations: Vec<Citation>,
    mut emit: impl FnMut(&str),
) -> Result<Answer> {
    validate_citations(&text, citations.len())?;
    if !has_answer_words(&text) {
        text = "I couldn't write an answer from these sources. See the sources below.".into();
        emit(&text);
    }
    // Provenance is application-owned: small provisional models may omit inline
    // citation syntax. Always stream a source footer for the supplied context.
    let footer = format!(
        "\n\nRetrieved sources: {}",
        citations
            .iter()
            .map(|c| format!("[{}]", c.number))
            .collect::<Vec<_>>()
            .join(" ")
    );
    emit(&footer);
    text.push_str(&footer);
    // Citation metadata always maps numbered source context to real persisted pages.
    Ok(Answer { text, citations })
}

fn has_answer_words(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            // Skip citation digits even in nested or unclosed brackets, then
            // keep scanning past malformed groups for actual answer content.
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
        } else if c.is_alphanumeric() {
            return true;
        }
    }
    false
}

fn validate_citations(text: &str, count: usize) -> Result<()> {
    for part in text.split('[').skip(1) {
        if let Some((label, _)) = part.split_once(']') {
            if let Ok(number) = label.parse::<usize>() {
                if number == 0 || number > count {
                    bail!("model cited an unknown source [{number}]");
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    include!("../tests/support/image_only_pdf.rs");

    #[tokio::test]
    async fn image_only_pdf_is_a_named_content_failure_alongside_good_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("scanned-image-only.pdf"),
            image_only_pdf(),
        )
        .unwrap();
        std::fs::write(
            source.path().join("z-good.txt"),
            "A good searchable document.",
        )
        .unwrap();
        let index = tempfile::tempdir().unwrap();
        let mut state = State::open(index.path()).unwrap();
        let engines = Engines::load_embedding(
            &root.join(".assets"),
            &root.join("spike/fixtures/models.json"),
            true,
        )
        .unwrap();
        let result = super::index(
            &mut state,
            source.path(),
            &engines,
            &root.join(".assets"),
            |_| {},
        )
        .await
        .unwrap();
        eprintln!(
            "image-only PDF indexing result: {}",
            serde_json::to_string(&result).unwrap()
        );
        assert_eq!(
            (
                result.completed,
                result.failed,
                result.infrastructure_failed
            ),
            (1, 1, 0)
        );
        assert!(result.failure_examples[0]
            .contains("scanned-image-only.pdf: needs text recognition (OCR)"));
        result.ensure_publishable().unwrap();
        assert_eq!(state.documents().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn infrastructure_examples_and_reindex_errors_keep_paths_private() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("scan.pdf"), image_only_pdf()).unwrap();
        let index = tempfile::tempdir().unwrap();
        let mut state = State::open(index.path()).unwrap();
        let engines = Engines::load_embedding(
            &root.join(".assets"),
            &root.join("spike/fixtures/models.json"),
            true,
        )
        .unwrap();
        let assets = source.path().join("missing-private-assets");
        let stats = super::index(&mut state, source.path(), &engines, &assets, |_| {})
            .await
            .unwrap();
        assert_eq!(stats.infrastructure_failed, 1);
        assert_eq!(
            stats.infrastructure_examples,
            ["scan.pdf: indexing infrastructure unavailable; re-index to retry"]
        );
        let error = stats.ensure_publishable().unwrap_err().to_string();
        assert!(error.contains("scan.pdf"));
        assert!(!error.contains(source.path().to_str().unwrap()));
        assert!(!error.contains(assets.to_str().unwrap()));
        let library = tempfile::tempdir().unwrap();
        let error = crate::active_index::reindex(
            library.path(),
            &source.path().join("missing-private-folder"),
            &engines,
            &assets,
            |_| {},
            || false,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("source missing"));
        assert!(!format!("{error:#}").contains(source.path().to_str().unwrap()));
    }

    #[tokio::test]
    async fn listing_refused_office_original_still_indexes_and_is_searchable() {
        use std::io::{Cursor, Read, Write};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = tempfile::tempdir().unwrap();
        let input = include_bytes!("../../vz-pack/tests/fixtures/office-public/SampleDoc.docx");
        let mut archive = zip::ZipArchive::new(Cursor::new(input)).unwrap();
        let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..archive.len() {
            let mut part = archive.by_index(i).unwrap();
            let mut bytes = Vec::new();
            part.read_to_end(&mut bytes).unwrap();
            output
                .start_file(part.name(), zip::write::SimpleFileOptions::default())
                .unwrap();
            output.write_all(&bytes).unwrap();
        }
        output
            .start_file(
                "word/charts/chart1.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        output.write_all(b"<unsupported-chart/>").unwrap();
        let bytes = output.finish().unwrap().into_inner();
        assert!(crate::office::rebuild("document.docx", &bytes, &Default::default()).is_err());
        std::fs::write(source.path().join("document.docx"), bytes).unwrap();
        let index = tempfile::tempdir().unwrap();
        let mut state = State::open(index.path()).unwrap();
        let engines = Engines::load_embedding(
            &root.join(".assets"),
            &root.join("spike/fixtures/models.json"),
            true,
        )
        .unwrap();
        let result = super::index(
            &mut state,
            source.path(),
            &engines,
            &root.join(".assets"),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(result.completed, 1);
        assert_eq!(result.failed, 0);
        assert_eq!(result.infrastructure_failed, 0);
        let documents = state.documents().unwrap();
        assert_eq!(documents.len(), 1);
        let query = &documents[0].chunks[0].text;
        let vector = engines.vector(query, true).unwrap();
        let hits = state.search(query, &vector, 3).await.unwrap();
        assert!(hits
            .iter()
            .any(|hit| hit.chunk.doc_id == documents[0].doc_id));
    }
    #[tokio::test]
    async fn unchanged_skip_reads_only_matching_document() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let mut state = State::open(root.path()).unwrap();
        let mut generations = Vec::new();
        for name in ["first.txt", "other.txt"] {
            let path = source.path().join(name);
            std::fs::write(&path, name).unwrap();
            let path = path.to_str().unwrap();
            let doc_id = sha256(path.as_bytes());
            let pages = vec![crate::pipeline::Page {
                page: 1,
                text: name.into(),
                needs_ocr: false,
            }];
            let mut chunks = chunk(&doc_id, &pages);
            for chunk in &mut chunks {
                chunk.embedding = vec![1.0, 0.0, 0.0, 0.0];
            }
            let doc = Document {
                doc_id,
                path: path.into(),
                sha256: crate::pipeline::file_sha256(Path::new(path)).unwrap(),
                bytes: name.len() as u64,
                pages: 2,
                kind: "txt".into(),
                needs_ocr_pages: vec![2],
                chunks,
            };
            let generation = state.stage(&doc).await.unwrap();
            state.commit(&doc, "old mtime", &generation).unwrap();
            generations.push(generation);
        }
        let document_path = |generation: &str| {
            root.path()
                .join("generations")
                .join(generation)
                .join("document.json")
        };
        std::fs::write(document_path(&generations[1]), b"invalid JSON").unwrap();
        assert!(state.documents().is_err());
        let path = source.path().join("first.txt");
        let hash = crate::pipeline::file_sha256(&path).unwrap();
        let workspace = ScanWorkspace::new(
            &root.path().join(".vz-scan"),
            root.path(),
            ScanRow::read(&state.db, "documents").unwrap(),
            source.path().to_str().unwrap(),
            "recipe",
        )
        .unwrap();
        let skipped = workspace
            .unchanged(path.to_str().unwrap(), &hash)
            .await
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(skipped.path, path.to_str().unwrap());
        assert_eq!(skipped.needs_ocr_pages, [2]);
        let mtime: String = state
            .db
            .query_row(
                "SELECT mtime FROM documents WHERE path=?1",
                [path.to_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            mtime, "old mtime",
            "scanning never updates published metadata"
        );
        assert!(workspace
            .unchanged(path.to_str().unwrap(), "changed hash")
            .await
            .unwrap()
            .is_none());
        // Damaged matching storage is a cache miss and gets re-indexed (R9).
        std::fs::write(document_path(&generations[0]), b"invalid JSON").unwrap();
        assert!(workspace
            .unchanged(path.to_str().unwrap(), &hash)
            .await
            .unwrap()
            .is_none());
        std::fs::remove_file(document_path(&generations[0])).unwrap();
        assert!(workspace
            .unchanged(path.to_str().unwrap(), &hash)
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn interrupted_initial_recipe_write_retries_without_cleanup() {
        let data = tempfile::tempdir().unwrap();
        let pins =
            serde_json::from_str(include_str!("../../../spike/fixtures/models.json")).unwrap();
        let recipe = crate::engine::Recipe::from_pins(&pins, 384).unwrap();
        let legacy = data.path().join("phase0-index");
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("recipe.json"), b"legacy unchanged").unwrap();
        let root = crate::active_index::recipe_dir(data.path(), &recipe).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        assert!(initialize_recipe(&root, &recipe, || {
            // Model a kill during serialization: only the temporary name exists.
            let temp = std::fs::read_dir(&root)?.next().unwrap()?.path();
            std::fs::write(temp, b"{\"model\":")?;
            anyhow::bail!("interrupted recipe creation")
        })
        .is_err());
        assert!(!root.join("recipe.json").exists());
        // A hard kill can also leave an orphan; retry needs no manual cleanup.
        std::fs::write(root.join(".recipe-killed.tmp"), b"{").unwrap();
        initialize_recipe(&root, &recipe, || Ok(())).unwrap();
        assert_eq!(
            serde_json::from_slice::<crate::engine::Recipe>(
                &std::fs::read(root.join("recipe.json")).unwrap()
            )
            .unwrap(),
            recipe
        );
        assert_eq!(
            std::fs::read(legacy.join("recipe.json")).unwrap(),
            b"legacy unchanged"
        );
        assert!(!data.path().join("active-index.json").exists());
    }

    #[test]
    fn refuses_unknown_model_citations() {
        assert!(validate_citations("supported [1]", 1).is_ok());
        assert!(validate_citations("invented [2]", 1).is_err());
        assert!(validate_citations("invalid [0]", 3).is_err());
    }

    #[test]
    fn falls_back_when_model_writes_no_answer_words() {
        for body in [
            "[1]",
            "",
            " \n[1] [2]\t",
            "[1] ...",
            "[] [1]",
            "[[1]]",
            "[1",
            "]",
            "[ ] [2]",
        ] {
            let citations = vec![
                Citation {
                    number: 1,
                    doc_id: "orbital".into(),
                    page: 2,
                    chunk_id: "first".into(),
                },
                Citation {
                    number: 2,
                    doc_id: "invoice".into(),
                    page: 1,
                    chunk_id: "second".into(),
                },
            ];
            let expected_citations = serde_json::to_value(&citations).unwrap();
            let mut emitted = String::new();
            let answer = finish_answer(body.into(), citations, |s| emitted.push_str(s)).unwrap();
            assert_eq!(answer.text, "I couldn't write an answer from these sources. See the sources below.\n\nRetrieved sources: [1] [2]");
            assert_eq!(emitted, answer.text);
            assert_eq!(
                serde_json::to_value(answer.citations).unwrap(),
                expected_citations
            );
        }
        for body in ["Thirty days [1].", "三十天 [1]", "30 [1]"] {
            assert!(has_answer_words(body));
        }
        assert!(finish_answer("[3]".into(), vec![], |_| {}).is_err());
    }
}
