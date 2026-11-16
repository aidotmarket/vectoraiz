"""Namespace-aware, read-only leak checks for delivered Open XML packages."""
import argparse
import hashlib
import json
import math
import re
from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile

EXTENSIONS = {".docx", ".xlsx", ".pptx"}
WORD = {"http://schemas.openxmlformats.org/wordprocessingml/2006/main",
        "http://purl.oclc.org/ooxml/wordprocessingml/main"}
CHART = {"http://schemas.openxmlformats.org/drawingml/2006/chart",
         "http://purl.oclc.org/ooxml/drawingml/chart"}
SHEET = {"http://schemas.openxmlformats.org/spreadsheetml/2006/main",
         "http://purl.oclc.org/ooxml/spreadsheetml/main"}
SHEET_TAGS = {f"{{{ns}}}sheet" for ns in SHEET}
DRAWING = {"http://schemas.openxmlformats.org/drawingml/2006/main",
           "http://purl.oclc.org/ooxml/drawingml/main"}
PRESENTATION = {"http://schemas.openxmlformats.org/presentationml/2006/main",
                "http://purl.oclc.org/ooxml/presentationml/main"}
XDR = {"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing",
       "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing"}
MC = "http://schemas.openxmlformats.org/markup-compatibility/2006"
X14 = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"
XM = "http://schemas.microsoft.com/office/excel/2006/main"
P14 = "http://schemas.microsoft.com/office/powerpoint/2010/main"
DGM = "http://schemas.openxmlformats.org/drawingml/2006/diagram"
DSP = "http://schemas.microsoft.com/office/drawing/2008/diagram"
REL = "http://schemas.openxmlformats.org/package/2006/relationships"
R = {"http://schemas.openxmlformats.org/officeDocument/2006/relationships",
     "http://purl.oclc.org/ooxml/officeDocument/relationships"}
# Source sites, helper rules, regressions and observability limits:
# REMOVAL_RULES.md (line numbers are bound to the documented source revision).
WORD_PRIVATE = """ins del moveFrom moveTo moveFromRangeStart moveFromRangeEnd
moveToRangeStart moveToRangeEnd customXmlInsRangeStart customXmlDelRangeStart
commentReference commentRangeStart commentRangeEnd docVars rsids
 documentProtection writeProtection attachedTemplate trackRevisions altChunk
 embedTrueTypeFonts embedSystemFonts saveSubsetFonts paperSrc savePreviewPicture
printerSettings rsid aliases numIdMacAtCleanup formsDesign proofErr shapeDefaults
hdrShapeDefaults doNotIncludeSubdocsInStats embedRegular embedBold embedItalic embedBoldItalic
 glossaryDocument cellIns cellDel cellMerge instrText fldChar fldSimple sdt sdtPr sdtEndPr sdtContent""".split()
FORBIDDEN = ({f"{{{ns}}}{name}" for ns in WORD for name in WORD_PRIVATE} |
             {f"{{{ns}}}{name}" for ns in CHART for name in
              "f externalData pivotSource pivotFmts printSettings headerFooter protection userShapes extLst strRef numRef multiLvlStrRef".split()} |
             {f"{{{ns}}}fld" for ns in DRAWING} |
             {f"{{{ns}}}{name}" for ns in SHEET for name in
              "legacyDrawing legacyDrawingHF fileVersion fileRecoveryPr sheetCalcPr mruColors calcPr calcChain fileSharing customWorkbookViews pivotCaches pivotTableParts phoneticPr rPh sheetProtection workbookProtection definedName f".split()} |
             {f"{{{ns}}}{name}" for ns in
              ("http://schemas.openxmlformats.org/presentationml/2006/main",
               "http://purl.oclc.org/ooxml/presentationml/main") for name in
              "clrMru printPr custDataLst custData tags embeddedFontLst".split()} |
             {"{http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac}absPath"})


def forbidden_element(tag):
    if tag in FORBIDDEN:
        return True
    if tag.startswith("{"):
        ns, local = tag[1:].split("}", 1)
        # Property revisions include rPrChange/pPrChange/sectPrChange and future
        # *Change carriers; none can occur in the delivery allowlist.
        return ns in WORD and local.endswith("Change")
    return False


IDENTITY = {"{http://purl.org/dc/elements/1.1/}creator",
            "{http://schemas.openxmlformats.org/package/2006/metadata/core-properties}lastModifiedBy"}


