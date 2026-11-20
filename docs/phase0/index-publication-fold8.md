# PR 153 fold 8

Contract: `aidotmarket/runbooks` `bf15990a`, `specs/VZ-INDEX-PUBLICATION-S1987.md`. Publication design and stored binding format are unchanged. The first fold-8 commit removes the stray tracked `.assets` symlink; assets remain local test inputs only.

## Review disposition

| Finding | Disposition |
| --- | --- |
| codex2 HIGH 1 | Fixed: cleanup requires an ordinary recipe, its content-derived directory name, an index/workspace schema, an existing writer lock, and containment under this library. Unrelated siblings survive startup and recipe replacement; a genuine superseded recipe is reclaimed. |
| codex2 HIGH 2 | Fixed: desktop command delegates source availability to its shared command backend and core scanner. The desktop backend test searches A, requests missing B and keeps A, removes and requests A and gets empty search. |
| codex2 HIGH 3 / CC L1 | Fixed using the suggested shared strict walker: v2 supplies prunes and the logical database entry; both versions reject over-depth directories and duplicate normalized paths. No binding format migration. |
| codex2 HIGH 4 | Fixed: flat cleanup syncs an ownership marker before deleting the database and removes it last. A subprocess is killed after database removal; startup reclaims old generations and preserves the new active index. |
| codex2 MED 5 / CC M1 | Fold 7 already fixes the candidate locked-file read: zero-length files use a zero-byte reader while retaining node, existence and hash checks. SQLite reads retry whole transactions within a bounded budget and report `ReviewDatabaseBusy`. Writer/rollback/commit and busy-without-invalidation tests retained. Windows execution remains unverified locally; no hosted CI monitoring was requested. |
| codex2 MED 6 | Fixed: production applies volume-aware comparison to the single already-resolved identity. Integration coverage branches on the actual filesystem case behavior and checks missing-source revocation through re-index. |
| codex2 MED 7 | Fixed: the watch sender and scan control register under the same mutex used by cancellation. A coordinated registration test observes the accepted cancellation in the installed control; the desktop backend startup-cancel test prevents publication. Existing post-commit rejection tests retained. |
| codex2 MED 8 / CC M2 | Fixed: unchanged-row comparisons use a path-keyed map. Published reuse checks the document, latest manifest and data-file footers without opening LanceDB per document; staged retries retain the table read. The opt-in R10 test compares the pinned baseline on the identical 200-document fixture and records generation, workspace and embedding counts. Measurements below. |
| CC L2 | Fixed: a mismatching workspace resets immediately before its first write. Missing/content-refused different-source requests preserve the existing workspace byte-for-byte and the original source resumes it. |
| CC L3 | Fixed: compute the exact revoked set before attempting the old writer fence; report pending revocation only for a nonempty write set. |
| CC L4 | Fixed conservatively: startup preserves recipe directories while the pointer needs repair. A successful pointer publication enables reclamation again. |
| CC L5 | Fixed: accept-first invokes acceptance; export-first invokes the reviewed-payload delivery gate. Both go through existing-generation native fence acquisition. |
| CC L6 | Fixed: missing lock files durably invalidate; other acquisition I/O errors propagate without durable invalidation. |
| CC NIT: compatibility API / explicit option | Retained: the public flat-State indexing API remains useful for callers and for the pinned-base measurement. Removing it is unnecessary to meet the publication contract. |
| CC NIT: test-only activation helpers | Retained for older regression coverage; production no longer uses them. The former test-only source comparison now runs in production. |
| CC NIT: revoke checkpoint context | Fixed: a checkpoint failure after revoke commit carries `PublishedFailure` context. |
| CC NIT: cancellation before pointer rename | Fixed: an accepted cancellation at pointer commit entry includes the retry/resume hint for the retained candidate. |
| CC NIT: Unix-only symlink test | Retained: Windows symlink creation depends on host privileges. No claim of Windows symlink-race execution; production resolves the request only once. |

No contract regressions were removed. The former invalid-folder guard test now covers empty request syntax; unavailable-folder behavior is covered by the desktop command backend test so it can reach core revocation.

The pipeline script runs its subprocesses through `rtk`; its acceptance assertions are unchanged.

Resume validation exposed an incorrect regression fixture: the existing decision table permits a completed empty-source switch to publish. The workspace-preservation test now uses a missing source and a content-refused source, both of which have no publication write set; it then verifies that the original source resumes its retained success. Publication policy is unchanged.

## R10 measurement

The ignored integration test `t15_r10_measured_nochange_against_ef34f139` creates 200 real searchable text generations, warms both builds, then records seven baseline/candidate pairs against the same source paths and flat index. Engines load before timing in both processes. Baseline timing includes `State::open` plus `index::index`; candidate timing uses `index_explicit`, including its fences and publication decision. Both are debug builds with the same lockfile/toolchain and machine. The test requires median candidate/base <= 1.2, zero re-embeddings, zero workspace rows, unchanged generation count, and byte-identical candidate `state.sqlite`.

