# VectorAIz — Active Diagnostic Reference

**Last updated:** S209 (3 March 2026)
**Platform:** Mac Studio M3 Ultra (Titan-1) — ARM64 native Docker
**Current version:** v1.20.17 (frictionless serial auto-activation)

---

## 1. Active Issues

### RC#22: Subprocess Hang on PDF/XLSX — Total Queue Freeze (CRITICAL)

- **Status:** ROOT CAUSE IDENTIFIED — fix pending
- **Discovered:** S209 (3 March 2026)
- **Version:** v1.20.17

**Symptoms:**
- 35 files uploaded via batch. 6 processed to Ready. 2 stuck forever (PDF extracting 0%, XLSX indexing 0%). 27 files queued behind them, never started.
- Container at 3% CPU — idle, not processing.
- Health check returns 200, frontend polling dataset status every few seconds.
- "API ready" message never appears in application log — startup may not have fully completed.
- Only 1 Python process visible (uvicorn), no child subprocesses — they died or were never spawned.

**Files that succeeded (6):** RTF (52KB), TXT, CSV (772B), JSON (2KB), ICS (10KB), ICS (120KB)

**Files that hung:**
- `us-budget-2025.pdf` (2.4MB) — status=extracting, no parquet output
- `eu-ecommerce-transactions-50k.xlsx` (3.9MB) — status=indexing, parquet exists (1MB extraction OK)

**Processing paths — what succeeded vs what hung:**

| File Type | Category | Extraction Path | Indexing Path | Result |
|-----------|----------|----------------|---------------|--------|
| RTF | DOCUMENT_TYPES | Subprocess → format_extractors (Python) | Subprocess → sentence-transformers | Ready |
| TXT | TEXT_TYPES | In-memory | Subprocess → sentence-transformers | Ready |
| CSV | TABULAR_TYPES | Subprocess → DuckDB streaming | Subprocess → sentence-transformers | Ready |
| JSON | TABULAR_TYPES | Subprocess → DuckDB streaming | Subprocess → sentence-transformers | Ready |
| ICS x2 | DOCUMENT_TYPES | Subprocess → format_extractors (Python) | Subprocess → sentence-transformers | Ready |
| **PDF** | **DOCUMENT_TYPES** | **Subprocess → pypdfium2 + pdfplumber (C libs)** | Never reached | **HUNG** |
| **XLSX** | **SPREADSHEET_TYPES** | In-memory → DuckDB (succeeded) | **Subprocess → sentence-transformers** | **HUNG** |

**Root cause (CONFIRMED — reproduced twice):**

**`ProcessWorkerManager` semaphore leak in the indexing path.**

The `ProcessWorkerManager` uses a `threading.Semaphore(N)` to bound concurrent subprocesses. On M3 Ultra (24 cores), N=6 (`max(2, min(cores//4, 8))`).

Two code paths consume the semaphore:
- **Extraction** (`submit_tabular`/`submit_document`): acquire → `handle.iter_data()` → `finally: _cleanup()` → **release** ✓
- **Indexing** (`submit_indexing`): acquire → manual `while is_alive()` poll loop → `handle.wait()` → **NEVER calls `_cleanup()`** → **semaphore slot permanently leaked** ✗

Each file requires 1 extraction + 1 indexing = extraction releases, indexing leaks. After exactly N files complete, the semaphore is exhausted. The (N+1)th file's extraction call to `submit_*` blocks on `self._semaphore.acquire()` forever.

**Reproduction:**
- Run 1 (original install): 6 files → Ready, file 7 (PDF) stuck extracting, file 8 (XLSX) stuck indexing
- Run 2 (clean restart, PDF+XLSX first): both succeed (2 of 6 slots used)
- Run 2 continued (+10 files): 4 more → Ready (6 total), file 7 (`api_usage_logs.json` 12KB!) stuck extracting, file 8 (`sec-edgar-readme.htm` 154KB) stuck indexing

File type and size are irrelevant. It is always the (N+1)th file that hangs, where N = semaphore size.

**The fix is a one-liner:** Add `handle._cleanup()` to the indexing completion path in `processing_service.py:_run_indexing()` (after `handle.wait()`).

**Contributing factors (secondary):**
- `ProcessWorkerManager._active_processes` list grows forever — completed subprocesses never removed
- Per-file timeout 1800s/3600s far too long for UX
- `recover_stuck_records()` resets status but doesn't re-queue (RC#15)
- Embedding `preload()` is synchronous, may block event loop during startup

**Fix plan:**

| # | Fix | Impact | File | Priority |
|---|-----|--------|------|----------|
| **F1** | **Add `handle._cleanup()` after indexing `handle.wait()`** | **Fixes the deadlock** | `processing_service.py` | **P0** |
| F2 | Wrap entire indexing path in try/finally with _cleanup() | Belt-and-suspenders | `processing_service.py` | P0 |
| F3 | Re-queue "uploaded" files on startup after recover_stuck_records | Files process after restart | `main.py` | P1 |
| F4 | Clean `_active_processes` after subprocess completion | Prevent zombie accumulation | `process_worker.py` | P1 |
| F5 | Reduce timeouts: 5min extraction, 15min indexing | UX improvement | `config.py` | P2 |
| F6 | Make embedding `preload()` async | Don't block event loop | `main.py` | P2 |