def forbidden_part(name):
    # Match part components anywhere in the package, independently of XML prefixes.
    name = name.replace("\\", "/").lower()
    if name.endswith(".rels") and name != "_rels/.rels":
        try:
            if forbidden_part(relationship_owner(name)):
                return True
        except ValueError:
            pass
    parts = name.split("/")
    return (any(p in {"docprops", "customxml", "[trash]", "docmetadata",
                     "embeddings", "externallinks", "activex", "_xmlsignatures",
                     "threadedcomments", "persons", "printersettings", "pivotcache",
                     "pivottables", "revisions", "webextensions", "tags", "changesinfos",
                     "glossary", "fonts", "ink"} for p in parts[:-1]) or
            any(p.startswith(("comments", "thumbnail", "vbaproject")) or
                p in {"people.xml", "commentauthors.xml", "revisioninfo.xml"}
                for p in parts) or
            any(s in name for s in ("/comments", "/threadedcomments", "vbaproject")) or
            name in {"word/websettings.xml", "ppt/viewprops.xml", "xl/calcchain.xml", "xl/sharedstrings.xml"} or
            ("/".join(parts).startswith("xl/drawings/") and parts[-1].endswith(".vml")))


def expanded(tag):
    return tag[1:].split("}", 1) if tag.startswith("{") else ("", tag)


def part_reasons(name, package_root, tree=None):
    if (not name.isascii() or any(ord(c) < 32 for c in name)
            or any(c in name for c in "\\:\0?#%") or name.startswith("/")
            or any(c in {"", ".", ".."} for c in name.split("/"))):
        yield "unsafe/noncanonical part path"
    if name.endswith(".rels"):
        return  # Owner name is checked separately, including absence.
    dynamic = ("xl/worksheets/", "xl/tables/", "xl/drawings/", "xl/chartsheets/",
               "ppt/slides/", "ppt/slideLayouts/", "ppt/slideMasters/", "ppt/notesSlides/",
               "ppt/notesMasters/", "ppt/handoutMasters/")
    if (name.startswith(dynamic) or any(s in name for s in ("/media/", "/theme/", "/charts/", "/diagrams/"))
            or name.startswith(("word/header", "word/footer"))):
        directory, filename = name.rsplit("/", 1)
        stem = "part"
        if "/media/" in name:
            expected_directory = name.split("/", 1)[0] + "/media"
        elif "/charts/" in name or "/diagrams/" in name or name.startswith("xl/chartsheets/"):
            family = "diagrams" if "/diagrams/" in name else "chartsheets" if name.startswith("xl/chartsheets/") else "charts"
            expected_directory = f"{package_root}/{family}"
        elif "/theme/" in name:
            expected_directory = name.split("/", 1)[0] + "/theme"
        elif name.startswith(("word/header", "word/footer")):
            expected_directory = "word"
        else:
            expected_directory = next(p.rstrip("/") for p in dynamic if name.startswith(p))
        if "/diagrams/" in name:
            stem = {"dataModel": "data", "layoutDef": "layout", "styleDef": "quickStyle",
                    "colorsDef": "colors", "drawing": "drawing"}.get(
                        expanded(tree.tag)[1] if tree is not None else "", "INVALID")
        elif "/charts/" in name and tree is not None:
            stem = {"chartStyle": "style", "colorStyle": "colors"}.get(expanded(tree.tag)[1], "part")
        elif name.startswith("word/header"):
            stem = "header"
        elif name.startswith("word/footer"):
            stem = "footer"
        if directory != expected_directory:
            yield "source part directory survived neutral naming"
        if not re.fullmatch(re.escape(stem) + r"-[0-9]{6}\.[a-z0-9]+", filename):
            yield "source part name survived neutral naming"


def relationship_owner(name):
    if name == "_rels/.rels":
        return ""
    if "/_rels/" not in name or not name.endswith(".rels"):
        raise ValueError("invalid relationships part path")
    directory, filename = name.rsplit("/_rels/", 1)
    return directory + "/" + filename[:-5]


