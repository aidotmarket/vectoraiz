# vectorAIz 0.0.8 → 0.0.9 check — Max's Windows PC (S1833 plan, run 28 Oct 2026)

Record of the by-hand check of vectorAIz 0.0.9 on Max's Windows 11 PC (same machine as `windows-11-maxpc-0.0.7-to-0.0.8`), 28 Oct 2026, 10:23–10:38 CET. 0.0.9 contains chunk D8 (#88): Index folder clears the previous Ask answer, citations and Search results; `VZ_ENGINE_TIMING` turns on only for `1`; an invalid model-import folder is rejected before the model cache is dropped. Only the first is visible on screen; the other two are covered by CI tests and were not checked by hand.

The check was run by a Claude session on the PC, with Max clicking the administrator prompt and every Task Manager view (Task Manager runs elevated, so computer use cannot operate it).

- Plan: project doc `claude/s1833-vectoraiz-windows-prompt-0.0.9.md`.
- Full log: project doc `claude/s1833-vectoraiz-windows-0.0.9-result.md`.
- Times are local (Europe/Madrid, CET).
- Evidence (`notes.md` and 20 screenshots) stays on the PC in `Downloads\vz-byhand-0.0.8-to-0.0.9` and is not committed.
- Machine: AMD Ryzen 7 5800X, 64 GB RAM, RTX 4070, Windows 11 Home 25H2 (build 26200.9711).
- Index: the three one-line files orbital.txt, invoice.txt and heron.txt from the earlier runs.

**How the times were measured.** After each click, the session took a screenshot about every second and counted frames until the result showed. Durations are estimates to about ±1 s; clock times are to about ±5 s. They are not instrumented timings.

## Results

| Step | Result | Exact text / what happened | Time (s, estimate) |
|---|---|---|---|
| 1 Update | PASS | Banner "vectorAIz 0.0.9 is available" at launch. After Install and restart, the taskbar shield and Yes, the app reopened as "Version 0.0.9 · Native pipeline spike" in one window (10:26:50). | — |
| 2 Memory, idle, nothing searched (information) | — | One vectoraiz.exe (PID 19552): Working set 44,340 K; Memory (private working set) 6,536 K. System in use 7.5 / 63.9 GB. | — |
| 3 First Search after launch ("orbital archive") | PASS | orbital.txt · Page 1 first, then invoice.txt, heron.txt | ~1–2 |
| 4 Ask "How long is the orbital archive kept?" | PASS | "The orbital archive is kept for thirty days after which it is deleted.", citing orbital.txt, invoice.txt and heron.txt | ~8–9 |
| 5 Memory, both models loaded, idle (information) | — | Same process (PID 19552): Working set 589,464 K (~576 MB); Memory (private working set) 89,192 K (~87 MB). System in use 8.4 GB, compressed 0 MB, available 55.6 GB, committed 10.0 / 67.9 GB. | — |
| 6 Index folder with the step-4 answer on screen (D8 fix) | PASS | On the first screenshot after the click the answer and citations were gone and only the index status showed ("2/3 · 0 indexed · 0 failed …", then "3/3 · 0 indexed · 0 failed"), no error. A Search afterwards still put orbital.txt first. | <1 |
| 7 Index folder with Search results on screen | PASS | Results gone on the first screenshot; only the index status | <1 |
| 8 Second Ask "What is the orbital archive retention policy?" | PASS | "The orbital archive retention policy is thirty days, after which the cold storage tier deletes the snapshot.", same citations | <1 |

## Findings

1. **D8 confirmed by hand:** Index folder clears the previous answer, citations and Search results as soon as indexing starts. This closes the 0.0.8 minor finding (answer left under the index status).
2. **Memory with both models loaded (answers the open 0.0.8 question).**
   - vectoraiz.exe working set grew from ~43 MB to ~576 MB after one Search and one Ask; its private working set is ~87 MB. The 88 MB seen on 0.0.8 was the private part only.
   - The ~533 MB difference is close to the two model files (67,308,128 + 491,400,032 bytes). That it is the mapped model files is inferred from the size and from llama.cpp mapping model files by default (`crates/vz-core/src/engine.rs` loads with default parameters); Task Manager does not show it directly.
   - Not included: the app's window runs in separate `msedgewebview2.exe` processes, which Details does not attribute to vectoraiz.exe and which were not measured.
   - **8 GB verdict (Phase 0, inference, not a test on an 8 GB machine):** the engine process needs about 0.6 GB with both models loaded, most of it file-backed pages Windows can drop and re-read under memory pressure, plus the unmeasured WebView2 processes. That should fit on an 8 GB PC alongside normal use. A run on a real 8 GB machine, including the WebView2 processes, is still needed before stating a supported minimum.
3. First Search after launch was ~1–2 s; first Ask ~8–9 s (chat model load); second Ask under 1 s.
4. Re-indexing unchanged files finishes in under a second with "0 indexed".
5. **Session note:** one Task Manager read (step 5) was taken from the screenshot just before Max scrolled the list. Only this Claude run wrote to the evidence folder (checked at 10:23 and 10:38).

## Left on the PC

vectorAIz 0.0.9 installed and open; models and index unchanged. Nothing deleted.
