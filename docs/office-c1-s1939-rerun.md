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
