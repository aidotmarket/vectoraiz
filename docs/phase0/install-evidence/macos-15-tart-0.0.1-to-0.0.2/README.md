# By-hand macOS run: native-v0.0.1 → native-v0.0.2 (2026-10-22, Minerva S1817)

Phase 0 exit item 2, by-hand macOS run (Gate 2 spec 4.4, Amendment 1). Windows 10/11 runs follow on a physical PC (Max decision, Event Ledger 2ec1164d).

## Machine

- Clean macOS 15.7.7 (24G720) arm64 VM on Koskadeux, tart 2.41.1, image `ghcr.io/cirruslabs/macos-sequoia-vanilla` (digest `sha256:57a83b1b…1587`).
- The vanilla image ships with Gatekeeper assessments **disabled**. They were re-enabled (`spctl --global-enable`) before the run so the VM behaves like a customer Mac. SIP enabled. No `~/vectorAIz` before the run. Facts: `vm-facts.txt`.

## Steps and result

| Step | What was done | Result | Screenshot |
|---|---|---|---|
| Download | Safari → release page `native-v0.0.1` → `vectorAIz_0.0.1_aarch64.dmg`; allowed downloads from github.com | PASS. SHA-256 `fd63bf17…a9c6` equals the release `SHA256SUMS`; file carries `com.apple.quarantine` (Safari) | 01, 02 |
| Install | Opened the DMG, dragged vectorAIz onto Applications, ejected | PASS. `/Applications/vectorAIz.app` (584 MB), quarantined | 03 |
| First open | Opened the app | Blocked: "vectorAIz Not Opened" (Done / Move to Trash), as expected for an ad-hoc build | 04 |
| Allow | System Settings → Privacy & Security → **Open Anyway** → dialog **Open Anyway** → **administrator user name and password** → OK | PASS. App opened, "Version 0.0.1" | 05–08 |
| Self-test | Run self-test | PASS. `status: OK`, `version: 0.0.1`, `os: macos`, `arch: aarch64` | 09 |
| Update | App quit; signed updater run against the published feed (`VZ_UPDATE_TO_VERSION=0.0.2 vectoraiz --update-test`) | PASS. Exit 0; bundle version 0.0.2; signature ad-hoc; no Gatekeeper block afterwards (quarantine gone) | — |
| Self-test after update | Opened the app, Run self-test | PASS. `status: OK`, `version: 0.0.2`. No keychain prompt appeared | 10, 11 |
| Data kept by update | SHA-256 of every file under `~/vectorAIz` before and after | PASS. Identical (`models/pdfium/lib/libpdfium.dylib`) | `vm-facts.txt` |
| Uninstall | Finder → Applications → vectorAIz → File → Move to Trash | PASS. App in Trash; `~/vectorAIz` still present and byte-identical. The install notes are what tell the user the folder is kept; the app shows nothing on a drag-to-Trash uninstall | 12, 13 |

## Findings (not fixed in this record)

1. **No user-facing update path.** 0.0.1 has no update prompt, menu item or button. The only way to run its signed updater is the test-only `--update-test` command-line flag, which is compiled into the published build (the smoke-test feature). So the update step above used that flag: it proves the signed update installs and keeps data on a clean, Gatekeeper-enabled Mac, but not an update a tester can do by hand. A tester today updates by downloading the new DMG and replacing the app.
2. **Open Anyway needs an administrator password** on macOS 15, and the confirm button is labelled "Open Anyway", not "Open". The install notes are corrected in this change.
3. The app icon is a plain teal square (placeholder).
4. One attempt at "Index folder" stayed on "Scanning..." for over a minute. It is unconfirmed whether the folder path was entered (the VNC typing may have missed the field); not counted as a defect until reproduced.

## Method notes

Driven over VNC (tart `--vnc-experimental`) with vncdotool and checked by OCR; the downloads, drag, Gatekeeper and Settings steps were done through the GUI, and `open` from a shell stood in for a double-click. Screenshots are scaled to 1280 px. Earlier screenshots were taken with incremental framebuffer updates, so screen areas below an active window can show stale content; 05 onwards were cross-checked with full refreshes.
