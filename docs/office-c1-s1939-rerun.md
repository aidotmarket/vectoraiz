# Office C1 S1939 feasibility rerun

**RETURN_TO_DESIGN; C1 FAIL. No enabled distribution.**

| Measurement | macOS |
|---|---:|
| Documents | 1000 |
| Chunks | 54099 |
| Pages | 7747 |
| Preparation busy, ms | 21343.05 |
| Preparation under search, ms | 25809.36 |
| Search max, ms | 263.57 |
| Confinement passes per role | 106 |
| Confinement probes per role | 110 |
| Strict socket checks per role | 12 |
| Writable replacement bytes | 2147483648 |
| SciFact queries | 300 |
| SciFact recall@10 | 0.773778 |
| SciFact nDCG@10 | 0.493511 |

Release preparation uses a streamed typed successor, two batches at most, and separately charged writer/lookup, append, FTS and promotion work. The loaded FTS phase is 24.084 s against 25 s. Loaded recovery owner-to-ready is 25.715 s versus 29.384 s for the E4 release control. Full owner extraction/embedding was not rerun.

Preserved earlier loaded observation: FTS 27.440 s; successor owner-to-ready 29.242 s versus E4 29.010 s. Its phase/no-regression misses remain failures despite the faster repeat.

Ingest and render each retain four unproven/failing broker/open-URL rows. All 22 E4 failures per role are accounted for; the five originally lacking controls remain labelled in history. AF_UNIX now has a successful external listener control. Literal socket checks pass before and after closing a supplied descriptor; this does not qualify routed outcome confinement.

Each role compares 60 multilingual office fixtures plus five hostile refusal controls. Fifty ordinary fixtures match; eight non-scanned PDFs change raster bytes and two scans are refused. OCR is blocked: the Tesseract header/link probe fails and all five offline traineddata/license pins are absent. No download or unconfined retry occurs.

The dedicated memory helper refuses an over-budget dirtyable baseline, but replacing excluded read-only mappings permits touching 2 GiB writable memory. The hard resident requirement remains FAIL. Real ingest/render startup and allocation probes are retained separately; OCR is blocked.

SciFact Exact reruns all 300 frozen queries on the retained E4 combined index and exactly matches its baseline; this is not re-extraction/index qualification. Windows is NOT RUN. Human relevance grading and the multi-address flood rig are UNRUN, DEFERRED (Mars #35415). Peak temporary disk, full structural accessibility/content qualification, installed/routed/both-host evidence and remaining C1 obligations remain unrun.

## PR #134 follow-up, stage 1

Harness only. Tesseract 5.5.1 and Leptonica 1.85.0 were built statically from SHA256-pinned source tarballs; codec, curl, archive and OpenMP support are disabled. Five tessdata_fast models (eng/deu/fra/spa/jpn) at `87416418657359cb625c412a48b6e1d6d41c29bd` and their Apache-2.0 license are pinned in `scripts/office-tesseract-pins.json`. Downloads occur only in setup, into the private assets directory; hashes are checked before build. Source licenses and installed file hashes are recorded in the private bundle manifest.

The parent rerasterized all 10 PDF fixtures for each role and launched the separately signed/hash-checked helper. All models and the C locale initialize before confinement and readiness. No tessdata path grant or runtime download exists; full nonempty UTF-8 output matches the independent unconfined control for **20/20** fixtures, including **4/4** scanned-PDF role controls and all five languages. This supersedes the missing-engine block above. These are regression comparisons, not human relevance grading. Image dimensions, generation/source hashes, deadlines and output bounds are checked; no snapshot is promoted.

Actual OCR pre-input RLIMIT_AS setup/readback and zero RLIMIT_NOFILE succeed; six memory probes record allocation/raise outcomes with the sampler disabled. The macOS hard resident guarantee remains **FAIL** (the previous excluded-read-only replacement escape is not cured by engine vendoring). Engine/model/build hashes, complete outputs and diagnostics are under `/var/tmp/koskadeux/private-evidence/a1faf60504d5/{assets,ocr}`. Windows NOT RUN; grading/flood rig DEFERRED.

## PR #134 follow-up, stage 2

Same pinned PDFium, inputs and target width (1600): both independent repeat controls are byte-stable for **10/10 PDFs per role**. Confined versus unconfined renders differ in **8/10 PDFs, 16/18 pages per role**, totaling **251,905 changed pixels per role**. All **18/18 extracted page texts and non-raster fields per role** match. Independent macOS ImageIO decoding exactly matches the Python decoder's full RGBA output for the Japanese control/confined example.

Diagnostic-only file/dir tracing identifies lazy font discovery after readiness: `opendir(/Library/Fonts)` and `opendir(/System/Library/Fonts)` fail with **EMFILE (24)** under zero RLIMIT_NOFILE; controls enumerate both and load fonts, including Supplemental/Arial.ttf. The /System read grant cannot overcome the descriptor ceiling. This is font fallback caused by descriptor confinement, not random rendering, PNG encoding, DPI or color configuration. Successful font-file reads are hash-inventoried privately. The tracer is never used to qualify a worker.

The Japanese ORBIT-00-09 raster visibly loses Japanese glyphs (confirmed on saved first-page PNGs) while its complete text layer remains equal. Latin shapes/metrics also differ. Thus unchanged extracted text and equal OCR output from the same degraded raster do not establish visible-content fidelity. **Rendering remains FAIL**. A bounded preloaded byte-backed PDFium font provider is still needed; merely granting font paths or preopening descriptors does not permit later fopen/dup at RLIMIT_NOFILE=0. This stage records the precise cause without changing the confinement design. Full comparisons, repeat outputs, traces and pixel hashes are under `/var/tmp/koskadeux/private-evidence/a1faf60504d5/{pdf,pdf-trace}`.

## PR #134 follow-up, stage 3

Both roles now have **4/4 successful external API controls**: registered private-scheme LaunchServices `LSOpenCFURLRef` returns OSStatus 0; both Mach lookups return KERN_SUCCESS; the synthetic read-only XPC request receives a non-error reply. No network URL/browser is used. LaunchServices URL AppleEvent consumption is not qualified; handler process observation is diagnostic only. Initial marker-based controls did not establish delivery and are preserved, including a sample blocked in the handler's file open.

Under the final descriptor-only profile, **3/4 rows per role have explicit endpoint-denial evidence**. Both raw Mach lookups return `BOOTSTRAP_NOT_PRIVILEGED` (**1100**, distinct from unknown service 1102). XPC's generic connection error is paired with an actual `bootstrap_look_up` of the identical `com.apple.cfprefsd.daemon` endpoint in the same process/profile, also 1100; a generic XPC error alone never passes. This corrects the earlier errno-only classification.

**Open-URL remains FAIL/unproven in both roles**: the actual confined LaunchServices call terminates with signal 5 before returning OSStatus; this is not an explicit sandbox denial. The successful external control does not upgrade that failure. Thus **6/8 broker rows pass, 2/8 remain unproven**. Complete controls, kernel results, bootstrap/readback and trap diagnostics are under `/var/tmp/koskadeux/private-evidence/a1faf60504d5/brokers-v5`. The probe change was rebuilt into the actual signed ingest/render feasibility executable; no product path is enabled.
