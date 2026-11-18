# Sale projection V2 implementation contract

The default listing path produces the unchanged six generated artifacts and
`originals:false`. Source admission does not change `sanitize.rs` delivered-byte
inspection. Source projection, frozen Office reconstruction, generated-artifact
readback and search remain distinct worker jobs. V2 results require projection
version 2; an older worker refuses capability matching rather than falling back.

Libraries are pinned in Cargo.lock: pdfium-render 0.8.35 (MIT), PDFium
chromium/8111 (BSD and bundled third-party notices), scraper 0.24.0 (ISC) with
html5ever 0.35.0 (MIT/Apache-2.0), cssparser 0.35.0 (MPL-2.0), lopdf 0.38.0
(MIT), and flate2 1.1.10 (MIT/Apache-2.0). cssparser is already transitive;
making it direct permits bounded declaration tokenization, including comments,
CSS escapes and `!important`, without interpreting CSS layout.

The lopdf dependency is used only for the metadata guard and source carrier
counts. It never supplies sale text. Classic xref tables, object streams, xref
streams and incremental updates proceed when the final merged graph validates.
Text comes exclusively from PDFium's final revision; Info, XMP, annotations,
actions and other metadata from every revision are never delivered. Replacing
or removing a carrier in a later revision does not turn its old strings into
projected page text.

An escaped-name preflight rejects encryption and optional content. Before lopdf
loads xref streams, the guard follows actual final/previous/hybrid xref offsets
and bounds their decoding. The synchronous object callback bounds object streams
before expansion; failures are stored outside the input graph. An in-band
`VZGuardFailure` key has no authority. Every live xref object must be present;
xref/object number or generation disagreement refuses as
`PROJECTION_PDF_XREF_OBJECT_MISMATCH`; other unparseable live objects refuse as `PROJECTION_PDF_OBJECT_UNREADABLE` (invalid
offset or unsupported object syntax), and compressed xref entries must match
object-stream header indices. Undefined references are null per ISO 32000.
Required structure still has to validate. Page trees and inherited page boxes
are bounded and must agree with PDFium's page count.

Page content and recursively invoked Form content pass the same syntax/sentinel
and text-object checks, with a depth-eight, cycle-safe traversal. Form recovery
cannot silently discard text beside a surviving valid page. Only text-path
content/resources, fonts/CMaps and structural streams undergo decode accounting.
Image XObjects and soft masks are counted at raw length and never decoded by
the guard, irrespective of filter/predictor. Other unused streams are raw counts.
Raw/ASCII85/Flate streams (single-filter arrays included) are bounded; Flate
supports PNG predictors 10–15 or identity, with bounded row geometry. Unsupported
filters or predictors on decoded streams refuse by name. The 100:1 ratio applies
to the aggregate decoded/encoded lengths of the streams actually decoded, with
256,000,000 bytes per stream and 512,000,000 bytes per document retained.
The guard follows xref offsets instead of searching for zlib-like bytes in text.
Direct dictionary nesting is limited to eight; worker memory/CPU bounds remain.

The pinned PDFium API exposes character render modes, character rectangles and
page boxes. It does not expose a proof of the document's optional-content default
configuration. All layered PDFs therefore refuse with
`PROJECTION_PDF_OPTIONAL_CONTENT_STATE_UNPROVEN`, including escaped PDF name
spellings. No object-active flag is used as a default-state oracle. Such PDFs
cannot be included through the hidden-content toggle. Unlayered PDFs proceed;
mode-3 and off-page text are flagged ordinary content and can be edited/excluded.
Unmapped glyphs become U+FFFD with a mandatory-drop count at the character location.
ActualText in parsed BDC/DP property operands (including escaped names and
named resource properties) conservatively flags text on the affected pages as potentially not-visible replacement text; PDFium still supplies
the text, and the seller may edit/exclude it. Annotation counts count array items
rather than dictionary keys. Blank/image-only pages are counted omissions, while an entirely textless document
refuses. There is no render/OCR fallback in this module.

