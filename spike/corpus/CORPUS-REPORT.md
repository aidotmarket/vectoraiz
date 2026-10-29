# vectorAIz golden corpus (s1837): build report

Authority: runbooks `specs/BQ-VECTORAIZ-NATIVE-DESKTOP-PHASE0-GATE2.md` §4.3 (origin/main, lines 121–131).
Built on Koskadeux on 2026-10-29. Nothing was committed, pushed or posted. Everything lives under `/Users/max/scratch/s1837/corpus/`.

## Deliverables

| path | what it is |
|---|---|
| `manifest.json` | 299 documents as delivered (298 after the change below): id, pinned URL, sha256, bytes, format, language, licence, licence_url, source, title, filename. Wikipedia entries also carry `"postprocess": "strip-mediawiki-render-id"` (see "Hash stability" below). |
| `questions.json` | 123 retrieval questions; 30 have `"chat": true`. |
| `files/` | the 299 downloaded files. Fetched by script, never committed. |
| `tools/fetch_corpus.py` | standalone fetcher (needs only the standard library and curl). It downloads every manifest entry, applies the postprocess step and checks sha256 and size. Exits 1 if anything does not match. |
| `tools/build_manifest.py`, `select_docs.py`, `*_probe.py`, `*_candidates.py` | how the selection and the manifest were built. |
| `tools/extract.py`, `tools/text/` | the text extraction used for the support strings (OCR for scanned PDFs). |
| `tools/make_questions.py` | the hand-written questions. PDF page numbers are found automatically. |
| `tools/verify.py`, `tools/verify.md` | the final checks (results below). |

## Counts

### Documents by format and language

| format | de | en | es | fr | ja | total |
|---|---|---|---|---|---|---|
| html | 22 | 22 | 22 | 22 | 22 | 110 |
| md | 8 | 8 | 8 | 8 | 8 | 40 |
| pdf | 8 | 29 | 8 | 8 | 6 | 59 |
| pdf-scanned | 0 | 14 | 0 | 0 | 0 | 14 |
| docx | 0 | 28 | 0 | 7 | 0 | 35 |
| xlsx | 8 | 26 | 0 | 7 | 0 | 41 |
| total | 46 | 127 | 38 | 52 | 36 | 299 |

Total size is 138.6 MB. The largest file is 12.2 MB (an arXiv PDF). Five files are over 3 MB and none is over 15 MB.

### Questions by format and language (chat questions in brackets)

| format | de | en | es | fr | ja | total |
|---|---|---|---|---|---|---|
| html | 12 (3) | 11 (2) | 12 (4) | 11 (2) | 14 (3) | 60 (14) |
| md | 2 (0) | 2 (1) | 2 (0) | 2 (1) | 3 (1) | 11 (3) |
| pdf | 3 (1) | 6 (2) | 3 (1) | 3 (1) | 3 (1) | 18 (6) |
| pdf-scanned | 0 | 9 (2) | 0 | 0 | 0 | 9 (2) |
| docx | 0 | 8 (2) | 0 | 5 (1) | 0 | 13 (3) |
| xlsx | 3 (1) | 6 (0) | 0 | 3 (1) | 0 | 12 (2) |
| total | 20 (5) | 42 (9) | 17 (5) | 24 (6) | 20 (5) | 123 (30) |

Every minimum is met: at least 15 questions per language, at least 8 per non-HTML format, and exactly 30 chat questions. The chat questions span all five languages and every format.

## Sources and licences

