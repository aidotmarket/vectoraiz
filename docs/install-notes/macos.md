# macOS stage 1 test builds

Apple Silicon, macOS 13 or later. These builds use an ad-hoc signature and are for testers.

1. Open the DMG and drag vectorAIz into Applications. Eject the DMG.
2. Open vectorAIz. macOS says it was not opened; choose **Done**. Open System Settings → Privacy & Security, scroll to Security and choose **Open Anyway**, choose **Open Anyway** again in the dialog, then enter an administrator user name and password. Only do this for a build supplied by the project.
3. Click **Run self-test**. The result should say `OK` and show the installed version.
4. After each update the changed ad-hoc signature can trigger a keychain prompt. Choose **Always Allow** when vectorAIz asks to read its stored secrets.

Collections, indexes and settings use `~/vectorAIz` (the Phase 0 index is in `phase0-index`). To uninstall, quit the app and move `/Applications/vectorAIz.app` to Trash. Uninstall preserves `~/vectorAIz`; remove that folder yourself only if you want to delete the data.

To update a stage 1 build, download the newer DMG from the test-build releases and replace the app in Applications. Removing the app does not touch `~/vectorAIz`. The app's signed updater reads the test-build release feed, but stage 1 has no in-app update control yet. By-hand macOS results: `docs/phase0/install-evidence/`.
