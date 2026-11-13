# Delivered Office validation inputs

Place public-corpus-derived DOCX, XLSX and PPTX outputs rebuilt by vectorAIz
here, including nested directories. The source corpus is Apache POI test data at
commit `77df76b9c990cadbb3c8ae72f05a03fa06194c7a` (Apache License 2.0).
Record each delivered file's source path, rebuild command/version, and SHA256
alongside it when adding outputs. Nothing private may be committed here.

Before delivered outputs arrive, CI downloads two small unmodified POI originals
into `_poi-sanity/`, from raw URLs at that exact commit, and verifies their SHA256
using `tools/office-validate/fetch-sanity.py`. These are bootstrap checks, not
vectorAIz-delivered outputs and not evidence of delivery correctness. See POI's
[license](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/LICENSE)
and [notice](https://github.com/apache/poi/blob/77df76b9c990cadbb3c8ae72f05a03fa06194c7a/NOTICE).

The workflow reports SDK validation for Microsoft365 and Office2007 separately;
any error in either version fails the job. LibreOffice PDF conversion and page
counts are separate checks. LibreOffice is not Microsoft Office, and successful
conversion does not establish Microsoft Office rendering fidelity.
