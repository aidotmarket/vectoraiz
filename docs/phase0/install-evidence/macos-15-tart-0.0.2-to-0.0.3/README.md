# By-hand macOS run: native-v0.0.2 → native-v0.0.3, manual DMG replacement (2026-10-23, Minerva S1817)

Phase 0 exit item 2, by-hand macOS run for the D2 release candidate (Gate 2 spec 4.4 and 4.4a, Amendment 2). This closes the by-hand update step left PENDING in `../macos-15-tart-0.0.1-to-0.0.2/`: a tester downloads the newer DMG and replaces the app in Applications. **Coverage: PASS for install, Gatekeeper, manual replacement update, self-tests, the 0.0.3 "Check for updates" control and uninstall; PARTIAL for data preservation** (the data folder held only the extracted PDFium library, because indexing stalled, finding 2). The update prompt itself ("an update is available") is proved at 0.0.4, not here, because 0.0.3 is the latest release.

## Machine

- Fresh clone `vz-s1817-003` of `ghcr.io/cirruslabs/macos-sequoia-vanilla:latest` (digest `sha256:57a83b1b…1587`), macOS 15.7.7 (24G720) arm64, tart 2.41.1, on Koskadeux. Never ran vectorAIz before: no `~/vectorAIz` at start.
- Gatekeeper assessments re-enabled with `spctl --global-enable` before the run (`assessments enabled`). Facts: `vm-facts.txt`. VM deleted after the run.

## Steps and result (times Europe/Madrid)

| Time | Step | What was done | Result | Screenshot |
|---|---|---|---|---|
| 17:17 | Download 0.0.2 | Safari → release page `native-v0.0.2` → `vectorAIz_0.0.2_aarch64.dmg`; allowed downloads from github.com | PASS. SHA-256 `7fac3da3…c78f` equals that release's `SHA256SUMS`; `com.apple.quarantine` set by Safari | 01, 02 |
| 17:19 | Install | Opened the DMG, dragged vectorAIz onto Applications | PASS. Quarantine kept on the app | 03 |
| 17:21 | First open | Opened the app | Blocked: "vectorAIz Not Opened" | 04 |
| 17:21 | Allow | Privacy & Security → **Open Anyway** → **Open Anyway** → administrator user name and password → **OK** | PASS. "Version 0.0.2" | 05–07 |
| 17:22 | Self-test 0.0.2 | Run self-test | PASS. `status: OK`, `version: 0.0.2`, `os: macos`, `arch: aarch64` | 08 |
| 17:25 | Index a folder | Folder `/Users/admin/Documents/vz-test` (one 107-byte text file) → Index folder | **Stalled** on "Scanning…" for over 2 minutes, app at 0% CPU, nothing written under `~/vectorAIz` except the PDFium library (finding 2) | 09 |
| 17:26 | Quit | vectorAIz menu → Quit vectorAIz | Process gone | — |
| 17:27 | Download 0.0.3 | Safari → release page `native-v0.0.3` → `vectorAIz_0.0.3_aarch64.dmg` | PASS. SHA-256 `21da3805…bc51` equals that release's `SHA256SUMS`; quarantined | 10 |
| 17:28 | Replace | Opened the DMG, dragged vectorAIz onto Applications → "An older item named "vectorAIz" already exists in this location" → **Replace** | PASS. `/Applications/vectorAIz.app` now `CFBundleShortVersionString` 0.0.3, quarantined | 11 |
| 17:29 | First open 0.0.3 | Opened the app | Blocked again: "vectorAIz Not Opened". The earlier Open Anyway does **not** carry over to the replaced ad-hoc build (finding 1) | 12 |
| 17:30 | Allow 0.0.3 | Privacy & Security → Open Anyway → Open Anyway → administrator password → **OK** | PASS. "Version 0.0.3 • Native pipeline spike" | 13 |
| 17:31 | Self-test 0.0.3 | Run self-test | PASS. `status: OK`, `version: 0.0.3` | 14 |
| 17:31 | Check for updates | Clicked **Check for updates** (new in D2) | PASS. "vectorAIz is up to date." No banner at startup (0.0.3 is the latest release) | 15 |
| 17:31 | Data kept by update | SHA-256 of every file under `~/vectorAIz` before (0.0.2) and after (0.0.3) | PASS for what the folder held: `models/pdfium/lib/libpdfium.dylib` `db696f0c…b7ae`, identical. No index existed (finding 2) | `hash-*.txt` |
| 17:34 | Uninstall | Quit, ejected the DMG, Finder → Applications → vectorAIz → File → Move to Trash | PASS. App in Trash; `~/vectorAIz` byte-identical | 16, 17 |

DMG hashes and quarantine attributes: `dmg-hashes.txt`. Folder hashes: `hash-0.0.2-before-update.txt`, `hash-0.0.3-after-update.txt`, `hash-after-trash.txt`.

## Findings (not fixed in this record)

1. **Every manual update repeats Open Anyway.** Replacing the ad-hoc-signed app with a newer DMG brings back "Not Opened", and the tester has to go through Privacy & Security → Open Anyway → administrator password again. The in-app signed updater does not have this problem (S1817 0.0.1→0.0.2 record). The macOS install notes should say so; Developer ID signing and notarization (stage 2) remove it.
2. **Index folder stalls on "Scanning…"** (second sighting; the first was in the 0.0.1→0.0.2 run). The process sat at 0% CPU with nothing written for over two minutes. Whether the typed path reached the field is still not shown on screen (finding 3), so this is recorded as reproduced in this VM, not yet as a confirmed defect on real hardware.
3. **Text in the Folder and Question fields is not visible.** The inputs use `text-slate-950` on the dark window background, so typed text cannot be read (and OCR finds none). The fields work otherwise.
4. On both opens the administrator-password sheet needed an **OK** click; pressing Enter after typing the password did not submit it.

## Method notes

Driven over one persistent VNC connection (tart `--vnc-experimental`, vncdotool) and checked by OCR. Downloads, drags, Gatekeeper, Settings, the app buttons and Move to Trash were done through the GUI; `open` from a shell stood in for double-clicks, and the first DMG was ejected with `hdiutil detach` from a shell. Screenshots are full-framebuffer captures scaled to 1024 px.
