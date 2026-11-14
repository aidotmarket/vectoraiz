# vectorAIz 0.0.24 delivered originals — winmax

These eight public-corpus-derived files are the unmodified members of
`originals-0001.zip` (462,383 bytes), received 14 November 2026.
ZIP SHA256: `2eb18958b69fbb5e78ebd98107f25d6896473214a712f7c13d6ea15c6bb3caa3`.
The ZIP hash was verified before extraction; no delivered member was rewritten.

Rebuild context supplied with the delivery: vectorAIz **0.0.24**, Stage 1 test
build with the test switch enabled, on Max's Windows 10/11 PC **winmax**.
Accepted generation: `b03af6b6-8df9-4429-99df-2bd48ffe1917`.
The exact Windows release and CLI rebuild command were not supplied; this was a
test-build review/delivery flow. The chart title in `document-0005.xlsx` was edited
to **“S1944 edited title”** before delivery.

Upstream: [Apache POI test-data](https://github.com/apache/poi/tree/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/test-data)
at commit `77df76b9c990cadbb3c8ae72f05a03fa06194c7a`,
[Apache License 2.0](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/LICENSE),
with upstream [NOTICE](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/NOTICE).

## Member identity

| Delivered member | SHA256 |
| --- | --- |
| `document-0002.docx` | `95abe913e100aefd6810713ccada980daf8d8d3fbec14737d6dd0058810a45a3` |
| `document-0003.xlsx` | `5b0e27e34f24ce1bd2a2ba3ed555362f10d751e41f939cbb5904caf07d3e7b94` |
| `document-0005.xlsx` | `d07ddfae9c45b00229904b02ae804b383688ec1c15e70d3c757ed22fc6343277` |
| `document-0006.xlsx` | `d0a464ca3dc00615836115314119adde6a523c1bfc102fa84170c33517994a62` |
| `document-0007.xlsx` | `21637db76cac50b62e94f955a94cb0f509843b1a588ddc5022bd9c7a9158bbee` |
| `document-0008.pptx` | `a29e2b3e0a01729111dc9e7dd5e0825b334f5a0a3a13d7e56afe44f7bccfccd7` |
| `document-0009.docx` | `7ddaf03303a75ab49ff4e6763ed263e1e8700906f5b4c0e774731537396afb55` |
| `document-0011.pptx` | `814b9acd27cc61240c2a58398434fb976194bae61e5a6aa811e7b86969906c67` |

## Source mapping and its limits

Document numbering is the review order of **11** sources, not a sorted listing
of the eight accepted files. The refused inputs were `45545_Comment.pptx`,
`chartex.docx`, and `testComment.docx`; no delivered files for them are in the ZIP.
The ZIP contains no source-name manifest. The original 11-file POI selection
was available for comparison; retained XML text, chart caches, sheet structure
and the reported title edit support the mappings below. Each remains **inferred**
because the review-order manifest was not supplied. Paths are relative to the
pinned POI repository.

| Delivered member | Source (all inferred) | Evidence / limitation |
| --- | --- | --- |
| `document-0002.docx` | `test-data/document/bug56075-changeTracking_on.docx` | Retained text matches “Change Tracking ON”. |
| `document-0003.xlsx` | `test-data/spreadsheet/SimpleWithComments.xlsx` | Three sheets and retained 1/one, 2/two, 3/three values match; comments removed. |
| `document-0005.xlsx` | `test-data/spreadsheet/chartTitle_withTitle.xlsx` | Selected title fixture and matching chart cache; title edited before delivery. |
| `document-0006.xlsx` | `test-data/spreadsheet/WithChart.xlsx` | Retained cell/chart text and values match, including “1st Column” / “2nd Column”. |
| `document-0007.xlsx` | `test-data/spreadsheet/TwoSheetsOneHidden.xlsx` | Sheet1A1 / Sheet2A1 values and hidden Sheet1 match. |
| `document-0008.pptx` | `test-data/slideshow/bar-chart.pptx` | Retained chart/slide text matches. |
| `document-0009.docx` | `test-data/document/documentProperties.docx` | Retained “Hello World” / “!” text matches; properties removed. |
| `document-0011.pptx` | `test-data/slideshow/smartart-rotated-text.pptx` | Retained SmartArt/slide text matches, including abc/def/ghi/jkl. |

Hidden sheets are retained by design with a seller warning (S1927). In particular,
`document-0007.xlsx` retains hidden `Sheet1`; the leak validator reports this as
information, not a failure.

These are validation inputs, not an Office qualification result. LibreOffice is
not Microsoft Office. Installed-Microsoft-Office checks remain **UNRUN**.
