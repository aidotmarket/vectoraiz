# Claude Desktop extension release notes

Connect Claude Desktop now creates a desktop extension file. Install it in Claude to let Claude search and read your vectorAIz library on this computer. vectorAIz asks for permission before connecting, and you can remove access in Settings.

Claude Desktop shows an install screen. Select Install, then Install again in the box that follows. If the file does not open in Claude, open Claude Desktop → Settings → Desktop app → Extensions and drag the file onto the page, or use Advanced settings → Install Extension. **Show the file** in vectorAIz reveals the generated file.

The extension is unsigned. Claude displays: “Any developer information shown has not been verified by Anthropic.”

Testers upgrading from 0.0.16 or 0.0.17 should delete any old `vectoraiz` entry from their Claude config file by hand. vectorAIz no longer edits that file.

Removing access in vectorAIz stops new library requests. To remove the extension too, uninstall vectorAIz under Settings → Desktop app → Extensions in Claude. Also uninstall that extension when uninstalling vectorAIz itself.

Reconnect after moving or reinstalling vectorAIz, then install the generated file in Claude again. If Claude keeps the old extension, uninstall it in Claude first and install the file again.

After updating, every connected app, including **Connect another app** clients, asks you to confirm the updated terms once.

Other apps still use **Connect another app**. If your organisation disables Claude extensions, the config-file route has not been proven with the Microsoft Store build of Claude.

The local account is the trust boundary: another process running as the same user can read the client file. On Unix the folder is private to the user and the file has mode 0600; on Windows they inherit the user profile ACL. Removing access revokes the client regardless of who has read its id.
