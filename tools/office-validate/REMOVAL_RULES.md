# Delivered Office removal inventory

Source revision: `73c3301ee07ea45e2ee354556fb7f0e753a4bce3`. Line numbers below
refer to that revision; production Rust is unchanged. Paths are relative to
`crates/vz-core/src/`. `O` means `office.rs`, `G` means
`office/graphics.rs`. Namespace names are expanded URI names, never prefixes.

There are **72 output rule families** in the numbered inventory below. Lists
inside a row are the complete alternatives at that removal site, not examples.
Each observable rule family has an independently written package in
`test_leak_check.py`; rows 24 and 64 explain the provenance-only exceptions.
The tests do not import or iterate the checker's deny sets. Regressions run each
package separately; a compound dirty package cannot conceal a missing check.
The four codex2 reproductions also have a separate explicit regression method.

Fold 6 audited all 72 rows against their production sites. Corrected rows are
22, 42, 43, 44, 56 and 62, each with a hand-written regression named below.
The remaining audit found no further partial value or condition checks:
01–21 cover complete removed paths and resolved relationship conditions;
23–41 cover removed wrappers/elements/attributes and documented provenance;
45–55 cover child counts, exact recomputed validation counts, complete attribute
removal and namespace/name refusals; 57–61 cover complete reference wrappers,
model graph conditions and empty blip values; 63–64 cover serialization and
documented provenance; 65–72 cover image marker/chunk allowlists, exact JFIF
bytes, exact Adobe version/flags with transform 0–2, framing and supplemental
carriers. No provenance exception or expectation was added by this audit.
For row 22, source suffixes are preserved by production; for row 46, counts
already compare against the complete decimal child count; rows 67–68 already
compare every normalized byte. These are not prefix-only checks.

Checker function names are the enforcement mapping. Regression method names in
the last column omit `test_`; `paths`, `word`, `sheet`, `chart`, `attrs`,
`flat`, `formula`, `rels`, `media`, `models`, `serial`, and `visibility` expand to:

| Short name | Method in `test_leak_check.py` |
| --- | --- |
| paths | forbidden_paths_at_any_depth |
| word | independently_enumerated_revision_and_field_carriers |
| sheet | independent_spreadsheet_and_presentation_carriers |
| chart | independently_enumerated_chart_and_identity_carriers |
| attrs | independent_attributes_under_renamed_prefixes |
| flat | independent_flattened_elements_and_chart_removals |
| formula | independent_formula_and_empty_wrapper_cases |
| rels | relationship_ownership_targets_and_external_policy |
| media | independent_media_and_binary_carriers |
| models | independent_smartart_source_and_hidden_carriers |
| serial | serialization_part_and_type_table_removals |
| visibility | independent_chart_refusals_and_surviving_extensions |

## Parts and relationships

`forbidden_part` checks case folded paths and recursively checks the owner of
every relationships part, even when the owner itself is absent. It intentionally
also recognizes nested metadata directories and strict namespace equivalents.
`check_relationships` resolves absolute and relative OPC targets, including
dot segments, before testing removal. Missing targets do not mask removed ones.

