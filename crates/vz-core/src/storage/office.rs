//! Test-only timing and immutable consolidated generation experiment.
use super::*;
use std::{collections::HashMap, time::Instant};
#[derive(Default, serde::Serialize)]
pub struct SearchTiming {
    pub documents_ms: f64,
    pub table_open_ms: f64,
    pub query_ms: f64,
    pub ranking_ms: f64,
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
impl State {
    pub fn office_open_timed(root: &Path) -> Result<(Self, serde_json::Value)> {
        let started = Instant::now();
        std::fs::create_dir_all(root.join("generations"))?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("writer.lock"))?;
        let lock_start = Instant::now();
        fs2::FileExt::try_lock_exclusive(&lock).context("index already open by another process")?;
        let exclusive_lock_ms = ms(lock_start);
        let sqlite_start = Instant::now();
        let db = Connection::open(root.join("state.sqlite"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS documents(doc_id TEXT PRIMARY KEY,path TEXT UNIQUE NOT NULL,size INTEGER NOT NULL,mtime TEXT NOT NULL,sha256 TEXT NOT NULL,generation TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS jobs(id INTEGER PRIMARY KEY,path TEXT NOT NULL,status TEXT NOT NULL,reason TEXT);
            UPDATE jobs SET status='failed',reason='interrupted before completion' WHERE status='running';")?;
        let sqlite_ms = ms(sqlite_start);
        let state = Self {
            db,
            root: root.to_owned(),
            _lock: FileLock(lock),
        };
        let recovery_start = Instant::now();
        state.recover()?;
        let recovery_ms = ms(recovery_start);
        Ok((
            state,
            serde_json::json!({"exclusive_lock_ms":exclusive_lock_ms,"sqlite_ms":sqlite_ms,"recovery_ms":recovery_ms,"total_ms":ms(started),"lock_is_nonblocking":true}),
        ))
    }

    pub async fn office_search_timed(
        &self,
        query: &str,
        vector: &[f32],
        top_k: usize,
    ) -> Result<(Vec<Hit>, SearchTiming)> {
        let mut timing = SearchTiming::default();
        let mut lexical = Vec::new();
        let mut semantic = Vec::new();
        let started = Instant::now();
        let docs = self.documents()?;
        timing.documents_ms = ms(started);
        let chunks = docs
            .into_iter()
            .flat_map(|d| d.chunks)
            .map(|c| (c.chunk_id.clone(), c))
            .collect::<std::collections::HashMap<_, _>>();
        for id in self.generations()? {
            let path = self.root.join("generations").join(id).join("lance");
            let started = Instant::now();
            let db = lancedb::connect(path.to_str().context("UTF-8 path")?)
                .execute()
                .await?;
            let table = db.open_table("chunks").execute().await?;
            timing.table_open_ms += ms(started);
            let started = Instant::now();
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
            timing.query_ms += ms(started);
        }
        let started = Instant::now();
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
        let hits = merged
            .into_iter()
            .take(top_k)
            .filter_map(|(id, score)| chunks.get(&id).cloned().map(|chunk| Hit { chunk, score }))
            .collect();
        timing.ranking_ms = ms(started);
        Ok((hits, timing))
    }
}
/// Build before publication, retain by Arc until all readers finish. No writer
/// methods are exposed. The owning generation must not be recovered/deleted.
pub struct Snapshot {
    table: lancedb::Table,
    chunks: HashMap<String, Chunk>,
}
impl Snapshot {
    pub async fn build(state: &State, root: &Path) -> Result<Self> {
        let chunks: HashMap<_, _> = state
            .documents()?
            .into_iter()
            .flat_map(|d| d.chunks)
            .map(|c| (c.chunk_id.clone(), c))
            .collect();
        let mut ordered: Vec<_> = chunks.values().cloned().collect();
        ordered.sort_by(|a, b| a.chunk_id.cmp(&b.chunk_id));
        let batch = chunks_batch(&ordered)?;
        let db = lancedb::connect(root.to_str().context("UTF-8 path")?)
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
        drop(table);
        let table = db.open_table("chunks").execute().await?;
        Ok(Self { table, chunks })
    }
    pub async fn search(&self, query: &str, vector: &[f32], top_k: usize) -> Result<Vec<Hit>> {
        Ok(self.search_timed(query, vector, top_k).await?.0)
    }
    pub async fn search_timed(
        &self,
        query: &str,
        vector: &[f32],
        top_k: usize,
    ) -> Result<(Vec<Hit>, SearchTiming)> {
        let mut timing = SearchTiming::default();
        let started = Instant::now();
        let mut lexical = Vec::new();
        let mut semantic = Vec::new();
        let batches = self
            .table
            .vector_search(vector)?
            .limit(top_k * 3)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        collect_scores(&batches, "_distance", &mut semantic)?;
        let batches = self
            .table
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
        timing.query_ms = ms(started);
        let started = Instant::now();
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
        let hits = merged
            .into_iter()
            .take(top_k)
            .filter_map(|(id, score)| {
                self.chunks
                    .get(&id)
                    .cloned()
                    .map(|chunk| Hit { chunk, score })
            })
            .collect();
        timing.ranking_ms = ms(started);
        Ok((hits, timing))
    }
}
