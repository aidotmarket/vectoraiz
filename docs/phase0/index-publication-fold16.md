# PR #153 — private best-effort markers and supervised validation (fold 16)

Base: `e073bfbad250760bda158c97600a33cecc1b28eb`. Publication branch: `build/vz-151-review-source-s1984`. Contract: runbooks `specs/VZ-INDEX-PUBLICATION-S1987.md` at `bf15990a`.

Read in full: codex2 REVISE `response-20261120-054755-503452-c6e4d75d.md` and CC APPROVE_WITH_MANDATES `response-20261120-054756-204327-2de3218f.md`. This fold implements the supplied merge-seat rulings; no additional Council review was requested.

## Changes

Validation records now live in `.vz-validation/` inside each index directory, including private staging indexes. There is no temporary-directory cache or fallback. The directory is created with mode `0700` on Unix; existing directories must be ordinary directories owned by the current effective UID with no group/other access before any marker is read. Windows creation uses a protected owner-only inheritable DACL and rejects reparse-point directories. Markers retain atomic replacement and file sync. Available markers move best-effort alongside staged generations during publication, so a subsequent no-change scan does not create new cache files or invalidate a strict version-1 review. Directory creation, serialization, sync, persistence and marker-move failures disable the cache without changing a successful validation result. Fingerprints still match generation identity, source path/hash, relative file names, sizes and nanosecond mtimes, with a second fingerprint after the probe.

The new reserved directory is excluded only from version-2 index review commitments, before traversal and accounting. Source and payload trees retain their existing rules, and version-1 reviews retain strict tree verification. Superseded flat-index cleanup removes the directory. This narrow new exclusion is necessary because the ruling moves disposable validation metadata into the index; writing or evicting a marker must not change reviewed published content.

All cold deep reads run in the existing parser worker supervisor: document decoding, fragment structural checks and decoding, full-text partition/posting reads, and production search. The worker has a five-second wall-clock deadline and the existing 1.5 GiB memory cap/watchdog (Unix allocation limit where supported, RSS supervision on macOS, Windows Job Object plus committed-memory supervision). Timeout, excessive memory, crash, panic or error returns a cache miss for that document. The parent never runs the deep Lance probe. The protocol uses the existing bounded frames and returns only OCR page status. A valid matching marker returns before any worker is launched.

The pre-recovery diagnostic WAL copy is opened owner-only from its first creation: mode `0600` on Unix, or a protected owner-only Windows DACL passed directly to `CreateFileW`. Copying and sync behavior are otherwise unchanged. `same_source` and its read-only identity helpers are unchanged.

## Regressions

- An unusable marker location: an unchanged document skips with its original generation intact, and an added document still publishes. A directory occupying the marker filename also verifies successful reuse after persistence fails. Staged and published validation both use best-effort marker persistence.
- Unix marker-directory trust checks reject insecure and symlinked directories and verify `0700` creation.
- A child test process sets umask `022`, exercises the real damaged-WAL diagnostic path, and asserts mode `0600` and exact byte equality. The process-local umask does not affect other tests.
- Two real indexed documents: invert the first third of `part_0_tokens.lance` in one generation, call `index_explicit`, require completion within 30 seconds, one rebuild and one skip, retain the untouched generation, and search the repaired document successfully.
- Fold-15 marker eviction/fingerprint/posting-damage tests now use the private in-index location. Existing version-2 review tests also exercise marker creation after review capture.

## Validation

All required local checks passed on macOS with Rust 1.90.0 and OpenSSL 3. Full untruncated logs, failed/aborted attempts, baseline verification and publication receipts are retained only in the assigned private evidence directory, outside the repository. Build output, assets, caches and the Paramiko 5.1.0 fixture environment were scratch-local.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed (`fmt-publish.log`). |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed (`clippy-publish.log`). |
| `cargo test --workspace -- --test-threads=1` | Passed: **450 tests across 20 suites, zero failures**; R10 was run separately (`tests-publish.log`, `tests-summary.json`). |
| Damaged-token bounded recovery | Passed: `index_explicit` returned in **2.773 seconds**, rebuilt one document, skipped the other and restored search (`token-latency.log`). |
| Pinned R10 | Passed: **0.8147×**, below 1.2 (`r10-publish.log`). |
| Windows permission helper cross-check | Passed for `x86_64-pc-windows-msvc` against the actual helper source and Windows API dependency/features (`windows-check.log`); compilation only, no Windows runtime claim. |
| `git diff --check` | Passed before publication; the staged diff was also checked. |

The full run includes all 167 core unit tests, 114 desktop tests, 15 publication tests, ten recovery tests, 92 pack unit tests, 27 pack integration tests and 15 worker-supervision tests, plus the remaining suites. The umask regression also runs one nested child test. The existing two-second review contention assertion and the strict version-1 no-change review assertion both passed unchanged.

R10 used the preserved `ef34f13986b3ea5b3f09d33b3f6f830a704f2342` driver from job `4ef395abe598`, also used by CC's pinned comparison. This job verified 32 pinned manifest, lockfile and core source/resource files byte-for-byte against that commit, verified that the driver is unchanged, and retained the executable hash and original build provenance in private evidence. Both builds use the debug profile. The existing production-path test creates 200 real one-chunk documents, loads engines outside the timers, warms both builds, then runs seven interleaved baseline/candidate pairs on the same files and flat index. It ran after all compilation and workspace tests had completed.

| Round | Baseline ms | Candidate ms |
|---|---:|---:|
| 1 | 209.379 | 116.169 |
| 2 | 361.125 | 176.097 |
| 3 | 216.150 | 183.683 |
| 4 | 117.220 | 56.042 |
| 5 | 187.750 | 365.165 |
| 6 | 227.952 | 190.465 |
| 7 | 282.366 | 160.991 |
| Median | **216.150** | **176.097** |

Every pair retained 200 generations, created zero workspace rows, performed zero re-embeddings and preserved database bytes. Matching records return before worker launch. These are warm-path measurements; cold validation deliberately uses a supervised child.

Initial validation exposed a missing optional field in one test request (`clippy.log`), the existing review contention timing assertion (`tests.log`), and the version-1 review regression caused by recreating markers after publication (`tests-final.log`). The field was added, and markers now move with their generations. The contention test passed in isolation in 0.93 seconds and in subsequent full runs without changing its code or assertion. No failed attempt is counted as passing validation; no assertions were weakened and no tests were removed or waived.

`same_source` and `directory_is_case_insensitive` remain byte-identical to the base, with hashes retained in `source-identity-readonly.json`. No hosted Native CI or Windows execution is claimed by these local results. CC's green hosted Native CI mandate (macOS and windows-2022) remains a prerequisite to merge. This fold is committed and pushed without merging.