| ID | Source file:line | Complete rule | Output enforcement | Regression |
| --- | --- | --- | --- | --- |
| 01 | O:662–668 | Remove `.rels` belonging to a removable part, recursively | `forbidden_part`, `relationship_owner`, `check_relationships` | rels, review_four_reproductions |
| 02 | O:669 | Remove `word/webSettings.xml`, `ppt/viewProps.xml` | `forbidden_part` | paths |
| 03 | O:672 | Remove `docProps/` | `forbidden_part` | paths |
| 04 | O:673 | Remove case insensitive `customXml/`, including item properties and owned relationships | `forbidden_part` | paths |
| 05 | O:674–675 | Remove `[trash]/`, `docMetadata/` | `forbidden_part` | paths |
| 06 | O:676 | Remove paths containing `/comments` (including extensions and non-directory suffixes) | `forbidden_part` | paths, serial |
| 07 | O:677–680 | Remove `/threadedComments`, `/persons/`, `/people.xml`, `/commentAuthors.xml` | `forbidden_part` | paths |
| 08 | O:681–683 | Remove `/printerSettings/`, `/pivotCache/`, `/pivotTables/` | `forbidden_part` | paths |
| 09 | O:684–688 | Remove `/revisions/`, `/webextensions/`, `/tags/`, `/revisionInfo.xml`, `/changesInfos/` | `forbidden_part` | paths |
| 10 | O:689,691–692 | Remove `/glossary/`, `/fonts/`, `/ink/` | `forbidden_part` | paths |
| 11 | O:690,2581–2599 | Remove Excel drawing VML; refuse non-note shapes, OLEObject/control dependencies | `forbidden_part` rejects all delivered VML in this family | paths |
| 12 | O:2601 | Remove `xl/calcChain.xml` | `forbidden_part` | paths, rels |
| 13 | O:2205–2250 | Inline referenced shared strings and remove whole `xl/sharedStrings.xml`, including orphan strings | `forbidden_part`; `element_reasons` rejects `s:c/@t=s` | serial, formula, rels |
| 14 | O:2569–2580; G:504–514 | Refuse CFB/OLE signatures, nested ZIP, any `vbaProject` path, ActiveX, embeddings, package signatures | binary signature scan and `forbidden_part` | media, paths |
| 15 | O:2827–2840 | Drop external attachedTemplate and non-HTTP(S) hyperlinks; refuse every other external dependency | `check_relationships` permits only HTTP(S) hyperlink type URI | rels |
| 16 | O:2841–2857 | Drop internal relationships to removable parts, calcChain and sharedStrings | `resolve_target`, `forbidden_part`, `check_relationships` | rels |
| 17 | O:2782–2787; G:657–666 | No owned relationships survive an absent owner | `check_relationships` | rels |
| 18 | O:2887–2917,3024–3047 | Unlink removed relationship IDs: flatten Word hyperlinks, drop drawing/worksheet links, remove pageSetup reference attributes; refuse other dependent content; refuse internal Word anchor and sheet location links | `check_relationships` rejects dangling IDs/absent `.rels`; `element_reasons` rejects anchors/locations | rels, attrs |
| 19 | G:558–619 | Remove relationships whose source references were deleted, and chart themeOverride; refuse unrelated OLE/control/VBA and unproven package dependencies | `check_relationships` rejects unused graphics/link relationships and package/oleObject/control/vbaProject/themeOverride kinds | rels |
| 20 | G:621–670 | Recursively remove orphan graphics dependencies reached through deleted references, including their `.rels` | `check_relationships` catches absent owners/targets; forbidden dependency kinds/parts catch carriers | rels; see provenance limit below |
| 21 | G:686–693; O:2963–2997 | Remove pruned type overrides; rebuild type table from retained parts | `check_file` rejects removed/dangling overrides | serial |
| 22 | O:2665–2773 | Replace source names for media, themes, charts, diagrams, chartsheets, worksheets, tables, drawings, Word headers/footers and presentation slide/layout/master/notes/handout parts; rename owned `.rels` and targets | `part_reasons` full-matches family/root-specific canonical directory and stem/serial/suffix; media/theme retain production's first root component; relationships check owner and target existence | serial, rule22_complete_neutral_paths_and_stems |

## Elements and attributes

`WORD`, `SHEET`, `CHART`, `DRAWING`, `PRESENTATION`, `XDR` in the checker include
both transitional and strict URIs. `MC`, `X14`, `XM`, `P14`, `DGM`, `DSP` are
the exact URIs declared in the checker. Word attributes require the Word URI;
unqualified shape actions are checked on presentation/spreadsheet drawing
elements. A default namespace does not put unqualified attributes in that URI.

