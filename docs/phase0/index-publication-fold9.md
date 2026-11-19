# PR 153 fold 9

Contract: `aidotmarket/runbooks` `bf15990a`, `specs/VZ-INDEX-PUBLICATION-S1987.md`. Publication design, recipes and stored review binding format are unchanged.

## Dispositions

| Finding | Disposition |
| --- | --- |
| Hosted smoke red | The smoke deliberately requests `fixture/directory-that-does-not-exist`, a different missing source. The fixture itself still exists. Its old `Folder not found` expectation predates the scanner-owned availability decision. Expect the specified refusal retaining the previous folder; retain the subsequent search-result preservation assertion. This is the different-source discard row, not missing-current-source RevokeSource. Existing CLI and desktop backend tests cover removal of the current source and empty search. |
| codex2 HIGH 1 | New flat cleanup records use exclusive `create_new`, never truncate an existing node. Recovery validates a structured record against the canonical root, retained ordinary recipe and recorded database digest; any surviving database must have the owned schema and matching digest. Unix hard links and symlinks are rejected. The recipe stays until destructive cleanup finishes. Invalid collisions fail closed and marker-only unrelated roots are preserved. Collision and killed-process restart regressions retained. |
| codex2 MED 2 | Published reuse decodes the latest Lance manifest, validates its typed schema and checks every referenced fragment file. It keeps cheap local reads rather than opening a LanceDB query for each published document. The regression corrupts protobuf while retaining the footer, offset and recorded length, verifies only the damaged document rebuilds and checks search afterward. |
| codex2 MED 3 | `validate_inner` propagates typed `ReviewDatabaseBusy` before any durable invalidation. The real native review test holds SQLite exclusive contention during accept-first and export-first validation, clears contention and accepts/packages the same generation successfully. |
| codex2 MED 4 | The cancellation mutex now spans operation-guard acquisition through fresh sender/control installation. The coordinated test pauses immediately after guard acquisition, verifies cancellation waits for registration, then verifies every acknowledged cancellation reaches the scanner and prevents a pointer publication. |
| codex2 MED 5 | Recipe candidates initialize their owned workspace schema even before a first success, so even crash-left empty candidates have ownership evidence; an unavailable walk closes both writer handles and immediately removes its unpublished candidate. Successful no-change publication removes a mismatched staged workspace. Tests check bounded candidates across repeated missing-source refusals, B cleanup after no-change A, byte-identical published A database, and preservation of A retry bytes across refused B. |
| codex2 MED 6 | Missing suffix comparison asks the volume itself to resolve both spellings inside an isolated temporary directory on that volume; no ASCII-only case folding remains. The production regression establishes whether `ÉcoleSource` and `éCOLEsOURCE` are equivalent before removal, then verifies the corresponding revoke/discard behavior. The actual requested source is still resolved only once. |
| codex2 coverage note T8 | The symlink regression now retargets the request during the first scan's progress callback, before the remaining document completes, and verifies both documents and selection retain the original resolved source. |
| LOW/NIT | Keep the compatibility API and older test-only activation helpers for existing coverage. Windows execution and a case-sensitive-volume run cannot be claimed from this macOS execution. Legacy common-parent inference and combined cancel/cleanup wording remain accepted LOW/NIT limits from the contract; changing either is unnecessary for these fixes. |
| CC review | The requested CC file did not exist when this job started. No review was requested or dispatched. |

No regression tests were removed. The new flat cleanup record is private recovery evidence, not a change to the published index, recipe, pack or review binding format. Unverifiable old/colliding cleanup markers are deliberately preserved rather than used as deletion authority.

## Validation

Budget-stop results, 19 November 2026 (macOS):

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check`, `git diff --check`, `node --check scripts/pipeline-smoke.js` | Passed. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed after the six finding fixes, before the final immediate empty-candidate deletion refinement. The final-source rerun (`clippy-final2`) was invoked and stopped while waiting for the desktop Cargo lock. It is unfinished, not a final-source pass. |
| `cargo test --workspace -- --test-threads=1` | Invoked against final manifests; stopped waiting for the Cargo lock. Zero final-source runtime tests completed. |
| `scripts/pipeline-e2e.py` | Invoked through a scratch-local launcher selecting the assigned SFTP port. Its CLI/worker build was still waiting for the Cargo lock at stop; acceptance did not run. An initial launcher import failure is preserved separately and was corrected before this invocation. |
| Desktop `--pipeline-smoke-test --cpu` | Not launched: the final-source `cargo build -p vectoraiz --features smoke-test,custom-protocol` was still compiling `vz-pack` when stopped for the explicit 50-minute push budget. The queued WebView harness therefore has no runtime result. |
| R10 and targeted new regressions | Queued against final source; no final-source runtime result before the budget stop. The fold-8 timing is not asserted for this changed probe. |

The cached base CLI locally reproduced the exact diagnostic and exit 1 for the deliberately nonexistent child source. That establishes the smoke's obsolete expectation but is **not** a completed final-source desktop reproduction. Full local reproduction of both requested acceptance journeys remains unverified. Windows and hosted CI were not run or awaited.

Full logs, failed/unfinished command receipts, baseline diagnostic, final diff and shutdown/cleanup receipts are retained outside the repository in this job's private evidence directory. Earlier commands loaded before the final dependency manifest were stopped and restarted; their failures are preserved, including the early clippy unresolved-`same_file` result. Temporary models, inputs, caches and build output were scratch-local; disposable processes are stopped before scratch reclamation. No runtime pass is claimed from preserved older test executables.
