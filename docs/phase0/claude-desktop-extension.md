# Claude Desktop extension release notes

Connect Claude Desktop now creates a desktop extension file. Install it in Claude to let Claude search and read your vectorAIz library on this computer. vectorAIz asks for permission before connecting, and you can remove access in Settings.

On Windows, Connect reveals the extension file in File Explorer. Drag this file onto Claude Desktop → Settings → Desktop app → Extensions, then select Install, and Install again in the box that follows. Or use Advanced settings → Install Extension and choose this file.

On macOS, Connect opens Claude Desktop’s install screen. Select Install, then Install again in the box that follows. If it does not open, drag the file onto the Extensions page or use Advanced settings → Install Extension. **Show the file** reveals the generated file.

The extension is unsigned. Claude displays: “Any developer information shown has not been verified by Anthropic.”

Testers upgrading from 0.0.16 or 0.0.17 should delete any old `vectoraiz` entry from their Claude config file by hand. vectorAIz no longer edits that file.

Removing access in vectorAIz stops new library requests. To remove the extension too, uninstall vectorAIz under Settings → Desktop app → Extensions in Claude. Also uninstall that extension when uninstalling vectorAIz itself.

After moving or reinstalling vectorAIz, reconnect. In Claude Desktop, uninstall vectorAIz under Settings → Desktop app → Extensions, then install the generated file. Connect reveals the file on both platforms after a move. A second Connect compares against the newly written bundle, so the moved-app reminder appears only on the first Connect even if Claude still runs the old bridge.

After updating, every connected app, including **Connect another app** clients, asks you to confirm the updated terms once.

Other apps still use **Connect another app**. If your organisation disables Claude extensions, the config-file route has not been proven with the Microsoft Store build of Claude.

The local account is the trust boundary: another process running as the same user can read the client file. On Unix the folder is private to the user and the file has mode 0600; on Windows they inherit the user profile ACL. Removing access revokes the client regardless of who has read its id.
