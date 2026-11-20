# PR #153 — WAL diagnostics and cached full generation validation (fold 15)

Base: `f089dbc9cbecdc3f0ab209dcb994ce34ce809722`. Branch: `build/vz-151-review-source-s1984`. Contract: runbooks `specs/VZ-INDEX-PUBLICATION-S1987.md` at `bf15990a`.

Read both REVISE reviews in full: codex2 `response-20261120-045157-071375-6ec61031.md` and CC `response-20261120-045157-838382-6cebba73.md`. This fold implements the merge-seat reconciliation supplied with the task.

## Changes

- Before SQLite opens an orphan WAL with an unusable header (short, bad magic, invalid page size or header checksum), copy its exact bytes to a timestamped diagnostic sibling and sync the copy. Leave the original for SQLite recovery. Diagnostic copies are never removed by index cleanup, including when the main database is corrupt.
- A checksum-valid unsupported WAL version returns a named error before recovery or generation cleanup and retains the original WAL. Real committed uncheckpointed WALs with only SHM absent still recover through SQLite.
- Extend orphan recovery to recipe-change scans under the old active index's writer fence, before cleanup. Existing held-reader migration behavior remains covered. `same_source` and its read-only identity comparison are unchanged.
- Fully validate a generation when staged, or when a matching validation record is absent. Within the panic guard, decode all fragment columns, load every FTS partition and decode every posting list (including positions) through Lance's `into_builder`, then open/query the production dataset/search path. Empty FTS vocabularies remain valid. Any error or panic makes that document a cache miss.
- Store an atomic versioned validation record keyed by generation ID, source path/hash, and the sorted relative file paths, sizes and nanosecond mtimes. Matching records return the stored OCR status without opening Lance or parsing the document. Records live in the per-user temporary cache outside the bound index tree; eviction safely triggers full validation. During this job the cache is under job scratch. Neither first validation of an existing generation nor ordinary reuse changes review-bound files. No per-scan generation copy or workspace row is added.

## Regressions

- Existing vz-pack zero-length and 32-zero-byte WAL diagnostic assertions remain unchanged.
- A real WAL with a flipped header salt byte is preserved byte-for-byte before recovery.
- A checksum-valid unknown-version WAL stops both same-recipe and recipe-change scans before an unreferenced generation sentinel can be deleted; main database and WAL bytes remain unchanged.
- Existing committed-WAL-without-SHM publication regression remains green in the targeted run.
- Two documents: replace exactly one compressed posting payload for the noninitial `zebras` term. The first term still searches successfully, while the damaged term fails. A fingerprint mismatch forces the guarded full probe, only that document rebuilds, the other generation ID is retained, and searching the damaged term succeeds afterward.
- Evict a validation record and re-index: the record is recreated, both documents skip, and generation files and mtimes stay identical. An ordinary subsequent reuse leaves the marker unchanged. Stop-word-only, punctuation-only and emoji-only documents exercise empty vocabularies.

## Hosted failure

Read the complete [Native CI log at the base SHA](https://github.com/aidotmarket/vectoraiz/actions/runs/44669290300). The `build (macos-15)` job failed in `review_build::tests::t15_review_accept_and_export_survive_private_scans_and_busy_does_not_invalidate`, at `crates/vz-pack/src/review_build.rs:1151`: SQLite consumed an invalid WAL before a diagnostic sibling was created. The pre-recovery copy fixes that cause without weakening the pack assertion.

## Validation

All commands ran in the assigned checkout on macOS with Rust 1.90.0 and OpenSSL 3. Build output, assets, the Python fixture environment and cache records were scratch-local. Full untruncated logs, failed attempts, baseline hashes and publication receipts are retained only in the assigned private evidence directory, outside the repository.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed (`fmt-publish.log`). |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed (`clippy-publish.log`). |
| `cargo test --workspace -- --test-threads=1` | Passed: **446 tests, zero failures**, across 20 suites, with R10 ignored here and run separately. Includes all 92 vz-pack unit tests and its `review_build` cases, 27 pack integration tests, 114 desktop tests, 165 core unit tests, 15 publication tests, 8 recovery tests and 15 worker supervision tests. SFTP fixtures ran with scratch-local Paramiko 5.1.0; no fixture import errors (`tests-publish.log`). |
| Pinned R10 | Passed: **0.3476×**, below 1.2 (`r10-publish.log`). |
| `git diff --check` | Passed. |

R10 used a fresh scratch extraction of `ef34f13986b3ea5b3f09d33b3f6f830a704f2342`, with the unchanged `examples/r10-baseline.rs` driver rebuilt against it. All 31 baseline core source/manifest/lock files were checked byte-for-byte against the pinned commit; driver and binary hashes are retained in private evidence. Both builds use the debug profile. The existing test creates 200 real one-chunk documents, loads engines outside the timers, warms both builds, then runs seven interleaved baseline/candidate pairs on the same paths and flat index. It ran serially after workspace checks and compilation had completed.

| Round | Baseline ms | Candidate ms |
|---|---:|---:|
| 1 | 123.975 | 39.472 |
| 2 | 114.829 | 41.795 |
| 3 | 131.134 | 40.687 |
| 4 | 111.811 | 43.760 |
| 5 | 116.201 | 42.463 |
| 6 | 117.270 | 37.048 |
| 7 | 128.000 | 40.766 |
| Median | **117.270** | **40.766** |

Every pair retained 200 generations, created zero workspace rows, performed zero re-embeddings and preserved database bytes. The baseline is near the unloaded timing reported by the reviewer, rather than the inflated fold-14 baseline.

Initial local setup attempts selected OpenSSL 4 through a copied CMake cache and failed in llama.cpp; those logs are retained. The scratch build was reconfigured for OpenSSL 3 before passing validation. A preliminary full workspace run also passed 446 tests, but lacked the optional SFTP fixture dependency; the final run above includes it. No failed attempt is counted as passing validation. No known-failing test remains.

Validation records are disposable cache data: eviction or fingerprint mismatch triggers another full validation. Fingerprints use file sizes and mtimes as authorized by the ruling. Cold probes retain the existing scan cancellation boundaries. Windows execution and a new hosted CI result are not claimed by these local results. This fold is pushed for review without merging.
