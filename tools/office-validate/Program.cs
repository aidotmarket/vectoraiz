using System.Security.Cryptography;
using System.Text.Json;
using DocumentFormat.OpenXml;
using DocumentFormat.OpenXml.Packaging;
using DocumentFormat.OpenXml.Validation;

if (args.Length != 1 || !Directory.Exists(args[0]))
{
    Console.Error.WriteLine("Usage: OfficeValidate <directory>");
    return 2;
}

var root = Path.GetFullPath(args[0]);
var files = new List<FileResult>();
foreach (var path in Directory.EnumerateFiles(root, "*", SearchOption.AllDirectories)
    .Where(p => new[] { ".docx", ".xlsx", ".pptx" }.Contains(Path.GetExtension(p).ToLowerInvariant()))
    .OrderBy(p => p, StringComparer.Ordinal))
{
    var result = new FileResult { Path = Path.GetRelativePath(root, path).Replace('\\', '/') };
    files.Add(result);
    try
    {
        using (var input = File.OpenRead(path))
            result.Sha256 = Convert.ToHexString(SHA256.HashData(input)).ToLowerInvariant();
        using OpenXmlPackage package = Path.GetExtension(path).ToLowerInvariant() switch
        {
            ".docx" => WordprocessingDocument.Open(path, false),
            ".xlsx" => SpreadsheetDocument.Open(path, false),
            _ => PresentationDocument.Open(path, false)
        };
        result.OpenOk = true;
        foreach (var version in new[] { FileFormatVersions.Microsoft365, FileFormatVersions.Office2007 })
        {
            var validation = new ValidationResult();
            result.Formats[version.ToString()] = validation;
            try
            {
                // Count every error; only the diagnostic sample is capped at 50.
                var validator = new OpenXmlValidator(version) { MaxNumberOfErrors = 0 };
                var count = 0;
                foreach (var error in validator.Validate(package))
                {
                    count++;
                    if (validation.Errors.Count < 50)
                        validation.Errors.Add(new ErrorResult(error.Id, error.Description,
                            error.Part?.Uri.ToString(), error.Path?.XPath));
                }
                validation.ErrorCount = count;
            }
            catch (Exception ex) { validation.Error = ex.ToString(); }
        }
    }
    catch (Exception ex) { result.OpenError = ex.ToString(); }
}

var ok = files.Count > 0 && files.All(f => f.OpenOk && f.OpenError is null &&
    f.Formats.Count == 2 && f.Formats.Values.All(v => v.Error is null && v.ErrorCount == 0));
Console.WriteLine(JsonSerializer.Serialize(new { sdkVersion = "3.5.1", ok, files },
    new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.CamelCase, WriteIndented = true }));
return ok ? 0 : 1;

sealed class FileResult
{
    public string Path { get; init; } = "";
    public string? Sha256 { get; set; }
    public bool OpenOk { get; set; }
    public string? OpenError { get; set; }
    public Dictionary<string, ValidationResult> Formats { get; } = new();
}
sealed class ValidationResult
{
    public int? ErrorCount { get; set; }
    public string? Error { get; set; }
    public List<ErrorResult> Errors { get; } = new();
}
sealed record ErrorResult(string Id, string Description, string? PartUri, string? XPath);
