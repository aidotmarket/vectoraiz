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

    def test_review_four_reproductions(self):
        cases = [
            {"word/document.xml": '<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:rsidR="Private"/>'},
            {"ppt/presentation.xml": '<a:hlinkClick xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" action="ppaction://macro?name=PrivateMacro"/>'},
            {"ppt/presentation.xml": '<a:fld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" id="Private" type="datetime"><a:t>Visible</a:t></a:fld>'},
            {"word/_rels/webSettings.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target="https://private.example/"/></Relationships>'},
        ]
        for parts in cases:
            with self.subTest(parts=parts):
                self.assertFalse(self.check(parts)["ok"])

    def test_independent_attributes_under_renamed_prefixes(self):
        cases = [
            ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "p", "http://schemas.openxmlformats.org/wordprocessingml/2006/main", "rsidR"),
            ("http://purl.oclc.org/ooxml/wordprocessingml/main", "p", "http://purl.oclc.org/ooxml/wordprocessingml/main", "rsidFuture"),
            ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "p", "http://schemas.openxmlformats.org/wordprocessingml/2006/main", "displacedByCustomXml"),
            ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "p", "http://schemas.microsoft.com/office/word/2010/wordml", "paraId"),
            ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "p", "http://schemas.microsoft.com/office/word/2010/wordml", "textId"),
            ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "p", "http://schemas.openxmlformats.org/markup-compatibility/2006", "Ignorable"),
            ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "p", "http://schemas.openxmlformats.org/markup-compatibility/2006", "PreserveAttributes"),
        ]
        for ns, local, ans, attr in cases:
            for prefix in ["n0", "x", "renamed"]:
                with self.subTest(attr=attr, prefix=prefix):
                    xml = f'<{local} xmlns="{ns}" xmlns:{prefix}="{ans}" {prefix}:{attr}="private"/>'
                    self.assertFalse(self.check({"word/document.xml": xml})["ok"])
        for ns, local, attr in [
            ("http://schemas.openxmlformats.org/spreadsheetml/2006/main", "workbookPr", "codeName"),
            ("http://schemas.openxmlformats.org/presentationml/2006/main", "sp", "macro"),
            ("http://purl.oclc.org/ooxml/presentationml/main", "sp", "textlink"),
            ("http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing", "sp", "macro"),
            ("http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing", "sp", "textlink"),
            ("http://schemas.openxmlformats.org/drawingml/2006/main", "hlinkClick", "action"),
            ("http://purl.oclc.org/ooxml/drawingml/main", "hlinkHover", "action"),
            ("http://schemas.openxmlformats.org/spreadsheetml/2006/main", "hyperlink", "location"),
        ]:
            for value in ["", "PrivateMacro"]:
                with self.subTest(ns=ns, attr=attr, value=value):
                    self.assertFalse(self.check({"custom/data.xml": f'<{local} xmlns="{ns}" {attr}="{value}"/>'})["ok"])
        self.assertFalse(self.check({"word/document.xml": '<w:hyperlink xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:anchor="private"/>'})["ok"])
        self.assertTrue(self.check({"custom/data.xml": '<p xmlns="urn:other" rsidR="safe" codeName="safe" macro="safe"/>'})["ok"])

    def test_independent_flattened_elements_and_chart_removals(self):
        cases = [
            ("http://schemas.openxmlformats.org/markup-compatibility/2006", ["AlternateContent", "Choice", "Fallback"]),
            ("http://schemas.openxmlformats.org/drawingml/2006/main", ["fld"]),
            ("http://purl.oclc.org/ooxml/drawingml/main", ["fld"]),
            ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", ["sdtContent", "sdtEndPr"]),
            ("http://schemas.openxmlformats.org/drawingml/2006/chart", ["pivotSource", "pivotFmts", "printSettings", "headerFooter", "protection", "userShapes", "extLst", "strRef", "numRef", "multiLvlStrRef"]),
            ("http://purl.oclc.org/ooxml/drawingml/chart", ["pivotSource", "pivotFmts", "printSettings", "headerFooter", "protection", "userShapes", "extLst", "strRef", "numRef", "multiLvlStrRef"]),
            ("http://schemas.openxmlformats.org/drawingml/2006/diagram", ["title", "desc", "catLst", "sampData", "styleData", "clrData"]),
            ("http://schemas.microsoft.com/office/drawing/2014/chartex", ["chartSpace"]),
        ]
        for ns, locals_ in cases:
            for local in locals_:
                for prefix in ["n0", "x", ""]:
                    tag = f"{prefix}:{local}" if prefix else local
                    decl = f"xmlns:{prefix}" if prefix else "xmlns"
                    with self.subTest(ns=ns, local=local, prefix=prefix):
                        self.assertFalse(self.check({"custom/data.xml": f'<{tag} {decl}="{ns}"/>'})["ok"])
        self.assertFalse(self.check({"word/settings.xml": '<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><m:mathPr xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"/></w:settings>'})["ok"])

    def test_independent_formula_and_empty_wrapper_cases(self):
        for ns in ["http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                   "http://purl.oclc.org/ooxml/spreadsheetml/main",
                   "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"]:
            for fragment in ['<f>PRIVATE()</f>', '<formula>PRIVATE()</formula>',
                             '<formula1>PRIVATE()</formula1>', '<formula2>PRIVATE()</formula2>',
                             '<calculatedColumnFormula>PRIVATE()</calculatedColumnFormula>',
                             '<totalsRowFormula>PRIVATE()</totalsRowFormula>',
                             '<cfvo type="formula" val="1"/>', '<cfvo val="PrivateName"/>',
                             '<dataValidation><formula1>PRIVATE()</formula1></dataValidation>',
                             '<cfRule><formula>PRIVATE()</formula></cfRule>',
                             '<dataValidations/>', '<conditionalFormatting/>',
                             '<conditionalFormattings/>', '<definedNames/>',
                             '<dataValidations count="9"><dataValidation/></dataValidations>']:
                with self.subTest(ns=ns, fragment=fragment):
                    self.assertFalse(self.check({"custom/data.xml": f'<root xmlns="{ns}">{fragment}</root>'})["ok"])
            self.assertTrue(self.check({"custom/data.xml": f'<cfvo xmlns="{ns}" type="num" val="3.5"/>'})["ok"])
        self.assertFalse(self.check({"custom/data.xml": '<x:f xmlns:x="http://schemas.microsoft.com/office/excel/2006/main">PRIVATE()</x:f>'})["ok"])
        self.assertFalse(self.check({"xl/workbook.xml": '<c xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" t="s"><v>0</v></c>'})["ok"])
        for ns in ["http://schemas.openxmlformats.org/spreadsheetml/2006/main", "http://schemas.openxmlformats.org/presentationml/2006/main"]:
            for xml in ['<ext/>', '<extLst/>', '<extLst><ext uri="private"><private xmlns="urn:private"/></ext></extLst>']:
                self.assertFalse(self.check({"custom/data.xml": f'<root xmlns="{ns}">{xml}</root>'})["ok"])

    def test_relationship_ownership_targets_and_external_policy(self):
        def rel(target, kind="hyperlink", mode="External"):
            return (f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
                    f'<Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" TargetMode="{mode}" Target="{target}"/></Relationships>')
        # These owners are independently listed, not taken from forbidden_part.
        for owner in ["word/webSettings.xml", "ppt/viewProps.xml", "xl/calcChain.xml",
                      "xl/sharedStrings.xml", "docProps/core.xml", "word/comments.xml",
                      "word/glossary/document.xml", "word/fonts/font.bin"]:
            directory, file = owner.rsplit("/", 1)
            with self.subTest(owner=owner):
                self.assertFalse(self.check({f"{directory}/_rels/{file}.rels": rel("https://private.example")})["ok"])
        for target in ["webSettings.xml", "../docProps/core.xml", "/xl/sharedStrings.xml",
                       "../xl/calcChain.xml", "comments.xml", "../customXml/item.xml"]:
            with self.subTest(target=target):
                result = self.check({"word/document.xml": "<root/>", "word/_rels/document.xml.rels": rel(target, "settings", "Internal")})
                self.assertTrue(any(v["reason"] == "relationship points to removed part" for v in result["violations"]))
        for target, kind in [("https://private.example", "attachedTemplate"), ("file:///private", "hyperlink"),
                             ("mailto:private@example.org", "hyperlink"), ("ppaction://macro", "hyperlink"),
                             ("https://private.example", "image"), ("https://private.example", "package"),
                             ("https://private.example", "oleObject"), ("https://private.example", "themeOverride")]:
            self.assertFalse(self.check({"_rels/.rels": rel(target, kind)})["ok"])
        self.assertTrue(self.check({"_rels/.rels": rel("https://public.example")})["ok"])
        self.assertFalse(self.check({"word/_rels/missing.xml.rels": rel("https://private.example")})["ok"])
        for target in ["../../outside.xml", "escaped%20name.xml", "missing.xml"]:
            self.assertFalse(self.check({"_rels/.rels": rel(target, "image", "Internal")})["ok"])
        self.assertFalse(self.check({"word/document.xml": '<root xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="removed"/>'})["ok"])

    def test_independent_media_and_binary_carriers(self):
        def png_chunk(kind, data=b""):
            # Chunk CRC is immaterial to metadata policy; no pixel decoder here.
            return len(data).to_bytes(4, "big") + kind + data + b"\0" * 4
        clean_png = b"\x89PNG\r\n\x1a\n" + png_chunk(b"IEND")
        for kind in [b"tEXt", b"iTXt", b"zTXt", b"eXIf", b"iCCP", b"acTL", b"ABCD"]:
            data = b"\x89PNG\r\n\x1a\n" + png_chunk(kind, b"private") + png_chunk(b"IEND")
            self.assertFalse(self.check({"word/media/part-000001.png": data})["ok"])
        self.assertFalse(self.check({"word/media/part-000001.png": clean_png + b"private"})["ok"])
        for marker, segment in [(0xe1, b"ExifPrivate"), (0xfe, b"PrivateComment"), (0xe2, b"ICC_PROFILEPrivate"),
                                (0xe0, b"JFIF\0private"), (0xee, b"AdobePrivate"), (0xe3, b"private")]:
            jpeg = b"\xff\xd8\xff" + bytes([marker]) + (len(segment) + 2).to_bytes(2, "big") + segment + b"\xff\xd9"
            self.assertFalse(self.check({"word/media/part-000001.jpg": jpeg})["ok"])
        self.assertFalse(self.check({"word/media/part-000001.jpg": b"\xff\xd8\xff\xd9private"})["ok"])
        for name, data in [("word/data.bin", b"PK\x03\x04private"),
                           ("word/data.bin", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1private"),
                           ("word/media/part-000001.svg", b"<svg/>"),
                           ("word/media/part-000001.png", b"broken"),
                           ("word/media/part-000001.jpg", b"broken")]:
            self.assertFalse(self.check({name: data})["ok"])

    def test_independent_smartart_source_and_hidden_carriers(self):
        ns = "http://schemas.openxmlformats.org/drawingml/2006/diagram"
        for attr in ["uniqueId", "loTypeId", "qsTypeId", "csTypeId", "presId"]:
            self.assertFalse(self.check({"custom/data.xml": f'<layoutDef xmlns="{ns}" {attr}="private-definition"/>'})["ok"])
        self.assertFalse(self.check({"custom/data.xml": f'<shape xmlns="{ns}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:blip=""/>'})["ok"])
        for fragment in ['<pt modelId="private"><t>undrawn private</t></pt>',
                         '<cxn srcId="removed" destId="absent"/>']:
            result = self.check({"word/diagrams/data-000001.xml": f'<dataModel xmlns="{ns}">{fragment}</dataModel>'})
            self.assertFalse(result["ok"])
            wanted = "undrawn/inconsistent SmartArt model text" if fragment.startswith("<pt") else "dangling SmartArt connection"
            self.assertIn(wanted, [v["reason"] for v in result["violations"]])

    def test_independent_chart_refusals_and_surviving_extensions(self):
        for local in ["filteredLineSeries", "filteredBarSeries", "categoryFilterExceptions", "categoryFilterException"]:
            self.assertFalse(self.check({"custom/data.xml": f'<x:{local} xmlns:x="http://schemas.microsoft.com/office/drawing/2012/chart"/>'})["ok"])
        for body in ["", '<plotVisOnly val="0"/>', '<plotVisOnly val="false"/>', '<plotVisOnly val="private"/>']:
            self.assertFalse(self.check({"custom/data.xml": f'<chart xmlns="http://schemas.openxmlformats.org/drawingml/2006/chart">{body}</chart>'})["ok"])
        self.assertTrue(self.check({"custom/data.xml": '<chart xmlns="http://schemas.openxmlformats.org/drawingml/2006/chart"><plotVisOnly val="1"/></chart>'})["ok"])
        for xml in [
            '<extLst xmlns="http://schemas.openxmlformats.org/presentationml/2006/main"><ext uri="sections"><p14:sectionLst xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main"/></ext></extLst>',
            '<extLst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><ext uri="rules"><x14:dataValidations xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" count="1"><x14:dataValidation/></x14:dataValidations></ext></extLst>',
        ]:
            self.assertTrue(self.check({"custom/data.xml": xml})["ok"])

    def test_serialization_part_and_type_table_removals(self):
        for name in ["xl/sharedStrings.xml", "word/xxcommentsPrivate.xml", "word/commentsPrivate.xml",
                     "word/vbaProjectSignature.bin", "word/headerPrivate.xml", "ppt/slides/private.xml",
                     "word/media/private.png", "../private.xml", "word/escaped%20name.xml"]:
            # xxcomments is deliberately not a pipeline prefix and stays legal.
            result = self.check({name: "<root/>"})
            self.assertEqual(result["ok"], name == "word/xxcommentsPrivate.xml")
            if name in {"word/headerPrivate.xml", "ppt/slides/private.xml", "word/media/private.png"}:
                self.assertIn("source part name survived neutral naming", [v["reason"] for v in result["violations"]])
        for xml in ["<root><!--private comment--></root>", "<root><?private secret?></root>",
                    '<!DOCTYPE root [<!ENTITY private "secret">]><root>&private;</root>']:
            self.assertFalse(self.check({"word/document.xml": xml})["ok"])
        for target in ["word/comments.xml", "word/missing.xml"]:
            types = f'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/{target}" ContentType="application/xml"/></Types>'
            self.assertFalse(self.check({"[Content_Types].xml": types})["ok"])

    def test_rule22_complete_neutral_paths_and_stems(self):
        chart = '<chartSpace xmlns="http://schemas.openxmlformats.org/drawingml/2006/chart"/>'
        for name in ["word/charts/PrivateClient/part-000001.xml",
                     "xl/charts/part-000001.xml", "word/charts/layout-000001.xml",
                     "xl/worksheets/PrivateClient/part-000001.xml",
                     "xl/tables/PrivateClient/part-000001.xml",
                     "xl/drawings/PrivateClient/part-000001.xml",
                     "xl/chartsheets/PrivateClient/part-000001.xml",
                     "ppt/slides/PrivateClient/part-000001.xml",
                     "ppt/slideLayouts/PrivateClient/part-000001.xml",
                     "ppt/slideMasters/PrivateClient/part-000001.xml",
                     "ppt/notesSlides/PrivateClient/part-000001.xml",
                     "ppt/notesMasters/PrivateClient/part-000001.xml",
                     "ppt/handoutMasters/PrivateClient/part-000001.xml",
                     "word/theme/PrivateClient/part-000001.xml",
                     "word/headerPrivate/header-000001.xml",
                     "word/footerPrivate/footer-000001.xml"]:
            with self.subTest(name=name):
                result = self.check({name: chart})
                self.assertFalse(result["ok"])
                self.assertTrue(any("neutral naming" in v["reason"] for v in result["violations"]))
        self.assertTrue(self.check({"word/charts/part-000001.xml": chart})["ok"])
        diagram = '<layoutDef xmlns="http://schemas.openxmlformats.org/drawingml/2006/diagram"/>'
        for name in ["word/diagrams/PrivateClient/layout-000001.xml",
                     "word/diagrams/part-000001.xml"]:
            self.assertFalse(self.check({name: diagram})["ok"])
        self.assertTrue(self.check({"word/diagrams/layout-000001.xml": diagram})["ok"])
        # Directory validation also applies to binaries.
        png = b"\x89PNG\r\n\x1a\n" + b"\0\0\0\0IEND\0\0\0\0"
        self.assertTrue(self.check({"word/media/part-000001.png": png})["ok"])
        self.assertFalse(self.check({"word/media/PrivateClient/part-000001.png": png})["ok"])

    def test_rule62_complete_numeric_diagram_urns(self):
        ns = "http://schemas.openxmlformats.org/drawingml/2006/diagram"
        for attr in ["uniqueId", "loTypeId", "qsTypeId", "csTypeId", "presId"]:
            for suffix in ["PrivateClient", "1PrivateClient", "1:PrivateClient", "01", "+1", "-1", "١", "1\n", ""]:
                with self.subTest(attr=attr, suffix=suffix):
                    result = self.check({"word/diagrams/layout-000001.xml":
                        f'<layoutDef xmlns="{ns}" {attr}="urn:vectoraiz:diagram:{suffix}"/>'})
                    self.assertIn("source SmartArt definition identifier", [v["reason"] for v in result["violations"]])
            for suffix in ["0", "1", "123456"]:
                self.assertTrue(self.check({"word/diagrams/layout-000001.xml":
                    f'<layoutDef xmlns="{ns}" {attr}="urn:vectoraiz:diagram:{suffix}"/>'})["ok"])

    def test_rule44_requires_direct_x14_rule_child(self):
        ns = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"
        xm = "http://schemas.microsoft.com/office/excel/2006/main"
        for body in [f'<xm:sqref xmlns:xm="{xm}">A1</xm:sqref>',
                     '<cfRule xmlns="urn:private"/>', '<wrapper><cfRule/></wrapper>']:
            result = self.check({"custom/data.xml": f'<conditionalFormatting xmlns="{ns}">{body}</conditionalFormatting>'})
            self.assertIn("conditional formatting without X14 cfRule", [v["reason"] for v in result["violations"]])
        self.assertTrue(self.check({"custom/data.xml": f'<conditionalFormatting xmlns="{ns}"><cfRule/></conditionalFormatting>'})["ok"])

    def test_rule42_numeric_cfvo_lexical_form(self):
        for ns in ["http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                   "http://purl.oclc.org/ooxml/spreadsheetml/main",
                   "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"]:
            for value in [" 1", "1 ", "1_000", "١", "NaN", "inf", "1e999", "1private", ""]:
                with self.subTest(ns=ns, value=value):
                    self.assertFalse(self.check({"custom/data.xml": f'<cfvo xmlns="{ns}" type="num" val="{value}"/>'})["ok"])
            for value in ["+1", "-.5", "1E-3", "3.5"]:
                self.assertTrue(self.check({"custom/data.xml": f'<cfvo xmlns="{ns}" type="num" val="{value}"/>'})["ok"])

    def test_rule43_empty_extension_child_in_other_namespace(self):
        # The source prunes childless ext children regardless of extLst namespace.
        for xml, reason in [
                ('<extLst xmlns="urn:other"><ext uri="PrivateClient"/></extLst>', "empty extension child"),
                ('<extLst xmlns="urn:other"/>', "empty extension wrapper")]:
            result = self.check({"custom/data.xml": xml})
            self.assertIn(reason, [v["reason"] for v in result["violations"]])

    def test_rule56_visibility_uses_first_setting_and_content(self):
        ns = "http://schemas.openxmlformats.org/drawingml/2006/chart"
        rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
        for xml in [f'<chart xmlns="{ns}"><plotVisOnly val="0"/><plotVisOnly val="1"/></chart>',
                    f'<chart xmlns="{ns}" xmlns:r="{rel}" r:private="secret"/>',
                    f'<chart xmlns="{ns}" xmlns:r="{rel}" r:id="r1"><plotVisOnly val="0"/></chart>']:
            result = self.check({"custom/data.xml": xml})
            self.assertIn("missing/unsafe chart visibility setting", [v["reason"] for v in result["violations"]])

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
