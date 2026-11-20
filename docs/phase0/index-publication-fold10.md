# PR 153 fold 10

Contract: `aidotmarket/runbooks` `bf15990a`, `specs/VZ-INDEX-PUBLICATION-S1987.md`. No publication design, recipe, pack, or stored review binding change.

## Dispositions

| Finding | Disposition |
| --- | --- |
| Hosted publication-failure test | The source disappears after its walk. Fold 9 deliberately clears its unusable staged work and removes the unpublished recipe candidate. Expect only the active directory, while retaining the assertions for the unchanged pointer, source selection and searchable document contents. The former expectation of a second retained directory contradicted this cleanup behavior and its no-resume diagnostic. |
| codex2 HIGH 1 | Pointer staging and cleanup validate `.vz-scan` with `symlink_metadata` and canonical containment as a real direct child of the library. Same-recipe scans apply the same check before opening a workspace. Cleanup only unlinks ordinary files with the exact generated `pointer-<UUID>.tmp` name. Regression coverage preserves an external directory and its matching file through startup/scan cleanup, rejects staging through its symlink, preserves symlink entries and unrelated names, and reclaims a genuine crash-left temp. |
| codex2 MED 2 | Missing-source comparison performs read-only identity comparisons of surviving ancestors (Unix `metadata` device/inode; Windows `same_file` handles with volume serial/file index). Unix comparison needs no directory read handle. Alternate ancestor spelling establishes case semantics without temporary source directories. Missing suffixes retain exact matching on case-sensitive volumes; macOS comparison includes Unicode decomposition, while Windows uses ordinal case comparison without normalization. The Unicode revocation regression removes write permission from the parent and checks the hierarchy stays empty. |
| codex2 MED 3 / R9 | Every referenced fragment must match the length recorded in the decoded manifest, in addition to being ordinary and carrying its footer. Unknown lengths conservatively cause a cache miss. The two-document recovery regression also truncates a fragment while preserving its complete footer, requires exactly one rebuild and one skip, and requires a search hit containing the damaged document's text. No per-document database/query open is added. |
| codex2 MED 4 | Cleanup records are serialized into a private temporary file, synced, then atomically installed with `NamedTempFile::persist_noclobber`. Its no-replacement installation preserves a collision; the library directory is synced before database deletion. An empty ordinary marker can be retired only with independent owned-database evidence (and a single link on Unix). Nonempty invalid records and symlinks stay untouched. Fault injection and killed-process restart cover before installation and after database removal; empty-marker repair and collision regressions cover the legacy partial-write case. The recovery record schema is unchanged. |
| LOW/NIT | Keep the contract's accepted legacy common-parent inference and combined cancel/cleanup wording limits. No test-only activation helpers were removed. This fold adds no copy-per-scan path. |
| CC review | `/Users/max/council/cc/response-20261119-224558-580032-d44ae97f.md` did not exist at job start. No Council review was requested. |

## Validation

All commands ran in the assigned checkout on macOS, using Rust 1.90.0 and OpenSSL 3. Shell/subprocess execution used RTK. Full logs, including setup failures, are retained outside the checkout in the job evidence directory.

| Command / check | Result |
| --- | --- |
| `cargo test -p vz-core active_index::tests::publication_failure_after_scan_keeps_old_recipe_documents_and_source -- --exact --nocapture` | Requested first. Setup attempts exposed missing Cargo PATH, cached OpenSSL 4 configuration, and the unbuilt parser worker. After the worker build, the unchanged test executable reproduced the hosted failure exactly: `left: 1`, `right: 2`. The corrected test subsequently passes. |
| `cargo test -p vectoraiz logical_comparison_rejects_same_size_persisted_value_change -- --nocapture` | Passed. Its expected panic is precisely `preflight changed database contents` at `desktop_pipeline.rs:323`; file-size preservation is checked before that panic. |
| `cargo clippy -p vz-core -p vz-pack --all-targets -- -D warnings` | Passed. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed, including the final-source rerun after Windows suffix separator normalization. |
| `cargo test -p vz-core -p vz-pack -- --test-threads=1` | The first run compiled before the assertion correction: 157 core unit passes, the stale assertion failure, and one missing Apple Vision helper failure. The helper was then built. Both complete corrected runs passed 297 tests with one ignored R10 baseline test, including all core/pack integration tests and doctests; the last run includes the Windows-only refinement. |
| `cargo test --workspace --exclude vz-core --exclude vz-pack -- --test-threads=1` | Passed: 139 additional tests, including 114 desktop tests and 15 worker supervision tests; all other workspace crates and their doctests completed. Together with core/pack, this covers every workspace package. |
| `npm --prefix app run build` | Passed, including the emitted CSS/font/CSP checks. Dependencies and emitted assets were scratch-local. |
| `cargo build -p vectoraiz --features smoke-test,custom-protocol` followed by `scripts/native-smoke.py` | Passed both real WebView modes, `--smoke-test --cpu` and `--pipeline-smoke-test --cpu`, with both completion markers. A scratch-local launcher selected the assigned Cargo target binary and isolated application data. The missing-child refusal preserves backend storage and fresh search results. |
| `scripts/pipeline-e2e.py` through the scratch-local launcher | Passed after installing NumPy. Verified 21 indexed / 21 unchanged, search/answers/citations, pack hashes, generated Python query, live SFTP (including lost acknowledgement), cancellation/resume, migration, refusal/recovery, and missing-current-source revocation. The default-off CLI was rebuilt in the script's finalizer. MinIO was not enabled. |
| `cargo fmt --all -- --check`, `git diff --check` | Passed. |

The parser worker and Apple Vision helper were built as native CI prerequisites. Verified embedding/chat models and PDFium were fetched into scratch. The CLI pipeline acceptance uses scratch-local PyArrow, NumPy and Paramiko, plus a launcher assigning its disposable SFTP fixture port 55008. Its first attempt stopped at the generated Python query example because NumPy was absent; that failed log is preserved separately.

Windows execution, a case-sensitive-volume runtime, MinIO acceptance, and an exact-head R10 baseline timing are not claimed. The ignored timing test requires a separately built `ef34f139` baseline driver. No runtime pass is inferred from a preserved checkout or older published binary.
