# By-hand Windows 11 run: native-v0.0.1 → native-v0.0.2 (2026-10-23, Minerva S1817)

Phase 0 exit item 2, by-hand Windows run (Gate 2 spec 4.4, Amendment 1), on Max's own PC as decided on 2026-10-22 (Event Ledger 2ec1164d). **Coverage: PARTIAL.** Download, checksum, first-run self-test, a by-hand upgrade from 0.0.1 to 0.0.2 with its own self-test, and uninstall are covered. The 0.0.1 installer screens (SmartScreen, UAC, wizard) were not captured because Max clicked through them before the installer window could be approved for computer use. The signed in-app updater was not run on this PC: there is no shell on it for the `--update-test` flag. The `windows-2022` CI update test (run 39834963327) is still only a stand-in.

## Machine

- Max's Windows 11 PC (`winmax`, x64, local account), driven through the Claude desktop app's computer use and its file bridge. It is not a clean machine: `%USERPROFILE%\vectoraiz` already held an old Docker-era vectorAIz source checkout from February 2026 (finding 4).
- Smart App Control was **on** at the start. Max turned it off during the run (step 3).
- Computer use limits on Windows: browsers are view-only, and File Explorer and terminals are click-only (no typing). Downloads went through the desktop app's built-in browser. Installers were opened by double-click in File Explorer, or by Max.

## Steps and result

| Step | What was done | Result | Screenshot |
|---|---|---|---|
| Download 0.0.1 | Built-in browser → release page `native-v0.0.1` → `vectorAIz_0.0.1_x64.msi` | PASS. 357,707,776 bytes; SHA-256 `d3d6d930…dc936c` equals the release digest | 01 |
| Install 0.0.1 | Double-click in Explorer started `msiexec`; Max finished the installer himself | PASS (installed, desktop shortcut created). SmartScreen, UAC and installer screens **not observed** | — |
| First open | Desktop shortcut | **Blocked by Smart App Control** ("Smart App Control blocked…"), as the install notes predict for an unsigned build. Max turned Smart App Control off | 02 |
| Self-test 0.0.1 | Opened the app, Run self-test | PASS. `status: OK`, `version: 0.0.1`, `os: windows`, `arch: x86_64` | 03 |
| Download 0.0.2 | Built-in browser → `vectorAIz_0.0.2_x64.msi`; Max confirmed a download prompt in the Claude app | PASS. 357,715,968 bytes; SHA-256 `ef9f841a…165075` equals the release digest | — |
| Upgrade by hand | Double-click on the 0.0.2 MSI did nothing, both from computer use and from Max (finding 1). Max ran `msiexec /i … /l*v` from PowerShell and finished the wizard; it asked to close files in use | PASS. Log: elevated per-machine install, `RemoveExistingProducts` removed 0.0.1 (`{B831281B…}`), "Product Version: 0.0.2 … Installation success or error status: 0" | — |
| Self-test 0.0.2 | Opened the app, Run self-test | PASS. `status: OK`, `version: 0.0.2` | 04 |
| Uninstall | Settings → Apps → Installed apps → vectorAIz 0.0.2 → Uninstall → Uninstall | PASS. App gone from the list; `C:\Program Files\vectorAIz` removed; `%USERPROFILE%\vectoraiz` unchanged. No UAC prompt was visible to computer use | 05–07 |
| Data kept | The app wrote nothing to `%USERPROFILE%\vectoraiz` during this run (all files dated February; no `models` or `phase0-index`) | Not shown: there was no app data to preserve | — |

## Findings (not fixed in this record)

1. **Double-clicking the 0.0.2 MSI did nothing** while 0.0.1 was installed. Two double-clicks from computer use and one from Max did not open the installer, and nothing was installed. `msiexec /i` from PowerShell worked straight away. The cause is unknown. A vectorAIz process was probably still running (see 2), but no log exists for the failed double-clicks.
2. **Two app instances after the upgrade.** The wizard's files-in-use step shut vectorAIz down and Restart Manager restarted it after install. Launching it again from the shortcut then gave two windows. The app is not single-instance, and something was still running after its window had been closed.
3. Smart App Control blocks the unsigned app with no per-app exception. Expected until code signing (phase 2); the install notes already say so.
4. The data folder is `%USERPROFILE%\vectorAIz`. Windows paths are not case-sensitive, so an existing `vectoraiz` folder (here, an old source checkout) is shared with app data. Max: not a concern with no previous customers (2026-10-23).

## Method notes

Screenshots come from computer use on the PC, cropped to the relevant window. They are **not committed here**: the session that ran the test worked from a cloud container with no GitHub access to this repository, and the only route to Koskadeux was a text channel. They were delivered to Max in the S1817 chat; the numbers in the table refer to those files (01 release page, 02 Smart App Control block, 03 and 04 self-tests, 05 to 07 uninstall). Windows not approved for computer use (installer, browser downloads bar, Smart App Control dialog) are masked, which is why the 0.0.1 install has no screenshots. Checksums were taken on copies of the downloaded files. Facts and log extracts: `pc-facts.txt`.