| ID | Source file:line | Complete rule | Output enforcement | Regression |
| --- | --- | --- | --- | --- |
| 23 | O:1018–1056; G:302–382 | Resolve MC AlternateContent, remove Choice/Fallback wrappers and unselected branches; refuse malformed/no supported branch | `element_reasons` rejects all MC elements | flat |
| 24 | O:1063 | Clear whitespace-only structural text except listed value/layout text and chart separator | No private non-whitespace payload is removed here; whitespace normalization cannot prove input provenance | No negative carrier exists; see serialization limit |
| 25 | O:1065–1097,1162–1193,1203–1210,1333 | Split mixed Word runs, flatten begin/separate/end markers and instruction region, preserve displayed result; refuse nested, unknown, missing-boundary or resultless fields | `forbidden_element` rejects `fldChar`, `instrText`; no markers/instructions can survive | word |
| 26 | O:1103–1132 | Flatten Word sdt; remove sdtPr/sdtEndPr, binding and sdtContent wrapper; refuse non-unique displayed content or instruction-region control | `forbidden_element` rejects all five wrapper/property names | word, flat |
| 27 | O:1134–1152; G:479–482 | Flatten DrawingML fld to r, clear ALL field attributes and pPr; remove chart CELLRANGE fld entirely; refuse missing displayed text/unknown children | `forbidden_element` rejects DrawingML `fld` regardless of prefix, ID/type or children | flat, review_four_reproductions |
| 28 | O:1154–1160 | Remove hlinkClick/hlinkHover when unqualified action is present, including empty action and ppaction://macro | `element_reasons` | attrs, review_four_reproductions |
| 29 | O:1195–1202,1354 | Remove inherited schema-ignorable elements (except P14/X14/DSP) and all ignorable attributes | MC attributes are forbidden, so no output can carry an Ignorable declaration; unknown former ignorable payload without a declaration is provenance limited | attrs; see provenance limit |
| 30 | O:1211 | Remove mathPr under Word settings | `element_reasons` | flat |
| 31 | O:1212 | Remove presentation clrMru, printPr | `forbidden_element` | sheet |
| 32 | O:1213 | Remove Word embedTrueTypeFonts, embedSystemFonts, saveSubsetFonts, paperSrc, savePreviewPicture, printerSettings, rsid, aliases, numIdMacAtCleanup, formsDesign, proofErr, shapeDefaults, hdrShapeDefaults, doNotIncludeSubdocsInStats, embedRegular, embedBold, embedItalic, embedBoldItalic | `forbidden_element` | word |
| 33 | O:1214 | Remove spreadsheet legacyDrawing, legacyDrawingHF | `forbidden_element` | sheet |
| 34 | O:1215 | Remove spreadsheet AC absPath | `forbidden_element` | sheet |
| 35 | O:1216 | Remove spreadsheet fileVersion, fileRecoveryPr | `forbidden_element` | sheet |
| 36 | O:1217 | Remove Word del, moveFrom, commentRangeStart, commentRangeEnd, commentReference, docVars, rsids, documentProtection, writeProtection, attachedTemplate, trackRevisions, altChunk, glossaryDocument, instrText | `forbidden_element` | word |
| 37 | O:1218 | Remove spreadsheet sheetCalcPr, mruColors, calcPr, calcChain, fileSharing, customWorkbookViews, pivotCaches, pivotTableParts, phoneticPr, rPh, sheetProtection, workbookProtection | `forbidden_element` | sheet |
| 38 | O:1219 | Remove presentation custDataLst, custData, tags, embeddedFontLst | `forbidden_element` | sheet |
| 39 | O:1228 | Refuse Word moveTo, moveFromRangeStart, moveFromRangeEnd, moveToRangeStart, moveToRangeEnd, customXmlInsRangeStart, customXmlDelRangeStart | `forbidden_element` | word |
| 40 | O:1229–1240 | Flatten ins/fldSimple, clearing wrapper attributes; refuse empty simple-field result | `forbidden_element` | word |
| 41 | O:1242–1268,924–1013 | Remove cell formulas after validating cached scalar/shared/array results; refuse missing/unsupported cache, shared anchors or array dependencies | `formula_element` rejects all formula elements, including orphan formulas | sheet, formula |
| 42 | O:802–813,1269–1279 | Remove definedName unconditionally; formula-bearing dataValidation/cfRule in S/X14; formula detector includes f, formula, formula1, formula2, calculatedColumnFormula, totalsRowFormula, XM f, and formula/non-number cfvo | `forbidden_element`, `formula_element`; cfvo val must full-match ASCII numeric lexical characters and parse to a finite number, without whitespace/underscores/Unicode digits | sheet, formula, rule42_numeric_cfvo_lexical_form |
| 43 | O:1281–1296 | extLst retains only DSP dataModelExt with DSP URI, P14 sectionLst, X14 dataValidations/conditionalFormattings; remove childless extension children and childless extLst in every namespace | `element_reasons` checks namespace/local/URI and actual child count independently of wrapper namespace | formula, visibility, rule43_empty_extension_child_in_other_namespace |
| 44 | O:1298–1306 | Remove X14 conditionalFormatting without a direct X14 cfRule child, including sqref-only content | `element_reasons` checks direct child expanded name | formula, rule44_requires_direct_x14_rule_child |
| 45 | O:1307–1314 | Remove empty S/P ext/extLst | `element_reasons` | formula |
| 46 | O:1317–1330 | Remove empty S/X14 dataValidations, conditionalFormatting, conditionalFormattings, definedNames; recompute dataValidations count | `element_reasons` | formula |
| 47 | O:1335–1350 | Remove empty P/XDR macro/textlink; refuse nonempty actions | `element_reasons` rejects either attribute, empty or populated | attrs |
| 48 | O:1353–1355 | Remove ALL Word rsid* and displacedByCustomXml attributes | `element_reasons`, expanded attribute URI/local | attrs, review_four_reproductions |
| 49 | O:1356 | Remove unqualified spreadsheet codeName on any S element | `element_reasons` | attrs |
| 50 | O:1357 | Remove ALL MC attributes, including compatibility directives | `element_reasons` | attrs |
| 51 | O:1358–1359 | Remove Word 2010 paraId, textId | `element_reasons` | attrs |
| 52 | O:1375–1530,444–580 | Word property revisions *Change and cellIns/cellDel/cellMerge have no retained allowlist | `forbidden_element` (including future *Change names) | word |
| 53 | G:275–291 | Refuse any chartex/chartEx namespace, including alternate Choice Requires | `element_reasons`; MC wrapper also forbidden | flat |
| 54 | G:384–394 | Refuse filtered* / categoryFilterExceptions / categoryFilterException in Microsoft drawing chart namespaces | `element_reasons` | visibility |
| 55 | G:404–407 | Refuse multiLvlStrRef | `forbidden_element` | flat |
| 56 | G:408–424 | Chart must plot visible cells only; first plotVisOnly must exist and have absent/1/true val | `element_reasons` checks the first direct setting; only a childless, textless chart with exactly one R id attribute is reference-only | visibility, rule56_visibility_uses_first_setting_and_content |
| 57 | G:426–473 | Convert strRef/numRef to strLit/numLit or series text v; clear source attributes/children; refuse missing/ambiguous cache | `forbidden_element` rejects original reference wrappers even without f | flat |
| 58 | G:479–482 | Remove chart f, externalData, pivotSource, pivotFmts, printSettings, headerFooter, protection, userShapes, extLst | `forbidden_element` | chart, flat |
| 59 | G:94–220 | Remove undrawn textual DGM pt and all cxn touching its ID (src/dest/parTrans/sibTrans); refuse dangling IDs, inconsistent cache text and hidden presAssocID semantics | `check_models` resolves drawing cache within owning graph, checks visible mapping/text and dangling connections | models |
| 60 | G:232–242 | Remove DGM title, desc, catLst, sampData, styleData, clrData recursively | `element_reasons` | flat |
| 61 | G:245–247 | Remove empty DGM shape R blip attribute | `element_reasons` | models |
| 62 | G:248–259,675–684 | Neutralize DGM definition uniqueId and references loTypeId/qsTypeId/csTypeId/presId to urn:vectoraiz:diagram:N | `element_reasons` full-matches `urn:vectoraiz:diagram:(?:0\|[1-9][0-9]*)`, including canonical ASCII integer syntax | models, rule62_complete_numeric_diagram_urns |
| 63 | O:202,317,343; O:351–435 | Refuse DTD/entities/PI; discard comments during parsing/serialization | XML preflight, comment/PI preserving TreeBuilder, `check_file` | serial |
| 64 | O:82–89,2257–2263,2999–3019 | Removals, staging chart source annotations and report remain outside delivered ZIP | Known carrier checks cover their XML/part forms; arbitrary text matching private input cannot be inferred from output alone | See provenance limit |

