# Phase 0 default model pins

Stage 1/release bundles carry `spike/fixtures/models.release.json` as
`gen/bundle/models.json`. Every model field is copied unchanged from
`spike/bakeoff/candidates.json`, including the original `load: unverified`
metadata. The local verification below is separate evidence.

All tiers use bge-m3 Q8_0 (MIT), 634,553,760 bytes. The shell reads total physical
RAM using macOS `sysctl hw.memsize` or Windows `GlobalMemoryStatusEx` and chooses
the highest threshold less than or equal to physical RAM, measured in GiB
(1,073,741,824 bytes). There is no tier picker.

| Minimum RAM | Chat model (Q4_K_M) | Chat bytes | Total bytes |
| --- | --- | ---: | ---: |
| 0 GiB (including 8 GiB) | Qwen3-4B-Instruct-2507 | 2,497,280,448 | 3,131,834,208 |
| 16 GiB (including 24 GiB) | Ministral 3 8B Instruct 2512 | 5,198,911,904 | 5,833,465,664 |
| 32 GiB and above | Qwen3.6-35B-A3B | 20,419,565,568 | 21,054,119,328 |

Chat models use Apache-2.0. Bundled THIRD_PARTY_NOTICES includes full MIT and
Apache-2.0 texts, attribution, source URLs/revisions and the upstream NOTICE
inventory in `model-licenses/`. No separate NOTICE file was supplied in the
pinned upstream source or GGUF trees. Chat uses each GGUF's embedded template
and the existing app Ask prompt. The existing bake-off path supplies one BOS
for Ministral (token 1) and none for Qwen; it is unchanged.

Debug/dev builds use `spike/fixtures/models.json` (bge-small + Qwen2.5-0.5B).
CI fetches only those small fixtures. Installers still contain release pins;
the installed acquisition smoke explicitly supplies the fixture path using
`VZ_SMOKE_MODELS`, honored only by a `smoke-test` build launched with a smoke
flag. Normal installed launches always use the bundled release pins. The shell
passes a resolved embedding/chat pair to the offline engine; downloads remain
in the shell and the engine rehashes each model before loading it.

## Re-index behaviour and implementation conflict

On launch, the shell compares the full stored embedding recipe with the
selected pins, without loading a model. A mismatch, invalid recipe, or missing
recipe with surviving index data shows “Your library needs re-indexing for the
new search model”. Search and Ask are disabled. The existing core recipe gate
also refuses mismatched queries, including direct CLI calls. Old model files
are retained.

**Decision 4 is blocked at the atomic Re-index action.** At base a33eefe6,
`State::stage` and `State::commit` stage and atomically switch one document at a
time; `index::index` stores a single global `recipe.json` outside SQLite and
refuses a different recipe. Recovery deletes unreferenced document generations.
There is no library-wide generation/recipe switch to reuse. Updating the global
recipe before all document commits would permit mixed models after interruption;
updating it afterwards would label the new vectors with the old recipe during
that interval. Per the brief's conflict rule, this change does not invent a new
storage protocol or wire a Re-index action to a partial migration. The atomic
rebuild and its after-reindex acceptance test remain unimplemented pending that
decision. The warning and query guard are implemented and tested.

## Verification limits

Speed and memory fit on 8/16 GiB machines remain unverified. Windows release
model inference is unverified; Windows CI uses fixture models. Koskadeux is an
M3 Ultra with 256 GiB RAM, so the explicit 8 GiB verification selection proves
the release pair's pipeline, not performance on an 8 GiB machine.

## Koskadeux local release verification (2026-10-30)

Built the assigned checkout with `cargo build --locked --release -p vz-cli -p
vz-parser-worker`. Ran the extended app asset fetcher with `--models
spike/fixtures/models.release.json --ram-gb 8 --selected-out <resolved.json>`.
Fresh downloads, not reused bake-off models, passed:

```text
bge-m3-Q8_0.gguf: sha256 OK 950f4a8e5e19477a6d3c26d2f162233c20002c601f75e4b002e3239997821167 (634553760 bytes)
Qwen3-4B-Instruct-2507-Q4_K_M.gguf: sha256 OK 8cdb57cbb880d313736a9bc4e3d3d2485f145b5e19cf33783746e753e82641fc (2497280448 bytes)
```

Selected 30 Markdown documents from `spike/corpus/manifest.json`: all English
Markdown entries first, then the other Markdown entries in manifest order until
30, verifying each input SHA-256. Copied the inputs read-only from the earlier
corpus checkout into this job's evidence directory. This subset covers English,
German, Spanish, French and Japanese. A fresh index completed **30/30**, zero
failures or skips, producing 550 chunks. Each CLI process used `--models
<resolved.json> --assets <checkout>/.assets --state <fresh-index>` with Metal
requested and CPU fallback available (offload was not independently confirmed).

| Command | End-to-end wall time (includes model hashes and loading) |
| --- | ---: |
| `index <corpus30>` | 43.861 s |
| `search "What can you do when an anchor-positioned element starts to overflow the viewport?"` | 8.009 s |
| `ask` with the same question | 8.851 s |

Search returned five hits. Ask returned:

> When an anchor-positioned element starts to overflow the viewport, you can use the `position-try-fallbacks` property to specify fallback options like `flip-block` or `flip-inline` to reposition the element and prevent overflow.
>
> Retrieved sources: [1] [2] [3]

Citation metadata was checked against persisted document generations and chunk
pages: [1]/[2] refer to `en-mdn-0001.md`, page 1; [3] refers to
`en-mdn-0008.md`, page 1. The persisted context contains `position-try-fallbacks`.
The answer uses the app's existing retrieved-source footer.

Full untruncated private evidence: `/var/tmp/koskadeux/private-evidence/s1837-h/`
(`download.log`, `corpus30-manifest.json`, `timings.json`, `index.json`,
`search.json`, `ask.json`, per-command stderr and `citation-check.json`).