def resolve_target(owner, target):
    if any(c in target for c in "\\:\0?%"):
        raise ValueError("unsupported relationship URI")
    target = target.split("#", 1)[0]
    parts = [] if target.startswith("/") else owner.split("/")[:-1]
    for component in target.split("/"):
        if component in {"", "."}:
            continue
        if component == "..":
            if not parts:
                raise ValueError("relationship escapes package root")
            parts.pop()
        else:
            parts.append(component)
    if not parts:
        raise ValueError("empty relationship target")
    return "/".join(parts)


def formula_element(element):
    ns, local = expanded(element.tag)
    if ns in SHEET | {X14}:
        if local in {"f", "formula", "formula1", "formula2",
                     "calculatedColumnFormula", "totalsRowFormula"}:
            return True
        if local == "cfvo":
            value = element.get("val")
            try:
                numeric = value is None or (re.fullmatch(r"[0-9+.eE-]+", value) is not None
                                            and math.isfinite(float(value)))
            except ValueError:
                numeric = False
            return element.get("type") == "formula" or not numeric
    return ns == XM and local == "f"


def element_reasons(element, parent):
    ns, local = expanded(element.tag)
    if ns == MC:
        yield "unresolved compatibility element"
    if formula_element(element):
        yield "formula carrier"
    if ns in DRAWING and local in {"hlinkClick", "hlinkHover"} and "action" in element.attrib:
        yield "PowerPoint action"
    if ns == "http://schemas.openxmlformats.org/officeDocument/2006/math" and local == "mathPr" and parent is not None and expanded(parent.tag)[0] in WORD:
        yield "Word math settings"
    if ns == DGM and local in {"title", "desc", "catLst", "sampData", "styleData", "clrData"}:
        yield "SmartArt definition free text"
    if "/chartex" in ns or "/chartEx" in ns:
        yield "unsupported extended chart"
    if (ns.startswith("http://schemas.microsoft.com/office/drawing/") and ns.endswith("/chart")
            and (local.startswith("filtered") or local in {"categoryFilterExceptions", "categoryFilterException"})):
        yield "filtered chart content"
    if ns in CHART and local == "chart":
        reference_only = (not len(element) and not (element.text or "").strip()
                          and len(element.attrib) == 1
                          and any(k == f"{{{r}}}id" for r in R for k in element.attrib))
        setting = next((c for c in element if c.tag == f"{{{ns}}}plotVisOnly"), None)
        if not reference_only and (setting is None or setting.get("val") not in {None, "1", "true"}):
            yield "missing/unsafe chart visibility setting"
    if local == "extLst":
        if not len(element):
            yield "empty extension wrapper"
        for ext in element:
            if not len(ext):
                yield "empty extension child"
            for child in ext:
                cns, clocal = expanded(child.tag)
                if not ((cns == DSP and clocal == "dataModelExt" and ext.get("uri") == DSP)
                        or (cns == P14 and clocal == "sectionLst")
                        or (cns == X14 and clocal in {"dataValidations", "conditionalFormattings"})):
                    yield "unsupported extension content"
    if ns in SHEET | PRESENTATION and local in {"ext", "extLst"} and not len(element):
        yield "empty extension wrapper"
    if ns == X14 and local == "conditionalFormatting" and not any(
            child.tag == f"{{{X14}}}cfRule" for child in element):
        yield "conditional formatting without X14 cfRule"
    if ns in SHEET | {X14} and local in {"dataValidations", "conditionalFormatting", "conditionalFormattings", "definedNames"}:
        if not len(element):
            yield "empty formula-rule wrapper"
        if local == "dataValidations" and element.get("count") != str(len(element)):
            yield "stale validation count"
    if ns in SHEET and local == "c" and element.get("t") == "s":
        yield "unflattened shared string"
    for key, value in element.attrib.items():
        ans, attr = expanded(key)
        if (ans in WORD and (attr.startswith("rsid") or attr == "displacedByCustomXml")
                or ans == MC
                or ans == "http://schemas.microsoft.com/office/word/2010/wordml" and attr in {"paraId", "textId"}
                or ns in SHEET and not ans and attr == "codeName"
                or ns in PRESENTATION | XDR and not ans and attr in {"macro", "textlink"}
                or ns in WORD and local == "hyperlink" and ans in WORD and attr == "anchor"
                or ns in SHEET and local == "hyperlink" and not ans and attr == "location"
                or ns == DGM and local == "shape" and ans in R and attr == "blip" and not value):
            yield f"forbidden attribute {key}"
        if ns == DGM and attr in {"uniqueId", "loTypeId", "qsTypeId", "csTypeId", "presId"} and not re.fullmatch(r"urn:vectoraiz:diagram:(?:0|[1-9][0-9]*)", value):
            yield "source SmartArt definition identifier"