## Media

The checker walks segments/chunks; it never searches compressed pixel bytes for
metadata keywords. Image pixel decoding is a production refusal and render/SDK
concern, not reimplemented by this standard-library leak check.

| ID | Source file:line | Complete rule | Output enforcement | Regression |
| --- | --- | --- | --- | --- |
| 65 | O:1967–2013 | PNG retains only IHDR/PLTE/IDAT/IEND/tRNS/sRGB/gAMA/cHRM/bKGD; remove every other ancillary chunk; refuse unknown required chunk/bad framing; remove bytes after IEND | `media_reasons` | media |
| 66 | O:2045–2047 | Refuse JPEG ICC_PROFILE APP2 | `media_reasons` | media |
| 67 | O:2048–2054 | Normalize JFIF APP0 to fixed version, unit density, no thumbnail | `media_reasons` compares exact retained bytes | media |
| 68 | O:2055–2069 | Normalize Adobe APP14 version/flags; retain supported transform 0–2; refuse malformed/unsupported transform | `media_reasons` | media |
| 69 | O:2071–2073 | Remove remaining APP0–APP15 and COM; retain only C0/C1/C2/C4/DA/DB/DD decoding markers; refuse others | `media_reasons` | media |
| 70 | O:2076–2098 | Preserve JPEG scan stuffing/restarts; require EOI; remove trailing bytes | `media_reasons` | media |
| 71 | O:2099–2108,2620–2633 | Refuse non-JPEG/PNG media, invalid signatures, invalid image decode/resource excess and content type mismatch | `media_reasons` checks format/signature/framing; pixel validity and type schema handled by render/SDK and production | media |
| 72 | O:661–693; O:2323–2326 | Supplemental delivered-file defenses: nonempty dc:creator/core:lastModifiedBy, thumbnail/externalLink carriers | `forbidden_element` identity check and `forbidden_part` | chart, paths |

