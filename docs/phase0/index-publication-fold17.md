# PR #153 — preserve publication on unavailable validation (fold 17)

Base: `d24d748afbbe09b09dab0b0ee692d83b4884bc60`. Publication branch: `build/vz-151-review-source-s1984`. Contract: runbooks `specs/VZ-INDEX-PUBLICATION-S1987.md` at `bf15990a`, read in full.

Read in full: codex2 REVISE `response-20261120-071212-457252-b8625afc.md` and CC REVISE `response-20261120-071213-161508-39918fcb.md`. No additional Council review was requested.

## Changes

Generation validation now has three outcomes: valid OCR status, confirmed damage, or an unavailable validator. `Result<Option<Vec<u32>>>` carries these separately, and an unavailable validation carries a typed error through staging and the scan. Missing binaries, spawn or sandbox startup failures, supervisor failures, protocol failures, and timeouts before validation starts abort the scan before the publication decision. Original published rows, jobs, selection and generations remain intact. An acknowledged probe that reports unreadable content or reaches its reading time/memory cap is damaged and remains eligible for per-document rebuilding. A crash without a damage report is unavailable. A generation changing during validation also aborts rather than authorizing revocation.

The worker acknowledges validation only after request decoding, resource-limit setup and runtime creation, immediately before storage reading. Both timeout and memory-limit classification check this acknowledgment. The existing five-second deadline and 1.5 GiB cap remain. Concurrent cold probes are reduced from eight to two (CC L3).

Only staging a newly created generation can persist a new validation marker. Unchanged reuse can read a trusted matching marker but neither creates a missing marker directory nor restores an evicted marker. Published staged markers still move with their generation. This preserves the strict tree of a markerless pre-fold-16 index, including version-1 review bindings. Such legacy generations continue to require full cold probes until rebuilt; this is the deliberate compatibility tradeoff. Warm generations with existing matching markers remain worker-free.

Windows creation explicitly sets the current user's SID as owner instead of relying on the token's potentially different default owner ([Microsoft ownership documentation](https://learn.microsoft.com/en-us/windows/win32/secauthz/owner-of-a-new-object)). On Windows, an existing marker directory is checked for reparse points, current-user ownership and its existing DACL before any marker is read. Unknown ACE forms and grants to other principals disable trust conservatively. Insecure contents are ignored, the owner and DACL are reset to the current user, and stale markers are deleted. A private sentinel disables later trust until deletion completes, including after interrupted or failed cleanup; in-process directory checks are serialized so another probe cannot trust the repaired ACL before stale markers are gone. The owner-only sentinel uses exclusive `CREATE_NEW`, avoiding writes through an untrusted symlink. New temporary marker files also use the private file creator before atomic persistence, ensuring their explicit owner and DACL do not depend on token defaults or inheritance.

The Unix umask/WAL regression starts its current test executable directly with `std::process::Command`, with an explicit spawn error message. It has no `rtk` dependency. `active_index.rs`, including `same_source` and all identity helpers, remains byte-identical to the exact base.

## Regressions

- Three real documents in the flat explicit/legacy layout: hide the worker, verify the warm scan skips all three, remove all validation markers, then require an infrastructure error and an identical complete index snapshot (database, selection, jobs, generation bytes and directory layout). Restore the worker with an unwind guard and verify all three documents remain readable.
- Capture strict version-1 and version-2 bindings after removing the marker directory from a real flat index, scan unchanged, and verify both original bindings and continued absence of the marker directory. The existing cancelled-success invalidation assertions remain.
- Worker supervisor fixtures distinguish no-start timeout, acknowledged reading timeout, startup exit and crash after acknowledgment.
- Windows-only tests reject a marker from a broad DACL, remove untrusted markers before allowing subsequent trust, and exercise the real scan with matching markers in a broad directory and an unavailable worker. That scan must attempt a full probe and return infrastructure failure.
- Existing damaged-token, posting-damage, marker-persistence-failure and WAL tests remain; the eviction regression now requires that unchanged reuse leave the missing marker absent.

## Validation

All required local checks passed on macOS with Rust 1.90.0. Full logs, command receipts, failed attempts, pinned-baseline provenance, contract and reviews are retained only in the assigned private evidence directory outside this checkout. Build output, models, fixture environment and caches are scratch-local.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed (`fmt-verified.log`). |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed (`clippy-verified.log`). |
| `cargo test --workspace -- --test-threads=1` | Passed: **452 workspace tests plus the isolated umask child, zero failures** (`tests-no-rtk-verified.log`, `tests-verified-summary.json`). The entire run used a PATH verified to have no `rtk`. |
| Worker build | Passed (`worker-build-verified.log`). |
| Windows helper and cfg-Windows regression | Cross-target clippy with warnings denied passed for `x86_64-pc-windows-msvc` (`windows-clippy-publish.log`); compile/lint evidence only. |
| Pinned R10 | Passed: **0.2408×**, below 1.2 (`r10-verified.log`). Warm worker independence is separately covered by the hidden-worker regression. |
| `git diff --check` | Passed before publication; staged changes were checked again. |

| Round | Baseline ms | Candidate ms |
|---|---:|---:|
| 1 | 123.570 | 66.196 |
| 2 | 169.675 | 70.719 |
| 3 | 1355.283 | 67.173 |
| 4 | 299.513 | 74.134 |
| 5 | 278.918 | 66.214 |
| 6 | 350.057 | 48.776 |
| 7 | 99.529 | 78.053 |
| Median | **278.918** | **67.173** |

Timings vary between rounds; preceding complete pinned runs also passed at **0.4931×** and **0.5573×** (`r10-final.log`, `r10-publish.log`).

The pinned R10 comparison uses the preserved baseline driver built against `ef34f13986b3ea5b3f09d33b3f6f830a704f2342`. This job verified 44 preserved manifest, lockfile and core source/resource/test files byte-for-byte against that commit and retained the executable SHA-256, driver source and original build receipt/log. Both profiles are debug. The existing unchanged production test seeds 200 real documents, warms both builds, runs seven interleaved measured pairs, and asserts unchanged database bytes, generation count, workspace-row count and re-embedding count.

Windows helper cross-compilation is compile/lint evidence only; no Windows runtime or hosted Native CI result is claimed here. The existing hosted macOS and Windows CI mandate remains a prerequisite to merge. No merge is performed.

CC's initial-staging validation cost (L2), unsupported-WAL-version recovery policy and broader marker pruning remain outside this correction: removing deep validation, guessing a newer WAL format, or mutating legacy review trees would require separate behavior decisions.
