//! Immutable Lance generations with SQLite as the sole visibility authority.
use crate::pipeline::{Chunk, Document};
use anyhow::{Context, Result};
use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator,
    StringArray, UInt32Array,
};
use arrow_schema::{Field, Schema};
use futures::TryStreamExt;
use lancedb::{
    index::{scalar::FtsIndexBuilder, Index},
    query::{ExecutableQuery, QueryBase},
};
use rusqlite::{params, Connection, OpenFlags};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct State {
    pub db: Connection,
    pub root: PathBuf,
    _lock: FileLock,
}
struct FileLock(std::fs::File);
impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
impl State {
    /// Inspect persisted visibility without opening/recovering a writable State.
    /// Unknown or damaged storage is an error, never evidence of an empty index.
    pub fn has_persisted_documents(root: &Path) -> Result<bool> {
        let path = root.join("state.sqlite");
        if !path.try_exists()? {
            // A lost database must not hide surviving on-disk index data.
            return match std::fs::read_dir(root.join("generations")) {
                Ok(mut entries) => Ok(entries.next().transpose()?.is_some()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.into()),
            };
        }
        // SQLite's bundled Unix/Windows VFS supports readonly_shm: WAL reads
        // must not create or initialize shared memory. If unavailable, let the
        // caller retain the original diagnostic rather than recover this state.
        let mode = if root.join("state.sqlite-wal").try_exists()? {
            "readonly_shm=1"
        } else {
            // A checkpointed WAL-mode DB also needs no new WAL/SHM files.
            "immutable=1"
        };
        let path = path
            .to_str()
            .context("index path must be UTF-8")?
            .replace('%', "%25")
            .replace('?', "%3F")
            .replace('#', "%23");
        let db = Connection::open_with_flags(
            format!("file:{path}?{mode}"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )?;
        Ok(
            db.query_row("SELECT EXISTS(SELECT 1 FROM documents)", [], |row| {
                row.get(0)
            })?,
        )
    }
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join("generations"))?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("writer.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock).context("index already open by another process")?;
        let db = Connection::open(root.join("state.sqlite"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS documents(doc_id TEXT PRIMARY KEY,path TEXT UNIQUE NOT NULL,size INTEGER NOT NULL,mtime TEXT NOT NULL,sha256 TEXT NOT NULL,generation TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS jobs(id INTEGER PRIMARY KEY,path TEXT NOT NULL,status TEXT NOT NULL,reason TEXT);
            UPDATE jobs SET status='failed',reason='interrupted before completion' WHERE status='running';")?;
        let state = Self {
            db,
            root: root.to_owned(),
            _lock: FileLock(lock),
        };
        state.recover()?;
        Ok(state)
    }
    /// All query/export entry points open through this recipe gate before inference.
    pub fn open_index(
        root: &Path,
        selected: &crate::engine::Recipe,
    ) -> Result<(Self, crate::engine::Recipe)> {
        let root = crate::active_index::resolve_state(root);
        let state = Self::open(&root)?;
        let recipe = state.validate_recipe(selected)?;
        Ok((state, recipe))
    }
    pub fn validate_recipe(
        &self,
        selected: &crate::engine::Recipe,
    ) -> Result<crate::engine::Recipe> {
        let recipe: crate::engine::Recipe = serde_json::from_slice(
            &std::fs::read(self.root.join("recipe.json"))
                .context("index recipe missing; re-index required")?,
        )
        .context("index recipe invalid; re-index required")?;
        anyhow::ensure!(
            recipe == *selected,
            "embedding recipe mismatch; re-index required (use a new index)"
        );
        Ok(recipe)
    }
    pub fn recover(&self) -> Result<()> {
        let active: HashSet<String> = self
            .db
            .prepare("SELECT generation FROM documents")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for entry in std::fs::read_dir(self.root.join("generations"))? {
            let entry = entry?;
            if !active.contains(&entry.file_name().to_string_lossy().into_owned()) {
                std::fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }
    pub fn generations(&self) -> Result<Vec<String>> {
        Ok(self
            .db
            .prepare("SELECT generation FROM documents ORDER BY doc_id")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn documents(&self) -> Result<Vec<Document>> {
        self.generations()?
            .iter()
            .map(|id| {
                Ok(serde_json::from_slice(&std::fs::read(
                    self.root.join("generations").join(id).join("document.json"),
                )?)?)
            })
            .collect()
    }
    pub async fn stage(&self, doc: &Document) -> Result<String> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let path = self.root.join("generations").join(&id);
        std::fs::create_dir(&path)?;
        let batch = chunks_batch(&doc.chunks)?;
        let db = lancedb::connect(
            path.join("lance")
                .to_str()
                .context("index path must be UTF-8")?,
        )
        .execute()
        .await?;
        let reader = RecordBatchIterator::new(vec![Ok(batch.clone())], batch.schema());
        let table = db
            .create_table("chunks", Box::new(reader))
            .execute()
            .await?;
        table
            .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
            .execute()
            .await?;
        let file = std::fs::File::create(path.join("document.json"))?;
        serde_json::to_writer(&file, doc)?;
        file.sync_all()?;
        #[cfg(unix)]
        std::fs::File::open(&path)?.sync_all()?;
        Ok(id)
    }
    pub fn commit(&mut self, doc: &Document, mtime: &str, generation: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO documents VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(doc_id) DO UPDATE SET path=excluded.path,size=excluded.size,mtime=excluded.mtime,sha256=excluded.sha256,generation=excluded.generation", params![doc.doc_id, doc.path, doc.bytes, mtime, doc.sha256, generation])?;
        tx.commit()?;
        Ok(())
    }
    pub async fn search(&self, query: &str, vector: &[f32], top_k: usize) -> Result<Vec<Hit>> {
        let _timer = crate::engine::PhaseTimer::new("search");
        let mut lexical = Vec::new();
        let mut semantic = Vec::new();
        let docs = self.documents()?;
        let chunks = docs
            .into_iter()
            .flat_map(|d| d.chunks)
            .map(|c| (c.chunk_id.clone(), c))
            .collect::<std::collections::HashMap<_, _>>();
        for id in self.generations()? {
            let path = self.root.join("generations").join(id).join("lance");
            let db = lancedb::connect(path.to_str().context("UTF-8 path")?)
                .execute()
                .await?;
            let table = db.open_table("chunks").execute().await?;
            let batches = table
                .vector_search(vector)?
                .limit(top_k * 3)
                .execute()
                .await?
                .try_collect::<Vec<_>>()
                .await?;
            collect_scores(&batches, "_distance", &mut semantic)?;
            let batches = table
                .query()
                .full_text_search(lance_index::scalar::FullTextSearchQuery::new(
                    query.to_owned(),
                ))
                .limit(top_k * 3)
                .execute()
                .await?
                .try_collect::<Vec<_>>()
                .await?;
            collect_scores(&batches, "_score", &mut lexical)?;
        }
        semantic.sort_by(|a, b| a.1.total_cmp(&b.1));
        lexical.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut scores = std::collections::HashMap::<String, f32>::new();
        for ranking in [&semantic, &lexical] {
            for (rank, (id, _)) in ranking.iter().enumerate() {
                *scores.entry(id.clone()).or_default() += 1.0 / (60 + rank + 1) as f32;
            }
        }
        let mut merged: Vec<_> = scores.into_iter().collect();
        merged.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(merged
            .into_iter()
            .take(top_k)
            .filter_map(|(id, score)| chunks.get(&id).cloned().map(|chunk| Hit { chunk, score }))
            .collect())
    }
}
#[derive(Clone, serde::Serialize, serde::Deserialize, Debug)]
pub struct Hit {
    pub chunk: Chunk,
    pub score: f32,
}
fn collect_scores(
    batches: &[RecordBatch],
    column: &str,
    out: &mut Vec<(String, f32)>,
) -> Result<()> {
    for batch in batches {
        let ids = batch
            .column_by_name("chunk_id")
            .context("chunk_id")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("string chunk_id")?;
        let scores = batch
            .column_by_name(column)
            .context("ranking score")?
            .as_any()
            .downcast_ref::<Float32Array>()
            .context("float ranking score")?;
        for i in 0..batch.num_rows() {
            out.push((ids.value(i).into(), scores.value(i)));
        }
    }
    Ok(())
}
pub fn chunks_batch(chunks: &[Chunk]) -> Result<RecordBatch> {
    let dimensions = chunks
        .first()
        .context("document has no indexable text")?
        .embedding
        .len();
    let embedding = FixedSizeListArray::from_iter_primitive::<arrow_array::types::Float32Type, _, _>(
        chunks
            .iter()
            .map(|c| Some(c.embedding.iter().copied().map(Some))),
        dimensions as i32,
    );
    let columns: Vec<(&str, ArrayRef)> = vec![
        (
            "chunk_id",
            Arc::new(StringArray::from_iter_values(
                chunks.iter().map(|c| &c.chunk_id),
            )),
        ),
        (
            "doc_id",
            Arc::new(StringArray::from_iter_values(
                chunks.iter().map(|c| &c.doc_id),
            )),
        ),
        (
            "text",
            Arc::new(StringArray::from_iter_values(
                chunks.iter().map(|c| &c.text),
            )),
        ),
        (
            "page",
            Arc::new(UInt32Array::from_iter_values(chunks.iter().map(|c| c.page))),
        ),
        (
            "heading_path",
            Arc::new(StringArray::from_iter_values(
                chunks
                    .iter()
                    .map(|c| serde_json::to_string(&c.heading_path).unwrap()),
            )),
        ),
        (
            "token_count",
            Arc::new(UInt32Array::from_iter_values(
                chunks.iter().map(|c| c.token_count as u32),
            )),
        ),
        ("embedding", Arc::new(embedding)),
    ];
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(name, array)| Field::new(*name, array.data_type().clone(), false))
            .collect::<Vec<_>>(),
    ));
    Ok(RecordBatch::try_new(
        schema,
        columns.into_iter().map(|(_, a)| a).collect(),
    )?)
}
pub fn documents_batch(documents: &[Document]) -> Result<RecordBatch> {
    let strings = |f: fn(usize, &Document) -> String| -> ArrayRef {
        Arc::new(StringArray::from_iter_values(
            documents.iter().enumerate().map(|(i, d)| f(i, d)),
        ))
    };
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("doc_id", strings(|_, d| d.doc_id.clone())),
        (
            "file_name",
            strings(|i, d| format!("document-{:04}.{}", i + 1, d.kind)),
        ),
        ("title", strings(|i, _| format!("Document {}", i + 1))),
        ("type", strings(|_, d| d.kind.clone())),
        ("sha256", strings(|_, d| d.sha256.clone())),
        ("language", strings(|_, _| "und".into())),
        (
            "pages",
            Arc::new(UInt32Array::from_iter_values(
                documents.iter().map(|d| d.pages),
            )),
        ),
        (
            "bytes",
            Arc::new(arrow_array::UInt64Array::from_iter_values(
                documents.iter().map(|d| d.bytes),
            )),
        ),
    ];
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(n, a)| Field::new(*n, a.data_type().clone(), false))
            .collect::<Vec<_>>(),
    ));
    Ok(RecordBatch::try_new(
        schema,
        columns.into_iter().map(|(_, a)| a).collect(),
    )?)
}

