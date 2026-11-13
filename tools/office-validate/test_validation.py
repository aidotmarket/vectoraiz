"""Regression checks for failure reporting and independent rendering."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
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
        self.file.write_bytes(b"public test placeholder")
        self.report = self.root / "report.json"
        self.conversion_dirs = []

    def fake_run(self, command, timeout):
        result = {"exitCode": 0, "timedOut": False, "stdout": "", "stderr": ""}
        if command[0] == "dotnet":
            files = [{"path": p.relative_to(self.input).as_posix(),
                      "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
                     for p in self.input.rglob("*") if p.is_file()]
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
        argv = ["run-validation.py", str(self.input), "--sdk-dll", "unused.dll",
                "--soffice", "soffice", "--report", str(self.report)]
        with patch("sys.argv", argv), patch.object(validation, "run", side_effect=runner), patch("sys.stdout", new=io.StringIO()):
            code = validation.main()
        return code, json.loads(self.report.read_text())

    def test_success_and_duplicate_stems_are_isolated(self):
        (self.input / "same.xlsx").write_bytes(b"another public placeholder")
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

    def test_empty_directory_fails(self):
        self.file.unlink()
        code, report = self.invoke(self.fake_run)
        self.assertEqual(code, 1)
        self.assertIn("No DOCX/XLSX/PPTX inputs found", report["errors"])


if __name__ == "__main__":
    unittest.main()
