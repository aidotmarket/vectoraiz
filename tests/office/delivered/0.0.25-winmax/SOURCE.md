# vectorAIz 0.0.25 delivered originals — winmax

These eight public-corpus-derived files are the unmodified members of
`originals-0001.zip` (545,369 bytes), received 14 November 2026.
ZIP SHA256: `3a0f369b891f0fdc27725caa6eaa834acd9c321301d8144564aa2008a3fff741`.
The ZIP hash was verified before extraction; no delivered member was rewritten.

Rebuild context supplied with the delivery: installed vectorAIz **0.0.25**,
Stage 1 test build (native `8552ec3e`, includes Office save fix #138), with the
test switch enabled, on Max's Windows 11 PC **winmax**. The inputs were the same
11 Apache POI test-data files used for the earlier delivery.
Accepted generation: `805c2c7d (prefix)`.
The exact CLI rebuild command was not supplied; this was a test-build
review/delivery flow. The chart title in `document-0006.xlsx` was edited
to **“S1944 edited title”** before delivery.

Upstream: [Apache POI test-data](https://github.com/apache/poi/tree/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/test-data)
at commit `77df76b9c990cadbb3c8ae72f05a03fa06194c7a`,
[Apache License 2.0](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/LICENSE),
with upstream [NOTICE](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/NOTICE).

## Member identity

| Delivered member | SHA256 |
| --- | --- |
| `document-0001.pptx` | `10bd20e06470702286581107f51ddfbc00c8d366ce90fdd0427b2535f0b48a50` |
| `document-0002.pptx` | `73e3eff6c8f44d2dc748488009c45dcb485ebd9f9eee4d5f486615ea33fbc947` |
| `document-0003.docx` | `f553f3574b14e44166162506fd3306c43cf974bb66a586c77908ccaeac8825bb` |
| `document-0004.xlsx` | `cff088fc5b03d02738557ee0785461b19c06747df129c652f61cbd7faa86286e` |
| `document-0006.xlsx` | `6ddacb2b6ceaa8226d880ab9b9664bbde2a61cbb38b8b84d85dbd80b3ab5f6d5` |
| `document-0007.xlsx` | `4105872c3f3ed37884c7b47fa35bbbf159f8dec2b2ec1e6acb2978f42728fb18` |
| `document-0008.docx` | `ab51f8ff29b90eea020039b199aed0457d73e0c5e7a563622a0afeafb1f42be8` |
| `document-0010.xlsx` | `71eefc5746b919061f7e0c6e32185872eef53bc5525ebae437f7a2f844869cda` |

## Source mapping and its limits

Document numbering is the review order of **11** sources, not a sorted listing
of the eight accepted files. The refused inputs were `45545_Comment.pptx`,
`chartex.docx`, and `testComment.docx`; no delivered files for them are in the ZIP.
The ZIP contains no source-name manifest. Comparison with the hash-verified
`fix3-s1944` packages and the reported title edit supports the mappings below.
Each remains **inferred** because the review-order manifest was not supplied.
Paths are relative to the pinned POI repository.

| Delivered member | Source (all inferred) | Evidence / limitation |
| --- | --- | --- |
| `document-0001.pptx` | `test-data/slideshow/bar-chart.pptx` | Byte-identical to `fix3-s1944/bar-chart.pptx`. |
| `document-0002.pptx` | `test-data/slideshow/smartart-rotated-text.pptx` | Byte-identical to `fix3-s1944/smartart-rotated-text.pptx`. |
| `document-0003.docx` | `test-data/document/documentProperties.docx` | Byte-identical to `fix3-s1944/documentProperties.docx`. |
| `document-0004.xlsx` | `test-data/spreadsheet/SimpleWithComments.xlsx` | Byte-identical to `fix3-s1944/SimpleWithComments.xlsx`. |
| `document-0006.xlsx` | `test-data/spreadsheet/chartTitle_withTitle.xlsx` | Matching chart cache and selected title fixture; title edited before delivery. |
| `document-0007.xlsx` | `test-data/spreadsheet/TwoSheetsOneHidden.xlsx` | Byte-identical to `fix3-s1944/TwoSheetsOneHidden.xlsx`. |
| `document-0008.docx` | `test-data/document/bug56075-changeTracking_on.docx` | Byte-identical to `fix3-s1944/bug56075-changeTracking_on.docx`. |
| `document-0010.xlsx` | `test-data/spreadsheet/WithChart.xlsx` | Byte-identical to `fix3-s1944/WithChart.xlsx`. |

Seven of eight mapped files match `fix3-s1944` byte-for-byte. The edited-title
`document-0006.xlsx` differs from `fix3-s1944/chartTitle_withTitle.xlsx`, so the
sets do not match file-for-file.

Hidden sheets are retained by design with a seller warning (S1927). In particular,
`document-0007.xlsx` retains hidden `Sheet1`; the leak validator reports this as
information, not a failure.

These are validation inputs, not an Office qualification result. LibreOffice is
not Microsoft Office. Installed-Microsoft-Office checks remain **UNRUN**.