impl crate::Store for State {
    fn put(
        &mut self,
        document: &Document,
        mtime: &str,
        generation: &str,
    ) -> crate::ComponentResult<()> {
        self.commit(document, mtime, generation)
            .map_err(|e| e.to_string())
    }
    fn get(&self, id: &str) -> crate::ComponentResult<Option<Document>> {
        self.documents()
            .map(|docs| docs.into_iter().find(|doc| doc.doc_id == id))
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_recipe_gate_refuses_same_dimension_model_changes() {
        let root = tempfile::tempdir().unwrap();
        let recipe: crate::engine::Recipe = serde_json::from_value(serde_json::json!({
            "model":"model-A", "sha256":"hash-A", "dimensions":4,
            "dtype":"float32", "pooling":"cls", "normalization":"l2",
            "tokenizer":"embedded", "max_tokens":512, "document_prefix":"",
            "query_prefix":"query: ", "chunker":{"version":"vz-1"},
            "llama_commit":crate::engine::LLAMA_COMMIT
        }))
        .unwrap();
        std::fs::write(
            root.path().join("recipe.json"),
            serde_json::to_vec(&recipe).unwrap(),
        )
        .unwrap();
        let (state, stored) = State::open_index(root.path(), &recipe).unwrap();
        assert_eq!(stored, recipe);
        drop(state);
        for field in ["model", "sha256"] {
            let mut changed = serde_json::to_value(&recipe).unwrap();
            changed[field] = "different-model".into();
            let changed = serde_json::from_value(changed).unwrap();
            let error = State::open_index(root.path(), &changed).err().unwrap();
            assert!(error.to_string().contains("re-index required"));
        }
        std::fs::write(root.path().join("recipe.json"), "invalid").unwrap();
        assert!(format!(
            "{:#}",
            State::open_index(root.path(), &recipe).err().unwrap()
        )
        .contains("re-index required"));
        std::fs::remove_file(root.path().join("recipe.json")).unwrap();
        assert!(format!(
            "{:#}",
            State::open_index(root.path(), &recipe).err().unwrap()
        )
        .contains("re-index required"));
    }
}