**Workaround:** Restart container, then manually trigger reprocessing for stuck files via the UI.

---

### RC#19: allAI LLM Status Shows "not_configured"
- **Status:** INVESTIGATING
- **Symptom:** `GET /api/allai/status` returns `"llm": {"provider": "not_configured", "model": "none"}`
- **Context:** LLM is now provided via allAI (ai.market proxy). Status endpoint may not yet reflect migration.

### RC#15: No Re-Queue on Startup Recovery
- **Status:** UNFIXED (tracked as F3 in RC#22 fix plan)
- **Mechanism:** `recover_stuck_records()` resets extracting/indexing → uploaded but doesn't call `processing_queue.submit()`
- **Fix:** In `main.py` lifespan, after `recover_stuck_records()`, query all status=uploaded records and submit each to processing queue

### RC#16: WorkerHandle Cleanup Not Guaranteed
- **Status:** UNFIXED (tracked as F4 in RC#22 fix plan)
- **Mechanism:** `ProcessWorkerManager._active_processes.append(proc)` but never removes completed processes
- **Fix:** Add `_active_processes` cleanup in `WorkerHandle._cleanup()` or periodic sweep

---

## 2. Recently Fixed

### RC#18: Unsupported Format Types — FIXED (v1.20.14)
- Added pure Python extractors in `format_extractors.py` for RTF, ICS, VCF, ODT, EPUB, EML, MBOX, XML
- Confirmed working in v1.20.17: RTF and ICS files process to Ready

### RC#20: DuckDB Thread-Safety (v1.20.11, commit 758d58e)
- All 12 services converted from singleton to ephemeral context manager

### RC#21: Release Pipeline — GitHub Release Not Created (v1.20.11)
- Added `step5_create_release()` with `gh release create --latest`

### RC#17: iter_data() Pipe Deadlock (v1.20.8)
- Subprocess exit detection + timeout in `iter_data()`

### RC#14: Indexing Subprocess Hang — fixed by same mechanism as RC#17
### RC#13: Event Loop Deadlock — fixed with asyncio.to_thread()

---

## 3. Monitoring Quick Reference

```bash
# Health check
curl -s http://localhost:8080/api/health | python3 -m json.tool

# Container resources
docker stats --no-stream

# Processing state (inside postgres container)
docker exec vectoraiz-postgres-1 psql -U vectoraiz -d vectoraiz -c \
  "SELECT count(*), status FROM dataset_records GROUP BY status ORDER BY count DESC;"

# Qdrant collections (from inside vectoraiz container)
docker exec vectoraiz-vectoraiz-1 curl -s http://qdrant:6333/collections

# Application logs (JSON)
docker exec vectoraiz-vectoraiz-1 cat /app/logs/vectoraiz.jsonl | tail -50

# Check for zombie subprocesses
docker top vectoraiz-vectoraiz-1

# Filtered container logs
docker logs vectoraiz-vectoraiz-1 2>&1 | grep -E "error|failed|worker|timeout|MemoryError|SIGKILL"

# Nuclear reset stuck records + restart
docker exec vectoraiz-postgres-1 psql -U vectoraiz -d vectoraiz -c \
  "UPDATE dataset_records SET status = 'error' WHERE status IN ('extracting', 'indexing');"
docker restart vectoraiz-vectoraiz-1
```

---

## 4. Key Files

| File | Role |
|------|------|
| `app/services/processing_queue.py` | Queue workers, bounded concurrency, progress tracking |
| `app/services/processing_service.py` | DOCUMENT/TABULAR/SPREADSHEET_TYPES routing, extraction, indexing |
| `app/services/process_worker.py` | Subprocess spawn, WorkerHandle, MemoryMonitor, IPC |
| `app/services/streaming_processor.py` | StreamingDocumentProcessor (PDF/DOCX/PPTX), StreamingTabularProcessor |
| `app/services/format_extractors.py` | Lightweight extractors for RTF/ICS/VCF/ODT/EPUB/etc |
| `app/services/indexing_service.py` | Qdrant vector indexing, index_streaming |
| `app/services/embedding_service.py` | sentence-transformers model loading, preload() |
| `app/main.py` | Lifespan startup (queue init, preload, recovery) |
| `app/config.py` | Timeouts, memory limits, concurrency settings |

---

## 5. Root Cause Catalog

| # | Root Cause | Status |
|---|-----------|--------|
| 1-12 | Historical (in-memory loading, nginx, threads, BigInteger, etc.) | ALL FIXED |
| 13 | Sync processing blocks event loop | FIXED (v1.20.8) |
| 14 | Indexing subprocess hang | FIXED (v1.20.8) |
| 15 | No re-queue on startup recovery | **UNFIXED** — tracked in RC#22 F3 |
| 16 | WorkerHandle cleanup not in finally | **UNFIXED** — tracked in RC#22 F4 |
| 17 | iter_data() pipe deadlock | FIXED (v1.20.8) |
| 18 | Unsupported formats freeze pipeline | FIXED (v1.20.14) |
| 19 | allAI LLM shows not_configured | INVESTIGATING |
| 20 | DuckDB thread-safety (singleton) | FIXED (v1.20.11) |
| 21 | Release pipeline missing GitHub Releases | FIXED (v1.20.11) |
| **22** | **Semaphore leak in indexing path — deadlock after N files** | **CONFIRMED — fix is one-liner** |
