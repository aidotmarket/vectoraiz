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
    pub skipped: usize,
    pub total: usize,
    pub path: String,
    pub reason: Option<String>,
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
    let recipe_path = state.root.join("recipe.json");
    if recipe_path.exists() {
        let recipe: crate::engine::Recipe = serde_json::from_slice(&std::fs::read(&recipe_path)?)?;
        if recipe != engines.recipe {
            bail!("embedding recipe changed; use a new index and re-index the source");
        }
    } else {
        let file = std::fs::File::create(&recipe_path)?;
        serde_json::to_writer_pretty(&file, &engines.recipe)?;
        file.sync_all()?;
    }
    let supported = [
        "pdf", "docx", "xlsx", "txt", "md", "markdown", "html", "htm",
    ];
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
            && supported.contains(
                &entry
                    .path()
                    .extension()
                    .and_then(|v| v.to_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .as_str(),
            )
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
        state.db.execute(
            "INSERT INTO jobs(path,status) VALUES(?1,'running')",
            [&path_text],
        )?;
        let job = state.db.last_insert_rowid();
        let result: Result<bool> = async {
            let metadata = std::fs::metadata(&path)?;
            let mtime = format!("{:?}", metadata.modified()?);
            let previous: Option<(u64, String, String)> = state
                .db
                .query_row(
                    "SELECT size,mtime,sha256 FROM documents WHERE path=?1",
                    [&path_text],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            // Metadata detects candidates; sha256 confirms content, including same-size edits.
            let hash = crate::pipeline::file_sha256(&path)?;
            if let Some((size, old_mtime, old_hash)) = previous {
                if old_hash == hash {
                    if size != metadata.len() || old_mtime != mtime {
                        state.db.execute(
                            "UPDATE documents SET size=?1,mtime=?2 WHERE path=?3",
                            params![metadata.len(), mtime, path_text],
                        )?;
                    }
                    return Ok(false);
                }
            }
            let library = assets.join(if cfg!(windows) {
                "pdfium/bin/pdfium.dll"
            } else {
                "pdfium/lib/libpdfium.dylib"
            });
            let parser = worker::SupervisedParser {
                executable: worker::executable()?,
                timeout: Duration::from_secs(60),
            };
            let pages = crate::Parser::parse(
                &parser,
                &worker::Request {
                    path: path.clone(),
                    pdfium: Some(library.canonicalize().context("PDFium asset missing")?),
                },
            )
            .map_err(anyhow::Error::msg)?;
            if crate::pipeline::file_sha256(&path)? != hash {
                bail!("source changed during parsing; retry indexing");
            }
            let doc_id = sha256(path_text.as_bytes());
            let mut chunks = chunk(&doc_id, &pages);
            if chunks.is_empty() {
                let reason = if pages.iter().any(|p| p.needs_ocr) {
                    "needs OCR"
                } else {
                    "document has no text"
                };
                bail!("{reason}");
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
                chunks,
            };
            let generation = state.stage(&doc).await?;
            crate::Store::put(state, &doc, &mtime, &generation).map_err(anyhow::Error::msg)?;
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
                state
                    .db
                    .execute("UPDATE jobs SET status='done' WHERE id=?1", [job])?;
            }
            Err(error) => {
                stats.failed += 1;
                let reason = format!("{error:#}");
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
    let mut text = engines.stream(question, &context, &mut emit)?;
    validate_citations(&text, citations.len())?;
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
    #[test]
    fn refuses_unknown_model_citations() {
        assert!(validate_citations("supported [1]", 1).is_ok());
        assert!(validate_citations("invented [2]", 1).is_err());
        assert!(validate_citations("invalid [0]", 3).is_err());
    }
}