def media_reasons(data, name):
    """Check metadata segments without searching compressed pixel bytes."""
    if name.lower().endswith(".png"):
        if not data.startswith(b"\x89PNG\r\n\x1a\n"):
            raise ValueError("invalid PNG signature")
        pos = 8
        while pos + 12 <= len(data):
            size = int.from_bytes(data[pos:pos + 4], "big")
            kind = data[pos + 4:pos + 8]
            pos += size + 12
            if pos > len(data):
                raise ValueError("truncated PNG chunk")
            if kind not in {b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"sRGB", b"gAMA", b"cHRM", b"bKGD"}:
                yield "PNG metadata/unsupported chunk"
            if kind == b"IEND":
                if pos != len(data):
                    yield "image trailing bytes"
                return
        raise ValueError("PNG lacks IEND")
    if not name.lower().endswith((".jpg", ".jpeg")):
        yield "unsupported media format"
        return
    if not data.startswith(b"\xff\xd8"):
        raise ValueError("invalid JPEG signature")
    pos = 2
    while pos < len(data):
        if data[pos] != 255:
            raise ValueError("invalid JPEG marker")
        while pos < len(data) and data[pos] == 255:
            pos += 1
        marker = data[pos]
        pos += 1
        if marker == 0xd9:
            if pos != len(data):
                yield "image trailing bytes"
            return
        size = int.from_bytes(data[pos:pos + 2], "big")
        segment = data[pos + 2:pos + size]
        if size < 2 or pos + size > len(data):
            raise ValueError("invalid JPEG segment length")
        if marker == 0xe0 and segment.startswith(b"JFIF\0"):
            if segment != bytes([74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0]):
                yield "non-neutral JFIF metadata"
        elif marker == 0xee and segment.startswith(b"Adobe"):
            if len(segment) != 12 or segment[:11] != b"Adobe\x00\x64\x00\x00\x00\x00" or segment[11] > 2:
                yield "non-neutral Adobe metadata"
        elif marker not in {0xc0, 0xc1, 0xc2, 0xc4, 0xda, 0xdb, 0xdd}:
            yield "JPEG metadata/unsupported marker"
        pos += size
        if marker == 0xda:
            while pos + 1 < len(data):
                if data[pos] != 255:
                    pos += 1
                elif data[pos + 1] in {0, 255} or 0xd0 <= data[pos + 1] <= 0xd7:
                    pos += 2
                else:
                    break
    raise ValueError("JPEG lacks EOI")



def check_relationships(trees, names, violations):
    for name, tree in trees.items():
        if not name.endswith(".rels"):
            continue
        def fail(reason):
            violations.append({"part": name, "reason": reason})
        try:
            owner = relationship_owner(name)
            if owner and (owner not in names or forbidden_part(owner)):
                fail("relationships owned by absent/removed part")
            ids = set()
            referenced = {v for e in trees[owner].iter() for k, v in e.attrib.items()
                          if expanded(k)[0] in R} if owner in trees else set()
            for rel in tree:
                if rel.tag != f"{{{REL}}}Relationship":
                    fail("unsupported relationship namespace/element")
                    continue
                identity, target, ty = (rel.get(k) for k in ("Id", "Target", "Type"))
                if not identity or identity in ids or not target or not ty:
                    fail("missing/duplicate relationship identity, target or type")
                    continue
                ids.add(identity)
                kind = ty.rsplit("/", 1)[-1]
                if rel.get("TargetMode") == "External":
                    if (kind != "hyperlink" or not target.lower().startswith(("http://", "https://"))
                            or not any(ty == ns + "/hyperlink" for ns in R)):
                        fail("removed/refused external relationship")
                else:
                    if rel.get("TargetMode") not in {None, "Internal"}:
                        fail("unknown relationship mode")
                    resolved = resolve_target(owner, target)
                    if forbidden_part(resolved):
                        fail("relationship points to removed part")
                    elif resolved not in names:
                        fail("dangling relationship target")
                if kind in {"themeOverride", "package", "oleObject", "control", "vbaProject"}:
                    fail("removed/refused graphics dependency")
                if kind in {"image", "chart", "diagramData", "diagramLayout", "diagramQuickStyle", "diagramColors", "hyperlink"} and owner and identity not in referenced:
                    fail("unused graphics/hyperlink relationship")
            if referenced - ids:
                fail("dangling content relationship reference")
        except Exception as error:
            fail(f"{type(error).__name__}: {error}")
    # Content may still reference a relationship whose entire .rels was removed.
    for name, tree in trees.items():
        if name.endswith(".rels"):
            continue
        refs = [v for e in tree.iter() for k, v in e.attrib.items() if expanded(k)[0] in R]
        if refs:
            directory, filename = name.rsplit("/", 1) if "/" in name else ("", name)
            relpath = f"{directory}/_rels/{filename}.rels"
            if relpath not in trees:
                violations.append({"part": name, "reason": "content references absent relationships"})