HTML uses an inert HTML5 tree with a 256-level traversal bound. A BOM wins over
a declared meta charset; UTF-8 is the default, and encoding_rs handles common
Windows/ISO encodings and UTF-16 BOMs. Unknown labels and invalid bytes refuse as
`PROJECTION_HTML_ENCODING_UNSUPPORTED`. Text nodes share a flow within a block,
preserving paragraph boundaries in captured embedding input. No attributes are
projected. Form state is a mandatory counted drop: input/textarea/select/output
subtrees disappear; visible buttons keep ordinary child text only. Hidden controls
remain mandatory drops regardless of seller inclusion. SVG/MathML subtrees are
counted mandatory drops even within hidden trees. Datalist, closed dialog,
canvas/video/audio fallback, noembed, noframes and rp are inherited hidden content,
omitted by default and seller-includable. Stylesheets/classes/layout are not
evaluated; the document warning and generated README state that limitation.

Both candidate and build paths invoke the confined projection worker. PDF jobs
retain the existing 1,536 MiB cap; other projection/readback jobs retain 1 GiB.
The supervisor uses the selected limit in watchdog and allocation-denial
refusals. Document timeout is 120 seconds; pack time remains cumulatively and
wall-clock bounded to 1,200 seconds. Incomplete staging is removed on errors.

CC0 fixture provenance is in `spike/fixtures/SOURCES.md` and
`crates/vz-core/tests/fixtures/LICENSE-projection-v2`. Tests cover mandatory
carriers, hidden categories, CSS tokenization, PDF render-mode/page classification,
no-text-page counts, named refusals, poisoned index text/vectors, captured fresh
embedding inputs, generation invalidation, payload allowlisting and seller UI.
These canaries do not qualify corpus capacity or installed delivery journeys.
Full command/corpus evidence belongs outside the checkout.

Real corpus regressions include a Wikipedia page and two NASA NTRS PDFs;
both NASA PDFs now pass the guard and text extraction. Object/xref streams and
incremental revisions use locally generated canary fixtures. Source URLs, exact
fixture hashes, licences and the HTML re-recording note accompany the fixtures.
`projection-pdf-render-trap.py` builds forwarding PDFium libraries on macOS and
Windows, verifies an exit-on-render positive control, then runs PDF regressions.
The worker PDF network trap exercises URI actions and remote file references.
Image-codec independence tests invoke invalid image payloads with six filters,
require surviving text and exact omitted-page counts, and classify OCR layers.
These tests and corpus counts are evidence of tested coverage, not capacity or
installed-journey qualification.


Fold 2 decoding rules: only streams reached as image XObjects and their image
masks avoid decoded accounting; a content, Form, font or CMap stream cannot opt
out with an Image label or a resource name. Each page/Form decompression requires
a bounded-decode receipt; absolute per-stream and document caps and the aggregate
100:1 expansion bound apply before content parsing or PDFium loading. Form
validation is memoized across the document by object and effective resource
context (including page fallback and inherited font state), retaining depth checks. All accounted page content, resource-reachable Forms
(including unused Forms), and Type3 CharProcs receive the same content syntax
validation, independently of invocation reachability. Page Contents arrays are
joined with a space before syntax validation, matching PDFium's stream joining;
each entry also receives the inline-image scan. Type3 width operators d0/d1
are recognized during syntax validation. Subtypes accept resolved
names or strings. Resource lookup uses the Form resources, or the invoking
parent resources when the Form has none, then the page category dictionary
when that category is absent, for Do, BDC and DP. Type3 glyphs use their own
resources ahead of the font's resources; a Form inside a glyph falls back to
the font's resources. A second resource-graph bound counts
all Do operators, including unresolved names. It retains the stream's, invoking
streams', font's and page's candidate resource dictionaries, and lets text
reach every document Type3 glyph unless an explicit Type1 selection proves
otherwise in every candidate dictionary. Local XObject category shadowing and
final Do operands prune impossible edges; unknown names keep the unrestricted
candidate set. It uses the largest possible child cost down each chain. Branching cycles refuse as `PROJECTION_PDF_FORM_GRAPH_UNBOUNDED`;
single-call cycles use the pinned PDFium Form and Type3 recursion limits.
Lexical stream costs and non-cyclic stream/resource bounds are cached
document-wide. A stream revisited with a larger scope must be explored before
calling it a cycle. This bound can conservatively refuse unproven routes;
the pinned-manifest corpus comparison is the gate, as detailed below.

Each Form invocation charges its full descendant invocation count and decoded
bytes once per invocation, including memo hits, against document budgets of 100,000 invocations and
512,000,000 expanded bytes. Exceeding the count refuses with
`PROJECTION_PDF_FORM_INVOCATIONS_EXCEEDED`; exceeding expanded bytes refuses with
`PROJECTION_DOCUMENT_BYTES_EXCEEDED`. Memo entries carry both costs so repeated
Form graphs are checked without exponential traversal.
Inline images remain outside this profile and refuse with
`PROJECTION_PDF_INLINE_IMAGE_UNSUPPORTED` before PDFium loading.

