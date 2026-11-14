# Serializer-fix fold 2 rebuilt Office packages — S1944

Eight public Apache POI corpus inputs rebuilt by vectorAIz PR
[#138](https://github.com/aidotmarket/vectoraiz/pull/138), producer commit
`aadcc3164f4e50fc7e4373502e1eac6c1813bea0`
(branch `build/office-serializer-prefixes-s1944`), received 14 November 2026.
The supplied manifest maps source names to rebuilt output names. All eight output
SHA256 hashes were verified against that manifest before copying; the packages
are unchanged. The exact rebuild command was not supplied with this delivery.
The producer manifest reports identical readback inventory and passed XML
validation for each output; those are producer observations, not SDK results.

Upstream: [Apache POI test-data](https://github.com/apache/poi/tree/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/test-data)
at commit `77df76b9c990cadbb3c8ae72f05a03fa06194c7a`, under the
[Apache License 2.0](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/LICENSE),
with upstream [NOTICE](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/NOTICE).
Source paths below are relative to that repository. Output names match source
basenames. The supplied manifest SHA256 is `27ba345f0e849b7a27c721931d8bcdddce170c3ac06e684a44a2c01efead0f5a`.

## Manifest identities

| Output / source basename | Source path | Source SHA256 | Output SHA256 |
| --- | --- | --- | --- |
| `SimpleWithComments.xlsx` | `test-data/spreadsheet/SimpleWithComments.xlsx` | `d7d337b3436e863487d26439a57ef749950c599862325d7bce7e830acf1df0ba` | `cff088fc5b03d02738557ee0785461b19c06747df129c652f61cbd7faa86286e` |
| `TwoSheetsOneHidden.xlsx` | `test-data/spreadsheet/TwoSheetsOneHidden.xlsx` | `3e9749837cac5a19a7628d7e60c1a1c6d0ff7657f27fa7904b217b96e0dc534f` | `4105872c3f3ed37884c7b47fa35bbbf159f8dec2b2ec1e6acb2978f42728fb18` |
| `WithChart.xlsx` | `test-data/spreadsheet/WithChart.xlsx` | `51f27aabf7417f5aaa0542857b6bb56a92d4afbabdc5c63f084c06f92b67429d` | `71eefc5746b919061f7e0c6e32185872eef53bc5525ebae437f7a2f844869cda` |
| `bar-chart.pptx` | `test-data/slideshow/bar-chart.pptx` | `79e1d218bfb2903e8dc8425a6b1997d9c1976f5a5f025bada85b0c47b5777969` | `10bd20e06470702286581107f51ddfbc00c8d366ce90fdd0427b2535f0b48a50` |
| `bug56075-changeTracking_on.docx` | `test-data/document/bug56075-changeTracking_on.docx` | `9c837e12aaf3deac10ac20ac1bc08dc6bebdec14b553698063742b9bc12f8920` | `ab51f8ff29b90eea020039b199aed0457d73e0c5e7a563622a0afeafb1f42be8` |
| `chartTitle_withTitle.xlsx` | `test-data/spreadsheet/chartTitle_withTitle.xlsx` | `586c174285621c081020da523c4a814a9ad62b75d86f49276faa1b6036919395` | `974097186868a70cc441f36fdc65b25fc238c149ea6f94df315bc51a978101be` |
| `documentProperties.docx` | `test-data/document/documentProperties.docx` | `1cff7a0a94dfce307a70032d21070d26ae34b9fdf742cf70fa66d4a2078ec9d5` | `f553f3574b14e44166162506fd3306c43cf974bb66a586c77908ccaeac8825bb` |
| `smartart-rotated-text.pptx` | `test-data/slideshow/smartart-rotated-text.pptx` | `a357985aec81b0bfa8a71138bbe94c9b3f6e00dee110c0da34302e22253de260` | `73e3eff6c8f44d2dc748488009c45dcb485ebd9f9eee4d5f486615ea33fbc947` |

## Refused inputs

The manifest records these three refusals; no outputs for them are included.

- `45545_Comment.pptx` (source SHA256 `0295a51f63150e3ee680154d07dab88062ba2306030b790561f85436f3ded7c8`):
  This Office file cannot be safely prepared for listing: document.pptx: VBA/ActiveX/OLE/signature refused at ppt/embeddings/oleObject1.bin. Save a supported, unencrypted .docx/.xlsx/.pptx without macros or external dependencies, then re-ingest, or exclude this file.
- `chartex.docx` (source SHA256 `5d7f0ca9aa26d0244eedbc8ff862f08910689ec3848a036c622b17d56a290b57`):
  This Office file cannot be safely prepared for listing: document.docx: Office 2016 chartEx is unsupported at word/document.xml/{http://schemas.openxmlformats.org/wordprocessingml/2006/main}body[1]/{http://schemas.openxmlformats.org/wordprocessingml/2006/main}p[11]/{http://schemas.openxmlformats.org/wordprocessingml/2006/main}r[2]/{http://schemas.openxmlformats.org/markup-compatibility/2006}AlternateContent[2]/{http://schemas.openxmlformats.org/markup-compatibility/2006}Choice[1]. Save a supported, unencrypted .docx/.xlsx/.pptx without macros or external dependencies, then re-ingest, or exclude this file.
- `testComment.docx` (source SHA256 `06b63b13f0b78a7179151af52c2282557c1ed204899feee9fcef4ca575344f97`):
  This Office file cannot be safely prepared for listing: document.docx: unsupported relationship type at word/_rels/document.xml.rels/rId7. Save a supported, unencrypted .docx/.xlsx/.pptx without macros or external dependencies, then re-ingest, or exclude this file.

The `0.0.24-winmax/` and `fix-s1944/` inputs remain unchanged for comparison. Hidden
spreadsheet sheets are retained by design with a seller warning (S1927); the
namespace-aware leak check reports them as information.

These are validation inputs, not an Office qualification result. LibreOffice is
not Microsoft Office. Installed-Microsoft-Office checks remain **UNRUN**.
