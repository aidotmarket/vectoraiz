# PR 153 fold 13

Contract: `aidotmarket/runbooks` `bf15990a`, `specs/VZ-INDEX-PUBLICATION-S1987.md`. Base: `888388740978cc503f3ca4ebd91d00c28a835fe9`. No publication design, recipe, pack, or stored review binding change.

## Dispositions

Both review files were read in full before editing: codex2 `response-20261120-024119-861774-615d3f5a.md` and CC `response-20261120-024120-504546-d3a630f0.md`. The pinned contract was read from the runbooks repository at `bf15990a`.

| Finding | Disposition |
| --- | --- |
| codex2 MED 1 / MED 2, R9 | Every staged or published reuse decision now calls one guarded probe. Manifest protobuf decoding and `Manifest::try_from` both run inside `catch_unwind`. Existing cheap structural checks precede fragment decoding. The probe opens every fragment with Lance's v2 `FileReader`, which decodes every column's metadata and nested page encodings, and drains every batch with all columns projected. Errors and unwinding become a per-document cache miss. Decoded Arrow buffers are also validated before reuse. There is no published early return or separate staged validation path. |
| Regressions | The cancelled two-document scan damages the first staged manifest's parent hierarchy, then requires exactly one rebuild and one skip on retry. The published two-document fixture separately damages only nested direct-page encoding bytes and fills a page's buffers with `0xff`, preserving outer metadata and record bounds. Each case requires one rebuild, one skip, removal of the damaged generation, and successful search returning the repaired document. Prior corruption regressions remain. |
| CC NIT, content damage | A general I/O bounds adapter rejects reversed or out-of-file requests before Lance splits them into chunks. This also contains the huge indirect read produced by all-`0xff` offsets, without enumerating encoding shapes. Actual Lance decoding and Arrow validation remain responsible for interpreting content. |
| CC L1, orphan WAL | Under the re-index writer fence, a WAL without SHM is probed using read-only SQLite with `readonly_shm=1`. Only SQLite's `CannotOpen` result for this condition triggers quarantine. Rename preserves the WAL as a timestamped, UUID-suffixed sibling of the index, outside review bindings and automatic index/workspace cleanup. Other errors propagate. Review stays read-only and its message accurately names re-index recovery. The accepted-review regression covers zero-length and 32-zero-byte WALs, verifies byte preservation before and after quarantine, re-index completion without sidecars, and successful accept/export afterward. |
| R10 | Probe at most eight immutable generations concurrently under the existing scan fence. Keep only scan-local OCR status keyed by the full row and staged/published location; reuse still checks this walk's source hash. Small fragments (at most 1 MiB) serve decoder reads from a bounded buffer; large fragments retain bounded file I/O. Use Lance's synchronous decoder on a blocking worker, avoiding per-buffer runtime scheduling while still decoding every column and batch. No persisted validation cache or snapshot copy is added. |
| R6 | `same_source` and its case helper are unchanged and remain read-only. |

## Validation

Validation runs in the assigned detached checkout on macOS, using RTK, Rust 1.90.0 and OpenSSL 3. Full command logs, receipts, the stopped corruption-probe run and its process sample are saved outside the checkout in the assigned private evidence directory. Assets, build caches, temporary files and a fresh `git archive ef34f139` baseline are scratch-local.

| Command / check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed (`fmt-publish.log`). |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed (`clippy-publish.log`). |
| `cargo test -p vz-core -p vz-pack -- --test-threads=1` | Passed on final source (`tests-publish.log`): **301 passed**, zero failures, one ignored R10 test run separately. Includes 163 core unit tests, 13 publication tests, 6 recovery tests, 92 pack unit tests and 27 pack integration tests; doctests completed. |
| R10 pinned-baseline measurement | Passed (`r10-publish.log`), serially after the full suite. Freshly rebuilt `git archive ef34f139` baseline with the unchanged `examples/r10-baseline.rs`; source, lock, driver and binary hashes are recorded outside the checkout. One warm-up pair and seven recorded pairs: baseline median **126.611 ms**, candidate median **137.211 ms**, ratio **1.0837**, below 1.2. Every pair preserved 200 generations, created no workspace rows, performed zero re-embeddings and left database bytes unchanged. |
| `git diff --check` | Passed (`diff-publish.log`). |

The first corruption run was stopped after the all-`0xff` page offsets exposed Lance's huge indirect-read splitting loop; its log and process sample are retained. The general I/O bounds adapter fixes that failure. Intermediate compile errors from using a private Lance module export were corrected; all final checks use the public export. Early R10 measurements failed at 2.2527 (serial async decoding) and 1.3533 (concurrent async decoding); bounded concurrency plus buffered synchronous decoding produced the passing final measurement above. Two earlier full-suite attempts hit the pre-existing lock-contention timing flake; isolated repeats and both subsequent full-suite runs passed. That test and its two-second bound are unchanged. No failed or stopped attempt is counted as a passing validation.

Windows-specific cases are cfg-gated and were not executed on macOS. The optional case-sensitive mount test was not configured in this job; the unconditional read-only source-identity tests passed. Hosted macOS/Windows CI is not claimed. No Council review was requested.
