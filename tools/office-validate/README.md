# Office validation (test tooling)

`dotnet run --project tools/office-validate -- <directory>` opens every DOCX,
XLSX and PPTX recursively, read-only. JSON on stdout contains relative paths,
SHA256, open status, and independent Microsoft365/Office2007 validation results.
Counts and per-ID histograms are uncapped; each format retains its first 50 diagnostic entries with
ID, description, part URI and XPath. Exceptions have null counts and explicit
errors. Any error, open failure, or empty input directory returns nonzero.

Pins: .NET SDK 8.0.428 / net8.0; DocumentFormat.OpenXml **3.5.1** (latest stable
3.x verified via NuGet at implementation time), exact package range and lockfile;
LibreOffice Fresh **26.2.6** via Chocolatey; Python 3.12.10; pypdf 6.1.1.
Workflow action SHAs are annotated with their release versions in the YAML.

For the combined report, build Release with `dotnet build
tools/office-validate/OfficeValidate.csproj --configuration Release -p:RestoreLockedMode=true`,
install `requirements.txt`, then run:

```powershell
python tools/office-validate/run-validation.py tests/office/delivered `
  --sdk-dll tools/office-validate/bin/Release/net8.0/OfficeValidate.dll `
  --soffice 'C:/Program Files/LibreOffice/program/soffice.com' `
  --report "$env:TEMP/office-delivered-validation.json"
```

The runner checks the installed LibreOffice version. Each conversion has a
60-second timeout and a fresh profile/output directory. It checks the exit code,
requires a nonempty PDF and uses pypdf to count pages. Timeouts kill the process
tree; stdout/stderr, exit status, hashes, page count and failures go into the
single JSON report. SDK execution has a separate 300-second timeout. Both checks
are attempted even if one fails, and CI uploads the report on failure.

The combined runner gates on each set's `EXPECTED.json`: exact file membership,
SHA256, SDK open result, and exact error-ID histograms/counts for both schema
targets, plus LibreOffice success and exit code. Expectations originate in raw
run 43633394595, not a new automatic acceptance of observed results. The 0.0.24
set expects eight SDK open failures and four LibreOffice load failures. Each
newer set expects only the two inherited POI schema issues: six
`Sch_AttributeValueDataTypeDetailed` errors in the chart presentation and one
`Sch_UnexpectedElementContentExpectingComplex` in the properties document,
per schema target. All other schemas and renders must pass. A changed hash,
new/missing error, missing/unexpected file or leak fails. SDK raw `ok: false`,
exit codes, errors and full process output remain in the JSON even when expected.
Timeouts, process failures and version-pin failures cannot be expected away.
`self-test-expectations.py <inputs> <report>` copies inputs to temporary storage
and proves a one-byte fixture mutation and an expected-entry mutation turn the
gate red in every set, including the bootstrap set.

LibreOffice is not Microsoft Office: a successful render is a useful independent
smoke check, not an Office fidelity guarantee.
The downloaded POI originals bootstrap the tool; they do not prove vectorAIz
delivery behavior. Add real public delivered outputs per the corpus README.


The workflow also runs `leak_check.py` in an independent job over every committed
DOCX/XLSX/PPTX under `tests/office/delivered`, recursively. It uses only Python's
standard library and expanded XML namespace names (transitional and strict), so
prefix changes cannot hide tracked edits, comment anchors, chart formulas or
external chart data. Nonempty Dublin Core creator / core-properties
lastModifiedBy fields and forbidden metadata, comment, thumbnail, macro,
embedding, external-link and ActiveX part paths fail the check. Invalid ZIP/XML
also fails. Hidden/veryHidden spreadsheet sheets are informational (S1927).

```sh
python tools/office-validate/leak_check.py tests/office/delivered --report /tmp/office-delivered-leaks.json
python -m unittest discover -s tools/office-validate -p 'test_*.py' -v
```

`office-delivered-leaks` is a separate JSON CI artifact containing paths, hashes,
violations and sheet information per file. The Windows job independently retains
both SDK targets and LibreOffice conversion/page counts. Its downloaded POI
bootstrap originals are not vectorAIz deliveries and are not present in the leak
job's checkout. A failure in either job fails the workflow; both preserve their
reports on failure.

Deny-set sources in `crates/vz-core/src/office.rs`: `removable`, lines 661–693,
removes metadata, custom XML (including identity-bearing item properties), comments,
threaded comments, persons/people, comment authors, printer/pivot/revision/web
extension/tag/glossary/font/ink parts and comment VML. Lines 1110–1238 flatten
content controls/fields/revisions and remove or refuse private Word carriers;
all Word `*Change` property revisions are denied as well as move/range revisions.
Lines 1212–1220 also remove spreadsheet and presentation private elements.
Lines 2564–2608 refuse binary active carriers/signatures and remove carrier parts.
Both transitional and strict namespaces are checked. Hand-written fixtures in
`test_leak_check.py` enumerate carriers independently, build one in-memory ZIP per
case, and exercise n0/x/default prefixes; the moved-away-text, threaded-comment
and presentation-author review reproductions are included.