def check_models(trees, violations):
    """A textual SmartArt point must map to a displayed cached shape."""
    for name, tree in trees.items():
        if tree.tag != f"{{{DGM}}}dataModel":
            continue
        targets = set()
        ids = {e.get("relId") for e in tree.iter(f"{{{DSP}}}dataModelExt")}
        owners = {name}
        for rp, rels in trees.items():
            if not rp.endswith(".rels"):
                continue
            try:
                own = relationship_owner(rp)
                for rel in rels:
                    if rel.get("Type", "").endswith("/diagramData") and rel.get("TargetMode") != "External" and resolve_target(own, rel.get("Target", "")) == name:
                        owners.add(own)
            except ValueError:
                continue  # Already diagnosed by check_relationships.
        for rp, rels in trees.items():
            if not rp.endswith(".rels"):
                continue
            try:
                own = relationship_owner(rp)
                if own in owners:
                    for rel in rels:
                        if rel.get("Id") in ids and rel.get("Type", "").endswith("/diagramDrawing") and rel.get("TargetMode") != "External":
                            targets.add(resolve_target(own, rel.get("Target", "")))
            except ValueError:
                continue
        cache = {}
        for target in targets:
            if target in trees:
                cache.update({e.get("modelId"): "".join(e.itertext()).strip()
                              for e in trees[target].iter(f"{{{DSP}}}sp")})
        if len(targets) != 1 or not cache:
            violations.append({"part": name, "reason": "missing/ambiguous SmartArt drawing cache"})
        connections = list(tree.iter(f"{{{DGM}}}cxn"))
        for point in tree.iter(f"{{{DGM}}}pt"):
            text = "".join(point.itertext()).strip()
            if not text:
                continue
            mapped = [c.get("destId") for c in connections
                      if c.get("type") == "presOf" and c.get("srcId") == point.get("modelId")
                      and c.get("destId") in cache]
            if not mapped or any(cache[dest].strip() not in {"", text} for dest in mapped):
                violations.append({"part": name, "reason": "undrawn/inconsistent SmartArt model text"})
        point_ids = {p.get("modelId") for p in tree.iter(f"{{{DGM}}}pt")}
        for connection in connections:
            if any(connection.get(key) not in point_ids for key in ("srcId", "destId")) or any(connection.get(key) is not None and connection.get(key) not in point_ids for key in ("parTransId", "sibTransId")):
                violations.append({"part": name, "reason": "dangling SmartArt connection"})


def check_source_removals(trees, names, source, violations):
    """Check source-derived identities even if copied into an allowed carrier."""
    for name in source["parts"]:
        if name in names:
            violations.append({"part": name, "reason": "source removed part survives"})
    # Joining text catches comment text split across runs; parsing decodes entities.
    values = []
    for tree in trees.values():
        values.append(" ".join("".join(tree.itertext()).split()))
        values.extend(" ".join(v.split()) for e in tree.iter() for v in e.attrib.values())
    for text in source["text"]:
        needle = " ".join(text.split())
        if any(needle in value for value in values):
            violations.append({"reason": "source removed text survives", "text": text})


