# PR #153 — publication recovery and search reuse (fold 14)

Base: `a5efe914d775b6327eaf23aa3f0dce0ffcee4700`. Publication branch: `build/vz-151-review-source-s1984`. Contract: runbooks `specs/VZ-INDEX-PUBLICATION-S1987.md` at `bf15990a`.

Read both reviews in full before editing: codex2 `response-20261120-034232-372137-232b12d6.md` and CC `response-20261120-034233-022229-1e91d2d5.md`.

The scan recovers an orphan WAL through a read-write SQLite connection under the existing writer fence before library or generation cleanup. A valid WAL header is never eligible for quarantine. Header checks cover magic, format version, page size and SQLite's endian-dependent checksum. Cleanup reads the recovered committed database. The regression restores a real committed WAL without SHM: its deletion, surviving row, WAL-only generation, selected folder and latest job reason survive a no-change re-index with zero re-embeddings.

Search and generation reuse now share `storage::search_generation`: open the Lance dataset, execute vector search, open/query the full-text index and consume the result streams. The scan runs at most eight probes on blocking workers, keeping dataset opening and query planning off the scan runtime thread, with no persisted validation cache. The entire reuse operation remains under the existing panic/error boundary; failure rebuilds only that document. Two-document regressions separately set an unsupported manifest reader-feature bit and corrupt the FTS artifacts. Each first proves real search fails, then requires one rebuilt document, one skipped control and a search hit for the repaired document.

`same_source` and its case helper are unchanged and remain read-only. The cheap flat-path naming nit is handled by canonicalizing the index directory when choosing a quarantine sibling, including `.`. Quarantine siblings remain outside generation/workspace cleanup so retries cannot delete diagnostic bytes. Recipe-change recovery and avoiding probes of changed files remain outside this fold.

## Validation

Validation runs in the assigned checkout on macOS with Rust 1.90.0 and OpenSSL 3. Assets, caches, build output and the fresh `git archive ef34f139` baseline live under job scratch. Full logs and failure receipts live only in the assigned private evidence directory.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed (`fmt-publish.log`). |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed (`clippy-publish.log`). |
| `cargo test -p vz-core -p vz-pack -- --test-threads=1` | The requested final-source invocation passed 276 tests and failed the existing invalid-WAL quarantine assertion (`tests-publish.log`); one R10 test was ignored and run separately. The remaining 27 pack integration tests passed separately (`pack-integration-publish.log`), for **303 passed, one failed**. The prior complete `--no-fail-fast` run had the same single failure. |
| R10 pinned measurement | Final-source repeat passed (`r10-publish.log`): baseline median **1473.337 ms**, candidate median **1651.165 ms**, ratio **1.1207**, below 1.2. The seven recorded pairs ran after the full suite and separate pack integration tests completed; fixture preparation overlapped the tail of those checks. All eight pairs preserved candidate database bytes, 200 generations, zero workspace rows and zero re-embeddings. The earlier worker-based run also passed at 1.1576 (`r10-final.log`) while the suite was active. |
| `git diff --check` | Passed; repeated before commit. |

## Unresolved contradictory WAL requirements

The bundled SQLite accepts both an empty WAL and a 32-zero-byte WAL on a normal read-write recovery, then removes them. A direct rusqlite probe reproduced this independently of indexing (`sqlite-invalid-wal-behavior.log`); SQLite's bundled `walIndexRecover` also explicitly ignores invalid magic/page sizes/checksums. Consequently, requiring both a failed read-write recovery before quarantine and a quarantine sibling for those two fixtures cannot hold for this SQLite build.

This implementation follows the requirement that quarantine is allowed only after read-write recovery fails and the WAL header is independently unusable. It does not invent a SQLite failure or rename a potentially committed WAL. The old pack assertion requiring a quarantine sibling is retained and fails; green validation is **not** claimed. The user was asked which rule should take precedence; no answer has been received. Preserving invalid bytes requires validating/quarantining them before SQLite can consume them, or relaxing the condition that native recovery must fail first. Alternatively, accepting successful native recovery requires changing the two quarantine expectations.

The first build attempts used a cached incompatible OpenSSL configuration; final builds use OpenSSL 3. The first full test attempt hit the existing 60-second OCR worker timeout; its isolated repeat and the next full core run passed. An initial guard-test assertion assumed invalid WAL bytes survive a corrupt-main error; SQLite consumes them there too, so the corrected test checks that the main database stays unchanged and no quarantine occurs. The first exploratory R10 run, before worker-based query planning, failed at 1.3134; it is not counted as a passing result. All failed logs are retained outside the checkout.

Hosted native CI and Windows execution are not claimed. No merge or Council request was made.
