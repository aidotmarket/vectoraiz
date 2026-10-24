# macOS stage 1 test builds

Apple Silicon, macOS 13 or later. These builds use an ad-hoc signature and are for testers.

1. Open the DMG and drag vectorAIz into Applications. Eject the DMG.
2. Open vectorAIz. On macOS 15 (the sequence verified so far), macOS says it was not opened; choose **Done**. Open System Settings → Privacy & Security, scroll to Security and choose **Open Anyway**, choose **Open Anyway** again in the dialog, then enter an administrator user name and password. Only do this for a build supplied by the project.
3. Click **Run self-test**. The result should say `OK` and show the installed version.
4. After each update the changed ad-hoc signature can trigger a keychain prompt. Choose **Always Allow** when vectorAIz asks to read its stored secrets.

Collections, indexes and settings use `~/vectorAIz` (the Phase 0 index is in `phase0-index`). To uninstall, quit the app and move `/Applications/vectorAIz.app` to Trash. Uninstall preserves `~/vectorAIz`; remove that folder yourself only if you want to delete the data.

At launch, vectorAIz checks for updates. You can also choose **Check for updates**. When the banner says “vectorAIz <version> is available”, choose **Install and restart** to download, verify the signature, replace the app and restart on that version. Nothing downloads or installs until you click it. **Later** hides the banner until the next launch. Installation is disabled while indexing, search or chat is running. If Applications is not writable, macOS asks for an administrator password. After restarting, run the self-test; keep choosing **Always Allow** for the keychain prompt described above.

If checking fails, try **Check for updates** later. If installation fails, the banner shows the error and the app keeps running. A download or signature failure leaves the current app unchanged; an installation error may mean the previous app could not be restored, so follow the error details. As a manual fallback, download the newer DMG from the test-build releases and replace the app in Applications. Updates and replacing the app do not touch `~/vectorAIz`. By-hand macOS results: `docs/phase0/install-evidence/`.

On first run, choose **Download models (about 560 MB)** in the **Models** panel before indexing, searching or asking. The two provisional models total 558,708,160 bytes (about 560 MB); the panel lists each name, size, licence and source. Downloads start only after your click, and size and SHA-256 must match before a model is installed. **Cancel** or quitting keeps the partial download; choose **Retry** or Download again to resume. Allow enough free space for the remaining download.

For an offline or restricted network, an administrator can use `python scripts/fetch-assets.py --out <folder>` from a vectorAIz checkout on a connected machine. This also fetches PDFium; copy only `bge-small-f16.gguf` and `qwen2.5-0.5b-instruct-q4_k_m.gguf` into a local folder. Choose **Import from a folder**, type its absolute path (for example `/Volumes/Models`), then **Import models**. Import accepts regular files directly in that folder, checks their size and SHA-256, and reports imported, missing or mismatched files. Links and Windows reparse points (including OneDrive placeholders) are refused; copy the files into an ordinary local folder first. Allow space for the staged copy.

Models live in `~/vectorAIz/models`. Updates and uninstall preserve them. After verification, choose your document Folder to index; an empty field or missing folder is refused without changing an existing index. Opening vectorAIz again restores and focuses its existing window.
