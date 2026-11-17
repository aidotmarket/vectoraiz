# vectorAIz native

Native desktop app, under construction on branch `native`. The Docker app lives on [`main`](https://github.com/aidotmarket/vectoraiz/tree/main).

[Native desktop design](https://github.com/aidotmarket/runbooks/blob/main/specs/VECTORAIZ-NATIVE-DESKTOP-DESIGN.md).

AI apps use the bundled `vectoraiz-mcp --client <client_id>` stdio bridge. The bridge talks to the desktop app over a local Unix socket on macOS or a named pipe on Windows. It opens no network port and never opens the library itself. Settings → AI apps controls consent, whole-library grants and removal.

The access-control boundary is the current operating-system user account. A program already running as that user can use a stolen client id. Other users are excluded by the socket directory and peer-uid check, or the pipe DACL and server-SID check. Treat the client id as local access material. Removal cancels pending deliveries before it returns. A result already handed to the bridge may still reach the AI app afterward. If saving removal fails, access remains blocked in this running app, but can return after restart unless Retry saves the change.

Vector pack V1 projection uses saved spreadsheet caches without recalculation.
Numeric date cells retain Excel serial numbers; cell number formats are not applied.
Worksheet header/footer text is dropped and counted privately. Paragraphs are
editable units; visibility changes within a paragraph retain separate segments.
VML and AlternateContent text boxes refuse with `PROJECTION_WORD_TEXTBOX_UNSUPPORTED`
until branch selection and visibility can be proven.

Projection and readback workers are limited to 1 GiB. Each delivered artifact is
currently read whole, so an artifact near the cap can refuse with
`PROJECTION_MEMORY_LIMIT_EXCEEDED`; the payload byte limit is not a promise that
an artifact of that size can be reviewed. Capacity and installed Office rendering
remain unqualified. This fold does not raise those limits.

Known projection limits: `x14:dataValidations` and other content-bearing worksheet
extensions refuse with `PROJECTION_CONTENT_UNSUPPORTED`. Drawings crossing hidden
rows or columns on their own sheet refuse with `PROJECTION_VISIBILITY_UNKNOWN`.
Fields without proven results (except instruction-only XE fields) refuse with
`PROJECTION_FIELD_RESULT_UNPROVEN`. Static layout/master text refuses with
`PROJECTION_LAYOUT_UNSUPPORTED`; slide AlternateContent and text-bearing extensions
also refuse. These cases remain outside qualification.
Crafted XE simple fields with child runs still expose those child runs as visible;
identity terms split across visibility segments are matched separately. Spreadsheet
drawing paragraphs currently use the `slide paragraph` inventory label.
