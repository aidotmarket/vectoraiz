# macOS stage 1 test builds

Apple Silicon, macOS 13 or later. These builds use an ad-hoc signature and are for testers.

1. Open the DMG and drag vectorAIz into Applications. Eject the DMG.
2. Open vectorAIz. If macOS blocks it, open System Settings → Privacy & Security, choose **Open Anyway**, then confirm **Open**. Only do this for a build supplied by the project.
3. Click **Run self-test**. The result should say `OK` and show the installed version.
4. After each update the changed ad-hoc signature can trigger a keychain prompt. Choose **Always Allow** when vectorAIz asks to read its stored secrets.

Collections, indexes and settings use `~/vectorAIz` (the Phase 0 index is in `phase0-index`). To uninstall, quit the app and move `/Applications/vectorAIz.app` to Trash. Uninstall preserves `~/vectorAIz`; remove that folder yourself only if you want to delete the data.

The updater is configured with the rotated signing public key and the test-build release endpoint. No real updater release is published by chunk D. Clean macOS VM tests and screenshots follow Gate 3; hosted `macos-14` install checks are stand-ins.
