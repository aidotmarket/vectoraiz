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
pub fn spool(source: &Path, output: &Path) -> Result<usize> {
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

pub async fn prepare(spool: &Path, root: &Path) -> Result<()> {
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
        anyhow::ensure!(limit <= 10, "internal evaluation limit");
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
        let mut statement = db.prepare("SELECT payload FROM chunks WHERE id=?1")?;
        let mut hits = Vec::new();
        for (id, score) in merged.into_iter().take(limit) {
            let payload: Vec<u8> = statement.query_row([id], |r| r.get(0))?;
            let mut chunk: Chunk = serde_json::from_slice(&payload)?;
            chunk.embedding.clear();
            hits.push(Hit { chunk, score });
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
        let mut distance = 0.;
        for q in vector {
            file.read_exact(&mut word)?;
            let d = f32::from_le_bytes(word) - q;
            distance += d * d;
        }
        best.push((String::from_utf8(id)?, distance));
        best.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        best.truncate(40);
    }
    Ok(best)
}
use std::io::BufRead;
