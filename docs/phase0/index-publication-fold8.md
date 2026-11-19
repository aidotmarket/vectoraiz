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
| CC L2 | Fixed: a mismatching workspace resets immediately before its first write. Missing/empty different-source requests preserve the existing workspace byte-for-byte and the original source resumes it. |
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

The pipeline script uses `rtk run` instead of the unavailable `rtk proxy` invocation in this environment; its acceptance assertions are unchanged.

## R10 measurement

The ignored integration test `t15_r10_measured_nochange_against_ef34f139` creates 200 real searchable text generations, warms both builds, then records seven baseline/candidate pairs against the same source paths and flat index. Engines load before timing in both processes. Baseline timing includes `State::open` plus `index::index`; candidate timing uses `index_explicit`, including its fences and publication decision. Both are debug builds with the same lockfile/toolchain and machine. The test requires median candidate/base <= 1.2, zero re-embeddings, zero workspace rows, unchanged generation count, and byte-identical candidate `state.sqlite`.

Build `examples/r10-baseline.rs` unchanged in a scratch extraction of `ef34f139`; do not edit another job's checkout. Set `VZ_R10_BASELINE_BIN` to that executable, then run:

```sh
rtk run cargo test -p vz-core --test publication t15_r10_measured_nochange_against_ef34f139 -- --ignored --exact --nocapture --test-threads=1
```

Measurement and validation results are recorded after execution.

## Limits

Windows hosted T13/T14 outcomes remain unverified in this local macOS run. Tests use the production desktop indexing backend without launching a webview. The volume regression exercises the case-insensitive APFS host. A case-sensitive APFS image was created successfully, but macOS denied its mount (`Permission denied`); no image remained attached. Execution on a case-sensitive volume remains unverified. The fast published-generation probe checks structure and node readability; it does not decode every stored row or full-text index. Deeper row/full-text corruption still requires search-time detection; staged retries and failed-file revocation retain full table reads. Missing data and a truncated latest manifest have explicit repair regressions. Ownership validation is conservative: directories whose ownership cannot be established are preserved rather than recursively deleted.
