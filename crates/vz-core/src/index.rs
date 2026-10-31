//! Folder scan and resumable per-file jobs.
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
        anyhow::ensure!(
            self.infrastructure_failed == 0 && (self.completed + self.skipped > 0 || self.total == 0),
            "Re-index did not finish: {} indexed, {} skipped, {} failed ({} infrastructure), {} total; {}; Re-index to resume",
            self.completed, self.skipped, self.failed, self.infrastructure_failed, self.total, if self.infrastructure_failed > 0 { &self.infrastructure_examples } else { &self.failure_examples }.join("; ")
        );
        Ok(())
    }
}

/// Write a complete recipe before exposing its name. An interrupted write leaves
/// only an unreferenced temporary file; retries never validate partial JSON.
fn initialize_recipe(
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

/// Read only the matching generation when an unchanged file can be skipped.
fn unchanged_document(
    state: &State,
    path: &str,
    size: u64,
    mtime: &str,
    hash: &str,
) -> Result<Option<Document>> {
    let previous: Option<(u64, String, String, String)> = state
        .db
        .query_row(
            "SELECT size,mtime,sha256,generation FROM documents WHERE path=?1",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    if let Some((old_size, old_mtime, old_hash, generation)) = previous {
        if old_hash == hash {
            if old_size != size || old_mtime != mtime {
                state.db.execute(
                    "UPDATE documents SET size=?1,mtime=?2 WHERE path=?3",
                    params![size, mtime, path],
                )?;
            }
            return Ok(Some(state.document(&generation)?));
        }
    }
    Ok(None)
}

pub async fn index(
    state: &mut State,
    folder: &Path,
    engines: &Engines,
    assets: &Path,
    mut progress: impl FnMut(&Progress),
) -> Result<Progress> {
    let unavailable = |state: &State| -> Result<()> {
        state.db.execute("DELETE FROM documents", [])?;
        state.recover()?;
        Ok(())
    };
    let folder = match folder.canonicalize() {
        Ok(folder) if folder.is_dir() => folder,
        _ => {
            unavailable(state)?;
            bail!("source unavailable");
        }
    };
    initialize_recipe(&state.root, &engines.recipe, || Ok(()))?;
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(&folder).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                unavailable(state)?;
                return Err(error).context("source unavailable during scan");
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
                        "pdf" | "docx" | "xlsx" | "txt" | "md" | "markdown" | "html" | "htm"
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
    let mut seen = HashSet::new();
    for path in files {
        let path_text = path
            .to_str()
            .context("source path must be UTF-8")?
            .to_owned();
        seen.insert(path_text.clone());
        stats.path = path_text.clone();
        stats.reason = None;
        stats.needs_ocr_pages.clear();
        state.db.execute(
            "INSERT INTO jobs(path,status) VALUES(?1,'running')",
            [&path_text],
        )?;
        let job = state.db.last_insert_rowid();
        let result: Result<bool> = async {
            let metadata =
                std::fs::metadata(&path).map_err(|e| crate::failures::source_io(e.into()))?;
            let mtime = format!("{:?}", metadata.modified()?);
            // Metadata detects candidates; sha256 confirms content, including same-size edits.
            let hash = crate::pipeline::file_sha256(&path)
                .map_err(|e| crate::failures::source_io(e.into()))?;
            if let Some(doc) = unchanged_document(state, &path_text, metadata.len(), &mtime, &hash)?
            {
                stats.needs_ocr_pages = doc.needs_ocr_pages;
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
                    pdfium: if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
                    {
                        Some(library.canonicalize().context("PDFium asset missing")?)
                    } else {
                        None
                    },
                },
                Duration::from_secs(60),
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
                let reason = if pages.iter().any(|p| p.needs_ocr) {
                    "needs OCR"
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
            let generation = state.stage(&doc).await?;
            crate::Store::put(state, &doc, &mtime, &generation).map_err(anyhow::Error::msg)?;
            stats.needs_ocr_pages = doc.needs_ocr_pages.clone();
            Ok(true)
        }
        .await;
        match result {
            Ok(changed) => {
                if changed {
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
                state.db.execute(
                    "UPDATE jobs SET status='done',reason=?1 WHERE id=?2",
                    params![stats.reason, job],
                )?;
            }
            Err(error) => {
                stats.failed += 1;
                if !crate::failures::is_content(&error) {
                    stats.infrastructure_failed += 1;
                    if stats.infrastructure_examples.len() < 3 {
                        stats
                            .infrastructure_examples
                            .push(format!("{path_text}: {error:#}"));
                    }
                }
                let reason = format!("{error:#}");
                if stats.failure_examples.len() < 3 {
                    stats
                        .failure_examples
                        .push(format!("{path_text}: {reason}"));
                }
                stats.reason = Some(reason.clone());
                // Never serve an older revision when replacement failed.
                state
                    .db
                    .execute("DELETE FROM documents WHERE path=?1", [&path_text])?;
                state.db.execute(
                    "UPDATE jobs SET status='failed',reason=?1 WHERE id=?2",
                    params![reason, job],
                )?;
            }
        }
        progress(&stats);
    }
    // This spike indexes one source folder per index. A subsequent successful scan
    // revokes deleted files; unreadable scans fail before querying is offered by the UI.
    let indexed: Vec<String> = state
        .db
        .prepare("SELECT path FROM documents")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for path in indexed {
        if !seen.contains(&path) {
            state
                .db
                .execute("DELETE FROM documents WHERE path=?1", [&path])?;
        }
    }
    state.recover()?;
    Ok(stats)
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

fn finish_answer(
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
        let skipped = unchanged_document(&state, path.to_str().unwrap(), 9, "new mtime", &hash)
            .unwrap()
            .expect("unchanged file must be skipped despite unrelated corruption");
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
        assert_eq!(mtime, "new mtime");
        assert!(unchanged_document(
            &state,
            path.to_str().unwrap(),
            9,
            "new mtime",
            "changed hash"
        )
        .unwrap()
        .is_none());
        // Corruption of the matching generation still fails as infrastructure.
        std::fs::write(document_path(&generations[0]), b"invalid JSON").unwrap();
        let error =
            unchanged_document(&state, path.to_str().unwrap(), 9, "new mtime", &hash).unwrap_err();
        assert!(!crate::failures::is_content(&error));
        std::fs::remove_file(document_path(&generations[0])).unwrap();
        let error =
            unchanged_document(&state, path.to_str().unwrap(), 9, "new mtime", &hash).unwrap_err();
        assert!(!crate::failures::is_content(&error));
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
