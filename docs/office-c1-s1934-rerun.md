# Office C1 S1934 macOS rerun

**RETURN_TO_DESIGN — FAIL. No enabled distribution.**

Base `0c152e7f4f694d3f3d2f7d552c600b161a62e105`; runbooks `f35ab5fe7ae3849499638a98622636d4a0588c86` (S1934 merged 68ae83d6) plus reviewer N1–N3.

Private manifest: `/var/tmp/koskadeux/private-evidence/8588b12d5b30/manifest-v3.json`. Corpus, queries, runfiles, licence/hash inventories and credentials stay private. Windows: **NOT RUN**; required-host qualification fails.

Apple M3 Ultra / Mac15,14; macOS 27.2.1 build 26B131; 256 GiB RAM. Shared host; no OS cache flush. Lance 0.38.2 / LanceDB 0.22.2; Lance/lance-index/lance-linalg opt-level 3.

Largest measured corpus: 1,000 documents / 54,099 chunks / 7,747 pages. **Largest qualified corpus: none.** Any access-path, recipe or corpus change requires requalification (N1).

## 1 search

**FAIL**

25 spawned identities × first/warm = 50 queries per path. First-open is not OS cold cache. Full per-identity scheduling/guard/embed/vector/FTS/ranking/fetch timings, absolute/baseline RSS, plans and index statistics are private.

| Path | Index ms | Max query ms | Incremental RSS MiB¹ | Vector recall@10 | Extraction regression |
|---|---:|---:|---:|---:|---:|
| exact | 2.27 | 289.62 | 349.05 | 1.000 | 36/50 |
| streamed | 2.33 | 2212.99 | 76.86 | 1.000 | 36/50 |
| flat | 2087.37 | 315.04 | 233.92 | 0.972 | 34/50 |
| pq | 32791.41 | 303.72 | 269.16 | 0.980 | 36/50 |
| hnswpq | 34808.11 | 310.59 | 332.78 | 0.964 | 36/50 |
| hnswsq | 3559.71 | 334.63 | 292.45 | 0.976 | 34/50 |

¹ Endpoint above idle-model baseline, not a peak bound.

Preparation: spool/vector-file 47629.30 ms + Lance/FTS 35253.51 ms = 82882.81 ms; RSS endpoint 1140.05 MiB. Combined preparation misses 60 s.

Diagnostic selection: **Lance Exact**. Explicit exact choice at E3 scale: measured query latency and endpoint RSS below bounds, no ANN training/approximation loss. No path qualifies all A1 and both-host requirements; IVF_PQ remains an unqualified larger-corpus fallback.

Refresh: no-op 0.0000 ms; 180 adds + 180 updates + 180 deletes (540/54,099 = 0.9982%); embedding 2542.89 ms; CoW clone 25.21 ms; FTS/table update 9308.23 ms; post-embedding preparation 9333.44 ms; aggregate incremental RSS endpoint 553.64 MiB. **Exact has no vector index to update (N2).**

Missing/failing: 32/50 extraction matching is not human relevance; held-out human A1 queries absent; combined spool plus Lance/FTS preparation exceeds 60 s; agreed topped-up mixed corpus absent; cold means first-open, no OS cache flush; RSS endpoints do not prove peaks/promotion bound; supplemental sampled trace starts at PQ; synthetic Refresh lacks source-hash/extraction accounting and concurrent 25-way search; recipe-change, repeated promotion/retirement, crash/recovery, answer/indexing/refusal maximum not qualified; Lance random training row IDs not exposed; Windows NOT RUN.

## Validation and remaining obligations

fmt: PASS; clippy: PASS; tests: PASS. Full logs retained privately.

All omissions are failures. These trials do not close C0/source integration, native full-run cache exclusion, tenant consent/propagation/outages, installed power/network/browser/viewer/accessibility or IT trust/certificate obligations. No Council/review request or enabled distribution.
