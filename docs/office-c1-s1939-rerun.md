# Office C1 S1939 feasibility rerun

**RETURN_TO_DESIGN; C1 FAIL. Harness only; nothing ships.**

PR #134 font/URL follow-up to `a4b8b929e26a3c43bdc6df2baf3827b776ac5f16`, within C1 design delta 3. Measured native source: `84cb0f3` (full SHA and source hashes in the manifest). Private manifest: `/var/tmp/koskadeux/private-evidence/916818344fd8/manifest-v6.json`. Manifest SHA256: `a1cb96dc6ba17f4e77f1d81426f7b501259dc5a45640ab12cc9cc7f2982aa75c`. Prior evidence is pinned at `/var/tmp/koskadeux/private-evidence/a1faf60504d5/manifest-v5.json`; carried results are not reruns. Each implementation stage and the results were committed and pushed independently.

| Measurement | macOS v6 |
|---|---:|
| Same-bundle PDF raster matches, each ingest/render role | 10/10 PDFs; 18/18 pages |
| Changed pixels, each role | 0 |
| Stable control / confined repeats, each role | 10/10; 10/10 |
| Extracted page text / non-raster fields equal, each role | 18/18; 18/18 |
| Diagnostic worker path attempts after readiness | 0 across 40 traces |
| Diagnostic responses equal to uninjected responses | 40/40 |
| Complete serialized fixture matches, each role | 60/60 |
| Non-PDF matches / hostile refusals, each role | 50/50; 5/5 |
| OCR fixture / full-page matches | 20/20; 36/36 |
| Scanned PDF OCR role controls | 4/4 |
| Missing / tampered bundle refused before readiness | 2/2 |
| Strict socket checks, each ingest/render role | 12/12 |
| Confinement probes, each ingest/render role | 109/110 |
| Broker external API controls / explicit-denial rows | 8/8; 6/8 |
| Open-URL explicit-denial rows | 0/2 |
| Hard resident memory guarantee (carried; design unchanged) | FAIL |
| Windows | NOT RUN |
| Grading/flood rig | DEFERRED |

The mandatory private font bundle contains **Noto Sans 2.008** and **Noto Serif 2.007**, each regular/bold/italic/bold-italic, plus **Noto Sans CJK JP 2.004** regular/bold: ten font files, **38,340,252 bytes**. `scripts/office-font-pins.json` records upstream immutable commits, font name-table versions, download URLs and SHA256 for every font and both **OFL-1.1** licence files. `scripts/office-font-setup.py --worker <private-target>/release/vz-office-worker` assembles and verifies the bundle beside the helper, outside the checkout. Setup alone downloads; the worker never downloads. Both licence texts and the verified bundle remain with private evidence.

Before confinement, the native helper reads every font into owned bounded memory, closes the files and validates compiled SHA256 pins and SFNT table bounds. Limits are 24 MiB per font and 64 MiB total. The helper installs a version-1 `FPDF_SYSFONTINFO` through `FPDF_SetSystemFontInfo` on the same loaded PDFium library. Its mapper selects bundled faces and serves whole files or SFNT tables from bytes; it never delegates to the OS mapper or opens a path in a callback. Font data and callback handles outlive all PDFium objects. Japanese CID requests map to the preloaded CJK face. Missing or modified bundles refuse before readiness. Font allocation is included in the existing pre-bootstrap baseline accounting; no memory policy was changed.

A small trusted PDF generated entirely in memory warms PDFium, raster/PNG and runtime logging before baseline/confinement. It reads no fixture or request bytes. This removed non-font lazy runtime attempts found in the first diagnostic pass (executable-directory lookup, logging preferences and `/etc/passwd`). No font path grant or descriptor-limit exception was added. Final confined helpers retain soft/hard **RLIMIT_NOFILE=0**. The diagnostic readiness marker includes the worker PID; complete traces retain wrapper activity separately, rather than attributing `rtk` history-database opens to the worker.