| source | n | format | licence (as recorded per document) |
|---|---|---|---|
| Wikipedia en/es/de/fr/ja. Random main-namespace articles of 18–90 kB; lists and disambiguation pages excluded. Each is pinned to the revision current at build time through the REST endpoint `/api/rest_v1/page/html/<title>/<revid>`. | 110 | html | CC BY-SA 4.0 |
| MDN Web Docs (`mdn/content` @ `94172439…`, `mdn/translated-content` @ `d81873b9…`), raw GitHub URLs at those commits, en/es/fr/ja | 32 | md | CC BY-SA 2.5 (both repos' LICENSE.md checked) |
| German federal law, `bundestag/gesetze` @ `77abca2c…` | 8 | md | Public domain (amtliches Werk, § 5 UrhG); repository is under The Unlicense |
| NIST (8), CDC MMWR (5) | 13 | pdf | US federal government work, public domain |
| NASA NTRS, filtered on the API's copyright determination `GOV_PUBLIC_USE_PERMITTED` | 12 | pdf | public domain (NASA) |
| arXiv, versioned PDF URLs; licence checked per paper through OAI-PMH | 4 | pdf | CC BY 4.0 |
| EU legislation (8 different acts per language, es/de/fr). The CELEX number is resolved through the Publications Office Cellar to a fixed `publications.europa.eu/resource/cellar/<uuid>.<lang manifestation>/DOC_1` URL. | 24 | pdf | EU reuse (Decision 2011/833/EU, EUR-Lex legal notice) |
| 令和6年版 情報通信白書 (MIC Japan), 6 chapter PDFs | 6 | pdf | CC BY 4.0, per the white paper's own reuse terms (r06riyou.pdf). Listed third-party figures are excluded; no question uses a figure. |
| NASA NTRS NACA-era scans (1940s–50s) with no text layer | 14 | pdf-scanned | public domain (NACA/NASA, `GOV_PUBLIC_USE_PERMITTED`) |
| GOV.UK publication attachments (assets.publishing.service.gov.uk), one per publication | 28 docx, 26 xlsx | docx/xlsx | Open Government Licence v3.0 |
| data.gouv.fr (static.data.gouv.fr resource URLs, which embed a timestamp) | 7 docx, 7 xlsx | docx/xlsx | Licence Ouverte v2.0 (10) / v1.0 (4), taken from the dataset metadata |
| Open-Data-Portal Schleswig-Holstein (Statistikamt Nord, Hansestadt Lübeck) | 8 | xlsx | Datenlizenz Deutschland – Namensnennung 2.0 (6), CC BY 4.0 (2), from the dataset metadata |

All of these licences allow redistribution and commercial use. None is NC, ND or "all rights reserved".

## Dropped or not used, and why

- **EUR-Lex HTML/PDF endpoints** answer bots with a 202 JavaScript challenge, so I used the Cellar content-negotiation URLs instead. 11 CELEX numbers first failed to resolve because older acts only have a PDF/A manifestation. The resolver now also tries `application/pdf;type=pdfa1a`, and all 24 resolve.
- **arXiv 2608.02045v1** (beekeeping) is over 15 MB and was dropped. That leaves 4 arXiv PDFs and 59 text PDFs in total.
- **NTRS 19930082640** (would have been the 15th scan) is served with a 128-byte MacBinary header in front of `%PDF`, so it is not a valid PDF. Dropped, leaving 14 scanned PDFs.
- **ISTAC / datos.canarias.es XLSX** (Spanish): the files are generated on request and every download has a different sha256, so they cannot be pinned. Dropped. As a result, **Spanish has no DOCX or XLSX** and **Japanese has no DOCX or XLSX**. Spanish and Japanese are covered by HTML, Markdown and PDF.
- **datos.gob.es CKAN API** returned non-JSON, and **opendata.aragon.es** returned nothing. Not used.
- **Library of Congress / Chronicling America** and the **CIA reading room** return 403 or a bot-challenge page to curl. Not used for scans.
- **Gallica (BnF)** was deliberately not used: its reuse terms are non-commercial.
- **Google patent images** were not used as "scanned" documents because they contain a hidden OCR text layer.
- **mdn/translated-content no longer has a German locale.** German Markdown therefore comes from `bundestag/gesetze`.
- **NIST SP 330 (SI brochure)** was left out because it is a translation of BIPM material and its licensing is less clear-cut.

## How support texts were extracted

- **HTML:** BeautifulSoup `get_text("\n")` with script and style removed.
- **Markdown:** the raw file.
- **DOCX:** python-docx paragraphs plus table cells, tab-joined.
- **XLSX:** openpyxl `data_only` cell values, tab-joined per row, with one `## sheet:` header per sheet.
- **Text PDF:** poppler `pdftotext` (UTF-8), pages split on form feed. `page` is the 1-based physical page index.
- **Scanned PDF:** `pdftoppm -r 300 -gray`, then `tesseract 5.5.3 -l eng` page by page (brew `tesseract` and `tesseract-lang` were installed on Koskadeux). The support strings for scanned documents are short, cleanly OCRed phrases from the summary page (page 2 in every case). Answers give the printed spelling (for example `6061-T6`, where the OCR reads `6061-16`).
- **Matching rule** used by `verify.py`: Unicode NFKC, casefold, remove soft hyphens and **all** whitespace, then a substring test. Removing all whitespace makes the check robust to the spaces that HTML link extraction adds in Japanese text and to tab-joined XLSX cells. For PDFs the test runs on the stated page only.

## Verification results (tools/verify.py, run at the end)

- All 299 manifest entries have a file in `files/` with matching sha256 and size. There are no extra files, no duplicate ids and no empty fields.
- Every question's `doc` exists. Every PDF question has a page, and every non-PDF question has `page: null`.
- **0 failures:** all 123 support strings are found in their document's extracted text (for PDFs, on the stated page; for scanned PDFs, in the OCR text). All supports are 200 characters or fewer.
- No support string occurs verbatim in any *other* document of the corpus.
- One question, **q019** (en-nasa-0008), has its support on two pages (3 and 14, because the abstract is repeated on the report documentation page). Both pages are listed in `expected`.
- **Scanned PDFs:** 13 of 14 have no embedded text at all (pdftotext returns 0 characters). en-nasascan-0013 has 833 characters of junk OCR (noise glyphs) over 12 pages, so it is effectively image-only. It has no question.
- **Full independent re-fetch:** `tools/fetch_corpus.py --dest <empty dir>` downloaded all 299 documents again and all 299 matched the manifest (exit 0).

## Hash stability: Wikipedia drift (open risk)

During the first re-fetch, 3 Wikipedia files changed bytes without any change of revision. One reason is a per-render `<meta http-equiv="x-mediawiki-render-id">` UUID. The other is a `utm_campaign=index|rest` value on media links, which varies with cache state. Both are now stripped before hashing: `postprocess: strip-mediawiki-render-id`, applied by `build_manifest.py` and `fetch_corpus.py`. After that change the full re-fetch matched 299/299.

**Unverified:** Wikipedia's HTML will still change whenever Parsoid's output format is upgraded (`data-mw-parsoid-version`). When that happens, `fetch_corpus.py` will report a sha256 mismatch for the affected HTML documents, and their hashes will need to be re-pinned. The revision ids, and so the article text, do not change. Every other source is a static file at a fixed URL (raw GitHub at a commit, Cellar manifestation, GOV.UK asset id, data.gouv.fr timestamped resource, NTRS download id, versioned arXiv id, NIST/CDC/MIC static PDFs), and all of them were byte-identical on re-fetch.

## Other things not verified

- **Licences** are taken from each source's published terms or metadata: the GOV.UK OGL default, data.gouv.fr and Schleswig-Holstein dataset metadata, NTRS copyright determination, arXiv OAI licence field, and MIC's reuse terms. I did not open every GOV.UK or data.gouv.fr file looking for embedded third-party material or logos.
- The **NTRS public-use determination** is NASA's own metadata. Several scans are NACA translations of foreign (German, French) reports, for example TM 1120, TM 1422 and TM 1423. NASA marks them public use, but the rights in the original foreign text were not checked.
- **XLSX extraction** reads cached cell values. Formulas without a cached value come out empty, but no question depends on a formula.
- **Near-duplicate documents** exist on purpose, because they are realistic: several GOV.UK insolvency requisition templates (en-govuk-xlsx-0010 to 0014) and three years of GPEX tables (0023 to 0025). No question targets them.
- **Questions:** I wrote each one by reading the passage. Each was checked to have a unique support string in the corpus, but I did not check whether a *paraphrase* of the answer appears in another document. The chat `answers` lists give the canonical form plus one or two variants. They do not cover every spelling.

## Changes after delivery (S1837)

- `de-sh-xlsx-0003.xlsx` removed from the manifest (now 298 documents): the Phase 0 parser refuses it with `zip compression ratio exceeds 100:1` (the archive-expansion guard in `crates/vz-core/src/parsers.rs`). No question used it. Recorded as a parser finding in `docs/phase0/model-bakeoff.md`.
- The files above were converted to the harness schemas (`retrieval-questions.json`, `chat-questions.json`; `size`/`license`/`license_url` added to each manifest entry). `scripts/corpus-fetch.py` applies `postprocess: strip-mediawiki-render-id` before verifying Wikipedia files.
- q019's second expected page is kept as `also_expected`; the harness scores the first page only.
