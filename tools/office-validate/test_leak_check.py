"""Synthetic package tests: prefixes must never control leak detection."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import leak_check

CONTENT_TYPES = '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>'


class LeakCheckTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.path = self.root / "delivered.docx"

    def check(self, parts):
        with zipfile.ZipFile(self.path, "w") as package:
            if "[Content_Types].xml" not in parts:
                package.writestr("[Content_Types].xml", CONTENT_TYPES)
            for name, content in parts.items():
                package.writestr(name, content)
        return leak_check.check_file(self.path, self.root)

    def test_planted_n0_prefix_catches_every_forbidden_element(self):
        for tag in sorted(leak_check.FORBIDDEN | leak_check.IDENTITY):
            namespace, local = tag[1:].split("}")
            with self.subTest(tag=tag):
                result = self.check({"custom/part.xml":
                    f'<n0:{local} xmlns:n0="{namespace}">planted</n0:{local}>'})
                self.assertFalse(result["ok"])
                self.assertEqual(result["violations"][0]["element"], tag)

    def test_default_namespace_is_caught(self):
        result = self.check({"word/document.xml":
            '<ins xmlns="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>'})
        self.assertFalse(result["ok"])

    def test_matching_local_names_in_other_namespaces_are_allowed(self):
        xml = '<root xmlns:n0="urn:unrelated"><n0:ins/><n0:f/><n0:creator>safe</n0:creator></root>'
        self.assertTrue(self.check({"custom/data.xml": xml})["ok"])

    def test_empty_identity_is_allowed_but_nested_text_is_not(self):
        for tag in leak_check.IDENTITY:
            namespace, local = tag[1:].split("}")
            for text, expected in [("  ", True), ("<value>identity</value>", False)]:
                with self.subTest(tag=tag, text=text):
                    xml = f'<n0:{local} xmlns:n0="{namespace}">{text}</n0:{local}>'
                    self.assertEqual(self.check({"custom/data.xml": xml})["ok"], expected)

    def test_forbidden_paths_at_any_depth(self):
        paths = ["docProps/core.xml", "docProps/custom.xml", "word/comments.xml",
                 "word/commentsExtended.xml", "word/commentsIds.xml",
                 "word/commentsExtensible.xml", "ppt/people.xml",
                 "media/thumbnail.jpeg", "xl/vbaProject.bin", "xl/embeddings/data.bin",
                 "xl/externalLinks/externalLink1.xml", "ppt/activeX/activeX1.bin"]
        for name in paths:
            with self.subTest(name=name):
                result = self.check({name: '<root/>'})
                self.assertFalse(result["ok"])
                self.assertEqual(result["violations"][0]["reason"], "forbidden part path")
        self.assertTrue(self.check({"custom/mycomments.xml": "<root/>"})["ok"])

    def test_hidden_sheets_are_info_only(self):
        for ns in leak_check.SHEET:
            xml = (f'<n0:workbook xmlns:n0="{ns}"><n0:sheets>'
                   '<n0:sheet name="Hidden" state="hidden"/>'
                   '<n0:sheet name="Very hidden" state="veryHidden"/>'
                   '<n0:sheet name="Visible" state="visible"/>'
                   '</n0:sheets></n0:workbook>')
            result = self.check({"xl/workbook.xml": xml})
            self.assertTrue(result["ok"])
            self.assertEqual([i["state"] for i in result["info"]], ["hidden", "veryHidden"])

    def test_malformed_xml_does_not_hide_other_violations(self):
        result = self.check({"word/document.xml": "<broken>", "word/comments.xml": "<root/>"})
        self.assertFalse(result["ok"])
        self.assertEqual(len(result["violations"]), 2)
        self.assertIn("ParseError", result["violations"][0]["reason"])

    def test_declared_xml_with_other_extension_is_checked(self):
        types = ('<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                 '<Default Extension="data" ContentType="application/xml"/>'
                 '<Override PartName="/custom/other.bin" ContentType="application/example+xml"/>'
                 '</Types>')
        xml = '<n0:ins xmlns:n0="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>'
        result = self.check({"[Content_Types].xml": types, "custom/part.data": xml,
                             "custom/other.bin": xml})
        self.assertFalse(result["ok"])
        self.assertEqual(len(result["violations"]), 2)

    def test_missing_content_types_fails(self):
        with zipfile.ZipFile(self.path, "w") as package:
            package.writestr("word/document.xml", "<root/>")
        self.assertFalse(leak_check.check_file(self.path, self.root)["ok"])

    def test_invalid_zip_fails(self):
        self.path.write_bytes(b"invalid zip")
        result = leak_check.check_file(self.path, self.root)
        self.assertFalse(result["ok"])
        self.assertIn("BadZipFile", result["violations"][0]["reason"])

    def test_cli_recursive_report_preserves_all_files_and_fails(self):
        self.check({"custom/data.xml": "<root/>"})
        nested = self.root / "nested"
        nested.mkdir()
        bad = nested / "leaking.XLSX"
        with zipfile.ZipFile(bad, "w") as package:
            package.writestr("[Content_Types].xml", CONTENT_TYPES)
            package.writestr("xl/comments1.xml", "<root/>")
        report = self.root / "report.json"
        with patch("sys.argv", ["leak_check.py", str(self.root), "--report", str(report)]):
            self.assertEqual(leak_check.main(), 1)
        data = json.loads(report.read_text())
        self.assertFalse(data["ok"])
        self.assertEqual([r["ok"] for r in data["files"]], [True, False])
        self.assertTrue(all(len(r["sha256"]) == 64 for r in data["files"]))

    def test_empty_or_missing_input_fails(self):
        report = self.root / "report.json"
        for root in [self.root, self.root / "missing"]:
            with patch("sys.argv", ["leak_check.py", str(root), "--report", str(report)]):
                self.assertEqual(leak_check.main(), 1)


if __name__ == "__main__":
    unittest.main()