Both arms use the **same pinned PDFium, mandatory bundled provider, target width 1600 and rendering configuration**. Two independent unconfined and two confined runs per fixture are stable: **10 PDFs / 18 pages per role, zero changed pixels**, with complete text and non-raster equality. Forty separate injected traces report zero worker path-open/directory attempts after readiness and outputs identical to uninjected runs. The trace is diagnostic; uninjected runs qualify the comparison. ImageIO independently confirms the Python decoder's complete RGBA output for the saved Japanese control/confined page. The Japanese raster was visually inspected and contains the previously missing glyphs.

Independent CoreText cmap checks cover the actual Latin/Japanese text assigned to bundled fonts. One source-fixture limitation is retained explicitly: Latin ReportLab fixtures already replace the table-label `表` with a ZapfDingbats black square (U+25A0). That glyph uses PDFium's compiled standard-font data, pinned by the PDFium hash; it is not a Noto Latin cmap requirement or a confinement loss. Decoded fixture streams and the initially overbroad cmap diagnostic are preserved. Same-bundle parity establishes no general original-font metrics, structural image/table/accessibility fidelity or recovery of source-fixture omissions. **No confinement-induced PDF raster differences remain in these fixtures.**

All **60 fixtures per role** now match in full serialized content; **50 non-PDF fixtures and five hostile refusals per role** were rerun. The raw content runner conservatively discards its two scan rows per role; raster equality is recorded there and the separate offline OCR pipeline qualifies their text. The statically pinned Tesseract **5.5.1**, Leptonica **1.85.0** and five `tessdata_fast` models from v5 were reused with provenance. Their full nonempty UTF-8 comparisons pass **36/36 pages / 20 role-fixtures**, including **4/4** scanned controls. OCR memory probes were not rerun; no snapshot is promoted.

Open-URL was retried through public **LSOpenFromURLSpec** with a registered private local scheme, without a browser or network URL and without warming LaunchServices rights before confinement. Both external API controls return OSStatus **0**. Both final-profile calls now return **-10661 (`kLSExecutableIncorrectFormat`: no compatible executable found)**, with `explicit_launch_denial=false`; no `kLSNoLaunchPermissionErr` is observed. The successful external controls rule out a generally invalid handler, but this generic resolution/format result and errno EPERM do not identify an explicit sandbox denial. The exact failed internal operation remains unestablished. Saved macOS crash reports from the stage-2 retry, before the trusted raster warmup, identify **CFRetain(NULL)** in CFNetwork error localization while LaunchServices loads preferences. After trusted runtime warmup, the final binary returns -10661, so the earlier trap stack is diagnostic history. The crash stack does not establish the underlying failed-resource operation and is not an explicit sandbox denial. The generic probe runner was tightened to require the explicit-denial field; a generic errno can no longer mark Open-URL as passing. **Both Open-URL rows remain FAIL/unproven**, with exact reasons and raw diagnostics in v6. Both Mach rows and paired XPC/kernel controls continue to pass: **6/8 broker rows**, **109/110 confinement probes per role**. Generic XPC errors and process-table observations alone never pass.

Memory design is **unchanged, awaiting Max decision**. The v5 excluded-read-only-mapping replacement escape and hard resident guarantee remain **FAIL**; no new memory qualification is claimed. Prior OCR memory refusals and OCR model/locale tracing are carried with provenance, not rerun here. Prior corpus/search/FTS/relevance measurements and all other unrun obligations remain as recorded in v5: routed LAN/internet, packet capture, inherited Mach-right qualification, full owner extraction/embedding, structural fidelity, peak temporary disk and installed/routed/both-host obligations are not repaired by this work.

Validation includes native release build, helper signatures, generic C probe build, Python compilation, font integrity refusals, independent glyph/pixel checks and whitespace checks. Full logs, failed attempts, immutable font/licence pins, raw IPC, profiles, crash reports and per-case comparisons remain in the new private evidence directory. **Windows NOT RUN; human relevance grading and multi-address flood rig DEFERRED (Mars #35415).**
