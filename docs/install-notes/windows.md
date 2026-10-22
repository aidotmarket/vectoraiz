# Windows stage 1 test builds

Windows 10 22H2 or Windows 11, x64. The unsigned MSI installs for all users and embeds the offline WebView2 installer. Installation, updates and uninstall require administrator approval (UAC).

1. On Windows 11, open Windows Security → App & browser control → **Smart App Control settings**. Use a dedicated test PC or VM where the state is **Off**. Smart App Control has no per-app exception for unsigned apps; turning it off can require reinstalling Windows to enable it again. Windows 10 has no Smart App Control.
2. Open the MSI. At “Windows protected your PC”, choose **More info → Run anyway**, then approve UAC. Only do this for a build supplied by the project.
3. Start vectorAIz and click **Run self-test**. The result should say `OK` and show the installed version.
4. If a later build prompts for keychain/credential access after an update, choose **Always Allow** when offered for vectorAIz.

Data lives in `%USERPROFILE%\vectorAIz`, with the Phase 0 index in `phase0-index`. Uninstall from Settings → Apps → Installed apps → vectorAIz → Uninstall and approve UAC. Uninstall keeps the data folder. Delete it separately only if you want to erase the data.

The updater is configured with the rotated signing public key and the test-build release endpoint. No real updater release is published by chunk D. CI uses fresh `windows-2022` Windows Server runners as stand-ins, not Windows 10/11 client evidence. Client VM checks and screenshots follow Gate 3.
