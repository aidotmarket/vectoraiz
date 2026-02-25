# BQ-VZ-LARGE-FILES — Streaming/Chunked Processing for Large Files
## Spec v1.2 (Gate 2 Council Revisions Applied)

### Changes from v1.1
- **R1**: Replaced PyMuPDF (AGPL) with `pypdfium2` (Apache 2.0) for PDF streaming — resolves licensing blocker
- **R2**: Temp directory changed from `/tmp/` to `{settings.data_directory}/temp/` — aligns with codebase convention
- **R3**: Explicit checkpoint strategy per file type; CSV byte-offset resume deferred
- **R4**: IPC architecture specified — worker writes Parquet, sends progress/status via `multiprocessing.Pipe`
- **R5**: Indexing scope clarified — chunked indexing included in Phase 1
- **R6**: Added zip bomb detection, timeout backoff, charset fallback (XAI notes)

---

### Problem
vectorAIz currently loads entire files into memory for processing. Files >500MB crash workers, >200MB cause significant latency and memory pressure. Enterprise customers routinely have multi-GB CSV/Parquet/PDF files. This is a beta blocker for any serious enterprise deployment.

### Current Architecture (What Changes)
- `processing_service.py` (849 lines): Loads files fully into memory, converts to DuckDB tables
- `document_service.py` (410 lines): PDF/DOCX/PPTX parsing via PyPDF/python-docx/python-pptx — all in-memory
- `indexing_service.py` (243 lines): Reads processed data, chunks text, sends to Qdrant
- `duckdb_service.py`: In-memory DuckDB for SQL queries on tabular data; already has `temp_directory` and `memory_limit` config
- `text_processor.py` (73 lines): Text chunking for embeddings

### Gate 1 Council Mandates (10 items)

**M1: Process Isolation** (P0)
- File processing MUST run in a separate subprocess, NOT inside uvicorn workers
- Use `concurrent.futures.ProcessPoolExecutor(max_workers=PROCESS_WORKER_MAX_CONCURRENT)`
- Worker crash must not take down the API server
- Memory limit per worker: configurable via `resource.setrlimit(resource.RLIMIT_AS, ...)` (Linux) / soft enforcement via RSS monitoring (macOS)
- Timeout per file: configurable, default 30 minutes; grace period of 60s after SIGTERM for checkpoint flush

**M2: Generator/Iterator Pattern** (P0)
- All processors must yield chunks, not return complete results
- `StreamingTabularProcessor.__iter__()` → yields `pyarrow.RecordBatch` (target 10K-50K rows per batch)
- `StreamingDocumentProcessor.__iter__()` → yields `TextBlock(page_num, text, tables, metadata)`
- Backpressure: bounded `multiprocessing.Queue(maxsize=8)` between producer and consumer

**IPC Architecture (R4):**
```
Worker Subprocess                          Parent Process
┌─────────────────────┐                   ┌──────────────────────┐
│ StreamingProcessor   │                   │ ProcessingService     │
│   ├─ yields chunks   │──Queue(8)───────→│   ├─ ParquetWriter    │
│   └─ sends progress  │──Pipe(progress)─→│   ├─ ChunkedIndexer   │
│                      │←─Pipe(control)───│   └─ ProgressRelay    │
└─────────────────────┘                   └──────────────────────┘
```
- **Queue**: carries serialized RecordBatch (Arrow IPC format, zero-copy) or TextBlock (msgpack)
- **Pipe (progress)**: lightweight JSON: `{"bytes_processed": N, "total_bytes": M, "phase": "extracting"}`
- **Pipe (control)**: cancel signal, pause/resume
- Max Queue message size: 16MB. Batches >16MB are split.
- Worker writes NO files directly; parent handles all I/O to ensure atomic writes

**M3: Incremental PyArrow ParquetWriter** (P1)
- Parent process writes Parquet incrementally as RecordBatch chunks arrive from Queue
- Uses `pyarrow.parquet.ParquetWriter(path, schema, compression='zstd')`
- Target row group size: 64MB (configurable)
- Atomic writes: write to `{dataset_id}.parquet.partial`, rename on completion
- On worker crash: `.partial` file deleted, dataset marked failed

