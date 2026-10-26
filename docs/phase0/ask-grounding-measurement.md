# Ask grounding measurement

Measurement date: 2026-10-26. MP task: `147d0c466875`.

The frozen [question set](../../spike/fixtures/ask-grounding-questions.json)
contains 108 pre-labelled cases: 60 on-topic and 48 off-topic, covering 20
fixture documents and a three-file orbital/invoice/heron corpus. Labels, source
passages, answer notes and stable IDs were frozen before retrieval or model
evaluation. Thirty on-topic questions are paraphrases with at most one exact
content-word overlap; five more are single-keyword prompts whose broad intent
is a limitation. SHA256:
`b6e318b57611c9eea95beb7661205e0e232a061e0eedd4078a9b27656aaf8c43`.

## CPU results

Each case used actual CLI embedding and top-three retrieval. B counts distinct
question content words found in those chunks; B1 and B2 mean thresholds of one
and two. Stemming adds simple suffix rules. A1 is a strict source-answerability
YES/NO model probe: only an exact trimmed, case-insensitive YES accepts. C is
first-hit cosine similarity at the specified threshold.

A false refusal rejects an on-topic case. A false acceptance accepts an
off-topic case; this is a gate proxy, not an observed hallucinated answer.
The table preserves the report's baseline and lexical/model comparisons plus
selected cosine combinations showing the strongest trade-offs.

| Method | False refusals / 60 | False acceptances / 48 | Median added ms |
| --- | ---: | ---: | ---: |
| No gate | 0 | 48 | 0.000000 |
| B>=1 | 13 | 38 | 0.041626 |
| B>=2 | 34 | 29 | 0.046249 |
| B>=2 stemming | 32 | 29 | 0.018000 |
| A1 | 17 | 6 | 270.063396 |
| A1 OR B2 | 14 | 30 | 270.090730 |
| A1 AND B1 | 26 | 6 | 270.084542 |
| A1 AND B2 | 37 | 5 | 270.090730 |
| A1 AND C>=0.3 (also C>=0.4) | 17 | 6 | 270.084709 |
| A1 AND C>=0.6 | 23 | 5 | 270.084709 |
| A1 AND C>=0.7 | 38 | 2 | 270.084709 |
| A1 OR C>=0.6 | 5 | 29 | 270.084709 |
| A1 OR C>=0.7 | 15 | 15 | 270.084709 |
| (A1 AND B1) OR C>=0.5 | 2 | 40 | 270.112042 |
| C>=0.8 | 55 | 0 | 0.021458 |
| A1 AND C>=0.8 | 55 | 0 | 270.084709 |

A1 had the smallest unweighted planned error count (23), tied by some cosine
combinations that add complexity. No planned method achieved zero false
acceptances with at most two false refusals. A post-hoc sweep of observed
cosine cutoffs found the best CPU zero-false-acceptance combination was A1 AND C at
0.7086963952302725, still refusing 39/60 on-topic cases. That exploratory
selection is descriptive, not held-out validation. All 60 answering passages
were retrieved in the top three; B>=2 refused 27/30 paraphrases anyway.

Latency is added after retrieval and excludes model loading. A1 includes
inference context creation and generation. Lexical timings use standalone
debug Rust; stemming and cosine use Python prototypes. Combinations sum signal
timings without short-circuit savings. These are not app end-to-end benchmarks.

## Decision

Max decided on 2026-10-26 at 13:38 CET to ship **no Ask grounding gate in
Phase 0**: the measured refusal cost for answerable questions is unacceptable.
This supersedes the report's earlier method recommendation for A1. Keep the
frozen set and rerun the comparison with the stronger local model in a later
phase before reconsidering a gate.

This is one designed synthetic set, not a deployment estimate. Variants share
facts, real traffic prevalence and relative error costs are unknown, and none
of these signals establishes a general grounding guarantee.
