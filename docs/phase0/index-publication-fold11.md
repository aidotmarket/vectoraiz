# PR 153 fold 11

Contract: `aidotmarket/runbooks` `bf15990a`, `specs/VZ-INDEX-PUBLICATION-S1987.md`. Base: `dcbbea6c8e5c9e0aa8ee14b28eb4a1bcf6cdb2dc`. No publication design, recipe, pack, or stored review binding change.

## Dispositions

Both review files were read in full before editing: codex2 `response-20261120-005811-026661-3294a41f.md` and CC `response-20261120-005811-829379-5d079d6c.md`.

| Finding | Disposition |
| --- | --- |
| codex2 HIGH 1 / R6 | Compare the persisted spelling component by component, without statting or resolving any historical ancestor. Only the canonical request's containing directory is queried for case rules. The regression indexes `A/docs`, renames A, installs A as a symlink to B, then requests missing `B/docs`; selection, pointer, complete document values, generation IDs and searchable contents remain unchanged. |
| codex2 MEDIUM 2 / CC L1, N1 | Read the containing directory's own case rules, including mount points and drive roots: macOS `pathconf(_PC_CASE_SENSITIVE)`, Windows read-attribute-only handles and `FileCaseSensitiveInfo`, with a conservative volume fallback. Missing intermediate containers use their nearest surviving request ancestor. Case-sensitive directories preserve distinct spellings. Windows-only integration regressions cover missing case variants directly under a drive root and preservation of documents/selection/generations under an enabled case-sensitive directory. A macOS regression checks the existing case-sensitive mount and its numeric descendant without creating anything there. |
| CC fold-9 M1 (closed) | Remains closed. `same_source` and its case helper only compare spelling, read metadata/case information, and open Windows handles for reading attributes. They create/remove no source-tree entries. The volume-case regression also checks the containing directory's entry count and modification time remain unchanged; existing read-only-parent Unicode coverage is retained. |
| codex2 MEDIUM 3 / R9, R10 | Read the full 40-byte v2 fragment footer. Validate the format version against the manifest (including Lance's equivalent footer 0.3 / manifest 2.0 spelling), metadata ordering, contiguous column descriptors, global-buffer bounds, both offset-table lengths and the footer boundary, alongside the existing file-length check. No per-document database/query open is added. The two-document recovery test damages version and metadata-offset fields (both beyond EOF and zero) without changing length or `LANC`, proves a real Lance query cannot read each mutation before repair, then requires one rebuild, one skip and a search hit from the repaired document. |
| codex2 MEDIUM 4 | Remove empty-marker repair: database ownership does not authenticate an unrelated empty `.vz-flat-cleanup`. The regression now requires collision refusal and byte-for-byte preservation of both marker and database. Atomic no-clobber marker installation, restart before installation, nonempty/symlink collision preservation and restart after database removal remain covered. The record schema is unchanged. |
| CC carried LOW L2, L3 | Published recipe candidates also remove orphan `scan.sqlite-wal` and `scan.sqlite-shm` sidecars. Failure while reclaiming an unavailable-source workspace/candidate retains the source-unavailable and previous-folder diagnostic as context, plus the underlying cleanup error. |

## Validation

Commands ran in the assigned checkout on macOS with Rust 1.90.0 and OpenSSL 3, using RTK. Full logs, including intermediate failures, are retained outside the checkout under the assigned job evidence directory. Build artifacts, verified model/PDFium assets and the separately extracted `ef34f139` baseline are scratch-local. Native parser worker and Apple Vision helper prerequisites were rebuilt.

| Command / check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed on final source (`clippy-complete.log`). |
| `cargo test -p vz-core -p vz-pack -- --test-threads=1` | Passed on final source (`tests-complete.log`): **299 passed**, zero failures, one ignored R10 test (run separately below). Includes 161 core unit tests, 13 publication tests, 6 recovery tests, 92 pack unit tests and 27 pack integration tests; doctests completed. |
| R10 pinned-baseline measurement | Passed. The baseline driver was rebuilt from a fresh `git archive ef34f139` with the unchanged current `examples/r10-baseline.rs`. Seven recorded pairs: baseline median 92.266 ms, candidate median 56.666 ms, ratio **0.6142**, below 1.2. Every pair preserves 200 generations, creates no workspace rows and performs zero re-embeddings; database bytes are unchanged. Full tests were also active during this local measurement. |
| `git diff --check` | Passed. |

The first focused recovery/full unit runs exposed a footer-version alias: Lance writes 0.3 into the fragment footer while its manifest records 2.0. The check now accepts that documented equivalence. The malformed-offset reader probe also exposed a Lance 0.38 query-task panic; the regression catches it as evidence of unreadability before verifying repair. The failing logs are preserved; those attempts are not counted as passes.

Windows-specific cases are cfg-gated and were not executed on macOS. Hosted macOS/Windows CI is not claimed. No Council review was requested.

The case-sensitive APFS mount regression actually ran on `/Volumes/Backups of Koskadeux` (`mount-case.log`); it was not skipped. The temporary-directory case regression and the existing Unicode/read-only-parent revocation test also ran on macOS.
