"""Regression checks for failure reporting and independent rendering."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import zipfile

import expectations
import unittest
from unittest.mock import patch

from pypdf import PdfWriter

SPEC = importlib.util.spec_from_file_location("validation", Path(__file__).with_name("run-validation.py"))
validation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validation)


class ValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.input = self.root / "input"
        self.input.mkdir()
        self.file = self.input / "same.DOCX"
        self.write_package(self.file)
        self.report = self.root / "report.json"
        self.conversion_dirs = []

    def write_package(self, path):
        with zipfile.ZipFile(path, "w") as package:
            package.writestr("[Content_Types].xml",
                            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>')

    def fake_run(self, command, timeout):
        result = {"exitCode": 0, "timedOut": False, "stdout": "", "stderr": ""}
        if command[0] == "dotnet":
            files = [{"path": p.relative_to(self.input).as_posix(),
                      "sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
                      "openOk": True, "openError": None,
                      "formats": {v: {"errorCount": 0, "errorIds": {}, "error": None}
                                  for v in ["Microsoft365", "Office2007"]}}
                     for p in self.input.rglob("*") if p.suffix.lower() in validation.EXTENSIONS]
            result["stdout"] = json.dumps({"ok": True, "files": files})
        elif "--version" in command:
            result["stdout"] = "LibreOffice 26.2.6.2 abcdef\n"
        else:
            outdir = Path(command[command.index("--outdir") + 1])
            self.conversion_dirs.append(outdir)
            writer = PdfWriter()
            writer.add_blank_page(width=72, height=72)
            writer.write(outdir / (Path(command[-1]).stem + ".pdf"))
        return result

    def invoke(self, runner):
        expected = {p.name: {"sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
                            "openOk": True, "formats": {
                                v: {"errorCount": 0, "errorIds": {}}
                                for v in ["Microsoft365", "Office2007"]}, "libreOfficeOk": True, "libreOfficeExitCode": 0}
                    for p in self.input.iterdir() if p.suffix.lower() in validation.EXTENSIONS}
        (self.input / "EXPECTED.json").write_text(json.dumps({"version": 1, "files": expected}))
        argv = ["run-validation.py", str(self.input), "--sdk-dll", "unused.dll",
                "--soffice", "soffice", "--report", str(self.report)]
        with patch("sys.argv", argv), patch.object(validation, "run", side_effect=runner), patch("sys.stdout", new=io.StringIO()):
            code = validation.main()
        return code, json.loads(self.report.read_text())

    def test_success_and_duplicate_stems_are_isolated(self):
        self.write_package(self.input / "same.xlsx")
        code, report = self.invoke(self.fake_run)
        self.assertEqual(code, 0)
        self.assertTrue(report["ok"])
        self.assertEqual([r["pageCount"] for r in report["libreOffice"]], [1, 1])
        self.assertEqual(len(set(self.conversion_dirs)), 2)

    def test_sdk_failure_does_not_skip_render(self):
        def runner(command, timeout):
            result = self.fake_run(command, timeout)
            if command[0] == "dotnet":
                result.update(exitCode=1, stdout="bad SDK output", stderr="SDK failure")
            return result
        code, report = self.invoke(runner)
        self.assertEqual(code, 1)
        self.assertEqual(report["sdkProcess"]["stderr"], "SDK failure")
        self.assertTrue(report["libreOffice"][0]["ok"])

    def test_zero_exit_without_pdf_is_failure(self):
        with patch.object(validation, "run", return_value={"exitCode": 0, "timedOut": False, "stdout": "", "stderr": ""}):
            result = validation.render(self.file, self.input, "soffice", True, 60)
        self.assertFalse(result["ok"])
        self.assertIn("no nonempty PDF", result["error"])

    def test_timeout_is_preserved(self):
        with patch.object(validation, "run", return_value={"exitCode": -9, "timedOut": True, "stdout": "partial", "stderr": "hung"}):
            result = validation.render(self.file, self.input, "soffice", True, 60)
        self.assertFalse(result["ok"])
        self.assertTrue(result["timedOut"])
        self.assertEqual(result["stdout"], "partial")

    def test_wrong_version_fails_even_with_pdf(self):
        with patch.object(validation, "run", side_effect=self.fake_run):
            result = validation.render(self.file, self.input, "soffice", False, 60)
        self.assertFalse(result["ok"])
        self.assertEqual(result["pageCount"], 1)

    def test_missing_render_executable_is_recorded(self):
        with patch.object(validation, "run", side_effect=FileNotFoundError("soffice")):
            result = validation.render(self.file, self.input, "soffice", False, 60)
        self.assertFalse(result["ok"])
        self.assertIn("FileNotFoundError", result["error"])

    def test_expected_schema_failure_is_green_and_raw_failure_retained(self):
        def runner(command, timeout):
            result = self.fake_run(command, timeout)
            if command[0] == "dotnet":
                data = json.loads(result["stdout"])
                data["ok"] = False
                for value in data["files"][0]["formats"].values():
                    value.update(errorCount=1, errorIds={"Inherited": 1})
                result.update(exitCode=1, stdout=json.dumps(data))
                manifest = self.input / "EXPECTED.json"
                expected = json.loads(manifest.read_text())
                for value in expected["files"][self.file.name]["formats"].values():
                    value.update(errorCount=1, errorIds={"Inherited": 1})
                manifest.write_text(json.dumps(expected))
            return result
        code, report = self.invoke(runner)
        self.assertEqual(code, 0)
        self.assertFalse(report["openXml"]["ok"])
        self.assertEqual(report["sdkProcess"]["exitCode"], 1)

    def test_unexpected_new_missing_or_different_error_ids_fail(self):
        for ids in [{"New": 1}, {"Other": 2}]:
            def runner(command, timeout):
                result = self.fake_run(command, timeout)
                if command[0] == "dotnet":
                    data = json.loads(result["stdout"])
                    data["files"][0]["formats"]["Microsoft365"].update(
                        errorCount=sum(ids.values()), errorIds=ids)
                    result["stdout"] = json.dumps(data)
                return result
            code, report = self.invoke(runner)
            self.assertEqual(code, 1)
            self.assertTrue(report["expectationErrors"])

    def test_missing_expected_schema_error_fails(self):
        def runner(command, timeout):
            result = self.fake_run(command, timeout)
            if command[0] == "dotnet":
                manifest = self.input / "EXPECTED.json"
                expected = json.loads(manifest.read_text())
                expected["files"][self.file.name]["formats"]["Microsoft365"].update(
                    errorCount=1, errorIds={"Missing": 1})
                manifest.write_text(json.dumps(expected))
            return result
        code, report = self.invoke(runner)
        self.assertEqual(code, 1)
        self.assertTrue(report["expectationErrors"])

    def test_one_fixture_byte_or_expected_entry_change_fails(self):
        code, report = self.invoke(self.fake_run)
        self.assertEqual(code, 0)
        expected = expectations.load(self.input)
        files = [self.file]
        original = self.file.read_bytes()
        self.file.write_bytes(original + b"x")
        self.assertTrue(expectations.compare(self.input, files, report["openXml"],
                                            report["libreOffice"], expected))
        self.file.write_bytes(original)
        expected[self.file.name]["formats"]["Microsoft365"]["errorIds"] = {"Missing": 1}
        self.assertTrue(expectations.compare(self.input, files, report["openXml"],
                                            report["libreOffice"], expected))

    def test_empty_directory_fails(self):
        self.file.unlink()
        code, report = self.invoke(self.fake_run)
        self.assertEqual(code, 1)
        self.assertIn("No DOCX/XLSX/PPTX inputs found", report["errors"])


if __name__ == "__main__":
    unittest.main()
