# Office C1 S1939 feasibility rerun

**RETURN_TO_DESIGN; C1 FAIL. Harness only; nothing ships.**

PR #134 follow-up to base `852614472b8ea9d460e71d5a6ffe690d17f20150`, under runbooks main `8c8ba764e434786439456c8ea9d8df16ccf1f380`, Gate 2 “C1 design delta 3 (S1939)” item 2. Final measured harness source: `eab18eda6047e4248c8eaba08040225429fa4505`. Private manifest: `/var/tmp/koskadeux/private-evidence/a1faf60504d5/manifest-v5.json`; prior results remain in `/var/tmp/koskadeux/private-evidence/4dc309568b61/manifest-v4.json`. Each stage was committed and pushed independently.

| Measurement | macOS |
|---|---:|
| OCR fixture matches | 20/20 |
| OCR full-page matches | 36/36 |
| Scanned PDF role controls | 4/4 |
| OCR post-readiness model/locale attempts | 0 |
| OCR expected memory refusals | 8/8 |
| Strict socket checks (ingest/render/OCR) | 12/12 each |
| Confinement probes per ingest/render role | 109/110 |
| Broker external API controls | 8/8 |
| Broker rows with explicit endpoint denial | 6/8 |
| PDF raster mismatches per role | 8/10 PDFs; 16/18 pages |
| Changed pixels per role | 251905 |
| PDF extracted page text equal per role | 18/18 |
| Hard resident memory guarantee | FAIL |
| Windows | NOT RUN |
| Grading/flood rig | DEFERRED |

Tesseract **5.5.1** and Leptonica **1.85.0** were built statically from pinned source tarballs. Exact download SHA256 values are in `scripts/office-tesseract-pins.json`; source licenses, installed libraries and build command records are retained privately. Five tessdata_fast models (`eng/deu/fra/spa/jpn`) at commit `87416418657359cb625c412a48b6e1d6d41c29bd` and their **Apache-2.0** license are hash-pinned. Leptonica's BSD-style source license is also retained. Setup alone downloads into the private assets directory and verifies every hash; the helper never downloads. Curl, archive, OpenMP and image codecs are disabled; the parent supplies validated, bounded raw RGB.

The parent launches a separate signed/hash-checked OCR helper after validating fixture/generation hash, page and dimensions. All five models, static libraries and C locale initialize before confinement and readiness. No tessdata path grant exists. Final full nonempty UTF-8 comparisons match on all **36 pages / 20 role-fixtures**, including the English and Japanese scans in both roles (**4/4** scanned controls). Diagnostic-only injected traces of both scans report **zero post-readiness tessdata/locale file attempts** and output identical to the uninjected final runs. Equality on the same raster is a regression check, not human accuracy grading or proof that the input raster retained all content. No snapshot is promoted; failures discard partial results and retain the old snapshot.

Actual OCR setup/readback uses baseline plus the remaining 1.5 GiB budget after counted dirtyable mappings, with zero soft/hard RLIMIT_NOFILE and no sampler. Seven allocation/limit-raise probes record kernel refusals; the eighth reserves 2 GiB before bootstrap and is refused before readiness (exit 77). The initialized OCR executable passes all 12 literal socket/socketpair controls, including after closing a supplied descriptor. Peak observed OCR-process RSS across these runs is **995,131,392 bytes**; per-process RSS and phys_footprint records are private diagnostics. These observations do **not** repair the prior excluded-read-only mapping replacement escape: macOS hard resident memory remains **FAIL**.

PDF diagnosis uses the same pinned PDFium, inputs and 1600-pixel target width. Independent unconfined repeats and confined repeats are byte-stable for **10/10 PDFs per role**. Confined versus unconfined output differs in **8 PDFs / 16 pages / 251,905 pixels per role**, while all **18 extracted page texts and non-raster fields** match. Independent macOS ImageIO decoding exactly matches the Python decoder's complete RGBA output on the saved Japanese control/confined example.

Diagnostic file/directory tracing identifies the cause: after readiness, PDFium's `opendir(/Library/Fonts)` and `opendir(/System/Library/Fonts)` return **EMFILE (24)** under RLIMIT_NOFILE=0. Controls enumerate those directories and open system fonts, including Supplemental/Arial.ttf. The /System read grant cannot overcome the descriptor ceiling. Stable repeats and identical dimensions/configuration exclude nondeterminism, PNG-only differences and a DPI/color configuration change in these cases. Successful font-file reads are SHA256-inventoried privately. The Japanese ORBIT-00-09 raster loses Japanese glyphs while its text layer remains equal; the saved first-page images were visually checked. Latin glyph shapes/metrics also change. **Visible-content fidelity remains FAIL.** No profile exception was added. A bounded preloaded byte-backed font provider remains needed; path grants/preopened files alone cannot enable later fopen/dup at a zero descriptor limit.

Both ingest/render roles have **4/4 successful external API controls**: LaunchServices `LSOpenCFURLRef` returns OSStatus 0 using a registered private local scheme; both Mach lookups succeed; the synthetic read-only XPC request receives a non-error reply. No network URL/browser is used. LaunchServices URL AppleEvent consumption is not qualified; process-table observation is diagnostic only. Earlier marker controls are preserved, including a sample blocked in the handler's file open.

Under confinement, both Mach rows return **BOOTSTRAP_NOT_PRIVILEGED (1100)**, distinct from unknown service (1102). XPC's generic connection error is paired with an actual same-process kernel lookup denial of the identical `com.apple.cfprefsd.daemon` endpoint, also 1100; generic XPC errors alone never pass. **Open-URL remains FAIL/unproven in both roles:** the actual final-profile LaunchServices call terminates with signal 5 before returning OSStatus. A trap is not an explicit sandbox denial. Thus **6/8 broker rows pass**, and the complete rerun passes **109/110 confinement probes per role**. Routed LAN/internet, packet-capture and inherited Mach-right qualification remain unrun.

Unchanged v4 evidence is carried with provenance, not rerun: 1,000 documents / 54,099 chunks / 7,747 pages; preparation busy 21,343.05 ms; preparation under search 25,809.36 ms; search maximum 263.57 ms. The earlier loaded FTS 27.440 s and successor 29.242 s versus E4 29.010 s remain failures despite a faster repeat. SciFact Exact retains 300 queries, recall@10 0.773778 and nDCG@10 0.493511 on the existing combined index; this is not re-extraction qualification. The 50 non-PDF fixture comparisons and five hostile refusals per role are carried from v4. Full owner extraction/embedding, structural image/table/accessibility qualification, peak temporary disk, installed/routed/both-host obligations remain unrun.

Validation: final native release build, generic C probe build, Python compilation, signature verification, diff whitespace checks and independent ImageIO pixel checks passed. Complete long outputs, failed attempts, source/build/model/profile pins and per-case comparisons stay in the new private evidence directory. **Windows NOT RUN; human relevance grading and multi-address flood rig DEFERRED (Mars #35415).**
