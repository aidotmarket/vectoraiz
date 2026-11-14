# Office validation (test tooling)

`dotnet run --project tools/office-validate -- <directory>` opens every DOCX,
XLSX and PPTX recursively, read-only. JSON on stdout contains relative paths,
SHA256, open status, and independent Microsoft365/Office2007 validation results.
Counts are uncapped; each format retains its first 50 diagnostic entries with
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

Both schema targets must pass. Office2007 errors in modern documents are visible
failures, not silently waived. LibreOffice is not Microsoft Office: a successful
render is a useful independent smoke check, not an Office fidelity guarantee.
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
