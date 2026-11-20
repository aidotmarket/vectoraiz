# PR 153 fold 12

Contract: `aidotmarket/runbooks` `bf15990a`, `specs/VZ-INDEX-PUBLICATION-S1987.md`. Base: `14ecb6328bed4a4ba2d008ceb75c60afe37dfa6f`. No publication design, recipe, pack, or stored review binding change.

## Dispositions

Both review files were read in full before editing: codex2 `response-20261120-020445-516733-791cf944.md` and CC `response-20261120-020446-131358-fe72c6ce.md`.

| Finding | Disposition |
| --- | --- |
| codex2 HIGH 1 / R6 | Case rules come only from the differing component's actual containing directory. A missing container requires exact spelling; no ancestor fallback. `same_source` and its case helper remain read-only. A portable production-path regression indexes `S/Docs`, renames S, refuses `S/docs`, and checks unchanged selection, pointer, full document values, generation IDs and search results. An exact-spelling request then revokes the unavailable source. The Windows case-sensitive-container regression covers the same rename sequence with case sensitivity explicitly enabled. An unconditional unit test also verifies missing-parent comparisons leave the parent's entries and mtime unchanged. |
| codex2 MEDIUM 2 / R9, R10 | The footer must use a version Lance 0.38.2 actually supports: 0.3, 2.1 or 2.2. Manifest 2.0 still maps to footer 0.3, but footer 2.0 is rejected. Decode every column metadata record with Lance's own protobuf type after validating its bounds. The two-document fixture separately changes the footer to 2.0 and inserts an invalid protobuf tag without changing record bounds; each requires one rebuild, one skip, removal of the damaged generation and a search hit from the repaired document. The direct `lance-file` dependency uses the already pinned 0.38.2 package. |
| codex2 MEDIUM 3 / CC LOW L1 | Run structural validation before all reuse queries, including staged retries and the failed-rebuild path. Catch unwinding from the remaining Lance query as a cache miss. Cancelled-scan retries cover malformed staged offsets, footer 2.0 and invalid column metadata as well as damaged document JSON, requiring exactly one document rebuilt. Forced PDF rebuild failures cover malformed column counts, global-buffer offsets, footer 2.0 and column metadata: indexing returns an error, revokes the damaged generation, and succeeds after assets are restored. |
| CC LOW L2 | A `state.sqlite-wal` without `state.sqlite-shm` returns the existing retryable `ReviewDatabaseBusy` classification, rather than durably invalidating the review. Read-only capture neither creates sidecars nor uses a potentially stale immutable database while WAL exists. The accepted-review regression checks accept and export both return this transient error, preserve database/WAL bytes, create no SHM, and succeed after the orphan is removed. |
| CC NITs | Replace the machine-specific sensitive-mount path with explicit `VZ_CASE_SENSITIVE_TEST_ROOT` configuration. Missing configuration produces a skip diagnostic; invalid configured roots fail. Missing-container spelling coverage runs unconditionally. Windows case sensitivity still reports unavailable capability explicitly; Windows execution and that capability must be checked in hosted CI. The pre-existing lock-contention timing bound is unchanged. |

## Validation

Commands run in the assigned detached checkout on macOS using RTK, Rust 1.90.0 and OpenSSL 3. Full logs and receipts, including failures, are retained outside the checkout under the assigned job evidence directory. Build artifacts, model/PDFium assets, temporary files and the extracted `ef34f139` baseline are scratch-local.

| Command / check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed (`fmt.log`). |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed (`clippy.log`). |
| `cargo test -p vz-core -p vz-pack -- --test-threads=1` | Passed (`tests.log`): **301 passed**, zero failures, one ignored R10 test (run separately). Includes 163 core unit tests, 13 publication tests, 6 recovery tests, 92 pack unit tests and 27 pack integration tests; doctests completed. |
| R10 pinned-baseline measurement | Passed (`r10-serial.log`). Rebuilt the driver from a fresh `git archive ef34f139` with the unchanged current `examples/r10-baseline.rs`; source/lock/driver/binary provenance is recorded outside the checkout. One warm-up pair and seven recorded pairs after the full suite finished: baseline median **111.388 ms**, candidate median **58.547 ms**, ratio **0.5256**, below 1.2. Every pair preserves 200 generations, creates no workspace rows, performs zero re-embeddings and leaves database bytes unchanged. |
| `git diff --check` | Passed (`diff-check.log`). |

Windows-specific cases are cfg-gated and were not executed on macOS. Hosted macOS/Windows CI is not claimed. No Council review was requested.

The configured case-sensitive APFS mount test actually ran and passed (`mount-case-executed.log`), using `VZ_CASE_SENSITIVE_TEST_ROOT=/Volumes/Backups of Koskadeux`. The earlier `mount-case.log` invocation used an incomplete exact filter and ran no tests; it is not counted as coverage. The portable missing-parent unit and rename integration regressions both ran in the full suite.

The first R10 run overlapped the end of the full suite and also passed (ratio 0.4831, `r10.log`); the serial repeat above is the reported measurement. No validation failures occurred.
