# Phase 0 C1 model bake-off harness

Python 3, standard library (pyarrow may be used by downstream pack tooling).
No dependency installation or model fetching happens during a measurement.
`psutil`, if already present, supplies sampled Linux/Windows RSS; otherwise RSS
is `unmeasured`. macOS uses `/usr/bin/time -l` (bytes). Commands run from the
repository root. `rtk proxy` is used for subprocesses when installed.

The corpus and questions arrive separately. See [corpus schemas](../corpus/README.md).
Do not invent production labels or interpret an empty result table as a run.

```sh
cargo build --locked --workspace
python3 scripts/fetch-assets.py
python3 scripts/corpus-fetch.py
python3 spike/bakeoff/fetch-models.py .assets --candidate bge-m3
python3 spike/bakeoff/run-embedding.py --candidate bge-m3
# Copy the state path printed by the previous command:
python3 spike/bakeoff/fetch-models.py .assets --candidate qwen3-4b-instruct-2507
python3 spike/bakeoff/run-chat.py --candidate qwen3-4b-instruct-2507 --embedding /absolute/state/path
python3 spike/bakeoff/hand-check.py
python3 spike/bakeoff/summarise.py
python3 -m unittest discover spike/bakeoff
python3 scripts/native-build-tests.py
```

Model fetch defaults to all candidates if no `--candidate` filters are given;
that is a large download. Fetching verifies pinned sizes and SHA256, resumes
`.part` files with Range and skips verified files. `candidates.json` uses the
pins entry shape plus `kind`, chat `tier`, revision, licence terms and recipes.
All ten named models exist: no model substitutions. Gemma 4 Q4_K_M files are
from established quantisers because maker/ggml-org repositories lack that
quantisation. E5 uses a community conversion because no maker/ggml-org or
usual quantiser GGUF was found. E5's trained context is 512, so its explicit
512-token limit is the exception to the >=1024 rule; estimated chunk size is
not a tokenizer guarantee. Qwen3 embedding is the additional candidate.

`run-embedding.py` accepts `--corpus`, `--questions`, `--state` (must be empty),
`--assets`, `--cli`, `--cpu`. Without `--state` it creates a fresh persistent
state under ignored `results/embedding-NAME/`. Its sibling `models.json` is the
embedding recipe plus today's fixture chat. Retain both for chat evaluation.
`run-chat.py --embedding-models` overrides that sibling pins path if needed.
Every chat run reuses cached sources keyed by index and questions; both prompts
receive the same three chunks in the same order. Sources are read from active
generation `document.json` metadata, so no export/re-index changes the input.
Grounding uses today's fixture embedding model and each candidate chat model;
it separately reconstructs the historical three-file corpus from frozen
on-topic passages and preserves all 108 labels and ids.

Outputs go to ignored `spike/bakeoff/results/`. Top-level embedding/chat JSON
files contain totals, raw per-question ranks/answers, timings and hardware.
Subdirectories retain complete CLI stdout/stderr, macOS time output, pins,
items and fixture indices. Index wall time includes model load, hashing,
parsing/OCR and storage; chunks/s is end-to-end indexing throughput.
Per-question chat time excludes model load but includes template rendering,
tokenization, prefill and generation. Process peak RSS includes loading.
A failed subprocess leaves its complete evidence and causes a nonzero exit;
no successful results file is written for that run.

`hand-check.py` uses seed 1837, samples ten cite answers per candidate sorted by
id, and prints sources beside each answer. The reviewing instance records the
verdict; Max is not the hand checker. `summarise.py --output PATH` writes tables
only, which can be inserted into the method document after measurements.

See [method and caveats](../../docs/phase0/model-bakeoff.md), including the
llama.cpp compatibility change. Model load, quality and speed remain UNVERIFIED
until the operator runs the actual candidates. Nothing selects app models.
