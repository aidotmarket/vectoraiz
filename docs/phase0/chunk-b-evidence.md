# Chunk B acceptance evidence

PR: https://github.com/aidotmarket/vectoraiz/pull/62 (base `native`).
Final hosted-runner status and logs are attached to the PR checks. This file
records local evidence separately from hosted CI and hardware claims.

| Acceptance | Evidence |
|---|---|
| Chunk boundaries and provenance | `cargo test --workspace`: UTF-8 bounds, overlap, document/page/heading retention |
| Atomic generation and crash recovery | Recovery integration test kills a child after a real Lance generation write and before its SQLite flip; old text survives; orphan is removed; a subsequent commit replaces it |
| Worker crash and timeout | Real child-process fault injection followed by a successful worker request |
| Expansion limits | Four real ZIP refusal tests cover entry count, part size, total expanded size and compression ratio; boundary tests also cover exact thresholds |
| macOS network denial | The same probe connects to a live local listener without the sandbox and fails under the production worker launcher |
| Worker memory | A macOS worker allocating beyond the supervised 1.5 GiB resident budget is killed; Windows uses a kernel Job Object memory limit |
| Parsers and OCR policy | Twenty CC0 documents across all six requested formats index; blank-page integration test executes Apple Vision on macOS and expects needs-OCR on Windows |
| CLI CPU indexing and search | 20 indexed with zero failures; second scan skips 20; the retention question retrieves the document stating thirty days |
| CLI chat citations | A generated sentence gives the thirty-day retention fact; streamed numbered sources resolve to existing document IDs and physical page numbers |
| Pack integrity | All six payload hashes match; Parquet references resolve; text first lines have no comma or tab; originals use neutral names |
| Example query | Clean Python dependency environment loads Parquet; recomputes the query embedding through the exact recorded recipe and linked CLI engine; expected top three match |
| Upload | Explicit CLI upload to pinned MinIO; all seven files including the manifest download byte-for-byte; server stopped unconditionally |
| Source revocation | An unavailable source fails explicitly and removes its searchable cached content |
| UI | Four Vitest tests; actual debug WebView logs `indexed`, `searched`, `answered` and `VECTORAIZ_PIPELINE_SMOKE_TEST_OK` |
| Build and notices | Rust 1.90 formatting, clippy with warnings denied, workspace tests and debug app build; TypeScript/Vite build; Rust/UI and native PDFium/llama.cpp notices generated |

Local commands and their complete output are retained privately under
`/tmp/vz-b-evidence/`; nothing in that directory is committed. Relevant captures
include `tests-final.log`, `clippy-final.log`, `pipeline-final.log`,
`native-smoke-final.log`, `ui-final.log`, `ui-build-final.log`, and the initial
failures and follow-up runs. Some earlier runs deliberately expose the failures
that led to fixes; they are not represented as passing final-head evidence.

CI runs these checks on macos-14 and windows-2022. Both CLI runs use `--cpu`.
The Windows build enables the linked Vulkan backend with a version/hash-pinned
SDK; it does not establish GPU execution. Native CI publishes notices and test
logs only. No tag, release or container image is created.

Pins are fully recorded in [models.json](../../spike/fixtures/models.json) and
[parsers.md](parsers.md). Provisional embeddings are BGE-small-en-v1.5 F16
(SHA256 f0b2fef971e8366438bfd2d9aefea1b0115919389448806d290237f638bae999).
Chat is Qwen2.5-0.5B-Instruct Q4_K_M
(SHA256 74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db).
The pinned llama.cpp commit is 244641955f6146f7e8474afff7772d427593a534.
PDFium is chromium/8111; both platform archive hashes are in parsers.md.

Unverified: Windows Vulkan on real GPU hardware; OCR accuracy and language
coverage on a scanned corpus; corporate PAC/proxy and trust-store integration;
model answer quality beyond the fixture. macOS memory enforcement is sampled,
not an allocation-time kernel boundary. Windows network isolation and
production filesystem confinement are Phase 1; full format coverage, model
selection, releases and public integrations belong to their separately scoped
chunks/phases. The minimal pack refuses originals over 2 GiB rather than
implementing later multipart export. See [pipeline.md](pipeline.md) for the
per-generation BM25 tradeoff and other spike limits.
