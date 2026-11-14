"""Namespace-aware, read-only leak checks for delivered Open XML packages."""
import argparse
import hashlib
import json
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
FORBIDDEN = ({f"{{{ns}}}{name}" for ns in WORD for name in
              ("ins", "del", "commentReference", "commentRangeStart", "commentRangeEnd")} |
             {f"{{{ns}}}{name}" for ns in CHART for name in ("f", "externalData")})
IDENTITY = {"{http://purl.org/dc/elements/1.1/}creator",
            "{http://schemas.openxmlformats.org/package/2006/metadata/core-properties}lastModifiedBy"}


def forbidden_part(name):
    # Match part components anywhere in the package, independently of XML prefixes.
    parts = name.replace("\\", "/").lower().split("/")
    return (any(p in {"docprops", "embeddings", "externallinks", "activex"} for p in parts[:-1]) or
            any(p.startswith(("comments", "thumbnail", "vbaproject")) or p == "people.xml"
                for p in parts))


def check_file(path, root):
    result = {"path": path.relative_to(root).as_posix(), "sha256": None,
              "ok": False, "violations": [], "info": []}
    try:
        result["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
        with zipfile.ZipFile(path) as package:
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
            for member in package.infolist():
                name = member.filename
                if forbidden_part(name):
                    result["violations"].append({"part": name, "reason": "forbidden part path"})
                declared_xml = overrides.get(name, defaults.get(name.rsplit(".", 1)[-1].lower(), False))
                if member.is_dir() or not (declared_xml or name.lower().endswith((".xml", ".rels"))):
                    continue
                try:
                    tree = ET.fromstring(package.read(member))
                    for element in tree.iter():
                        if element.tag in FORBIDDEN or (element.tag in IDENTITY and
                                                       "".join(element.itertext()).strip()):
                            result["violations"].append({"part": name, "element": element.tag,
                                                         "reason": "forbidden element"})
                        if (element.tag in SHEET_TAGS and
                                element.get("state") in {"hidden", "veryHidden"}):
                            result["info"].append({"part": name, "sheet": element.get("name"),
                                                   "state": element.get("state")})
                except Exception as error:
                    result["violations"].append({"part": name,
                                                 "reason": f"{type(error).__name__}: {error}"})
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
    results = [check_file(p, root) for p in files]
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
