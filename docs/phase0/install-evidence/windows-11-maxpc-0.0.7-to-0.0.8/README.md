# vectorAIz 0.0.7 → 0.0.8 check — Max's Windows PC (S1829)

Record of the by-hand check of vectorAIz 0.0.8 on Max's Windows 11 PC (same machine as `windows-11-maxpc-0.0.6-to-0.0.7`), 27 Oct 2026. 0.0.8 contains chunk D6 (Search and Ask clear each other's results, #84) and chunk D7 (models loaded and checked once per app session, #85).

The check was run by a Claude session on the PC, with Max clicking the administrator prompt and the Task Manager views.

- Plan: project doc `claude/s1829-vectoraiz-windows-prompt-0.0.8.md`.
- Full log: project doc `claude/s1829-vectoraiz-windows-0.0.8-result.md`.
- Times are local (Europe/Madrid, CET).
- Evidence (`notes.md` and 20 screenshots) stays on the PC in `Downloads\vz-byhand-0.0.7-to-0.0.8` and is not committed.
- Machine: AMD Ryzen 7 5800X, 64 GB RAM, RTX 4070, Windows 11 Home 25H2 (build 26200.9711).
- Index: the three one-line files orbital.txt, invoice.txt and heron.txt from the earlier runs.

**How the times were measured.** After each click, the session took a screenshot about every second (every 0.5 s for step 3) and counted frames until the result showed. Durations are estimates to about ±1 s; clock times are to about ±5 s. They are not instrumented timings.

## Results

| Step | Result | Exact text / what happened | Time (s, estimate) |
|---|---|---|---|
| 1 Update | PASS | The banner "vectorAIz 0.0.8 is available" showed at launch. After Install and restart, the taskbar shield and Yes (17:10–17:11), the app reopened as "Version 0.0.8 · Native pipeline spike" in one window. | — |
| 2 First Search after launch ("orbital archive") | PASS | orbital.txt · Page 1 first | ~2–3 |
| 3 Second Search ("cold storage snapshot") | PASS | orbital.txt first | ~0.5–1 |
| 4 Ask "How long is the orbital archive kept?" | PASS | "The orbital archive is kept for thirty days after which it is deleted.", citing orbital.txt, invoice.txt and heron.txt | ~7–8 |
| 5 Second Ask "What is the orbital archive retention policy?" | PASS | "The orbital archive retention policy is thirty days, after which the cold storage tier deletes the snapshot.", same citations | ~1–2 |
| 6 Search and Ask clear each other | PASS | Search "invoice" removed the previous answer. Ask "Who sent the invoice?" removed the results and answered "The invoice was sent by Northwind Supplies." | within 6 |
| 7 Re-index `Documents\vz-test` | PASS | "3/3 · 0 indexed · 0 failed"; a Search afterwards still put orbital.txt first | <1 |
| 8 Memory (information only) | — | Task Manager Processes: one vectorAIz process, 88.0 MB, idle. System memory in use: 8.1 of 63.9 GB. | — |
| 9 Ask "Who painted the Mona Lisa?" (information only) | — | "The Mona Lisa was painted by Leonardo da Vinci.", citing the three unrelated files | — |

## Findings

1. **Repeat Search and Ask are fast on Windows (D7 confirmed).**
   - Second Search: about 1 s. Second Ask: about 1–2 s.
   - For comparison, D7 (#85) measured Search at 12.1 s before and 0.43 s after on macOS; there is no instrumented Windows baseline from earlier versions.
   - The first Search after launch was also only about 2–3 s on this machine.
2. **The memory figure does not answer the 8 GB question.**
   - The 88 MB is Task Manager's default Memory column, which counts only memory private to the process.
   - The engine loads the models with llama.cpp's default parameters (`crates/vz-core/src/engine.rs`), and llama.cpp maps model files into memory by default. Mapped file pages are not counted in that column. So the ~560 MB of model files are most likely not in the 88 MB. This is inferred from the code and llama.cpp defaults, not measured.
   - The open item "measure memory on an 8 GB Windows machine" stays open. It needs the working-set column (Task Manager > Details) or a machine with 8 GB.
3. Re-indexing unchanged files finishes at once with "0 indexed"; nothing is embedded again.
4. **Minor:** after a re-index, the previous Ask answer stays on screen under the index status. D6 clears results only between Search and Ask; a new Search clears it.
5. The off-topic question gets the same answer as on 0.0.7: from general knowledge, with unrelated files cited. This is a known Phase 0 limit (Max, 2026-10-26).
6. **Session note:** a misclick at 15:34 opened Device Manager, which computer use could not close. Nothing was changed in it, and the run paused until Max closed it at 17:10. Only this Claude run wrote to the evidence folder.

## Left on the PC

vectorAIz 0.0.8 installed; models and index unchanged. Nothing deleted.
