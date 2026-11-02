# vectorAIz native

Native desktop app, under construction on branch `native`. The Docker app lives on [`main`](https://github.com/aidotmarket/vectoraiz/tree/main).

[Native desktop design](https://github.com/aidotmarket/runbooks/blob/main/specs/VECTORAIZ-NATIVE-DESKTOP-DESIGN.md).

AI apps use the bundled `vectoraiz-mcp --client <client_id>` stdio bridge. The bridge talks to the desktop app over a local Unix socket on macOS or a named pipe on Windows. It opens no network port and never opens the library itself. Settings → AI apps controls consent, whole-library grants and removal.

The access-control boundary is the current operating-system user account. A program already running as that user can use a stolen client id. Other users are excluded by the socket directory and peer-uid check, or the pipe DACL and server-SID check. Treat the client id as local access material. Removal cancels pending deliveries before it returns. A result already handed to the bridge may still reach the AI app afterward. If saving removal fails, access remains blocked in this running app, but can return after restart unless Retry saves the change.
