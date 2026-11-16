# vectorAIz 0.0.26 delivered originals — winmax

These ten public-corpus-derived files are the unmodified members of
`originals-0001.zip` (795,380 bytes), received 16 November 2026.
ZIP SHA256: `58e51cec590fc230ee1d27114c744d8d52e58aadc62987aee090c88638e5084d`.
Size and SHA256 were verified before extraction; no member was rewritten.
The private delivery ZIP is not committed.

Produced on Max's Windows 11 PC (winmax) by installed native **0.0.26**,
release `native-v0.0.26`, native revision `f4b5813a` (C1 fixes vectorAIz #140).
Accepted generation: `2f164d69-01c6-4e8e-834b-b6856761b490`.
The exact CLI command was not supplied; this was the installed native delivery flow.
The mapping below is supplied by the delivery owner. No chart-title edit was
made this run. `testComment.docx` is now ACCEPTED, with comments, commentsExtended,
commentsExtensible and commentsIds parts removed. The presentation comment
fixture has commentAuthors and comment2 removed. `Sheet1` stays hidden in
TwoSheetsOneHidden, as designed (S1927).

Sources: [Apache POI test-data](https://github.com/apache/poi/tree/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/test-data),
revision `77df76b9c990cadbb3c8ae72f05a03fa06194c7a`,
[Apache License 2.0](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/LICENSE)
and [NOTICE](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/NOTICE).
`SampleShow-WithComments.pptx` is the constructed test fixture at
`crates/vz-pack/tests/fixtures/office-public/` in native `f4b5813a`, not an
unmodified Office-saved deck. Its `s1944-comments-provenance.json` records
SHA256 `af689ea64567d459d082e63ef887bb639929f8216a0f4bbd67cf3731ae8403ce`:
`scripts/office-comment-fixture.py` combines pinned POI SampleShow.pptx with
real comment/author parts from 45545_Comment.pptx, without that donor's OLE.

## Member identity and source mapping

POI paths below are relative to `test-data/`; the constructed fixture uses the
native path above. EXPECTED.json also records every source SHA256.

| Member | Source | Delivered SHA256 |
| --- | --- | --- |
| `document-0002.docx` | `document/testComment.docx` | `509b4425a6931be6e928808f5859556159dace926bb844ccd076d940a407b82c` |
| `document-0003.xlsx` | `spreadsheet/SimpleWithComments.xlsx` | `cff088fc5b03d02738557ee0785461b19c06747df129c652f61cbd7faa86286e` |
| `document-0004.docx` | `document/documentProperties.docx` | `f553f3574b14e44166162506fd3306c43cf974bb66a586c77908ccaeac8825bb` |
| `document-0005.docx` | `document/bug56075-changeTracking_on.docx` | `ab51f8ff29b90eea020039b199aed0457d73e0c5e7a563622a0afeafb1f42be8` |
| `document-0006.xlsx` | `spreadsheet/WithChart.xlsx` | `71eefc5746b919061f7e0c6e32185872eef53bc5525ebae437f7a2f844869cda` |
| `document-0007.xlsx` | `spreadsheet/TwoSheetsOneHidden.xlsx` | `4105872c3f3ed37884c7b47fa35bbbf159f8dec2b2ec1e6acb2978f42728fb18` |
| `document-0008.xlsx` | `spreadsheet/chartTitle_withTitle.xlsx` | `974097186868a70cc441f36fdc65b25fc238c149ea6f94df315bc51a978101be` |
| `document-0009.pptx` | `slideshow/smartart-rotated-text.pptx` | `73e3eff6c8f44d2dc748488009c45dcb485ebd9f9eee4d5f486615ea33fbc947` |
| `document-0011.pptx` | `slideshow/bar-chart.pptx` | `10bd20e06470702286581107f51ddfbc00c8d366ce90fdd0427b2535f0b48a50` |
| `document-0012.pptx` | `SampleShow-WithComments.pptx` | `e45cf0aa87f587b235b8925d6bf8f0d0840fd68a1d5f030b4b4dceefdd32e7e8` |

## Validation contract

Every member requires SDK open and LibreOffice conversion success. Both SDK
schema targets require zero errors except the same inherited POI errors as
0.0.25: documentProperties has one Sch_UnexpectedElementContentExpectingComplex;
bar-chart has six Sch_AttributeValueDataTypeDetailed. Raw diagnostics remain
visible. New documents require zero schema errors; CI must confirm this contract.

The unchanged 72 removal-rule families apply to every member. Additionally,
EXPECTED.json sourceRemovals records forbidden source part names, source core
creator/lastModifiedBy and revision authors, plus joined comment text and author
names extracted from each original. The leak checker searches parsed output text
(including text split across runs) and attribute values for these source values,
even if moved into an otherwise allowed carrier. Word comment text is
`comment content`, author `poi`; presentation text is `test phrase`, author
`XPVMWARE01`. Short initials/structural numeric IDs are covered by carrier removal,
not ambiguous text matching. This is a targeted source comparison, not proof that
all retained displayed content is public. Original sources and audit evidence
remain outside the checkout. No existing rule or expectation is relaxed.

These are test-tooling inputs, not an Office qualification result.
LibreOffice is not Microsoft Office. Installed-Microsoft-Office checks are UNRUN.
