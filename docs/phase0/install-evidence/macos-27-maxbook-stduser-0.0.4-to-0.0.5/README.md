# vectorAIz macOS standard-user update test, 0.0.4 → 0.0.5: result (S1823)

**Status: COMPLETE, 25 Oct 2026 10:26–11:56 CET.** Steps 1, 3, 4, 5 and 6 passed. Step 2 did not behave as the prompt expected. Step 3 was first blocked for about 35 minutes by a stale Gatekeeper prompt in Max's own account, a test-machine problem rather than a vectorAIz fault; after that was cleared it passed.

Machine: Max's MacBook (macOS 27.2, 26B5091g, Apple silicon), macOS account "Test account" (`testaccount`, uid 502, Standard). Tester: Claude via computer use from a cloud session linked to the Mac; Max handled password prompts, macOS system dialogs (which computer use cannot see or click) and two text entries. Times are Europe/Madrid (CET, UTC+01:00).
Runbook followed: `vectoraiz-native-test-builds.md` on origin/main, section "By-hand installs on clean client machines" (macOS steps 6–7 and the Windows update/model notes in steps 9–10 as reference).
Evidence: screenshots and a log excerpt in `/Users/Shared/vz-s1823-macos-evidence/` on the MacBook (27 files, written by Test account; run `chmod -R a+r /Users/Shared/vz-s1823-macos-evidence` as Test account if another account cannot read them). Diagnostic outputs `/Users/Shared/vz-diag.txt` and `/Users/Shared/vz-log.txt` and the helper `/Users/Shared/vz-log.sh` were left in place. Nothing was deleted.

## Results

