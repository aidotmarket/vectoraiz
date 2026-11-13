//! Opt-in S1934 experiment. No product wiring or listener.
//! SQLite is the indexed text lookup; Lance holds bounded, streamed batches.
use super::*;
use lancedb::{index::vector::*, query::Select, DistanceType};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::{
    io::{BufReader, BufWriter, Read, Write},
    time::Instant,
};

pub const BATCH_ROWS: usize = 1024;
pub const BATCH_BYTES: usize = 16 * 1024 * 1024;

/// Typed opaque-ID lookup. Embeddings never pass through SQLite or JSON.
pub const LOOKUP_SCHEMA: &str = "CREATE TABLE chunks(id TEXT PRIMARY KEY,doc_id TEXT NOT NULL,text TEXT NOT NULL,page INTEGER NOT NULL,heading TEXT NOT NULL,tokens INTEGER NOT NULL)";
pub const LOOKUP_INSERT: &str =
    "INSERT INTO chunks(id,doc_id,text,page,heading,tokens) VALUES(?1,?2,?3,?4,?5,?6)";
pub fn insert_lookup(statement: &mut rusqlite::Statement<'_>, c: &Chunk) -> Result<()> {
    statement.execute(params![
        c.chunk_id,
        c.doc_id,
        c.text,
        c.page,
        serde_json::to_string(&c.heading_path)?,
        c.token_count
    ])?;
    Ok(())
}
pub fn lookup_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<Chunk> {
    let heading: String = row.get(4)?;
    let heading_path = serde_json::from_str(&heading).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Chunk {
        chunk_id: row.get(0)?,
        doc_id: row.get(1)?,
        text: row.get(2)?,
        page: row.get(3)?,
        heading_path,
        token_count: row.get(5)?,
        embedding: Vec::new(),
    })
}
// A callback visitor recovers old generations with exactly one streaming decode.
struct TypedChunks<'a>(&'a mut dyn FnMut(Chunk) -> Result<()>);
impl<'de> DeserializeSeed<'de> for TypedChunks<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        struct Fields<'a>(&'a mut dyn FnMut(Chunk) -> Result<()>);
        impl<'de> Visitor<'de> for Fields<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("document")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
                while let Some(key) = map.next_key::<String>()? {
                    if key == "chunks" {
                        map.next_value_seed(TypedRows(self.0))?;
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(())
            }
        }
        d.deserialize_map(Fields(self.0))
    }
}
struct TypedRows<'a>(&'a mut dyn FnMut(Chunk) -> Result<()>);
impl<'de> DeserializeSeed<'de> for TypedRows<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        struct Rows<'a>(&'a mut dyn FnMut(Chunk) -> Result<()>);
        impl<'de> Visitor<'de> for Rows<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("chunks")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
                while let Some(c) = seq.next_element::<Chunk>()? {
                    (self.0)(c).map_err(serde::de::Error::custom)?;
                }
                Ok(())
            }
        }
        d.deserialize_seq(Rows(self.0))
    }
}
struct TypedBatches {
    receiver: std::sync::mpsc::Receiver<RecordBatch>,
    schema: Arc<Schema>,
    wait_ns: Arc<std::sync::atomic::AtomicU64>,
}
impl Iterator for TypedBatches {
    type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;
    fn next(&mut self) -> Option<Self::Item> {
        let start = Instant::now();
        let batch = self.receiver.recv().ok();
        self.wait_ns.fetch_add(
            start.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        batch.map(Ok)
    }
}
impl arrow_array::RecordBatchReader for TypedBatches {
    fn schema(&self) -> Arc<Schema> {
        self.schema.clone()
    }
}
/// Rendezvous channel: at most one producer batch plus one consumer batch.
/// A failed producer/append never publishes a partial successor.
pub async fn prepare_streamed(
    source: &Path,
    lookup: &Path,
    root: &Path,
) -> Result<serde_json::Value> {
    let owner = Instant::now();
    let successor = root.with_extension("pending");
    anyhow::ensure!(
        !root.exists() && !successor.exists(),
        "fresh immutable successor required"
    );
    let source = source.to_owned();
    let lookup = lookup.to_owned();
    let (sender, receiver) = std::sync::mpsc::sync_channel::<RecordBatch>(0);
    let (schema_tx, schema_rx) = std::sync::mpsc::sync_channel(1);
    let producer = std::thread::spawn(move || -> Result<(usize, f64)> {
        let start = Instant::now();
        let mut blocked = std::time::Duration::ZERO;
        let mut db = Connection::open(&lookup)?;
        db.execute_batch("PRAGMA cache_size=-8192")?;
        db.execute_batch(LOOKUP_SCHEMA)?;
        let tx = db.transaction()?;
        let mut insert = tx.prepare(LOOKUP_INSERT)?;
        let source_db = Connection::open_with_flags(
            source.join("state.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let mut docs = source_db.prepare("SELECT generation FROM documents ORDER BY doc_id")?;
        let mut pending = Vec::new();
        let mut bytes = 0;
        let mut count = 0;
        let mut schema_sent = false;
        let mut flush = |pending: &mut Vec<Chunk>| -> Result<()> {
            if pending.is_empty() {
                return Ok(());
            }
            let batch = chunks_batch(pending)?;
            anyhow::ensure!(
                batch.num_rows() <= BATCH_ROWS && batch.get_array_memory_size() <= BATCH_BYTES,
                "decoded batch budget"
            );
            if !schema_sent {
                schema_tx.send(batch.schema())?;
                schema_sent = true;
            }
            // Drop chunk copies before blocking: only typed Arrow is in flight.
            pending.clear();
            let wait = Instant::now();
            sender.send(batch)?;
            blocked += wait.elapsed();
            Ok(())
        };
        for row in docs.query_map([], |r| r.get::<_, String>(0))? {
            let generation = row?;
            anyhow::ensure!(
                generation
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
                "unsafe generation"
            );
            let file = std::fs::File::open(
                source
                    .join("generations")
                    .join(generation)
                    .join("document.json"),
            )?;
            TypedChunks(&mut |c: Chunk| {
                let size = c.text.len()
                    + c.chunk_id.len()
                    + c.doc_id.len()
                    + c.heading_path.iter().map(String::len).sum::<usize>()
                    + c.embedding.len() * 4
                    + 256;
                anyhow::ensure!(size <= BATCH_BYTES / 2, "oversize chunk");
                if pending.len() == BATCH_ROWS || bytes + size > BATCH_BYTES / 2 {
                    flush(&mut pending)?;
                    bytes = 0;
                }
                insert_lookup(&mut insert, &c)?;
                bytes += size;
                pending.push(c);
                count += 1;
                Ok(())
            })
            .deserialize(&mut serde_json::Deserializer::from_reader(
                BufReader::new(file),
            ))?;
        }
        flush(&mut pending)?;
        drop(insert);
        tx.commit()?;
        Ok((count, (start.elapsed() - blocked).as_secs_f64() * 1000.))
    });
    let schema = schema_rx
        .recv()
        .context("producer failed before first batch")?;
    let wait_ns = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let batches = TypedBatches {
        receiver,
        schema,
        wait_ns: wait_ns.clone(),
    };
    let start = Instant::now();
    let connection = lancedb::connect(successor.to_str().context("UTF-8 root")?)
        .execute()
        .await?;
    let appended = connection
        .create_table(
            "chunks",
            Box::new(batches) as Box<dyn arrow_array::RecordBatchReader + Send>,
        )
        .execute()
        .await;
    let append_ms = start.elapsed().as_secs_f64() * 1000.
        - wait_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6;
    let production = producer
        .join()
        .map_err(|_| anyhow::anyhow!("producer panic"))?;
    let table = appended?;
    let (rows, writer_lookup_ms) = production?;
    let start = Instant::now();
    table
        .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await?;
    let fts_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    anyhow::ensure!(table.count_rows(None).await? == rows, "row validation");
    drop(table);
    drop(connection);
    let validation_close_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    std::fs::rename(&successor, root)?;
    std::fs::File::open(root.parent().context("successor parent")?)?.sync_all()?;
    let promotion_ms = start.elapsed().as_secs_f64() * 1000.;
    Ok(
        serde_json::json!({"rows":rows,"writer_lookup_busy_ms":writer_lookup_ms,"lance_append_busy_ms":append_ms,"fts_ms":fts_ms,"validation_close_ms":validation_close_ms,"promotion_ms":promotion_ms,"preparation_busy_ms":writer_lookup_ms+append_ms+fts_ms+validation_close_ms+promotion_ms,"owner_to_ready_ms":owner.elapsed().as_secs_f64()*1000.,"owner_scope":"recovery from E4 extracted/embedded corpus; extraction/embedding not rerun","fts_operation":"initial full text-column scan/build; no vector index for Exact","promotion_status":"atomic private successor directory rename; product generation swap remains unrun","batch_rows":BATCH_ROWS,"batch_bytes":BATCH_BYTES,"max_batches_in_flight":2,"fts_num_shards":*lance_index::scalar::inverted::builder::LANCE_FTS_NUM_SHARDS,"fts_partition_mib":*lance_index::scalar::inverted::builder::LANCE_FTS_PARTITION_SIZE,"fts_flush_mib":std::env::var("LANCE_FTS_FLUSH_SIZE").unwrap_or_else(|_|"16".into()),"profile":"release: whole dependency graph optimized"}),
    )
}

// E4 controls only, never used by the measured successor.
/// Stream the chunks field without materializing a document or a corpus map.
struct Chunks<'a>(&'a Connection);
impl<'de> DeserializeSeed<'de> for Chunks<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        struct Fields<'a>(&'a Connection);
        impl<'de> Visitor<'de> for Fields<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("document")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
                while let Some(key) = map.next_key::<String>()? {
                    if key == "chunks" {
                        map.next_value_seed(ChunkList(self.0))?;
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(())
            }
        }
        d.deserialize_map(Fields(self.0))
    }
}
struct ChunkList<'a>(&'a Connection);
impl<'de> DeserializeSeed<'de> for ChunkList<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        struct Rows<'a>(&'a Connection);
        impl<'de> Visitor<'de> for Rows<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("chunks")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
                while let Some(chunk) = seq.next_element::<Chunk>()? {
                    let bytes = serde_json::to_vec(&chunk).map_err(serde::de::Error::custom)?;
                    if bytes.len() > BATCH_BYTES / 2 {
                        return Err(serde::de::Error::custom("oversize chunk"));
                    }
                    self.0
                        .execute(
                            "INSERT INTO chunks(id,payload) VALUES(?1,?2)",
                            params![chunk.chunk_id, bytes],
                        )
                        .map_err(serde::de::Error::custom)?;
                }
                Ok(())
            }
        }
        d.deserialize_seq(Rows(self.0))
    }
}

