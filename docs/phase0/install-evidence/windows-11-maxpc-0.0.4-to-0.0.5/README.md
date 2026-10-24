# By-hand Windows 11 run: native-v0.0.4 → native-v0.0.5 (2026-10-24/25, Minerva S1823)

Release proof step 2 of Gate 2 spec 4.4b (Amendment 3, runbooks `76a4b086`), plan in the ai.market project doc `claude/s1823-vectoraiz-windows-prompt-0.0.5.md`. **All ten steps PASS.** This also closes the 4.4a cancel-at-UAC case that was not exercised on 2026-10-24 (record `windows-11-maxpc` 0.0.3 → 0.0.4 in the project doc `claude/s1817-vectoraiz-windows-prompt-0.0.4.md`).

## Machine

- Max's Windows 11 PC (`winmax`, Windows 11 Home 10.0.26200, x64), **administrator** account, driven by a Claude session on the PC with Max clicking the installer, the admin prompt and app buttons.
- Not a clean machine: `%USERPROFILE%\vectorAIz` also holds the old February 2026 `vectoraiz` source checkout (same folder on Windows; Max: not a concern).
- Run 2026-10-24 23:28 to 2026-10-25 00:11 CEST.

## Evidence

On Max's PC in `%USERPROFILE%\Downloads\vz-byhand-0.0.4-to-0.0.5` (not committed): `notes.md` (timed log and results table), screenshots 01–34, `install-004.log`, `snapshot-before.csv`, `snapshot-after.csv`, `setup-output.txt`, `models-output.txt`, `verify-step9.txt` and the helper scripts. Screenshots can be added to this folder when collected from the PC.

## Steps and result

| Step | Result | Key facts |
|---|---|---|
| Setup | PASS | 0.0.4 MSI 358,383,616 bytes, SHA-256 `9c78d54f…455a8` matches SHA256SUMS. Nothing installed beforehand. |
| 1 Install 0.0.4 | PASS | `msiexec /i … /l*v`: status 0; app shows 0.0.4. Double-clicking the MSI also opened the installer this time (the earlier Explorer failure did not reproduce). |
| 2 Models by hand | PASS | Both pinned files placed in `%USERPROFILE%\vectorAIz\models` with exact size and SHA-256. |
| 3 Index and search | PASS | 3 of 3 files indexed. Top result for "orbital archive": "Orbital archive retention is thirty days, after which the cold storage tier deletes the snapshot." |
| 4 Quiet snapshot before | PASS | 805 files under `%USERPROFILE%\vectorAIz`, app not running. |
| 5 Cancel at the admin prompt | PASS | The prompt appeared only as a taskbar shield. **No** was clicked; no message shown. Installed apps still 0.0.4; app reopened as 0.0.4 with the banner back; same search result. |
| 6 Accept | PASS | Shield, then **Yes** (~23:51:30). The app reopened by itself as 0.0.5 in one window; Installed apps shows 0.0.5. The new admin-prompt hint was not visible, as expected (it can only show on an update from 0.0.5). |
| 7 Quiet snapshot after | PASS | `snapshot-after.csv` **byte-identical** to the before snapshot (path, size, SHA-256 of all 805 files, SQLite files included). 0.0.5 returned the same top result without re-indexing. |
| 8 Second launch | PASS | Start menu with the window open, Start menu with it minimised, and an Explorer double-click on `vectoraiz.exe` each brought the existing window to the front. No second window. |
| 9 Download, cancel, resume | PASS | Models panel lists size, licence and source; the source is plain text, not a link. Index/Search/Ask disabled until models arrived. The first download (~80 MB/s) finished before it could be cancelled, so the step was repeated: Cancel kept a qwen `.partial` of 333,216,366 bytes and Download resumed it. All SHA-256 match; search works. |
| 10 Import from a folder | PASS | A folder with only the embedding file reported "imported / missing" with no disk-space error. Importing the full folder imported and verified both files; search works. Import copies and leaves the source in place. |

## Findings (not fixed in this record)

1. **"Models needed" during the startup check.** On a normal 0.0.5 start with verified models present, the panel shows "Checking models…", the Download/Import buttons and "Models needed: download or import them first." for about 20 seconds while the files are hashed. A user could start an unnecessary download. Worth fixing (show only "Checking models…" until the check completes).
2. Model lines keep saying "Needed" during a download until every file is finished.
3. After an import the Models panel stays visible until the app is reopened.
4. While a search runs, the Run self-test button reads "Running…".
5. A `msedgewebview2.exe` window appeared for under 4 seconds during the minimised second launch.
6. The downloaded model folders have no `pdfium` folder; after the import, `models` has one again. PDF parsing was not tested in this run.

## State left on the PC

0.0.5 installed; `%USERPROFILE%\vectorAIz\models` holds both verified models. Extra folders `models-aside`, `models-download1`, `models-downloaded`, `models-onefile` can be deleted.
