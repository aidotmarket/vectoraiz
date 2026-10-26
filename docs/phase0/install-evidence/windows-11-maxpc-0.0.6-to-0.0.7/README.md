# vectorAIz 0.0.6 → 0.0.7 Ask check — Max's Windows PC (S1823)

Record of the by-hand check of the chunk D5 Ask fix (vectorAIz #80) on Max's Windows 11 PC (same machine as `windows-11-maxpc-0.0.5-to-0.0.6`), run by a Claude session on the PC with Max clicking the administrator prompt, 26 Oct 2026. Plan: project doc `claude/s1823-vectoraiz-windows-prompt-0.0.7.md`; full log: project doc `claude/s1823-vectoraiz-windows-0.0.7-result.md`. Times are local (Europe/Madrid, CET). Evidence (notes.md and screenshots) stays on the PC in `Downloads\vz-byhand-0.0.6-to-0.0.7`; not committed. Index: the three one-line files orbital.txt, invoice.txt, heron.txt from the 0.0.6 run.

## Results

| Step | Result | Exact text / what happened | Time |
|---|---|---|---|
| 1 Update | PASS | 0.0.6 had been open overnight, so the banner appeared only after Check for updates (the automatic check runs at launch); "vectorAIz 0.0.7 is available" + admin hint; shield → Yes; one window as "Version 0.0.7" | 10:07–10:11 |
| 2 Ask: How long is the orbital archive kept? | PASS | "The orbital archive is kept for thirty days after which it is deleted. Retrieved sources: [1] [2] [3]"; citations orbital.txt, invoice.txt, heron.txt · Page 1 | 10:11 |
| 3 Ask: What is the orbital archive retention policy? | PASS | "The orbital archive retention policy is thirty days, after which the cold storage tier deletes the snapshot. Retrieved sources: [1] [2] [3]"; same citations | 10:12 |
| 4 Ask: Who painted the Mona Lisa? (information only) | — | "The Mona Lisa was painted by Leonardo da Vinci. Retrieved sources: [1] [2] [3]"; citations invoice.txt, orbital.txt, heron.txt | 10:12 |
| 5 Search "orbital archive" | PASS | orbital.txt · Page 1 first | 10:13 |

## Findings

1. **Off-topic question answered from general knowledge, with unrelated files cited (open defect).** The answer for step 4 is not in any indexed file, yet an answer was given and the app listed all three files as sources. Confirmed in code: retrieval returns up to the top three available hits with no relevance cutoff (storage.rs search, desktop.rs), Ask uses every hit and rejects only an empty set (index.rs), and the app itself appends all retrieved citations, so the citations do not show the model chose them. The prompt (engine.rs) does say "If the sources do not answer say so." The reported answer is consistent with the model not following that instruction (inferred, not verified).
2. The previous Ask answer stays on screen under later Search results (minor).
3. A second Claude run worked on the same PC and evidence folder at 10:08–10:11 and most likely clicked Ask once; steps 2–5 were run and screenshotted by the main run and no interference was observed in its log; the overlap at 10:11 means this is not proven.

## Left on the PC

vectorAIz 0.0.7 installed; models and index unchanged. Nothing deleted.