| Step | Result | Key facts |
|---|---|---|
| 1. Account check | PASS | Apple menu: "Log Out Test account…". Users & Groups: Test account = Standard, Max Robbins = Admin. The menu bar has no user name because Fast User Switching is disabled for the whole Mac (its Menu Bar checkbox is greyed out). Not changed. |
| 2. Cannot replace the app | **NOT AS EXPECTED** | Dragging /Applications/vectorAIz to the Trash gave **no administrator prompt**. Finder said: "Are you sure you want to delete "vectorAIz"? This item will be deleted immediately. You can't undo this action." [Cancel] [Delete]. Clicked Cancel; the app stayed. Get Info: 0.0.4, created 23 Oct 2026 19:22, 612,008,626 bytes, owner `max` Read & Write, `admin` and everyone Read only ("You have custom access"). Delete was not clicked, so whether a standard user can actually remove it is unverified. |
| 3. Open 0.0.4 | PASS (after test-machine fix) | First launches (10:30, 10:36, 10:53, 11:02) hung before any app code ran: root cause below. After syspolicyd was restarted (11:03): "vectorAIz" Not Opened / Apple could not verify… [Move to Trash] [Done] → Done → Privacy & Security "vectorAIz" was blocked to protect your Mac → **Open Anyway** (11:04) → Max cleared a follow-up dialog (text not captured; whether it asked for an admin password was not recorded) → app opened 11:05 (pid 2357): "Version 0.0.4 · Native pipeline spike", banner "vectorAIz 0.0.5 is available · Install and restart · Later". Self-test `{"status":"OK","version":"0.0.4","os":"macos","arch":"aarch64"}`. |
| 4. Cancel at the admin prompt | PASS | Install and restart (11:06) → "Downloading update… n / 173188803 bytes" → "Installing update…" → macOS prompt **"Allow administrator access for "vectorAIz"?** This will allow "vectorAIz" to access and modify your data, files, and settings on this Mac. Apple could not verify this app is free of malware… Enter an administrator's name and password to continue with "vectorAIz"." [Allow Access] [**Don't Allow**] (there is no Cancel button on this macOS). Don't Allow → app shows **"The update could not be installed."** plus a second line **"Failed to move the new app into place"**, and *not* "Your current version is unchanged." Quit and reopen (11:09): still 0.0.4, banner back. Repeated 11:11–11:14 to capture the prompt: identical result. |
| 5. Approve | PASS | Install and restart (11:14), Max entered his admin credentials and clicked Allow Access (11:17). The app closed and reopened by itself as **0.0.5** (pid 2665) with **one window**; no Gatekeeper block on the updated app. Self-test (11:38) `{"status":"OK","version":"0.0.5","os":"macos","arch":"aarch64"}`. |
| 6. First-run models, index, search | PASS | First capture after relaunch (11:17) already showed the Models panel in its final state, so the "Checking models…" phase was not observed: both models "Needed" (correct, none present): BAAI/bge-small-en-v1.5 67,308,128 bytes MIT; Qwen2.5-0.5B-Instruct Q4_K_M 491,400,032 bytes Apache-2.0; buttons "Download models (about 560 MB)" and "Import from a folder"; "Models needed: download or import them first." Download started 11:18 ("bge-small-f16.gguf: 4552684 / 67308128 bytes · attempt 1", with Cancel) and finished within about a minute; the panel disappeared; `~/vectorAIz/models` holds bge-small-f16.gguf (67.3 MB) and qwen2.5-0.5b-instruct-q4_k_m.gguf (491.4 MB), no `.partial`. `~/vz-test-docs`: lighthouse.txt (88 B), budget.txt (79 B), railway.txt (78 B), each one distinct sentence. Index folder (11:53) → "3/3 · 3 indexed · 0 failed". Search "orange cats" (11:54, results by ~11:56) → top result "The lighthouse keeper on Saltmarsh Island feeds three orange cats every morning at dawn." (Document 8d3039d3 · Page 1), then the railway and budget sentences. |
| 7. Report | DONE | This doc; screenshots on the MacBook (see Evidence); bus message to Minerva. |

## Root cause of the step 3 hang (verified from the unified log)

- 10:21:30, in **Max's own account** (`gui/501`, pid 1048), vectorAIz was launched. AMFI: "is adhoc signed"; syspolicyd GK scan: `-67018 "Code did not match any currently allowed policy"`; 10:21:37 `GK evaluateScanResult: 0 …` and **`Prompt shown (6, 0), waiting for response`**. That prompt sat in Max's background session (Test account in front, Fast User Switching disabled) and was never answered. Who or what launched it at 10:21 is not recorded.
- Every launch in Test account (pids 1448, 1635, 2034, 2254) was spawned but syspolicyd started no scan for it (only `GK process assessment` at 10:53:35). Each process sat at `_dyld_start + 0` (1 thread, 0.01 s CPU, 112 KB; `sample` 2,565 and 2,586 samples), Dock icon bouncing, no window, no dialog, no block notice in Privacy & Security; a relaunch gave "You can't open the application "vectorAIz" because it is not responding."
- Not enough: killing pid 1048 (`su max -c 'kill 1048'`), killing CoreServicesUIAgent in Max's session (`su max -c 'killall CoreServicesUIAgent'`), and relaunching.
- Fixed by restarting the policy daemon: `su max -c 'sudo killall syspolicyd'` (11:03). The next launch got its own Gatekeeper evaluation in Test account and the normal Not Opened / Open Anyway path.
- `codesign -dv`: `Signature=adhoc`, `TeamIdentifier=not set`; `spctl -a -vv`: `rejected`; bundle and executable carry `com.apple.quarantine` `0181;6addc5a5;Chrome;…`.

## Timed log

- 10:26 Task received. Runbook index fetched on Koskadeux; page `vectoraiz-native-test-builds.md`.
- 10:29 Computer use on; Users & Groups shows Test account Standard. Step 1 PASS.
- 10:29–10:30 Step 2: drag to Trash (two drags did not register; third did) → delete-immediately dialog → Cancel. Get Info: owner max, 0.0.4.
- 10:30:35 Step 3 launch → pid 1448 hangs. 10:31 no block in Privacy & Security.
- 10:33 Asked Max. 10:34 no dialog on screen; Max's photo shows two "not responding" alerts from my relaunches.
- 10:35–10:39 Activity Monitor sample of 1448 and of relaunched 1635 (10:36:47): `_dyld_start + 0`.
- 10:42 `/Users/Shared/vz-diag.txt`: ps shows pid 1048 started 10:21:30 as well; adhoc signature; spctl rejected; `su` for log failed (prompt went to file).
- 10:47–10:50 `log show` blocked by zsh's built-in `log`, then by script permissions; 10:50 log captured (11,639 lines) → 10:21 prompt in uid 501.
- 10:52 Max killed 1048. 10:53 force-quit 1635, relaunched → 2034 hangs; log shows no scan.
- 10:58 Fast User Switching cannot be enabled (greyed out). 11:00 `killall CoreServicesUIAgent` (Max) → no change. 11:01 2034 killed; 11:02 relaunch 2254 hangs.
- 11:03 Max: `kill -9 2254; su max -c 'sudo killall syspolicyd'` → "vectorAIz" Not Opened dialog; Max clicked Done.
- 11:04 Privacy & Security → Open Anyway. 11:05 Max cleared a follow-up dialog; app open as 0.0.4 with 0.0.5 banner; self-test OK. Step 3 PASS.
- 11:06 Install and restart; 11:07 download complete (173,188,803 bytes), "Installing update…".
- 11:08 Max clicked Don't Allow → "The update could not be installed. / Failed to move the new app into place".
- 11:09 Quit, reopen: 0.0.4, banner. Step 4 PASS.
- 11:11 Repeat: download restarted from 0 bytes. 11:13 prompt captured (Max), Don't Allow, same message. 11:14 reopen: 0.0.4, banner.
- 11:14 Install and restart; push notification to Max. 11:17 Allow Access → relaunch pid 2665, 0.0.5, one window. Step 5 PASS.
- 11:17 Models panel: both Needed, Download/Import offered.
- 11:18 Download models (third click registered) → started; done by 11:19 (files dated 11:18).
- 11:18–11:21 TextEdit: three plain-text files saved to Documents (Save dialog controls not clickable by computer use, so saved by name), then moved in Finder to new `~/vz-test-docs`; two doubled `.txt.txt` names fixed in Finder.
- 11:22–11:38 Computer-use typing never reached the app's Folder or Question fields (clicks did reach buttons); 11:38 self-test 0.0.5 OK.
- 11:53 Max typed the folder path and "orange cats"; he also clicked Ask once before indexing → "index recipe missing; re-index required: No such file or directory (os error 2)".
- 11:53 Index folder → "Scanning…" → 11:54 "3/3 · 3 indexed · 0 failed".
- 11:54 Search → "Running…" ~60–90 s → lighthouse sentence first. Step 6 PASS.

## Unexpected findings

1. **Stale Gatekeeper prompt in another account blocks every launch of the same app (macOS 27.2).** With an unanswered "Prompt shown, waiting for response" for the bundle in another user's session, launches in the current account hang at `_dyld_start` with no UI. Only restarting syspolicyd cleared it. Test-machine hygiene: make sure no other logged-in account has a pending vectorAIz prompt (or log other accounts out) before a standard-user run.
2. **Step 2 expectation does not hold for this install.** No admin prompt on drag-to-Trash; Finder offers "delete immediately". The bundle is owned by `max`, read-only for others. Whether Delete would succeed or prompt is untested.
3. **Admin prompt is "Allow administrator access for "vectorAIz"?" with Allow Access / Don't Allow**, and it repeats "Apple could not verify this app is free of malware". Test scripts that say "click Cancel" should say "Don't Allow".
4. **Decline message shows a second, technical line** "Failed to move the new app into place" under "The update could not be installed." The forbidden "Your current version is unchanged." did not appear.
5. **Each Install and restart downloads the 173 MB update again from zero**; a declined attempt keeps nothing.
6. **"Checking models…" not observed on macOS**: the first capture, seconds after relaunch, already showed the final "Needed" state (correct, no models present). The Windows ~20 s false "Models needed" quirk could not be checked here because models really were missing.
7. **First search after indexing took about 60–90 s** ("Running…" on the self-test button, all buttons disabled). Later searches not timed.
8. **Ask before any index shows a raw error**: "index recipe missing; re-index required: No such file or directory (os error 2)".
9. **Search results show "Document <id> · Page 1", not file names**; the top hit was identified by its sentence.
10. **Tooling limits (not product bugs):** computer use cannot see or click macOS system dialogs (Gatekeeper, admin prompt, Save/Open panels) and could not type into vectorAIz's web text fields; full-screen clicks into vectorAIz were often ignored while background (accessibility) clicks worked. Max did the dialogs and two text entries.

## Machine state left behind

vectorAIz 0.0.5 installed in /Applications and running in Test account; models in `~/vectorAIz/models`; index of `~/vz-test-docs`; Documents empty again; files in `/Users/Shared` listed above. syspolicyd was restarted by Max; hung vectorAIz processes 1048, 1448, 1635, 2034, 2254 were ended. No settings changed.