## Refusal audit and output observability

The removal inventory is not a claim that a negative leak result certifies the
entire production allowlist. The pipeline additionally refuses the following
input states. They are listed here to avoid silently omitting safety checks that
do not represent an additional removable output carrier:

| Source file:line | Refusal class | Output counterpart or reason it cannot be reconstructed |
| --- | --- | --- |
| O:162–349 | Unbound/duplicate expanded names, malformed XML, multiple/missing roots, non-UTF8, invalid entity characters, unsupported Ignorable URI/Choice Requires, node/depth/tag/attribute/text limits | ElementTree rejects malformed/duplicate/unbound names; MC/DTD/PI are forbidden. Original parse workload and rejected input cannot be inferred from a small normalized output. |
| O:582–613,622–660 | Non-ASCII/control, unsafe/escaped/dot part names, invalid relationship paths/URIs/root escape | `part_reasons`, `relationship_owner`, `resolve_target` check output names and relationships. |
| O:795–1013 | Invalid cell coordinates, unsupported shared-formula syntax/dependency, absent cache/array expression/range/result, inventory limits | Formula carriers cannot survive. A values-only output cannot prove original formula support, cache freshness or that removed input was within bounds. |
| O:1375–1530,1531–1652; office/{chart,diagram,layout,style}_allowlist.rs | Unsupported element/attribute/context/value/layout, mixed text, ambiguous shape identity, duplicate/absent sheet/column names, invalid typed cell | Production positive allowlists and independent SDK schema validation. These are structural acceptance rules, not additional metadata deletion rules; leak checking is not a second schema implementation. |
| O:1664–1872; O:2456–2502 | Unsupported edit target/dependency, prior-value hash mismatch, invalid sheet/table/column names, duplicate cells, missed edits | EXPECTED hashes/SDK/render gate checks delivered identity/structure. Prior input values, requested edit intent and selector uniqueness cannot be proven from output without the edit request. |
| O:1874–1952 | Invalid/duplicate/canonical-colliding ZIP members, special/encrypted entries, size/count/ratio limits | ZIP reader/duplicate name check covers readable output framing. Original compression/resource use is not preserved by stored-member rebuild; this leak tool is not an untrusted-input resource sandbox. |
| O:2112–2202,2512–2565,2612–2618,2643–2653 | Wrong extension/format/root/content type, duplicate/default/override type definitions, absent/dangling parts | EXPECTED file membership/hash and SDK gates; leak check also rejects dangling/removed override targets. Original type identifiers/defaults are rebuilt and cannot be inferred. |
| O:2784–2885,3024–3047 | Absent owner, duplicate/missing rel ID/type/target, unsupported type/kind/mode, dangling target, multiple/mismatched Office roots, orphan references | `check_relationships` checks owners/IDs/mode/targets/references and external policy; SDK handles root/type/schema structure. |
| O:2329–2444 | Readback requires rebuilt type table, media type consistency, unique tables, root relationship, graphics link/model/reference consistency | EXPECTED hashes, SDK/render, relationships/model checks. Full production readback still applies before delivery. |
| G:30–220,805–869 | Missing/ambiguous SmartArt cache/shape/modelId, dangling model connections, text/cache inconsistency, broken hidden presAssocID, mismatched chart/diagram target type/root | `check_models` checks resolved cache, visible textual mapping and connections; relationship checks reject missing graph dependencies. Schema and full production graphics link validation remain required. |
| G:397–473,558–599 | Missing chart visibility/cache; ambiguous series text; unrelated OLE or unproven chart package removal | Visibility/ref/active-dependency checks reject surviving carriers. Whether a removed package was proven to be a chart workbook needs the original graph. |
| G:706–803; office/chart_sources.rs:263–394 | Numeric-cache text edits, ambiguous/split source mappings, inconsistent paired edits | Input edit authorization and private source mapping are absent from delivery; output alone cannot establish these conditions. |

Provenance limit: an orphan graph part that was never present in the output,
an unselected branch, field instruction text that has already lost its markers,
an arbitrary formerly ignorable namespace after removal of its declaration, or
arbitrary private text cannot be compared to its original source without that
source. The checker rejects observable source wrappers, metadata, actions and
removed dependencies, and reports no new exceptions for the 32 deliveries.
It does not claim that all retained displayed text, alt text, cropped/hidden
pixels, cached chart values or permitted HTTP(S) hyperlinks are public.

Serialization limit: attribute order, namespace prefix spellings, whitespace,
ZIP compression and original type identifiers are normalized by rebuild. The
leak checker deliberately uses expanded names and resolved targets; byte-for-byte
proof that normalization happened belongs to the pipeline's readback and the
hash-bound fixture expectations. Hidden sheets remain informational.
