# Windows stage 1 test builds

Windows 10 22H2 or Windows 11, x64. The unsigned MSI installs for all users and embeds the offline WebView2 installer. Installation, updates and uninstall require administrator approval (UAC).

1. On Windows 11, open Windows Security → App & browser control → **Smart App Control settings**. Use a dedicated test PC or VM where the state is **Off**. Smart App Control has no per-app exception for unsigned apps; turning it off can require reinstalling Windows to enable it again. Windows 10 has no Smart App Control.
2. Open the MSI. At “Windows protected your PC”, choose **More info → Run anyway**, then approve UAC. Only do this for a build supplied by the project.
3. Start vectorAIz and click **Run self-test**. The result should say `OK` and show the installed version.
4. As a manual update fallback, close every vectorAIz window, then open the newer MSI; it replaces the installed version. If double-clicking the MSI does nothing, run it from PowerShell: `msiexec /i "$env:USERPROFILE\Downloads\<file>.msi"`.
5. If a later build prompts for keychain/credential access after an update, choose **Always Allow** when offered for vectorAIz.

At launch, vectorAIz checks for updates. You can also choose **Check for updates**. When the banner says “vectorAIz <version> is available”, choose **Install and restart** to download and verify the signed update. Nothing downloads or installs until you click it. **Later** hides the banner until the next launch. Installation is disabled while indexing, search or chat is running.

The app closes as soon as the installer starts. Windows asks for administrator approval (a consent prompt for an administrator account, or administrator credentials for a standard account), and the installer shows a progress window. After approval and a successful install, it reopens vectorAIz; run the self-test to check the installed version. If you cancel approval or installation fails, the previous version stays installed through Windows Installer rollback. If the app does not come back, reopen vectorAIz from the Start menu. The closed app cannot show an error after the installer has started. Use the manual MSI fallback in step 4 if needed.

A failure before the installer starts appears in the banner and the app keeps running; download or signature failures leave the installed app unchanged. Updates do not touch the data folder.

Data lives in `%USERPROFILE%\vectorAIz`, with the Phase 0 index in `phase0-index`. Uninstall from Settings → Apps → Installed apps → vectorAIz → Uninstall and approve UAC. Uninstall keeps the data folder. Delete it separately only if you want to erase the data.

The updater is configured with the rotated signing public key and the test-build release endpoint. No real updater release is published by chunk D. CI uses fresh `windows-2022` Windows Server runners as stand-ins, not Windows 10/11 client evidence. Client VM checks and screenshots follow Gate 3.

On first run, choose **Download models (about 560 MB)** in the **Models** panel before indexing, searching or asking. The two provisional models total 558,708,160 bytes (about 560 MB); the panel lists each name, size, licence and source. Downloads start only after your click, and size and SHA-256 must match before a model is installed. **Cancel** or quitting keeps the partial download; choose **Retry** or Download again to resume. Allow enough free space for the remaining download.

For an offline or restricted network, an administrator can use `python scripts/fetch-assets.py --out <folder>` from a vectorAIz checkout on a connected machine. This also fetches PDFium; copy only `bge-small-f16.gguf` and `qwen2.5-0.5b-instruct-q4_k_m.gguf` into a local folder. Choose **Import from a folder**, type its absolute path (for example `C:\Models`), then **Import models**. Import accepts regular files directly in that folder, checks their size and SHA-256, and reports imported, missing or mismatched files. Links and Windows reparse points (including OneDrive placeholders) are refused; copy the files into an ordinary local folder first. Allow space for the staged copy.

Models live in `%USERPROFILE%\vectorAIz\models`. Updates and uninstall preserve them. After verification, choose your document Folder to index; an empty field or missing folder is refused without changing an existing index. Opening vectorAIz again restores and focuses its existing window.