PDF text maps non-whitespace C0 controls, including NUL, and U+007F–U+009F
controls to counted U+FFFD
unmapped-glyph replacements. U+0002 is PDFium's line-end hyphen marker and is
removed to join the word and counted as an unmapped-glyph drop. Page visibility uses the intersection of normalized
MediaBox and CropBox bounds. Undefined, unreferenced Quartz xref entries at offset
zero are treated as free; referenced entries still refuse.

HTML meta declarations resolving to UTF-16LE/BE select UTF-8; a UTF-16 BOM remains
authoritative. Meta x-user-defined selects windows-1252. Progress and meter fallback text shares the UA-hidden handling:
omitted by default, counted, and seller-includable. Details, summary, center,
dialog and option establish block boundaries.

## Known limits

M6's three Type3 routes are now counted: glyph-owned /Resources, font-resource
fallback inside a glyph, and padded Tf operands with an inherited font. The
crafted 2.5 KB attacks refuse by name before PDFium; the original issue was
resource exhaustion, not a leak. L6's padded Do and BDC/DP operands retain the
ActualText flag.

The resource bound retains candidate dictionaries across invoking streams,
fonts and pages, including saved font scopes from earlier selections when a
Type3 font has no Resources dictionary. It removes edges proven impossible by
pinned PDFium rules:
a local XObject category shadows fallback categories, Do reads its final
operand, and an explicit, successfully loaded Type1 font cannot execute Type3
glyphs. Unresolved names and unproven/inherited fonts retain the conservative
superset. This refines the literal unrestricted BETTER union so the required
bounded controls pass. L4-style unproven branching cycles still refuse as
`PROJECTION_PDF_FORM_GRAPH_UNBOUNDED`, separately from exceeded numeric budgets.

L5's non-cyclic bounds are memoised by stream and complete resource scope.
Compact lexical summaries are cached, and the joined page operations are shared
between checks. The former 67-second, 1,000-page guard probe now passes in about
0.12 seconds. The 100,000-invocation document budget is unchanged; direct/split
8x6, Type3 3x3 and p11 controls remain required passes.

Projection deadlines return `PROJECTION_RESOURCE_LIMIT`; RSS watchdog refusals
retain `PROJECTION_MEMORY_LIMIT_EXCEEDED`. A dedicated test executable bypasses
the guard for synthetic resource probes and proves the hook ran. macOS and
Windows CI exercise both named worker refusals, including the production
memory limit, without aborts or crash dialogs. On Windows the watchdog runs
64 MiB below the job-object commit cap so it can refuse before allocations are
denied. The shipping parser contains no resource-probe hook.

On Windows, two rare PDFium out-of-memory paths (an aligned-allocation failure
and a failed C++ allocation inside PDFium) end as a generic "worker crashed"
refusal instead of the named memory refusal. The document is still refused and
nothing is exposed; only the seller-facing wording is generic. Reviewed
statically, not measured.

The corpus gate is the manifest with SHA-256
`551b34a5eedb541be1a0b1507f66f452b1f50109e11acbc1c5ec31af4b516d04`,
kept with its file copies in Koskadeux private evidence. It pins 740 files:
127 real PDFs, 100 synthetic PDFs and 513 HTML files. One of the previously
available 128 real PDFs was lost before pinning; its manifest row marked
`MISSING` is excluded from the gate. Compare base vs head on every other
manifest row for each change; a missing file or a hash that differs from the
manifest fails the gate. Each change must preserve the base's per-file results,
including node, character, drop and warning details and projection output.

For the procedure, see [vectoraiz-vector-pack-corpus-gate.md](https://github.com/aidotmarket/runbooks/blob/main/vectoraiz-vector-pack-corpus-gate.md)
in aidotmarket/runbooks.

The current reference counts are CC's 18 November 2026 comparison: 102 of 127
real PDFs pass, 60 of 100 synthetic PDFs pass, and 505 of 513 HTML files pass.
These are reference measurements, not a new corpus run for this follow-up.
The pinned-manifest comparison replaces the original full-list 115/60/510
seller-enablement gate.
