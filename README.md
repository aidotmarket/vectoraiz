# vectorAIz native

Native desktop app, under construction on branch `native`. The Docker app lives on [`main`](https://github.com/aidotmarket/vectoraiz/tree/main).

[Native desktop design](https://github.com/aidotmarket/runbooks/blob/main/specs/VECTORAIZ-NATIVE-DESKTOP-DESIGN.md).

AI apps use the bundled `vectoraiz-mcp --client <client_id>` stdio bridge. The bridge talks to the desktop app over a local Unix socket on macOS or a named pipe on Windows. It opens no network port and never opens the library itself. Settings → AI apps controls consent, whole-library grants and removal.

The access-control boundary is the current operating-system user account. A program already running as that user can use a stolen client id. Other users are excluded by the socket directory and peer-uid check, or the pipe DACL and server-SID check. Treat the client id as local access material. Removal cancels pending deliveries before it returns. A result already handed to the bridge may still reach the AI app afterward. If saving removal fails, access remains blocked in this running app, but can return after restart unless Retry saves the change.

Vector pack V2 accepts TXT/MD, DOCX/XLSX/PPTX, PDF and HTML/HTM through a
source-only sale projection worker. Buyers receive exactly the six generated
vector-pack files, with `originals:false`; source bytes and private omission
records stay local. Fresh embeddings use reviewed projection text, never search
index text. Hidden content is omitted by default and can be included by the
seller; mandatory exclusions cannot be included. The review lists per-document
formats, omission counts, hidden locations and named refusals.

PDF uses PDFium character text/render-mode and page geometry inspection without
rendering or OCR. Invisible (mode 3) and off-page text remain flagged content,
editable/excludable by the seller. No-text pages are counted omissions; all-no-text
documents refuse with `PROJECTION_PDF_NEEDS_OCR`. Properties, annotations,
attachments, actions/JavaScript and form state never become sale text. Layered
PDFs refuse with `PROJECTION_PDF_OPTIONAL_CONTENT_STATE_UNPROVEN`; the pinned API
cannot prove their default visibility. Classic, incremental, object-stream and
xref-stream PDFs proceed after bounded structural and nested Form validation.
Image XObjects and soft masks are counted at raw length without decoding.
Decoded text-path streams support raw, ASCII85 and Flate (including single-filter
arrays and supported PNG predictors); other filters/parameters refuse with
`PROJECTION_PDF_STREAM_UNSUPPORTED`. Inherited page boxes are resolved; undefined
references are null. Unmapped glyphs are counted and replaced with U+FFFD.
Pages using ActualText carry a conservative not-visible replacement-text flag.
Non-whitespace C0 controls become counted replacements; PDFium’s U+0002 line-end
hyphen marker is removed to join words. Inline images refuse by name.
Encrypted, malformed and unclassifiable PDFs refuse without a search fallback.

HTML parses inert local bytes through pinned scraper/html5ever. UTF-8, declared
common encodings (including Windows-1252 and ISO-8859 variants), and UTF-16 BOMs
are supported; unknown/invalid encodings refuse by name. Only body, heading,
table text and supported labels are projected, joining inline text within blocks.
Metadata, active markup, comments, attributes and form state are omitted with
counts. Visible buttons retain ordinary text children only; input, textarea,
select and output values are mandatory drops. SVG/MathML subtrees are counted
drops, including inside hidden content. Hidden attributes, inline `display:none`,
inline `visibility:hidden`, template contents and browser fallback/inactive
content are inherited visibility signals. Datalist, closed dialog, canvas,
video/audio/progress/meter fallback, noembed, noframes and rp are hidden and
seller-includable. Meta UTF-16 labels select UTF-8; BOMs remain authoritative.
Stylesheets, classes and CSS layout are not evaluated; style carriers produce a
per-document warning. No resource is fetched.

Projection uses ingestion limits: 256,000,000 bytes per part/input PDF or HTML,
512,000,000 per document, 10,000 parts, 5,000,000 inventory entries, and 100:1
document decoded-PDF-stream/archive expansion. HTML depth is bounded to 256; PDF direct-object depth
to 8. Pack limits remain 2 GiB expanded, 20,000,000 entries and 100,000 parts.
Each worker has a 120-second deadline; extraction and generated readback share
a cumulative and wall-clock 1,200-second pack deadline. Cancellation kills the
worker group and prevents promotion.

Spreadsheet projection uses saved spreadsheet caches without recalculation.
Numeric date cells retain Excel serial numbers; cell number formats are not applied.
Worksheet header/footer text is dropped and counted privately. Paragraphs are
editable units; visibility changes within a paragraph retain separate segments.
VML and AlternateContent text boxes refuse with `PROJECTION_WORD_TEXTBOX_UNSUPPORTED`
until branch selection and visibility can be proven.

Projection and readback workers are limited to 1 GiB, except PDF projection
workers, which retain the ingestion worker cap of 1,536 MiB. Each delivered artifact is
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