/// The source is opened read-only, without recovery or a writer fence.
pub fn e4_spool(source: &Path, output: &Path) -> Result<usize> {
    let source_db = Connection::open_with_flags(
        source.join("state.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut db = Connection::open(output)?;
    db.execute_batch(
        "PRAGMA cache_size=-8192; CREATE TABLE chunks(id TEXT PRIMARY KEY,payload BLOB NOT NULL);",
    )?;
    let tx = db.transaction()?;
    let mut statement = source_db.prepare("SELECT generation FROM documents ORDER BY doc_id")?;
    for row in statement.query_map([], |r| r.get::<_, String>(0))? {
        let generation = row?;
        anyhow::ensure!(
            generation
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "unsafe generation"
        );
        let file = std::fs::File::open(
            source
                .join("generations")
                .join(generation)
                .join("document.json"),
        )?;
        Chunks(&tx).deserialize(&mut serde_json::Deserializer::from_reader(BufReader::new(
            file,
        )))?;
    }
    tx.commit()?;
    Ok(db.query_row("SELECT count(*) FROM chunks", [], |r| r.get(0))?)
}

struct Batches {
    db: Connection,
    after: String,
    schema: Arc<Schema>,
}
impl Iterator for Batches {
    type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;
    fn next(&mut self) -> Option<Self::Item> {
        let result = (|| -> Result<Option<RecordBatch>> {
            let mut statement = self
                .db
                .prepare("SELECT id,payload FROM chunks WHERE id>?1 ORDER BY id LIMIT 1024")?;
            let mut rows = statement.query([&self.after])?;
            let mut chunks = Vec::new();
            let mut bytes = 0;
            while let Some(row) = rows.next()? {
                let payload: Vec<u8> = row.get(1)?;
                if bytes + payload.len() > BATCH_BYTES / 2 {
                    break;
                }
                bytes += payload.len();
                self.after = row.get(0)?;
                chunks.push(serde_json::from_slice(&payload)?);
            }
            if chunks.is_empty() {
                return Ok(None);
            }
            let batch = chunks_batch(&chunks)?;
            anyhow::ensure!(
                batch.get_array_memory_size() <= BATCH_BYTES,
                "decoded batch budget"
            );
            Ok(Some(batch))
        })();
        match result {
            Ok(value) => value.map(Ok),
            Err(e) => Some(Err(arrow_schema::ArrowError::ExternalError(e.into()))),
        }
    }
}
impl arrow_array::RecordBatchReader for Batches {
    fn schema(&self) -> Arc<Schema> {
        self.schema.clone()
    }
}

pub async fn e4_prepare(spool: &Path, root: &Path) -> Result<()> {
    let db = Connection::open_with_flags(spool, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.execute_batch("PRAGMA cache_size=-8192")?;
    let payload: Vec<u8> =
        db.query_row("SELECT payload FROM chunks ORDER BY id LIMIT 1", [], |r| {
            r.get(0)
        })?;
    let first: Chunk = serde_json::from_slice(&payload)?;
    let schema = chunks_batch(&[first])?.schema();
    let batches = Batches {
        db,
        after: String::new(),
        schema,
    };
    let connection = lancedb::connect(root.to_str().context("UTF-8 root")?)
        .execute()
        .await?;
    let table = connection
        .create_table(
            "chunks",
            Box::new(batches) as Box<dyn arrow_array::RecordBatchReader + Send>,
        )
        .execute()
        .await?;
    drop(table);
    let params = lancedb::table::ReadParams {
        index_cache_size_bytes: 128 * 1024 * 1024,
        metadata_cache_size_bytes: 32 * 1024 * 1024,
        ..Default::default()
    };
    let table = connection
        .open_table("chunks")
        .lance_read_params(params)
        .execute()
        .await?;
    table
        .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await?;
    Ok(())
}

#[derive(Clone, Copy, Debug, serde::Serialize, PartialEq, Eq)]
pub enum Candidate {
    Exact,
    StreamedExact,
    IvfFlat,
    IvfPq,
    IvfHnswPq,
    IvfHnswSq,
}
pub struct Snapshot {
    pub table: lancedb::Table,
    lookup: PathBuf,
    vectors: PathBuf,
    pub candidate: Candidate,
    pub partitions: usize,
}
impl Snapshot {
    pub async fn open(
        root: &Path,
        lookup: &Path,
        candidate: Candidate,
        rows: usize,
    ) -> Result<Self> {
        let connection = lancedb::connect(root.to_str().context("UTF-8 root")?)
            .execute()
            .await?;
        let params = lancedb::table::ReadParams {
            index_cache_size_bytes: 128 * 1024 * 1024,
            metadata_cache_size_bytes: 32 * 1024 * 1024,
            ..Default::default()
        };
        let table = connection
            .open_table("chunks")
            .lance_read_params(params)
            .execute()
            .await?;
        Ok(Self {
            table,
            lookup: lookup.into(),
            vectors: lookup.with_extension("vectors"),
            candidate,
            partitions: (rows / 128).clamp(1, 256),
        })
    }
    pub async fn index(&self) -> Result<()> {
        let p = self.partitions as u32;
        let index = match self.candidate {
            Candidate::Exact | Candidate::StreamedExact => return Ok(()),
            Candidate::IvfFlat => Index::IvfFlat(
                IvfFlatIndexBuilder::default()
                    .distance_type(DistanceType::L2)
                    .num_partitions(p)
                    .sample_rate(128)
                    .max_iterations(50),
            ),
            Candidate::IvfPq => Index::IvfPq(
                IvfPqIndexBuilder::default()
                    .distance_type(DistanceType::L2)
                    .num_partitions(p)
                    .sample_rate(128)
                    .max_iterations(50)
                    .num_sub_vectors(48)
                    .num_bits(8),
            ),
            Candidate::IvfHnswPq => Index::IvfHnswPq(
                IvfHnswPqIndexBuilder::default()
                    .distance_type(DistanceType::L2)
                    .num_partitions(p)
                    .sample_rate(128)
                    .max_iterations(50)
                    .num_sub_vectors(48)
                    .num_bits(8)
                    .num_edges(20)
                    .ef_construction(300),
            ),
            Candidate::IvfHnswSq => Index::IvfHnswSq(
                IvfHnswSqIndexBuilder::default()
                    .distance_type(DistanceType::L2)
                    .num_partitions(p)
                    .sample_rate(128)
                    .max_iterations(50)
                    .num_edges(20)
                    .ef_construction(300),
            ),
        };
        self.table
            .create_index(&["embedding"], index)
            .execute()
            .await?;
        Ok(())
    }
    fn query(&self, vector: &[f32]) -> Result<lancedb::query::VectorQuery> {
        let query = self
            .table
            .vector_search(vector)?
            .distance_type(DistanceType::L2)
            .limit(40)
            .select(Select::columns(&["chunk_id"]));
        Ok(
            if matches!(self.candidate, Candidate::Exact | Candidate::StreamedExact) {
                query.bypass_vector_index()
            } else {
                query
                    .nprobes(self.partitions.min(32))
                    .refine_factor(4)
                    .ef(256)
            },
        )
    }
    pub async fn plan(&self, vector: &[f32]) -> Result<String> {
        if self.candidate == Candidate::StreamedExact {
            return Ok("bounded contiguous vector file; 1 MiB reader buffer; exact L2; ID ties; top40; SQLite PRIMARY KEY fetch".into());
        }
        Ok(self.query(vector)?.explain_plan(true).await?)
    }
    pub async fn vector(&self, vector: &[f32]) -> Result<Vec<(String, f32)>> {
        if self.candidate == Candidate::StreamedExact {
            return scan(&self.vectors, vector);
        }
        let batches = self
            .query(vector)?
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let mut out = Vec::new();
        collect_scores(&batches, "_distance", &mut out)?;
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        Ok(out)
    }
    pub async fn search(
        &self,
        query: &str,
        vector: &[f32],
        limit: usize,
    ) -> Result<(Vec<Hit>, serde_json::Value, Vec<String>)> {
        // Human document metrics deduplicate the full 40-vector + 40-FTS
        // candidate pool before selecting documents. Browser calls stay at 8.
        anyhow::ensure!(limit <= 80, "internal evaluation limit");
        let start = Instant::now();
        let semantic = self.vector(vector).await?;
        let vector_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let batches = self
            .table
            .query()
            .full_text_search(lance_index::scalar::FullTextSearchQuery::new(
                query.to_owned(),
            ))
            .select(Select::columns(&["chunk_id"]))
            .limit(40)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let mut lexical = Vec::new();
        collect_scores(&batches, "_score", &mut lexical)?;
        lexical.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let fts_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let mut scores = std::collections::HashMap::<String, f32>::new();
        for list in [&semantic, &lexical] {
            for (rank, (id, _)) in list.iter().enumerate() {
                *scores.entry(id.clone()).or_default() += 1.0 / (61 + rank) as f32;
            }
        }
        let mut merged: Vec<_> = scores.into_iter().collect();
        merged.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let ranking_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let db = Connection::open_with_flags(&self.lookup, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.execute_batch("PRAGMA cache_size=-1024")?;
        let typed = db.prepare("SELECT doc_id FROM chunks LIMIT 0").is_ok();
        let mut statement = db.prepare(if typed {
            "SELECT id,doc_id,text,page,heading,tokens FROM chunks WHERE id=?1"
        } else {
            "SELECT payload FROM chunks WHERE id=?1"
        })?;
        let mut hits = Vec::new();
        for (id, score) in merged.into_iter().take(limit) {
            let mut chunk: Chunk = if typed {
                statement.query_row([id], lookup_chunk)?
            } else {
                let payload: Vec<u8> = statement.query_row([id], |r| r.get(0))?;
                serde_json::from_slice(&payload)?
            };
            chunk.embedding.clear();
            hits.push(Hit { chunk, score });
        }
        if limit <= 8 {
            anyhow::ensure!(
                hits.iter()
                    .map(|h| h.chunk.text.len()
                        + h.chunk.chunk_id.len()
                        + h.chunk.doc_id.len()
                        + h.chunk.heading_path.iter().map(String::len).sum::<usize>())
                    .sum::<usize>()
                    <= 128 * 1024,
                "browser output budget"
            );
        }
        Ok((
            hits,
            serde_json::json!({"vector_ms":vector_ms,"fts_ms":fts_ms,"ranking_ms":ranking_ms,"fetch_ms":start.elapsed().as_secs_f64()*1000.}),
            semantic.into_iter().take(10).map(|(id, _)| id).collect(),
        ))
    }
}

pub fn write_vectors(lookup: &Path) -> Result<()> {
    let db = Connection::open_with_flags(lookup, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = db.prepare("SELECT payload FROM chunks ORDER BY id")?;
    let mut file = BufWriter::new(std::fs::File::create(lookup.with_extension("vectors"))?);
    for row in statement.query_map([], |r| r.get::<_, Vec<u8>>(0))? {
        let chunk: Chunk = serde_json::from_slice(&row?)?;
        file.write_all(&(chunk.chunk_id.len() as u32).to_le_bytes())?;
        file.write_all(chunk.chunk_id.as_bytes())?;
        file.write_all(&(chunk.embedding.len() as u32).to_le_bytes())?;
        for v in chunk.embedding {
            file.write_all(&v.to_le_bytes())?;
        }
    }
    file.flush()?;
    Ok(())
}
fn scan(path: &Path, vector: &[f32]) -> Result<Vec<(String, f32)>> {
    let mut file = BufReader::with_capacity(1024 * 1024, std::fs::File::open(path)?);
    let mut best = Vec::new();
    loop {
        let mut word = [0; 4];
        if file.fill_buf()?.is_empty() {
            break;
        }
        file.read_exact(&mut word)?;
        let length = u32::from_le_bytes(word) as usize;
        anyhow::ensure!(length <= 1024, "id limit");
        let mut id = vec![0; length];
        file.read_exact(&mut id)?;
        file.read_exact(&mut word)?;
        anyhow::ensure!(
            u32::from_le_bytes(word) as usize == vector.len(),
            "dimensions"
        );
        let mut values = vec![0.0f32; vector.len()];
        // f32 storage is aligned; every bit pattern is valid. The private vector
        // file is explicitly little endian; macOS arm64 is little endian.
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(values.as_mut_ptr().cast::<u8>(), values.len() * 4)
        };
        file.read_exact(bytes)?;
        let distance = l2(&values, vector);
        best.push((String::from_utf8(id)?, distance));
        best.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        best.truncate(40);
    }
    Ok(best)
}
use std::io::BufRead;

fn l2(values: &[f32], query: &[f32]) -> f32 {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "Accelerate", kind = "framework")]
        unsafe extern "C" {
            fn vDSP_distancesq(
                a: *const f32,
                ia: isize,
                b: *const f32,
                ib: isize,
                result: *mut f32,
                n: usize,
            );
        }
        let mut result = 0.;
        unsafe {
            vDSP_distancesq(
                values.as_ptr(),
                1,
                query.as_ptr(),
                1,
                &mut result,
                values.len(),
            );
        }
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        values
            .iter()
            .zip(query)
            .map(|(a, b)| (*a - b).powi(2))
            .sum()
    }
}
