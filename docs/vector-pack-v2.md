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

A conservative escaped-name preflight rejects encryption and unbounded filters.
Before lopdf loads xref streams, a streaming zlib preflight caps possible Flate
payloads in every revision. An object callback checks object-stream filters and
limits before lopdf automatically expands them. Every live xref object must be
present after load because lopdf can otherwise skip malformed objects; compressed
xref entries must match their object-stream header indices. Every indirect
reference must resolve, and the bounded page tree must agree with its declared
and PDFium page counts. Page content syntax is checked through a private trailing
parser sentinel, preventing prefix-only parser recovery from certifying an
omitted page. The stream guard counts actual raw or bounded FlateDecode bytes,
enforces per-stream/document/100:1 limits, and refuses unsupported
filters/predictors. Limited PNG predictor parameters are admitted for xref
streams. References are not recursively expanded when counting dictionaries;
direct nesting is limited to eight. The existing supervised worker bounds memory
and CPU time. Conservative byte scanning may refuse ambiguous stream data.

The pinned PDFium API exposes character render modes, character rectangles and
page boxes. It does not expose a proof of the document's optional-content default
configuration. All layered PDFs therefore refuse with
`PROJECTION_PDF_OPTIONAL_CONTENT_STATE_UNPROVEN`, including escaped PDF name
spellings. No object-active flag is used as a default-state oracle. Such PDFs
cannot be included through the hidden-content toggle. Unlayered PDFs proceed;
mode-3 and off-page text are flagged ordinary content and can be edited/excluded.
Blank/image-only pages are counted omissions, while an entirely textless document
refuses. There is no render/OCR fallback in this module.

HTML uses an inert HTML5 tree with a 256-level traversal bound and UTF-8 input
requirement. It outputs text nodes only, with body/heading/table/label kinds.
The closed hidden signals are inherited. Hidden form control values are mandatory
omissions, and unsupported visible interactive control state refuses with
`PROJECTION_HTML_FORM_UNSUPPORTED`. SVG/MathML refuses rather than silently
discarding unknown text semantics. CSS stylesheets/classes/layout are not
evaluated. The per-document warning and generated README state that limitation.

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

Real corpus regressions include 1,236 projected nodes from a Wikipedia page and
two NASA NTRS PDFs that refuse the expansion-ratio limit and unsupported decode
parameters, respectively. Object/xref streams and incremental revisions are
covered with locally generated canary fixtures. Their source
URLs, exact fixture hashes, licences and the HTML re-recording note accompany the
fixtures. These observed refusals are coverage limits, not a PDF capacity claim.
