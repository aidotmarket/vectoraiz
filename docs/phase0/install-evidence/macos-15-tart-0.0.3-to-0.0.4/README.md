# By-hand macOS run: native-v0.0.3 → native-v0.0.4, update-available prompt (2026-10-23, Minerva S1817)

Phase 0 exit item 2, by-hand macOS proof of the D2 update-available prompt (Gate 2 spec section 4.4a, Amendment 2). native-v0.0.4 was the latest test build, so a fresh 0.0.3 install should offer it at startup and install it only after consent. **Coverage: PASS for install, Gatekeeper, the startup banner, Later, consented in-app install and restart, self-test 0.0.4, "Check for updates" on 0.0.4 and uninstall. NOT exercised: cancelling an administrator prompt during the update, because macOS asked for none (finding 2). PARTIAL for data preservation**: the data folder held only the extracted PDFium library, because Index folder stalled again (finding 1).

Labels: *Captured* means a screenshot, hash file or shell output in this folder shows it. *Operator* means the agent driving the VM observed it and it is not otherwise captured.

## Machine

- Fresh clone `vz-s1817-004` of `ghcr.io/cirruslabs/macos-sequoia-vanilla:latest` (digest `sha256:57a83b1b…1587`), macOS 15.7.7 (24G720) arm64, tart 2.41.1, on Koskadeux. No `~/vectorAIz` at start.
- Gatekeeper assessments re-enabled with `spctl --global-enable` before the run (`assessments enabled`). Facts: `vm-facts.txt`. The VM and the tart image cache were deleted after the run.
- Test folder `~/Documents/vz-test` with two one-line text files, created over SSH before the run.

## Steps and result (times Europe/Madrid)

| Time | Step | What was done | Result | Evidence |
|---|---|---|---|---|
| 20:14 | Download 0.0.3 | Safari → release page `native-v0.0.3` → `vectorAIz_0.0.3_aarch64.dmg`; allowed downloads from github.com | PASS. SHA-256 `21da3805…bc51` equals that release's `SHA256SUMS`; `com.apple.quarantine` set by Safari | Captured: 01, 02, `dmg-hashes.txt` |
| 20:15 | Install | Opened the DMG, dragged vectorAIz onto Applications | PASS. Bundle 0.0.3, quarantined | Captured: 03, 04 (shell readback in `timeline.txt`) |
| 20:15–20:23 | First open | Opened the app | Blocked: "vectorAIz Not Opened" | Captured: 05 |
| 20:23 | Allow | Privacy & Security → **Open Anyway** → **Open Anyway** → administrator password → **OK** | PASS. "Version 0.0.3" | Captured: 06–08 |
| 20:23 | Startup banner | No click; the app checked for updates at startup | PASS. Banner "vectorAIz 0.0.4 is available" with **Install and restart** and **Later** | Captured: 09 |
| 20:24 | Later | Clicked **Later** | PASS. Banner hidden; no update files under the app caches and no `~/vectorAIz` (shell readback, operator) | Captured: 10 |
| 20:24 | Reopen | vectorAIz menu → Quit, then opened again | PASS. No Gatekeeper prompt; banner shown again | Captured: 11 |
| 20:25–20:28 | Index a folder | Folder field: End, 30 × Backspace, typed `/Users/admin/Documents/vz-test` → **Index folder** | **Stalled** on "Scanning…" for over 3 minutes; the process used 0.91 s CPU in total and wrote nothing (finding 1) | Captured: screenshots only; CPU total, process start time, quarantine and Gatekeeper details are operator observations (narrative in timeline.txt, not raw readbacks) |
| 20:28 | Install and restart | Clicked **Install and restart** (while "Scanning…" was still shown) | Download progress shown ("Downloading update... 83886080 / 172998948 bytes"); no administrator prompt appeared (finding 2) | Captured: 16, 17 |
| 20:28:45 | Restart | — | PASS. The app restarted itself (process start time 20:28:45) as "Version 0.0.4"; bundle 0.0.4 with no quarantine attribute; no Gatekeeper block; no banner | Captured: screenshots only; CPU total, process start time, quarantine and Gatekeeper details are operator observations (narrative in timeline.txt, not raw readbacks) |
| 20:29 | Self-test 0.0.4 | Run self-test | PASS. `status: OK`, `version: 0.0.4`, `os: macos`, `arch: aarch64` | Captured: 19 |
| 20:29 | Check for updates | Clicked **Check for updates** | PASS. "vectorAIz is up to date." | Captured: 20 |
| 20:29 | Data kept by update | SHA-256 of every file under `~/vectorAIz` before (0.0.3) and after (0.0.4) | PASS for what the folder held: `models/pdfium/lib/libpdfium.dylib` `db696f0c…b7ae`, identical. No index existed (finding 1) | Captured: `hash-0.0.3-before-update.txt`, `hash-0.0.4-after-update.txt` |
| 20:30 | Uninstall | Quit, ejected the DMG, Finder → Applications → vectorAIz → File → Move to Trash | PASS. App in Trash; `~/vectorAIz` byte-identical | Captured: 21, 22, `hash-after-trash.txt` |

Timeline as written during the run: `timeline.txt`.

## Findings (not fixed in this record)

1. **Index folder stalls on "Scanning…"** (third sighting in a VM, after the 0.0.1 and 0.0.2 runs). This time the process was sampled during the stall (`sample-index-stall.txt`): no thread was inside `pipeline_index` or `vz_core`, only idle tokio and WebKit threads, so the indexing command does not appear to be running at all. The typed path still cannot be read back, because the field text has the same colour as the field background (finding 3 of the 0.0.2→0.0.3 record). Still unconfirmed on real hardware.
2. **No administrator prompt during the in-app update.** The VM user `admin` is an administrator and owns `/Applications/vectorAIz.app`, so macOS replaced the app without asking. The spec's cancel case (cancel the prompt, app stays on 0.0.3 and offers a retry) could not be exercised here; it needs a standard-user account, or Windows UAC on Max's PC.
3. **Install and restart was accepted while "Scanning…" was shown.** Operator observation, with screenshot 16: the update downloaded and installed although an index run appeared to be in progress. Because of finding 1 it is not shown whether a pipeline task was actually running.
4. As in the earlier runs, the administrator-password sheet needed an **OK** click.

## Method notes

Driven over one persistent VNC connection (tart `--vnc-experimental`, vncdotool, daemon under launchd) and checked by OCR. Downloads, drags, Gatekeeper, Settings, the app's buttons and Move to Trash were done through the GUI. `open` from a shell stood in for double-clicks, and the DMG was ejected with `hdiutil detach` from a shell. Partway through the run the VNC connection stopped updating everything outside the top-left 1280×720 of the framebuffer, so the bottom half of Privacy & Security looked unscrollable. Restarting the vncd daemon once (a new connection) fixed it, and tart kept running. Screenshots are full-framebuffer captures scaled to 1024 px.
