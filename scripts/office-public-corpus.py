#!/usr/bin/env python3
"""Extract a reproducible, offline Office 2013+ public corpus from Apache POI.

Usage: office-public-corpus.py PINNED_POI_TAR_GZ OUTPUT_DIRECTORY
Download upstream separately, then this selector works entirely offline.
The output includes original upstream bytes and provenance; never synthesizes Office files.
AppVersion/Application are selection evidence, not proof that a regression fixture
was never deliberately modified after an Office save. Keep malformed cases in results.
"""
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import xml.etree.ElementTree as ET
import zipfile

REVISION = "77df76b9c990cadbb3c8ae72f05a03fa06194c7a"
PREFIX = f"poi-{REVISION}/test-data/"
archive, output = Path(sys.argv[1]), Path(sys.argv[2])
output.mkdir(parents=True, exist_ok=False)
records = []
with tarfile.open(archive) as tar:
    for member in tar:
        ext = Path(member.name).suffix
        if not member.name.startswith(PREFIX) or ext not in (".docx", ".xlsx", ".pptx"):
            continue
        data = tar.extractfile(member).read()
        try:
            with zipfile.ZipFile(io.BytesIO(data)) as package:
                props = ET.fromstring(package.read("docProps/app.xml"))
            fields = {node.tag.split("}")[-1]: node.text for node in props}
            app = fields.get("Application", "") or ""
            version = fields.get("AppVersion", "0")
            if float(version) < 15 or not any(s in app.lower() for s in ("microsoft", "powerpoint", "excel", "word")):
                continue
        except (ValueError, KeyError, zipfile.BadZipFile, ET.ParseError):
            continue
        relative = Path(ext[1:]) / Path(member.name).name
        destination = output / relative
        destination.parent.mkdir(exist_ok=True)
        if destination.exists():
            raise ValueError(f"Filename collision: {relative}")
        destination.write_bytes(data)
        records.append({"upstream": member.name.removeprefix(f"poi-{REVISION}/"), "file": str(relative), "format": ext[1:], "sha256": hashlib.sha256(data).hexdigest(), "application": app, "app_version": version})
manifest = {"repository": "https://github.com/apache/poi", "revision": REVISION, "licence": "Apache-2.0", "archive_sha256": hashlib.file_digest(archive.open("rb"), "sha256").hexdigest(), "files": records}
(output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps({ext: sum(r["format"] == ext for r in records) for ext in ("docx", "xlsx", "pptx")}))
