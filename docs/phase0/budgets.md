# Phase 0 budgets (spec 4.6)

Measured 2026-10-29 (S1837) on Koskadeux: Apple M3 Ultra, 256 GB RAM, macOS, Metal requested. Build: `vectoraiz-cli` release build from native `c481bbe` (chunk C1). Embedding model bge-m3 Q8_0 (the Phase 0 pick, [model-bakeoff.md](model-bakeoff.md)). Times are wall clock from `/usr/bin/time -l`; peak memory is its maximum resident set size, which counts the memory-mapped model file. Windows and 8/16 GB machines are **not measured** in this round (Max, 2026-10-29).

## Results

| Measure | 1,000 documents | 50,000 documents |
|---|---:|---:|
| Set | 1,000 real files from the bake-off corpus (352 HTML, 236 PDF incl. scanned, 151 XLSX, 133 DOCX, 128 MD; 447 MB) | 50,000 synthetic Markdown notes built from corpus text (207 MB, about 4 KB each) |
| Indexed | 996 (4 failed, see below) | 50,000 (0 failed) |
| Index time | 5,250 s (87.5 min), 0.19 docs/s | 16,934 s (4 h 42 min), 2.95 docs/s |
| Peak memory (index) | 2.57 GB | 2.57 GB |
| Index size on disk | 1.0 GB | 4.8 GB |
| One search, cold process (`search "invoice payment terms"`) | 13.7 s, 1.68 GB | **312.6 s**, 2.35 GB |

From the bake-off (298 documents, 123 questions, M3 Ultra): recall@10 1.000 and MRR@10 0.948 for bge-m3 with the app's hybrid search (this set hits a ceiling; see the bake-off note), time to first token 0.76 s for the 8 GB chat pick Qwen3-4B-Instruct-2507 (138 tokens/s), 1.29 s for the 16 GB pick Ministral 3 8B.

### Recovery

| Case | Result |
|---|---|
| Forced kill during indexing (30 documents, killed after 9) | The rerun skipped the 9 finished documents, indexed the other 21 and exited 0; search worked; a third run skipped all 30. |
| Full disk (40 MB disk image, 120 documents) | Indexing stopped with exit 1 after 20 documents (`No space left on device (os error 28)`; SQLite `database is full`). **Search also failed while the disk was full** (exit 1, `I/O error within the xShmMap method`). After the image was enlarged, the rerun skipped the 20, indexed the other 100 and exited 0; search worked. |

A first full-disk attempt with 30 documents did not fill the 40 MB image and is not counted.

## Findings

1. **Search time grows with the number of documents.** Each document generation is its own LanceDB table (`State::stage` creates a `chunks` table per generation), and `State::search` (`crates/vz-core/src/storage.rs`) opens every generation's table and runs a vector query and a full-text query on each, after loading every document's chunks into memory. 1,000 documents take 13.7 s per search; 50,000 take 5.2 minutes. The design's store choice is not the limit (LanceDB keeps vectors and full text for millions of rows in one table); the per-document table layout is. Phase 1 needs one chunks table with a generation column, a vector index and the full-text index, keeping the per-document atomic flip in SQLite (design 6.1 step 5), and an index migration (existing indexes are re-indexed).
2. **Search fails on a full disk.** A read should not need free space. The shared-memory file of SQLite's write-ahead log cannot be created when the disk is full. Phase 1: open the state database read-only for search and show "disk full" as its own error.
3. **Scanned PDFs can hit the worker timeout.** The 4 failures in the 1,000 run were scanned NASA PDFs (`en-nasascan-0008/0009/0010/0012`) that hit the 60-second worker timeout; 38 other scanned PDFs of the same source indexed. The likely cause is OCR time under load (the bake-off OCR'd its 14 scanned PDFs without a timeout); not confirmed. Phase 1 should give OCR a page-based time limit instead of one limit per document.
4. **Memory is set by the models, not the collection.** Peak memory was the same (2.57 GB) for 1,000 and 50,000 documents.

## Budgets for Phase 1 (proposed)

These are proposals to be confirmed with the Phase 1 Gate 2. The measured value is the M3 Ultra baseline; regressions are checked against it.

| Budget | Measured (Phase 0) | Phase 1 target, M3 Ultra | Phase 1 target, 8 GB machine |
|---|---|---|---|
| Index 1,000 mixed documents | 87.5 min | ≤ 60 min | to measure (UNVERIFIED) |
| Index 50,000 documents | 4 h 42 min (synthetic MD) | ≤ 5 h | to measure |
| Peak memory while indexing (8 GB tier) | 2.57 GB | ≤ 3 GB | ≤ 3 GB |
| Search, warm app, 50,000 documents | 312.6 s (cold CLI) | ≤ 1 s | ≤ 2 s |
| Time to first token, 8 GB chat pick | 0.76 s | ≤ 1 s | to measure |
| Recall@10 on the bake-off set | 1.000 | ≥ 0.98 | same set |
| Forced kill | resumes, no half-written document | same | same |
| Full disk | indexing stops cleanly; search fails | indexing stops cleanly, search keeps working | same |

## Reproduce

Scripts and raw outputs (not committed): `/Users/max/koskadeux-state/s1837/budgets/` on Koskadeux (`run-index.sh`, `recover-tests.sh`, `recover-disk2.sh`, `*.time`, `*.stderr` progress lines, `recovery*.log`). Command shape:

```
vectoraiz-cli --assets <assets> --models models-bge-m3.json --state <state-dir> index <folder>
vectoraiz-cli --assets <assets> --models models-bge-m3.json --state <state-dir> search "<query>"
```