def check_file(path, root, source=None):
    result = {"path": path.relative_to(root).as_posix(), "sha256": None,
              "ok": False, "violations": [], "info": []}
    try:
        result["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
        with zipfile.ZipFile(path) as package:
            names = set(package.namelist())
            trees = {}
            if len(names) != len(package.infolist()):
                result["violations"].append({"reason": "duplicate ZIP member"})
            content_types = ET.fromstring(package.read("[Content_Types].xml"))
            defaults = {}
            overrides = {}
            for entry in content_types:
                mime = entry.get("ContentType", "").lower()
                is_xml = mime.endswith(("+xml", "/xml"))
                if entry.tag == "{http://schemas.openxmlformats.org/package/2006/content-types}Default":
                    defaults[entry.get("Extension", "").lower()] = is_xml
                elif entry.tag == "{http://schemas.openxmlformats.org/package/2006/content-types}Override":
                    overrides[entry.get("PartName", "").lstrip("/")] = is_xml
                    target = entry.get("PartName", "").lstrip("/")
                    if forbidden_part(target) or target not in names:
                        result["violations"].append({"part": "[Content_Types].xml", "reason": "removed/dangling content type override"})
            for member in package.infolist():
                name = member.filename
                data = package.read(member)
                if forbidden_part(name):
                    result["violations"].append({"part": name, "reason": "forbidden part path"})
                if data.startswith((b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", b"PK\x03\x04")):
                    result["violations"].append({"part": name, "reason": "embedded/legacy binary"})
                if "/media/" in name.lower():
                    try:
                        for reason in media_reasons(data, name):
                            result["violations"].append({"part": name, "reason": reason})
                    except Exception as error:
                        result["violations"].append({"part": name, "reason": f"{type(error).__name__}: {error}"})
                declared_xml = overrides.get(name, defaults.get(name.rsplit(".", 1)[-1].lower(), False))
                if member.is_dir() or not (declared_xml or name.lower().endswith((".xml", ".rels"))):
                    continue
                try:
                    # ElementTree normally discards comments and processing
                    # instructions. Retain them so serialization removals fail.
                    parser = ET.XMLParser(target=ET.TreeBuilder(insert_comments=True, insert_pis=True))
                    if b"<!DOCTYPE" in data:
                        raise ValueError("DTD/entities forbidden")
                    tree = ET.fromstring(data, parser=parser)
                    trees[name] = tree
                    parents = {child: parent for parent in tree.iter() for child in parent}
                    for element in tree.iter():
                        if not isinstance(element.tag, str):
                            result["violations"].append({"part": name, "reason": "XML comment/processing instruction"})
                            continue
                        if forbidden_element(element.tag) or (element.tag in IDENTITY and
                                                       "".join(element.itertext()).strip()):
                            result["violations"].append({"part": name, "element": element.tag,
                                                         "reason": "forbidden element"})
                        for reason in element_reasons(element, parents.get(element)):
                            result["violations"].append({"part": name, "element": element.tag, "reason": reason})
                        if (element.tag in SHEET_TAGS and
                                element.get("state") in {"hidden", "veryHidden"}):
                            result["info"].append({"part": name, "sheet": element.get("name"),
                                                   "state": element.get("state")})
                except Exception as error:
                    result["violations"].append({"part": name,
                                                 "reason": f"{type(error).__name__}: {error}"})
            package_root = {".docx": "word", ".xlsx": "xl", ".pptx": "ppt"}[path.suffix.lower()]
            for name in names:
                for reason in part_reasons(name, package_root, trees.get(name)):
                    result["violations"].append({"part": name, "reason": reason})
            check_relationships(trees, names, result["violations"])
            check_models(trees, result["violations"])
            if source is not None:
                before = len(result["violations"])
                check_source_removals(trees, names, source, result["violations"])
                result["sourceRemovals"] = {
                    "source": source["source"], "sha256": source["sha256"],
                    "partsChecked": len(source["parts"]), "textChecked": len(source["text"]),
                    "ok": len(result["violations"]) == before,
                }
        result["ok"] = not result["violations"]
    except Exception as error:
        result["violations"].append({"reason": f"{type(error).__name__}: {error}"})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    root = args.directory.resolve()
    files = sorted(p for p in root.rglob("*") if p.is_file() and p.suffix.lower() in EXTENSIONS)
    from expectations import load
    expected = load(root)
    results = [check_file(p, root, expected.get(p.relative_to(root).as_posix(), {}).get("sourceRemovals"))
               for p in files]
    report = {"ok": bool(files) and all(r["ok"] for r in results), "files": results}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    for result in results:
        print(f"{result['path']}: ok={result['ok']}; violations={len(result['violations'])}; info={result['info']}")
    if not files:
        print("No DOCX/XLSX/PPTX inputs found")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
