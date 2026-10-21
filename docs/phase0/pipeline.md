# Chunk B pipeline spike

Authority: runbooks main `BQ-VECTORAIZ-NATIVE-DESKTOP-PHASE0-GATE2.md` sections
3, 4.2 and 7, and design sections 6.1–6.3 and 6.5. The scope is a local Phase 0
spike. No release, tag, container image or production integration is involved.

From the repository root:

```sh
python scripts/fetch-assets.py
cargo build --locked -p vz-cli -p vz-parser-worker
# macOS only
swiftc scripts/vision-ocr.swift -O -o target/debug/vz-vision-ocr
./target/debug/vectoraiz-cli --cpu index spike/fixtures/documents
./target/debug/vectoraiz-cli --cpu search 'orbital archive retention'
./target/debug/vectoraiz-cli --cpu ask 'What is the orbital archive retention policy?'
./target/debug/vectoraiz-cli --cpu pack spike/pack-output
```

Each command also accepts `--state`, `--assets`, and `--models` before the
subcommand. Assets are provisioned separately; normal indexing/search/chat
never downloads anything. Model bytes are rehashed before loading. The
provisional models, immutable source revisions and SHA256s are in
`spike/fixtures/models.json`. BGE-small is an English CI model; it is not the
multilingual model decision reserved for chunk C. Qwen2.5-0.5B-Instruct Q4_K_M
is a small test model, not a production quality promise.

llama-cpp-2 and llama-cpp-sys-2 are both locked to 0.1.138, which bundles
llama.cpp **244641955f6146f7e8474afff7772d427593a534** (binding source revision
408345defbf8b385b0a6b4fec7208b1cefeb3661). They statically link the engine;
no inference subprocess or HTTP endpoint is used. macOS builds include Metal;
`--cpu` disables layer offload. Windows builds may enable Vulkan with
`cargo build -p vz-cli --features vulkan` and an installed Vulkan SDK. Failure
to load an offloaded model retries with zero offloaded layers. Windows CI compiles Vulkan with SDK 1.4.321.1 (installer SHA256
baaa4f7ca11ed3d82aa1c102b21208915485bbaa473068c763daa425cca468bd)
and runs CPU inference. Actual Windows GPU execution remains unverified.

LanceDB 0.22.2 supplies vector search and BM25. Lance generations are immutable
and include a per-document FTS index. SQLite FULL-synchronous WAL transactions
choose the current generation. A process lock excludes concurrent readers and
writers for this single-source spike. At startup unreferenced generations are
removed and interrupted jobs become failed. The recovery test kills a process
after its real Lance write and before its SQLite flip, then verifies the old
content is still visible. Deletions are removed after a successful scan. An unavailable source or failed
directory traversal revokes the single-source index before reporting the error.
Metadata is recorded and content hashes confirm changes, including same-size
edits. An individual failed replacement removes old searchable content.

The structure-aware chunker preserves page and Markdown/HTML heading path,
targets 500 estimated tokens, and overlaps up to 160 bytes. The estimate is
ceil(UTF-8 bytes / 4), explicitly recorded in the recipe. BGE's tokenizer caps
model inputs at 512 tokens. These are spike settings to measure in chunk C.
Chat streams generated text followed by an application-owned numbered source
footer, so provenance remains available when a provisional model omits inline
citations. Generated references outside the retrieved source list are refused;
semantic support for every generated claim remains a chunk C evaluation.
RRF uses k=60 over vector distance and BM25 rankings. BM25 indexes are per
generation, so their scores are not collection-global IDF statistics; this
tradeoff needs measurement before the Phase 1 collection store design.

A pack is staged beside its destination and renamed after verification.
The pretty manifest inventories every payload hash (it excludes its own
self-referential hash), exact embedding recipe and counts. Arrow Parquet files
contain document metadata, chunks, source-page references, headings and
float32 embeddings. Original archive members have neutral names and are
verified against indexed source hashes before export. This minimal spike
refuses originals beyond 2 GiB rather than splitting them; multipart export
and the full listing journey remain later work. Text files start without
commas or tabs. `examples/query.py` verifies hashes and citations and runs an
offline cosine query with a query vector computed by the linked engine using
the recorded recipe. It reproduces the expected top three exported vectors;
CI also recomputes that query vector from the recorded model using the linked
CLI engine and verifies recipe equality before querying the Parquet data.

Upload is explicit and uses SigV4 with native trust roots and reqwest system
proxy discovery, with environment overrides. Credentials are read only by
this operation; they are never sent to workers or written into packs.
Temporary AWS credentials with a session token are supported. For example:

```sh
vectoraiz-cli upload spike/pack-output --endpoint https://s3.example.org \
  --bucket neutral-bucket --prefix vz/random-pack-id
```

`pipeline-e2e.py` indexes all 20 fixtures twice, validates query citations,
checks pack hashes, runs the exported query example, and optionally invokes
the upload round trip. `minio-test-server.py` builds MinIO at
f19c534b9f457773dcd043d977433e1a71525c3b
(RELEASE.2025-04-22T22-12-26Z); its source tarball SHA256 is
ac3bc23e5d91981a89921800a931a59aad27146434f952e9a79076b919d8a661.
The round trip starts it with ephemeral test data and test-only credentials,
compares every downloaded byte, then stops it unconditionally. No image is
built or pulled. The desktop uses a private app-data index and the same
pipeline commands over restricted local-main-window IPC. Debug smoke tests
click indexing, search and chat controls in the real WebView.

Phase 0 limitations: Windows network isolation, full format coverage,
production filesystem confinement, model quality, corporate PAC/trust-store
integration, Windows Vulkan GPU execution and OCR accuracy on a real scanned
corpus are not established by a passing fixture test. Runtime assets currently
come from the repository's provisioned `.assets` and `models.json`; distributing
and signing these assets belongs to chunk D. The MCP binary stays a stub.
