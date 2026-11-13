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
    // Last field: table is dropped before generation storage is retired.
    _storage: tempfile::TempDir,
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
        std::fs::create_dir_all(root)?;
        let storage = tempfile::tempdir_in(root)?;
        let db = lancedb::connect(storage.path().to_str().context("UTF-8 path")?)
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
        Ok(Self {
            table,
            chunks,
            _storage: storage,
        })
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

/// Test-only publication boundary. Snapshot data and its recipe/scope identity
/// are immutable; cancellation is shared with readers of retired scopes.
pub struct Generation {
    pub id: u64,
    pub scope: String,
    pub recipe: String,
    pub snapshot: Snapshot,
    cancelled: tokio::sync::watch::Sender<bool>,
}
impl Generation {
    pub fn new(id: u64, scope: &str, recipe: &str, snapshot: Snapshot) -> Self {
        Self {
            id,
            scope: scope.into(),
            recipe: recipe.into(),
            snapshot,
            cancelled: tokio::sync::watch::channel(false).0,
        }
    }
    pub fn cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }
}
#[derive(Default)]
struct Published {
    current: Option<std::sync::Arc<Generation>>,
    retired: Vec<std::sync::Weak<Generation>>,
    draining: bool,
}
#[derive(Default)]
pub struct Publication(std::sync::RwLock<Published>);
impl Publication {
    /// A scope change first cancels/removes every old snapshot. Publication is
    /// refused until old readers drain; caller prepares/retries independently.
    pub fn publish(&self, next: Generation) -> Result<()> {
        let mut state = self.0.write().unwrap();
        state.retired.retain(|g| g.strong_count() > 0);
        if let Some(old) = state.current.as_ref() {
            anyhow::ensure!(next.id > old.id, "nonmonotonic generation");
            if old.scope != next.scope {
                let old = state.current.take().unwrap();
                old.cancelled.send_replace(true);
                state.retired.push(std::sync::Arc::downgrade(&old));
                for old in state.retired.iter().filter_map(|g| g.upgrade()) {
                    old.cancelled.send_replace(true);
                }
                state.draining = true;
            }
        }
        if state.draining {
            anyhow::ensure!(
                state.retired.iter().all(|g| g.strong_count() == 0),
                "scope readers still draining"
            );
            state.draining = false;
        }
        if let Some(old) = state.current.take() {
            state.retired.push(std::sync::Arc::downgrade(&old));
        }
        state.current = Some(std::sync::Arc::new(next));
        Ok(())
    }
    pub fn stop(&self) {
        let mut state = self.0.write().unwrap();
        if let Some(old) = state.current.take() {
            old.cancelled.send_replace(true);
            state.retired.push(std::sync::Arc::downgrade(&old));
        }
        for old in state.retired.iter().filter_map(|g| g.upgrade()) {
            old.cancelled.send_replace(true);
        }
        state.draining = true;
    }
    pub fn capture(&self) -> Result<std::sync::Arc<Generation>> {
        self.0
            .read()
            .unwrap()
            .current
            .clone()
            .context("office unavailable")
    }
    /// Embedding callback owns/releases the inference guard; storage is outside
    /// it. Each retry embeds with the captured recipe. Final gate is atomic with
    /// publication. Authorization is supplied by the caller and checked twice.
    pub async fn search(
        &self,
        query: &str,
        top_k: usize,
        mut embed: impl FnMut(&str, &str) -> Result<Vec<f32>>,
        authorized: impl Fn() -> bool,
    ) -> Result<Vec<Hit>> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        for _ in 0..2 {
            anyhow::ensure!(authorized(), "delivery revoked");
            let generation = self.capture()?;
            let vector = embed(query, &generation.recipe)?;
            let mut cancel = generation.cancelled.subscribe();
            anyhow::ensure!(!*cancel.borrow(), "delivery revoked");
            let hits = tokio::select! {
                result = tokio::time::timeout_at(deadline,
                    generation.snapshot.search(query, &vector, top_k)) => result??,
                _ = cancel.changed() => anyhow::bail!("delivery revoked"),
            };
            anyhow::ensure!(!generation.cancelled() && authorized(), "delivery revoked");
            anyhow::ensure!(tokio::time::Instant::now() <= deadline, "search deadline");
            let current = self.0.read().unwrap();
            if current
                .current
                .as_ref()
                .is_some_and(|g| std::sync::Arc::ptr_eq(g, &generation))
            {
                return Ok(hits);
            }
        }
        anyhow::bail!("generation changed twice; refuse within search deadline")
    }
}
