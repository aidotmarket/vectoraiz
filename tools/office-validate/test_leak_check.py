"""Synthetic package tests: prefixes must never control leak detection."""
import io
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
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as package:
            if "[Content_Types].xml" not in parts:
                package.writestr("[Content_Types].xml", CONTENT_TYPES)
            for name, content in parts.items():
                package.writestr(name, content)
        self.path.write_bytes(buffer.getvalue())
        return leak_check.check_file(self.path, self.root)

    def test_independently_enumerated_revision_and_field_carriers(self):
        # Hand-written fixtures: never derive test cases from the checker.
        carriers = ["ins", "del", "moveFrom", "moveTo", "moveFromRangeStart",
                    "moveFromRangeEnd", "moveToRangeStart", "moveToRangeEnd",
                    "customXmlInsRangeStart", "customXmlDelRangeStart",
                    "rPrChange", "pPrChange", "sectPrChange", "tblPrChange",
                    "tblPrExChange", "trPrChange", "tcPrChange", "tblGridChange",
                    "numberingChange", "cellIns", "cellDel", "cellMerge",
                    "commentReference", "commentRangeStart", "commentRangeEnd",
                    "docVars", "rsids", "documentProtection", "writeProtection",
                    "attachedTemplate", "trackRevisions", "altChunk",
                    "embedTrueTypeFonts", "embedSystemFonts", "saveSubsetFonts", "paperSrc",
                    "savePreviewPicture", "printerSettings", "rsid", "aliases", "numIdMacAtCleanup",
                    "formsDesign", "proofErr", "shapeDefaults", "hdrShapeDefaults",
                    "doNotIncludeSubdocsInStats", "embedRegular", "embedBold", "embedItalic",
                    "embedBoldItalic", "glossaryDocument", "instrText", "fldChar", "fldSimple", "sdt", "sdtPr"]
        for ns in ["http://schemas.openxmlformats.org/wordprocessingml/2006/main",
                   "http://purl.oclc.org/ooxml/wordprocessingml/main"]:
            for local in carriers:
                for prefix in ["n0", "x", ""]:
                    with self.subTest(namespace=ns, carrier=local, prefix=prefix):
                        tag = f"{prefix}:{local}" if prefix else local
                        declaration = f'xmlns:{prefix}' if prefix else 'xmlns'
                        # Each case is a separate package with identity and text.
                        xml = (f'<{tag} {declaration}="{ns}" author="Private Author">'
                               f'private moved/revised content</{tag}>')
                        result = self.check({"word/document.xml": xml})
                        self.assertFalse(result["ok"])
                        self.assertEqual(result["violations"][0]["element"], f"{{{ns}}}{local}")

    def test_independently_enumerated_chart_and_identity_carriers(self):
        cases = [
            ("http://schemas.openxmlformats.org/drawingml/2006/chart", "f"),
            ("http://schemas.openxmlformats.org/drawingml/2006/chart", "externalData"),
            ("http://purl.oclc.org/ooxml/drawingml/chart", "f"),
            ("http://purl.oclc.org/ooxml/drawingml/chart", "externalData"),
            ("http://purl.org/dc/elements/1.1/", "creator"),
            ("http://schemas.openxmlformats.org/package/2006/metadata/core-properties", "lastModifiedBy")]
        for ns, local in cases:
            for prefix in ["n0", "x", ""]:
                with self.subTest(namespace=ns, carrier=local, prefix=prefix):
                    tag = f"{prefix}:{local}" if prefix else local
                    declaration = f'xmlns:{prefix}' if prefix else 'xmlns'
                    self.assertFalse(self.check({"custom/part.xml":
                        f'<{tag} {declaration}="{ns}">private</{tag}>'})["ok"])

    def test_independent_spreadsheet_and_presentation_carriers(self):
        cases = [
            (["http://schemas.openxmlformats.org/spreadsheetml/2006/main",
              "http://purl.oclc.org/ooxml/spreadsheetml/main"],
             ["legacyDrawing", "legacyDrawingHF", "fileVersion", "fileRecoveryPr",
              "sheetCalcPr", "mruColors", "calcPr", "calcChain", "fileSharing",
              "customWorkbookViews", "pivotCaches", "pivotTableParts", "phoneticPr", "rPh",
              "sheetProtection", "workbookProtection", "definedName", "f"]),
            (["http://schemas.openxmlformats.org/presentationml/2006/main",
              "http://purl.oclc.org/ooxml/presentationml/main"],
             ["clrMru", "printPr", "custDataLst", "custData", "tags", "embeddedFontLst"]),
            (["http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac"], ["absPath"])]
        for namespaces, carriers in cases:
            for ns in namespaces:
                for local in carriers:
                    for prefix in ["n0", "x", ""]:
                        with self.subTest(namespace=ns, carrier=local, prefix=prefix):
                            tag = f"{prefix}:{local}" if prefix else local
                            declaration = f'xmlns:{prefix}' if prefix else 'xmlns'
                            self.assertFalse(self.check({"custom/part.xml":
                                f'<{tag} {declaration}="{ns}">private</{tag}>'})["ok"])

    def test_matching_local_names_in_other_namespaces_are_allowed(self):
        xml = '<root xmlns:n0="urn:unrelated"><n0:ins/><n0:f/><n0:creator>safe</n0:creator></root>'
        self.assertTrue(self.check({"custom/data.xml": xml})["ok"])

    def test_empty_identity_is_allowed_but_nested_text_is_not(self):
        for tag in ["{http://purl.org/dc/elements/1.1/}creator",
                    "{http://schemas.openxmlformats.org/package/2006/metadata/core-properties}lastModifiedBy"]:
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
                 "xl/externalLinks/externalLink1.xml", "ppt/activeX/activeX1.bin",
                 "xl/threadedComments/threadedComment1.xml", "xl/persons/person.xml",
                 "ppt/commentAuthors.xml", "ppt/comments/comment1.xml", "word/people.xml",
                 "customXml/item1.xml", "customXml/itemProps1.xml", "customXml/_rels/item1.xml.rels",
                 "[trash]/private.xml", "docMetadata/identity.xml", "word/webSettings.xml",
                 "ppt/viewProps.xml", "xl/printerSettings/printerSettings1.bin",
                 "xl/pivotCache/pivotCacheDefinition1.xml", "xl/pivotTables/pivotTable1.xml",
                 "xl/revisions/revision1.xml", "word/webextensions/webextension1.xml",
                 "ppt/tags/tag1.xml", "xl/revisionInfo.xml", "xl/changesInfos/info.xml",
                 "word/glossary/document.xml", "word/fonts/font1.odttf", "word/ink/ink1.xml",
                 "xl/drawings/vmlDrawing1.vml", "xl/calcChain.xml", "_xmlsignatures/sig1.xml"]
        for name in paths:
            with self.subTest(name=name):
                result = self.check({name: '<root author="Private Author">private content</root>'})
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
