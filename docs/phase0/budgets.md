# Phase 0 budgets (spec 4.6)

Measured 2026-10-29 (S1837) on Koskadeux: Apple M3 Ultra, 256 GB RAM, macOS, Metal requested. Build: `vectoraiz-cli` release build from native `c481bbe` (chunk C1). Embedding model bge-m3 Q8_0 (the Phase 0 pick, [model-bakeoff.md](model-bakeoff.md)). Times are wall clock from `/usr/bin/time -l`; peak memory is its maximum resident set size, which counts the memory-mapped model file. Units: memory in GB (10^9 bytes, from the `time -l` byte counts); index size on disk is allocated storage from `du -k`, in GiB (2^30 bytes). Windows and 8/16 GB machines are **not measured** in this round (Max, 2026-10-29).

## Results

| Measure | 1,000 documents | 50,000 documents (Markdown) | 50,000 documents (mixed) |
|---|---:|---:|---:|
| Set | 1,000 real mixed files from the bake-off corpus (352 HTML, 236 PDF incl. scanned, 151 XLSX, 133 DOCX, 128 MD; 447 MiB allocated on disk) | 50,000 synthetic **Markdown-only** notes built from corpus text (207 MB logical size, about 4 KB each). A document-count baseline only | 50,000 synthetic files with the 1,000 set's format mix and sizes (17,600 HTML, 9,700 PDF, 2,100 scanned PDF, 7,550 XLSX, 6,650 DOCX, 6,400 MD; English, French, German, Spanish, Japanese; 23.6 GB). This is the set spec 4.6 asks for |
| Build | native `c481bbe` | native `c481bbe` | native `a33eefe` (before #100) |
| Indexed | 996 (4 failed, see below) | 50,000 (0 failed) | 49,995 (5 failed, all scanned PDFs: worker timeout after 60,000 ms) |
| Index time | 5,250 s (87.5 min), 0.19 docs/s | 16,934 s (4 h 42 min), 2.95 docs/s | **about 117 h** wall in three segments (0.12 docs/s overall; 0.13 docs/s excluding segment 2, see below) |
| Peak memory (index) | 2.57 GB | 2.57 GB | 4.55 GB (segment 3; segment 2 3.36 GB; segment 1 not recorded) |
| Index size on disk (allocated) | 0.98 GiB (1,022,824 KiB) | 4.75 GiB (4,984,668 KiB) | **64.45 GiB** (67,582,660 KiB) |
| One search, cold process (`search "invoice payment terms"`) | 13.7 s, 1.68 GB | **312.6 s**, 2.35 GB | **381.7 s, 20.18 GB** |

### The mixed 50,000 run (S1837-S1874, 2026-10-30 to 2026-11-04)

It ran on Koskadeux while the machine did its normal work (other agents' builds, the GitHub runner VM), so the times are not from an idle machine. It ran in three segments because the first two were stopped:

| Segment | Wall time | New documents | Note |
|---|---:|---:|---|
| 1 (2026-10-30 13:51 to about 22:50 CET) | about 9.0 h | 5,416 | Stopped by a Koskadeux service restart; no exit line |
| 2 (2026-10-31 01:01 to 09:31 CET) | 30,583 s (8.5 h) | 0 | Only re-checked 4,713 already indexed documents, then stopped for a reboot checkpoint. This build read every stored document to skip one, which #100 fixed (one document per skip) |
| 3 (2026-10-31 11:04 CET to 2026-11-04 14:33 CET) | 358,109 s (99.5 h) | 44,579 (+5 failed) | Started by skipping the 5,416 from segment 1 with the same slow skip; how long that took was not recorded |

Over segments 1 and 3 (about 108.5 h) it indexed 49,995 documents, about 0.13 docs/s. That figure includes segment 3's slow skip and the shared machine, so it overstates the true indexing time by an unknown amount. Segment 3 used 88,490 s user and 136,002 s system CPU time. Raw outputs: `RESULTS-r3.txt`, `mixed50000-r2.time`, `mixed50000-r3.time`, `pipeline.log` and the corpus manifest (`mixed50000-manifest.json`, generator and seed) under `/Users/max/koskadeux-state/s1837/budgets/` on Koskadeux.

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
4. **Indexing peak memory was the same in both runs.** Peak resident memory while indexing was 2.57 GB for the 1,000 mixed and the 50,000 Markdown runs (2,570,272,768 and 2,565,095,424 bytes respectively). That the models dominate it is an inference: the runs record no memory attribution and differ in workload. Search memory does grow with the collection (1.68 GB at 1,000, 2.35 GB at 50,000; 45,390 versus 132,999 stored chunks), because search loads every document's chunks into memory (`storage.rs`). On the mixed 50,000 set, one cold search took 381.7 s and peaked at 20.18 GB resident memory (18.89 GB footprint), far above the 3 GB budget, and the highest recorded indexing peak was 4.55 GB (segment 1 was not recorded). Which part of the indexing peak comes from the parsers rather than the models was not measured.
5. **The index is large for mixed documents.** 64.45 GiB for 50,000 mixed documents (23.6 GB of files), against 4.75 GiB for 50,000 short Markdown notes. The per-document table layout in finding 1 is one likely contributor; not measured.
6. **Resume after a stop must not re-read the library.** On the build used here, every skipped document cost a read of all stored documents (segment 2 above: 8.5 h without indexing anything). Fixed by #100 in 0.0.11; not re-measured at 50,000.

## Outstanding

- **50,000 mixed documents on a current build and an idle machine.** The mixed run above used a build from before #100 on a busy machine, so its 117 h is an upper bound, not a budget. Re-run it on the Phase 1 store (one chunks table) to set the budget.

## Budgets for Phase 1 (proposed)

These are proposals to be confirmed with the Phase 1 Gate 2. The measured value is the M3 Ultra baseline; regressions are checked against it.

| Budget | Measured (Phase 0) | Phase 1 target, M3 Ultra | Phase 1 target, 8 GB machine |
|---|---|---|---|
| Index 1,000 mixed documents | 87.5 min | ≤ 60 min | to measure (UNVERIFIED) |
| Index 50,000 mixed documents | about 117 h wall (shared machine, pre-#100 build; 0.12 docs/s overall, 0.13 excluding segment 2). Markdown-only baseline 4 h 42 min | set after a re-run on the Phase 1 store | to measure |
| Peak memory while indexing (8 GB tier) | 2.57 GB (1,000 mixed; 50,000 Markdown); 4.55 GB (50,000 mixed: highest recorded peak, segment 1 unrecorded) | ≤ 3 GB | ≤ 3 GB |
| Peak memory while searching, 50,000 documents | 2.35 GB (Markdown, cold CLI); 20.18 GB (mixed, cold CLI) | ≤ 3 GB | ≤ 3 GB |
| Search, warm app, 50,000 documents | 312.6 s (Markdown, cold CLI); 381.7 s (mixed, cold CLI) | ≤ 1 s | ≤ 2 s |
| Index size on disk, 50,000 mixed documents | 64.45 GiB | to set with the Phase 1 store | same |
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