Build `examples/r10-baseline.rs` unchanged in a scratch extraction of `ef34f139`; do not edit another job's checkout. Set `VZ_R10_BASELINE_BIN` to that executable, then run:

```sh
rtk run cargo test -p vz-core --test publication t15_r10_measured_nochange_against_ef34f139 -- --ignored --exact --nocapture --test-threads=1
```

Measured on 19 November 2026 on this macOS host, debug profile, pinned baseline `ef34f139` and candidate production code at preserved head `67d830a2` (the resume changes only another test fixture and these notes). The preserved, successfully compiled publication test executable ran the measurement; the baseline driver was rebuilt from the verified scratch extraction. All 42 baseline core files checked match the pinned commit, and the lockfile and driver match the candidate. The final-source workspace run is reported separately below.

| Round | `ef34f139` ms | Candidate ms | Generations before → after | Workspace rows | Re-embeddings |
| --- | ---: | ---: | --- | ---: | ---: |
| 1 | 1062.211 | 1658.534 | 200 → 200 | 0 | 0 |
| 2 | 3089.338 | 1831.727 | 200 → 200 | 0 | 0 |
| 3 | 3149.836 | 1980.560 | 200 → 200 | 0 | 0 |
| 4 | 2973.022 | 523.339 | 200 → 200 | 0 | 0 |
| 5 | 4609.017 | 610.304 | 200 → 200 | 0 | 0 |
| 6 | 2575.619 | 241.405 | 200 → 200 | 0 | 0 |
| 7 | 2234.745 | 622.470 | 200 → 200 | 0 | 0 |
| Median | **2973.022** | **622.470** | **200 → 200** | **0** | **0** |

Median candidate/base = **0.2094×**, passing the required **≤1.2×** bound; measurement test: **1 passed**. Each candidate scan also preserved `state.sqlite` byte-for-byte and created no workspace database. The host was heavily loaded and samples have substantial variance; this establishes the recorded fixture result, not a claim of a stable 4.8× speedup. Apple Metal initialization delayed a baseline process before its timer started; its sampled stack and the complete timing log are retained in private evidence.

## Resume validation

`cargo fmt --all -- --check` and `git diff --check` passed. The resumed job reused a filesystem-cloned copy of the existing target directory; no clean was run, and other preserved jobs remained read-only.

The preserved fold-8 executables were run from this job's target copy, with scratch-local temporary data and one test thread. Production code is identical to the final source. Results:

| Preserved executable / subset | Passed | Failed | Notes |
| --- | ---: | ---: | --- |
| Publication | 11 | 0 | R10 ignored here and executed separately; the corrected refusal fixture excluded because this executable predates that test-only correction. |
| Recovery | 6 | 0 | Includes generation/manifest damage and repair. |
| Binding (`review_store::tests`) | 29 | 0 | Includes strict depth/path rules, writer transaction, and lock contention. |
| Desktop command backend | 1 | 0 | Missing B preserves A; missing A revokes; startup cancel prevents publication. |
| Atomic registration | 1 | 0 | Accepted cancellation reaches the registered scan control. |
| Interrupted flat cleanup | 1 | 0 | Process death after database removal, followed by restart cleanup. |
| R10 measurement | 1 | 0 | Seven pairs, table above. |
| **Total** | **50** | **0** | Preserved-artifact validation, not a completed final-source workspace suite. |

The previous workspace failure and its full log are preserved as `prior-workspace.log` in private evidence; it was the empty-source expectation corrected above. The corrected missing/content-refused fixture still requires execution from the final source.

Required final-source commands were invoked with full logs retained: `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo test --locked --workspace -- --test-threads=1`, and `scripts/pipeline-e2e.py`. Publication-time status: **all three required final-source commands are unfinished**, stopped for the explicit 45-minute push budget. The workspace command was still compiling dependencies and had completed **zero runtime tests**. Clippy and the pipeline's initial CLI build were waiting for the shared Cargo target lock; neither has a completed validation result. These are not passes. The corrected refusal fixture remains unexecuted from final source. Full logs and the process/assigned-port shutdown receipt are retained under this job's private evidence directory.

## Limits

Windows hosted T13/T14 outcomes remain unverified in this local macOS run. Tests use the production desktop indexing backend without launching a webview. The volume regression exercises the case-insensitive APFS host. A case-sensitive APFS image was created successfully, but macOS denied its mount (`Permission denied`); no image remained attached. Execution on a case-sensitive volume remains unverified. The fast published-generation probe checks structure and node readability; it does not decode every stored row or full-text index. Deeper row/full-text corruption still requires search-time detection; staged retries and failed-file revocation retain full table reads. Missing data and a truncated latest manifest have explicit repair regressions. Ownership validation is conservative: directories whose ownership cannot be established are preserved rather than recursively deleted.
