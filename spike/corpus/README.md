# Golden corpus inputs (delivered separately)

C1 supplies readers, fetching and scoring. This checkout intentionally contains
no golden manifest, golden questions, downloaded documents or benchmark results.
The intended corpus is about 300 openly licensed documents across PDF (text and
scanned), DOCX, XLSX, HTML and Markdown, covering English, Spanish, German,
French and a non-Latin script. Freeze labels before evaluating models.

`manifest.json`: an array (or `{ "documents": [...] }`) of objects:

```json
[{"filename":"example.pdf","url":"https://example.org/example.pdf","size":1234,"sha256":"64 lowercase hexadecimal characters","license":"CC-BY-4.0","license_url":"https://creativecommons.org/licenses/by/4.0/"}]
```

`retrieval-questions.json`: JSON array or JSONL, at least 100 labelled questions:

```json
[{"id":"R001","question":"How long are snapshots retained?","doc_path":"example.pdf","page":2}]
```

`doc_path` is relative to `.corpus/files`; paths use `/`. `page` is the same
one-based page number the parser stores and is omitted for document-only labels.
Each id is unique. C1 scores one expected document per question; document
recall is the hit rate across those questions, with repeated chunks counted
only once for success. MRR uses the original chunk-hit rank of the first match.

`chat-questions.json`: exactly 30 questions, JSON array or JSONL:

```json
[{"id":"C001","question":"How long are snapshots retained?","answers":["thirty days","30 days"],"support":"retention is thirty days"}]
```

`answers` lists accepted substrings; `support` is the literal supporting passage
used to check every cited source. Freeze both independently of model outputs.

```sh
python3 scripts/corpus-fetch.py
python3 scripts/corpus-fetch.py --manifest /path/to/manifest.json --output .corpus/files
# Operator-only preparation: print missing digest/size values, never bless them:
python3 scripts/corpus-fetch.py --record
```

Fetching streams to `.part`, verifies size and SHA256 before installing, skips
already verified files, tries three times and reports every failure with a
nonzero exit. `--record` prints metadata for entries missing size/digest; the
operator must review and add it to the manifest before a verified fetch. It
leaves those downloads as `.part` rather than making them accepted inputs.
`.corpus/` is ignored. Do not commit documents or model files.

Run [the harness](../bakeoff/README.md) after the manifest and questions arrive.
