# Phase 0 model bake-off method

C1 (PR #91) added the harness and the golden corpus. The measured results and
the provisional model picks are in [Results](#results-s1837-2026-10-29) below
(PR #93). Authority: Phase 0 Gate 2 section 4.3.

Use [the run instructions](../../spike/bakeoff/README.md) and
[input schemas](../../spike/corpus/README.md). Freeze corpus, licences,
question ids and labels before running. Record the input revision along with
the result artifacts. Target about 300 documents across the formats, five
languages/scripts and OCR cases listed in the corpus README, at least 100
retrieval questions, and exactly 30 chat questions.

## Recipes and compatibility

Candidate pins override pooling, prefixes, embedding context/batch size and
EOS, plus chat template, context and output limits. Defaults preserve today's
CLS/BGE recipe values and literal ChatML Ask prompt. The recipe regression test
compares serialized bytes to the old literal fields and includes negative
controls. EOS is recorded in the existing tokenizer descriptor to prevent
mixing incompatible indices; no index schema is added.

The brief explicitly permits a llama.cpp bump for candidate support. The
original `llama-cpp-2`/sys `0.1.138` vendors llama.cpp
`244641955f6146f7e8474afff7772d427593a534`, which lacks Gemma 4 architecture
support. `0.1.140` and `0.1.141` also lack it; `0.1.142` is the first release
with it, vendoring `b8635075ffe27b135c49afb9a8b5c434bd42c502`.
This is the only dependency bump. The vendored MIT licence is unchanged.
The existing recipe's `llama_commit` consequently records that new revision;
all other default recipe bytes stay identical. This is the necessary exception
to total byte identity under the brief's permitted bump: old indices need
re-indexing because the existing recipe gate checks the backend revision.
Actual full candidate loads remain UNVERIFIED; architecture availability is
not evidence of successful inference. No multi-GB candidate was downloaded
for this code build. Operator load errors must be retained with the raw
measurement evidence; do not silently substitute a model or fabricate results.

GGUF prompts use the embedded Jinja template via llama-cpp-2's chat-template
API with system and user messages. Thinking is disabled via template parameters;
Qwen3-8B also receives `/no_think` in extra system text. ChatML defaults retain
the app's exact literal Ask wording. Prompt budget is context minus output
limit minus 16 tokens. Sampling is greedy. EmbeddingGemma uses the decoder
embedding path; BERT models retain encode; Qwen last-token embeddings append
EOS after truncating to leave room for it. All vectors are L2 normalized.

## Metrics and scoring

Retrieval is the app's hybrid `State::search` over the actual query vector.
Take ten chunk hits in their original order. A question's document recall is
1 if its labelled document appears, otherwise 0; report the mean. MRR@10 is
1 / the first matching hit's one-based rank, otherwise 0. Duplicate chunks do
not earn additional success and do not compress ranks. Page recall additionally
requires the labelled page and averages only questions with a page label.
Raw hits contain paths, pages, chunk ids, scores and query latency, no text.
Index throughput is chunks / wall seconds including loading, hashing, parsing,
OCR and storage. Also record chunk count, index bytes and peak process RSS.

For each chat candidate, reuse the same frozen top-three sources from a chosen
embedding state, preserving order. The answer prompt is today's Ask prompt.
The cite prompt appends exactly: "Cite each fact with its source number in
square brackets, for example [2]." `answer_ok` accepts any nonempty labelled
answer substring after case folding and whitespace collapsing. `citation_ok`
requires at least one numeric `[n]`; every cited number must exist and every
cited source must contain the normalized support substring. The overlap
variant uses unique Unicode word tokens: support/source intersection divided
by support-token count must be at least 0.8 for every citation. Unknown
numbers fail. These string proxies require a reviewing instance's seeded
hand check of ten answers per candidate; they do not establish factual truth.

TTFT starts before prompt rendering/tokenization and ends at the first sampled
non-EOG token. Output tokens count actual sampled non-EOG tokens, including
pieces with empty decoded text. Tokens/s is (output tokens - 1) divided by
generation time after the first token; fewer than two tokens gives zero.
No-token output has null TTFT. Total includes prefill/context construction and
generation, excludes model load. Means are reported across questions; raw
per-item timings and full command stdout/stderr are retained. Process peak RSS
includes model loading. macOS `time -l` reports bytes; other systems use
optional psutil sampled process-tree RSS or explicitly `unmeasured`.

Grounding runs all 108 existing labelled cases, preserving fixture versus
three-file retrieval corpora and today's fixture embedding. The latter is
reconstructed from the frozen on-topic source passages. As in
[the previous measurement](ask-grounding-measurement.md), retrieve three
chunks with the actual CLI and retain on/off-topic labels. That report measured
A1/lexical/cosine gate proxies, not generated refusal classifications; no gate
is added here. This round measures raw answers with a conservative refusal
phrase heuristic in `common.refusal`, reporting off-topic refusal/non-refusal
shares and on-topic refusals. Non-refusal is a proxy for answering from general
knowledge, requiring manual review; ambiguous and mixed answers can be
misclassified. This is not directly comparable to the old A1 false-acceptance
rate and does not reverse the decision to ship no grounding gate.

## Hardware limits

Record OS, CPU, RAM, requested acceleration and measurement method with every
result. GPU requests can fall back to CPU; inspect the run environment before
interpreting speed. Koskadeux is M3 Ultra / 256 GB, above the target tiers:
**8/16 GB speed figures are UNVERIFIED on Koskadeux** until measured on those
machines. **Windows figures are UNTESTED in this round**, per Max 2026-10-29.
Memory footprint alone does not prove fit or speed on a smaller machine.

## Candidate provenance and licence terms

All ten requested models exist; no model substitutions. Additional embedding:
Qwen3-Embedding-0.6B. Maker repositories are preferred, then ggml-org; Q4_K_M
availability requires community quantisers for the two Gemma 4 models and
Qwen3-4B-Instruct-2507. E5 has no located maker/ggml-org conversion and uses
Ralriki. GGUF sizes and SHA256 are from Hugging Face LFS metadata queried again
at each pinned commit. Full hashes, source revisions, licence URLs, terms,
recipes and notices are in [candidates.json](../../spike/bakeoff/candidates.json).
Commercial use is permitted by MIT, Apache-2.0, and conditionally Gemma terms.
MIT requires retaining its notice; Apache requires licence, attribution and
applicable NOTICE, modification notices, and carries patent terms. Gemma
requires passing through its agreement and use restrictions, preserving
notices/marking modifications and the prescribed Notice file. Its policy is
linked in the manifest. No candidate with unclear terms is selected.
Gemma 4 is Apache-2.0 according to Google's maker metadata; it does not inherit
EmbeddingGemma's Gemma terms.

| Candidate | Kind / tier | GGUF revision | Bytes | Licence |
|---|---|---|---:|---|
| [bge-m3](https://huggingface.co/gpustack/bge-m3-GGUF/resolve/2d48f1737679ad900d5c26c5aad5410e9c70fdca/bge-m3-Q8_0.gguf) | embedding / — | `2d48f1737679ad900d5c26c5aad5410e9c70fdca` | 634553760 | [MIT](https://huggingface.co/BAAI/bge-m3/blob/5617a9f61b028005a4858fdac845db406aefb181/README.md) |
| [multilingual-e5-large-instruct](https://huggingface.co/Ralriki/multilingual-e5-large-instruct-GGUF/resolve/f43c843a00e7c3bdd087cbaf481464c1fa7fb37d/multilingual-e5-large-instruct-q8_0.gguf) | embedding / — | `f43c843a00e7c3bdd087cbaf481464c1fa7fb37d` | 603097568 | [MIT](https://huggingface.co/intfloat/multilingual-e5-large-instruct/blob/274baa43b0e13e37fafa6428dbc7938e62e5c439/README.md) |
| [embeddinggemma-300m](https://huggingface.co/ggml-org/embeddinggemma-300M-GGUF/resolve/0f741b5a6585bd53aeb15cd1372c56f2a0f65e12/embeddinggemma-300M-Q8_0.gguf) | embedding / — | `0f741b5a6585bd53aeb15cd1372c56f2a0f65e12` | 333590944 | [Gemma](https://ai.google.dev/gemma/terms) |
| [qwen3-embedding-0.6b](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/370f27d7550e0def9b39c1f16d3fbaa13aa67728/Qwen3-Embedding-0.6B-Q8_0.gguf) | embedding / — | `370f27d7550e0def9b39c1f16d3fbaa13aa67728` | 639150592 | [Apache-2.0](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B/blob/97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3/README.md) |
| [qwen3-4b-instruct-2507](https://huggingface.co/lmstudio-community/Qwen3-4B-Instruct-2507-GGUF/resolve/4edb920b6f14e3b9284d4502a6485103d72cde05/Qwen3-4B-Instruct-2507-Q4_K_M.gguf) | chat / 8gb | `4edb920b6f14e3b9284d4502a6485103d72cde05` | 2497280448 | [Apache-2.0](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507/blob/cdbee75f17c01a7cc42f958dc650907174af0554/LICENSE) |
| [gemma-4-e4b](https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF/resolve/bfc15c382204943c3a8fff0c750b94ae2364d7a3/gemma-4-E4B-it-Q4_K_M.gguf) | chat / 8gb | `bfc15c382204943c3a8fff0c750b94ae2364d7a3` | 4977171584 | [Apache-2.0](https://huggingface.co/google/gemma-4-E4B-it/blob/ee0ef6023621cff504d758262d4e04895a5af4a2/README.md) |
| [qwen3-8b](https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/7c41481f57cb95916b40956ab2f0b139b296d974/Qwen3-8B-Q4_K_M.gguf) | chat / 16gb | `7c41481f57cb95916b40956ab2f0b139b296d974` | 5027783488 | [Apache-2.0](https://huggingface.co/Qwen/Qwen3-8B/blob/b968826d9c46dd6066d109eabc6255188de91218/LICENSE) |
| [ministral-3-8b](https://huggingface.co/mistralai/Ministral-3-8B-Instruct-2512-GGUF/resolve/0102285ad796bd99af90f58de616092e5630e970/Ministral-3-8B-Instruct-2512-Q4_K_M.gguf) | chat / 16gb | `0102285ad796bd99af90f58de616092e5630e970` | 5198911904 | [Apache-2.0](https://huggingface.co/mistralai/Ministral-3-8B-Instruct-2512/blob/5b26027e7b19eeb4b7352e1fed3926375dd2cb4d/README.md) |
| [qwen3.6-35b-a3b](https://huggingface.co/ggml-org/Qwen3.6-35B-A3B-GGUF/resolve/7878dc9a905c7c9379e5e8d6a1aa9affdd6839c5/Qwen3.6-35B-A3B-Q4_K_M.gguf) | chat / 32gb | `7878dc9a905c7c9379e5e8d6a1aa9affdd6839c5` | 20419565568 | [Apache-2.0](https://huggingface.co/Qwen/Qwen3.6-35B-A3B/blob/995ad96eacd98c81ed38be0c5b274b04031597b0/LICENSE) |
| [gemma-4-26b-a4b](https://huggingface.co/lmstudio-community/gemma-4-26B-A4B-it-GGUF/resolve/f6e6747823b2912661935db7e0009287c4838073/gemma-4-26B-A4B-it-Q4_K_M.gguf) | chat / 32gb | `f6e6747823b2912661935db7e0009287c4838073` | 16796017248 | [Apache-2.0](https://huggingface.co/google/gemma-4-26B-A4B-it/blob/4d7ae4984b7db7de8f8457170b3f1a419ee76d52/README.md) |

E5 is explicitly limited to 512 tokens by the supplied recipe/model context;
other candidates have at least 1024. An estimated 500-token chunk may exceed
512 real tokens, so record this truncation risk when interpreting E5 results.

## Results (S1837, 2026-10-29)

Measured on Koskadeux (Mac Studio M3 Ultra, 256 GB, macOS, Metal) with the C1 harness at `c481bbee` and the release CLI built from it. Corpus: 298 documents, 123 retrieval questions (27 page-labelled), 30 chat questions (`spike/corpus/`). Raw per-question results, hand-check sheet and all CLI stdout/stderr/time files are operator evidence on Koskadeux under `/Users/max/worktrees/vz-c1/spike/bakeoff/results/` (attempt folders), recorded in the BQ.

### Embedding (app hybrid search, top 10)

| Embedding | Licence | Recall@10 | MRR@10 | Page recall@10 | Index s | Peak RSS | Chunks/s | Index size |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| bge-m3 (Q8_0) | MIT | 1.000 | 0.948 | 0.926 | 1433 | 2.53 GB | 9.2 | 295 MB |
| EmbeddingGemma-300M (Q8_0) | Gemma terms | 1.000 | 0.931 | 0.963 | 1368 | 3.22 GB | 9.7 | 238 MB |
| multilingual-e5-large-instruct (Q8_0) | MIT | 0.992 | 0.921 | 0.926 | 1206 | 1.97 GB | 11.0 | 294 MB |
| Qwen3-Embedding-0.6B (Q8_0, additional) | Apache-2.0 | 1.000 | 0.933 | 0.889 | 2377 | 2.86 GB | 5.6 | 295 MB |

All runs indexed 298/298 documents into 13,235 chunks. Index time is end to end (model load, hashing, parsing, Apple Vision OCR of 14 scanned PDFs, storage). Qwen3-Embedding ran after the Gate 3 fold (single EOS); the other three are unaffected by that fold.

**Ceiling effect.** Every candidate finds the expected document in the top 10 for 122–123 of 123 questions. The app's search is hybrid (full-text plus vector), and the corpus questions share distinctive words with their passages, so this set separates the embedding models only weakly (MRR range 0.921–0.948, page recall 0.889–0.963; one of the 123 questions moves document recall by 0.81 points, but one of the 27 page-labelled questions moves page recall by 3.70 points). A harder set (paraphrased and cross-language questions, vector-only scoring) is needed before reading small differences; recorded for Phase 1.

### Chat (same top-3 sources from the bge-m3 index for every candidate; 30 questions; Q4_K_M)

| Chat | Tier | Answer correct (app prompt) | Answer (cite prompt) | Citation correct | Citation (token overlap) | Off-topic refused (48) | On-topic refused (60) | TTFT (app prompt) | Tokens/s (app prompt) | Peak RSS (app prompt) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Qwen3-4B-Instruct-2507 | 8 GB | 0.667 | 0.633 | 0.600 | 0.667 | 0.979 | 0.067 | 0.76 s | 138 | 3.24 GB |
| Gemma 4 E4B | 8 GB | 0.667 | 0.600 | 0.400 | 0.467 | 1.000 | 0.067 | 0.76 s | 90 | 5.44 GB |
| Qwen3-8B (no thinking) | 16 GB | 0.733 | 0.800 | 0.500 | 0.600 | 0.979 | 0.033 | 1.28 s | 88 | 5.79 GB |
| Ministral 3 8B Instruct | 16 GB | 0.800 | 0.800 | 0.567 | 0.667 | 1.000 | 0.033 | 1.29 s | 83 | 5.94 GB |
| Gemma 4 26B-A4B | 32 GB+ | 0.700 | 0.767 | 0.633 | 0.667 | 1.000 | 0.117 | 0.89 s | 86 | 17.95 GB |
| Qwen3.6-35B-A3B | 32 GB+ | 0.767 | 0.700 | 0.633 | 0.733 | 1.000 | 0.083 | 0.80 s | 82 | 20.78 GB |

- **Grounding (the open Phase 0 defect).** With the provisional Qwen2.5-0.5B, off-topic questions were answered from general knowledge. Every candidate refuses 98–100% of the 48 off-topic cases in `spike/fixtures/ask-grounding-questions.json`, with 3–12% refusals on the 60 on-topic cases (refusal is a text heuristic, not a proof).
- **BOS check.** `first_prompt_token` was 2 for both Gemma 4 models and 1 for Ministral (their BOS); Qwen prompts start with `<|im_start|>` (151644; 248045 for Qwen3.6) and no BOS, as expected.
- **Noise.** 30 questions means one question is 3.3 points, so small differences are within noise and no difference here is shown to be statistically significant. Example: Qwen3-4B beat Gemma 4 E4B on citations in 7 paired questions and lost 1 (exact two-sided McNemar p = 0.07), an observed advantage, not an established one.
- **Hand check (spec 4.3, codex2 at Gate 3).** Ten answers per candidate (`hand-check.md`, seed 1837), checked against the shown sources; q114 is unanswerable from its sources and every candidate rightly abstained (counted separately):

  | Candidate | Supported answers | Citation correct | Right to abstain |
  |---|---:|---:|---|
  | Qwen3-4B-Instruct-2507 | 9/10 | 9/10 | q114 |
  | Gemma 4 E4B | 8/10 | 6/10 | q114 |
  | Qwen3-8B | 9/10 | 7/10 | q114 |
  | Ministral 3 8B | 8/10 | 8/10 | q114 |
  | Gemma 4 26B-A4B | 9/10 | 9/10 | q114 |
  | Qwen3.6-35B-A3B | 9/10 | 9/10 | q114 |

  Automatic-scorer disagreements: Ministral q012 passed both automatic checks but first says the source gives no year, then gives 2015 (false positive); Ministral q043 failed the automatic citation check though source [2] supports the answer (false negative). Qwen3-8B cited non-existent sources [6] and [32] in q069 and q084; Gemma 4 E4B refused q012 although source [1] gives the answer and cited the wrong source in q083 and q106. These match the method's known limits and do not change the picks.
- **Speed and memory** are M3 Ultra figures with Metal requested; full offload was not confirmed (stderr empty, and the loader allows CPU fallback). Speed for 8 and 16 GB machines is UNVERIFIED (spec 4.3); Windows figures are UNTESTED in this round (Max 2026-10-29). Peak RSS counts the memory-mapped model file. On an 8 GB Mac or PC we expect Qwen3-4B (3.2 GB) to leave room for the OS, embedding model and app and Gemma 4 E4B (5.4 GB) to be tight; neither fit is verified on a small machine.

### Picks

| Role | Pick | Reasons |
|---|---|---|
| Embedding | **bge-m3** (MIT) | Best MRR@10 and full recall; middle index speed; MIT licence with no extra terms. EmbeddingGemma had the best page recall and smallest index but carries Gemma's terms of use (prohibited-use policy and notice obligations passed to users), which is a heavier licence for a default download; differences are within the ceiling noise. |
| Chat, 8 GB | **Qwen3-4B-Instruct-2507** (Apache-2.0) | Same answer rate as Gemma 4 E4B, better observed citations (0.60 vs 0.40; not statistically established), 1.5× faster and 2.2 GB less memory on the M3 Ultra. |
| Chat, 16 GB | **Ministral 3 8B Instruct** (Apache-2.0) | Best answer rate of all candidates in this run (0.80 with both prompts), best citations in its tier, 100% off-topic refusal. Qwen3-8B is within noise and is the fallback. |
| Chat, 32 GB and above | **Qwen3.6-35B-A3B** (Apache-2.0) | Best answer rate in its tier with the app's prompt and best overlap citation; Gemma 4 26B-A4B is within noise, uses 2.8 GB less memory, but refuses more on-topic questions (0.117). Near tie; revisit with the harder set. |

Licence notes for the app (spec 4.3): bge-m3 MIT (include the MIT notice); Qwen3-4B-Instruct-2507, Ministral 3 8B, Qwen3.6-35B-A3B Apache-2.0 (include the licence and any NOTICE file from the model repository). Exact revisions, sizes and hashes are in `spike/bakeoff/candidates.json`.

**Not decided here:** switching the app's default pins (`spike/fixtures/models.json`, model download sizes, RAM-tier selection, re-index on upgrade) is a separate chunk with its own Gate 3. The Ask prompt stays the app prompt; the cite prompt's inline citations are not adopted.

## Parser findings from the corpus run (S1837)

- A real open-data XLSX (`de-sh-xlsx-0003.xlsx`, Schleswig-Holstein open data) is refused by the archive-expansion guard (`zip compression ratio exceeds 100:1`). It was removed from the bake-off corpus. Phase 1 should revisit the guard (ratio plus absolute expanded-size cap) so ordinary highly compressible spreadsheets index.
- Scanned PDFs need the `vz-vision-ocr` helper next to `vectoraiz-cli` (CI builds it with `swiftc scripts/vision-ocr.swift -O -o target/<profile>/vz-vision-ocr`); without it they fail with `start Apple Vision OCR helper`.

## Gate 3 fold notes (S1837)

- **Index compatibility after the llama.cpp bump.** `llama-cpp-2` 0.1.142 changes `Recipe.llama_commit`, so every index written by 0.0.9 or earlier fails `validate_recipe` on Index, Search and Ask after the next release. The app shows "embedding recipe mismatch; re-index required (use a new index)" and has no reset action. Recovery for testers is in `docs/install-notes/macos.md` and `windows.md` (quit, rename `phase0-index`, index again); the release record for the next version must say so. A Phase 1 follow-up should rebuild the index in place on a recipe mismatch instead of erroring (CC Gate 3 BETTER).
- **Raw results.** Per-question raw results stay in the ignored `spike/bakeoff/results/` attempt folders (operator evidence on Koskadeux, path recorded in the BQ). This document commits the summary tables, the picks and their reasons, and the per-candidate totals.
- **Measurement fixes before the chat runs.** GGUF-template chat prompts are tokenized with `AddBos::Always`, so Gemma 4 and Ministral receive their BOS (`bakeoff-chat` now reports `prompt_tokens` and `first_prompt_token`; Gemma 4 should start with token 2, Ministral with 1, Qwen unchanged). Last-token embedding no longer appends a second EOS when the vocab already added one (Qwen3-Embedding). Citation checks compare NFKC/casefolded text with all whitespace removed, the rule the corpus support strings were verified with, and report citation and answer rates per language. An alternate labelled page (`also_expected`) counts for page recall. Each run uses a fresh attempt folder; a failed rerun writes `<kind>-<name>.failed.json`, and the summary flags a candidate whose latest attempt failed.
- **bge-m3 and multilingual-e5 were measured before this fold.** It changes neither of their recipes nor the retrieval scoring, except q019's alternate page. `spike/bakeoff/rescore.py` re-scored all four embedding results from their saved hits with the final rule; totals are unchanged.

## Results PR notes (S1837)

- CC Gate 3 advisory items folded here: `compact()` also removes soft hyphens; chat results record `prompt_tokens` (first token and prompt length range per prompt) at the top level, and a run stops unless every prompt the candidate saw (all corpus prompts and the grounding prompts that report token metadata; older grounding rows without it are skipped) starts with the same token; the summary shows a "First token" column (results produced before this change show `n/a`; their first tokens are checked from the raw rows above) and a row for a candidate with no completed attempt; every attempt folder also keeps its own `result.json`; `rescore.py` re-scores saved embedding results and writes when any per-question row changes, not only the totals (codex2 LOW-1; this updated q019's page ranks to [1, 2] for bge-m3 and EmbeddingGemma, totals unchanged).
