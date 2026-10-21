# Phase 0 parsers

The fixture is synthetic CC0 content. It exercises TXT, Markdown, HTML, DOCX,
XLSX and text-layer PDF. PDFium is fetched by `scripts/fetch-assets.py`, never
committed. The version is **chromium/8111**:

| Archive | SHA256 |
|---|---|
| pdfium-mac-arm64.tgz | 46c674298c60123f1195c2a03c65ee08c4783c84874ce4f5b3a054abb3ecaf30 |
| pdfium-win-x64.tgz | 3452d11031050ec65e57f180215b57b9e6d465e3fbedafa8a77ad90885c7d23e |

PDFium runs through pdfium-render 0.8.35 inside a one-document worker. Office
containers undergo both central-directory checks and a bounded inflation pass
before XML/calamine see them. Limits are 512 MiB total, 256 MiB per member,
10,000 members, and 100:1 per-member expansion. Refusals become failed SQLite
jobs with a reason; subsequent files continue. Non-text PDFs use the compiled
Apple Vision helper on macOS; Windows records a needs-OCR failure if no page has
text. In mixed PDFs text pages remain indexable and blank pages retain their
physical page numbers. XLSX page numbers mean sheet ordinal; flowing formats
use page 1. DOCX paragraphs and spreadsheet rows survive extraction; DOCX style
headings and pagination are not reconstructed in this spike.

Workers receive a path and an explicit local PDFium path using little-endian
u32-length-prefixed JSON. Frames are capped at 32 MiB. Environment inheritance
is cleared (Windows preserves only SystemRoot). The supervisor enforces a 1.5 GiB resident-memory budget on macOS by polling
proc_pidinfo every 10 ms and killing an over-budget process group. macOS
rejects finite RLIMIT_AS/DATA values on the development host; the watchdog is
explicitly a sampled budget rather than an allocation-time kernel boundary.
Other Unix systems use an address-space limit. Windows uses a memory-limited
Job Object. Timeouts
are 60 seconds per file. macOS launches under sandbox-exec with network denied.
This profile allows filesystem access: it is not production document isolation.
Windows network isolation is explicitly **not implemented**, as required by
Phase 0 section 7; AppContainer or equivalent belongs to Phase 1.

Build the macOS OCR helper beside the worker:

```sh
swiftc scripts/vision-ocr.swift -O -o target/debug/vz-vision-ocr
```

The Phase 1 format list remains the design list: PDF; DOCX, PPTX and ODF via
ZIP/XML; XLSX, XLS and ODS via calamine; RTF, HTML, Markdown, TXT, CSV, JSON,
EML/MSG and EPUB. Legacy DOC/PPT must be skipped with a reason. This spike does
not establish a need to replace PDFium or calamine, and makes no claim that the
remaining formats are implemented. Further corpus work must assess DOCX style
fidelity and HTML structure, scanned-PDF quality and languages, malformed
containers, oversized pages, and Windows OCR after Store packaging.