**M4: PDF Streaming — pypdfium2** (P0) *(Changed from PyMuPDF — AGPL license incompatible with customer deployment)*
- Use `pypdfium2` (Apache 2.0 license, wraps Google's PDFium)
- Process page-by-page: `pdf = pdfium.PdfDocument(path); for page in pdf: page.get_textpage().get_text_bounded()`
- Extract text per page, yield `TextBlock` immediately
- Table extraction: use `pdfplumber` (MIT license) for pages with detected table regions
- Fallback: current PyPDF for simple text-only if pypdfium2 unavailable

**M5: DuckDB Disk-Spill Configuration** (P1) *(Moved to Phase 1 per MP recommendation)*
- Configure DuckDB with `SET temp_directory = '{settings.data_directory}/temp/'` (aligns with existing `duckdb_service.py` pattern)
- Set `SET memory_limit = '{DUCKDB_MEMORY_LIMIT_MB}MB'`
- Enable disk-based spill for large SQL queries
- Add vacuum/cleanup on dataset deletion: `DROP TABLE IF EXISTS` + remove temp files
- DuckDB already does this partially — spec ensures it's explicit and configurable

**M6: Arrow-Based Parquet Sampling** (P1) *(Moved to Phase 1 as quick win)*
- Replace `COUNT(*)` full-scan with `parquet_file.metadata.num_rows` (zero I/O)
- Replace `DESCRIBE SELECT *` with `parquet_file.schema_arrow` (metadata only)
- Preview/sampling: `pf.read_row_group(0)` for first N rows, never full file
- Add `LIMIT` to all DuckDB preview queries as safety net

**M7: Progress Reporting** (P2)
- Worker sends progress via Pipe every 5 seconds or every chunk (whichever is less frequent)
- Parent relays to WebSocket/SSE channel for dataset
- Progress payload: `{bytes_processed, total_bytes, chunks_processed, estimated_time_remaining}`

**M8: Resumable Processing** (P2)
- **Checkpoint strategy per format (R3):**
  - **Parquet**: checkpoint = last completed row group index
  - **PDF**: checkpoint = last completed page number
  - **CSV/TSV**: checkpoint = row count processed (NOT byte offset — deferred due to quoted-field complexity)
  - **JSON (JSONL)**: checkpoint = line count processed
  - **DOCX/PPTX**: no resume (typically small enough); restart from beginning
- Checkpoint stored in `dataset_records.metadata` JSON field
- On resume: skip already-processed chunks, append to existing `.partial` Parquet

**M9: Memory Monitoring** (P1)
- Parent monitors worker RSS via `psutil.Process(pid).memory_info().rss` every 5 seconds
- Auto-kill (SIGTERM + 60s grace → SIGKILL) if exceeds 2x configured limit
- Log high-water mark per file in dataset metadata for capacity planning
- Alert if >80% of limit reached (logged, not user-facing)

**M10: Graceful Degradation** (P1)
- If streaming processor fails, fall back to current in-memory path for files <100MB
- For files ≥100MB where streaming fails: mark dataset as failed with clear error
- Never silently fall back — always record in `dataset_records.metadata.processing_mode`
- Log degraded-mode usage for tracking

### Security Hardening (R6)
- **Zip bomb detection**: Check compression ratio before extraction; reject if uncompressed_size / compressed_size > 100x
- **File size validation**: Reject files >10GB at upload (configurable `MAX_UPLOAD_SIZE_GB`)
- **Charset detection**: Try UTF-8 → UTF-8-SIG → Latin-1 → `chardet` detection → replace mode (same chain as BQ-106)
- **Timeout with backoff**: Per-phase timeouts (extraction: 80% of total, indexing: 20%); exponential backoff on transient failures (3 retries)

### Architecture

```
Upload API (uvicorn)
    │
    ├─ Small files (<LARGE_FILE_THRESHOLD_MB): Direct in-process (existing path, unchanged)
    │
    └─ Large files (≥LARGE_FILE_THRESHOLD_MB):
         │
         └─ ProcessPoolExecutor (max_workers=PROCESS_WORKER_MAX_CONCURRENT)
              │
              ├─ Worker subprocess
              │    ├─ StreamingTabularProcessor (yields RecordBatch via Queue)
              │    ├─ StreamingDocumentProcessor (yields TextBlock via Queue)
              │    └─ Progress reporter (→ parent via Pipe)
              │
              └─ Parent process
                   ├─ PyArrow ParquetWriter (atomic .partial → rename)
                   ├─ ChunkedIndexer (batch embed+upsert per chunk to Qdrant)
                   ├─ Checkpoint writer (metadata update per chunk)
                   ├─ Memory monitor (RSS watchdog)
                   └─ WebSocket/SSE progress relay
```

### Chunked Indexing (R5)
Current `IndexingService._extract_rows()` loads `LIMIT row_limit` rows into memory then embeds all. For large files this must change:

- New method: `IndexingService.index_streaming(dataset_id, chunk_iterator)`
- Receives chunks from the parent process (after Parquet write)
- For each chunk: extract text fields → `text_processor.chunk_text()` → `embedding_service.embed_batch()` → `qdrant_service.upsert_batch()`
- Batch size for Qdrant upsert: 100 points per call (existing pattern)
- Stable point IDs: `{dataset_id}:{chunk_index}:{row_index}` — enables idempotent upsert on resume
- Memory bounded: only one chunk in memory at a time

### Implementation Plan (Phased)

**Phase 1: Foundation + Quick Wins (24h)** — M1 + M2 + M5 + M6 + M10
- New file: `app/services/streaming_processor.py`
  - `StreamingTabularProcessor`: CSV/TSV via `pandas.read_csv(chunksize=)`, Parquet via `pyarrow.ParquetFile.iter_batches()`, JSON/JSONL via line-buffered reader
  - `StreamingDocumentProcessor`: PDF page-by-page (pypdfium2), DOCX paragraph-by-paragraph
  - Both implement `__iter__` yielding typed chunks
- New file: `app/services/process_worker.py`
  - Subprocess entry point
  - Memory limit enforcement
  - Queue + Pipe communication with parent
- Modify: `processing_service.py`
  - Route to streaming path when `file_size >= LARGE_FILE_THRESHOLD`
  - Fallback logic (M10)
- Modify: `duckdb_service.py`
  - Explicit disk-spill config (M5)
  - Cleanup on deletion
- Modify: `preview_service.py` / `duckdb_service.py`
  - Arrow-based metadata + sampling (M6)
- Modify: `indexing_service.py`
  - New `index_streaming()` method for chunked embed+upsert
- Security: zip bomb check, charset fallback chain

**Phase 2: Optimized I/O (16h)** — M3 + M4
- PyArrow ParquetWriter integration (atomic partial writes)
- pypdfium2 + pdfplumber page-by-page extraction
- Arrow IPC serialization for Queue messages

**Phase 3: Observability + Resilience (12h)** — M7 + M8 + M9
- Progress events via WebSocket/SSE
- Checkpoint storage + resume logic per format
- Memory monitoring with auto-kill + high-water logging

**Phase 4: Testing (8h)**
- Unit tests for each streaming processor (mock data, verify chunk counts)
- Integration tests with generated 500MB+ test files
- Memory profiling tests (verify RSS stays bounded)
- Crash recovery tests (kill worker mid-processing → verify cleanup)
- Fallback path tests (streaming fails → in-memory for small files)
- Zip bomb rejection test
- Checkpoint resume test (Parquet + PDF)

### New Dependencies
- `pypdfium2` — PDF streaming extraction (Apache 2.0 license ✅)
- `pdfplumber` — Table extraction from PDF pages (MIT license ✅)
- `psutil` — Worker memory monitoring (BSD license ✅)
- No other new dependencies (pyarrow, pandas already installed)

### Configuration (environment variables)
```
LARGE_FILE_THRESHOLD_MB=100         # Files above this use streaming path
PROCESS_WORKER_MEMORY_LIMIT_MB=2048 # Per-worker memory cap
PROCESS_WORKER_TIMEOUT_S=1800       # 30 min per file default
PROCESS_WORKER_GRACE_PERIOD_S=60    # Seconds for checkpoint flush after SIGTERM
PROCESS_WORKER_MAX_CONCURRENT=2     # Max parallel workers
DUCKDB_MEMORY_LIMIT_MB=512          # DuckDB in-memory budget
DUCKDB_TEMP_DIR=                    # Defaults to {data_directory}/temp/
PARQUET_ROW_GROUP_SIZE_MB=64        # Target row group size
MAX_UPLOAD_SIZE_GB=10               # Hard upload limit
STREAMING_QUEUE_MAXSIZE=8           # Backpressure queue depth
STREAMING_BATCH_TARGET_ROWS=50000   # Target rows per RecordBatch
```

### Acceptance Criteria
1. 1GB CSV file processes without exceeding 2GB worker RSS
2. 500MB Parquet file processes using only 2 row groups in memory at a time
3. 200-page PDF processes page-by-page (memory flat, not linearly growing)
4. Worker crash does not affect API server health; `.partial` files cleaned up
5. Files <100MB use existing path unchanged (regression-safe)
6. Progress events emitted every 5 seconds during large file processing
7. DuckDB queries on large datasets spill to disk instead of OOM
8. Preview/sampling for large Parquet uses metadata only (no full scan)
9. Zip bomb (100x+ compression ratio) rejected at upload
10. Chunked indexing produces stable Qdrant point IDs (idempotent on resume)

### Estimate
- **Total: 60 hours** (Council consensus from Gate 1)
- Phase 1: 24h, Phase 2: 16h, Phase 3: 12h, Phase 4: 8h
- Recommended: 4 build dispatches, each ~15h

### Risk Assessment
- **pypdfium2 maturity**: Less battle-tested than PyMuPDF for complex PDFs; mitigated by pdfplumber fallback for tables and PyPDF fallback for basic text
- **multiprocessing on Docker**: Verify `/dev/shm` sizing in customer Docker Compose; add `shm_size: 256m` to docker-compose.customer.yml
- **DuckDB temp storage**: Customer disk must accommodate spill files; document minimum free disk = 2x largest dataset
- **CSV resume correctness**: Deferred to future BQ — restart-from-beginning is acceptable for v1
