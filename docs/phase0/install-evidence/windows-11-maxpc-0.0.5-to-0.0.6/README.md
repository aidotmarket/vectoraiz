# vectorAIz 0.0.5 → 0.0.6 by-hand check — Max's Windows PC (S1823)

Record of the by-hand check of the chunk D4 fixes (vectorAIz #77) on Max's Windows 11 PC (the same machine as `windows-11-maxpc-0.0.4-to-0.0.5`), run by a Claude session on the PC with Max clicking the administrator prompt. Plan: project doc `claude/s1823-vectoraiz-windows-prompt-0.0.6.md`; full log: project doc `claude/s1823-vectoraiz-windows-0.0.6-result.md`. Times are local (Europe/Madrid, CET). Evidence (notes.md and 21 screenshots) stays on the PC in `Downloads\vz-byhand-0.0.5-to-0.0.6`; not committed.

## Results

| Step | Result | What happened | Times |
|---|---|---|---|
| 1 Banner and hint | PASS | 0.0.5 showed "vectorAIz 0.0.6 is available", Install and restart, Later, and the hint "Windows will ask for administrator approval. If no prompt appears, click the shield in the taskbar." | 19:06 |
| 2 Update | PASS | Prompt only as taskbar shield; Max: shield → Yes (~19:07); app reopened itself as 0.0.6, one window | 19:07–19:08 |
| 3 Startup model check | PASS | Only "Checking models..." (no buttons, no "Models needed"); gone after roughly 10–17 s | 19:08 |
| 4 Search | PASS | "Searching...", Run self-test greyed with its own label, results show "orbital.txt · Page 1", then invoice.txt and heron.txt | 19:08–19:09 |
| 4 Ask | FAIL (answer text) | "Thinking...", citations show file names, but the answer is only "[1]" followed by the app's "Retrieved sources: [1] [2] [3]" footer (twice) | 19:09–19:10 |
| 5 Download labels | PASS | "Downloading" → "Downloaded" per file; panel closed after verification; both files match size and SHA-256 | 19:21–19:22 |
| 6 Import closes panel | PASS | Import from models-006-aside: panel closed, "Models imported and verified.", search works | 20:23–20:24 |
| 7 No index yet | PASS | With phase0-index renamed away: "Index a folder first." and no new phase0-index created; after renaming back, same top result | 20:38–20:51 |

## Ask finding

Not a 0.0.6 regression: vz-core's answer generation is unchanged since 0.0.5, and Ask had not been tried on a real index in earlier by-hand runs. Reproduced by Minerva on Koskadeux (macOS) with `vectoraiz-cli` at native `1579a2f1`, three one-line text files (orbital, invoice, heron) and both questions "How long is the orbital archive kept?" and "What is the orbital archive retention policy?": the answer text was `[1]\n\nRetrieved sources: [1] [2] [3]` with `--cpu` and with Metal, so Windows and GPU execution are not required to trigger it; suspected cause (not isolated): model/prompt sensitivity with short sources. `scripts/pipeline-e2e.py` passes only on the 20 longer fixture documents. Open defect; fix to be scoped separately.

## Other notes

- The administrator prompt again appeared only as a taskbar shield; the 0.0.6 hint covers this.
- Computer-use approval expired after 30 minutes idle between steps 5 and 6 (tooling, not the app).

## Left on the PC

vectorAIz 0.0.6 installed; models and index in place. Nothing deleted. Spare model copies kept: models-006-aside, models-006-downloaded (plus the 0.0.5-run folders).
