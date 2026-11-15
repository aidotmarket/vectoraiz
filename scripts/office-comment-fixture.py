#!/usr/bin/env python3
"""Build a test-only POI comment fixture; never used by the sanitizer.

Both inputs are pristine at POI 77df76b9c990cadbb3c8ae72f05a03fa06194c7a.
That corpus has no complete deck with slide-comment parts and without OLE.
Copy its real comment/author parts into the existing benign SampleShow deck.
Keep 45545_Comment.pptx unchanged as the OLE-refusal control.
"""
import hashlib
import io
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[1] / "crates/vz-pack/tests/fixtures/office-public"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
inputs = {"SampleShow.pptx": "bfb4b2f07c9233afd2f32aa5781d849d0c7d225bf03721a10becf487b481d828",
          "45545_Comment.pptx": "0295a51f63150e3ee680154d07dab88062ba2306030b790561f85436f3ded7c8"}
for name, digest in inputs.items():
    assert hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == digest
with zipfile.ZipFile(ROOT / "SampleShow.pptx") as package:
    parts = {name: package.read(name) for name in package.namelist()}
with zipfile.ZipFile(ROOT / "45545_Comment.pptx") as donor:
    for name in ["ppt/comments/comment2.xml", "ppt/commentAuthors.xml"]:
        parts[name] = donor.read(name)
for part, target, kind in [
    ("ppt/slides/_rels/slide1.xml.rels", "../comments/comment2.xml", "comments"),
    ("ppt/_rels/presentation.xml.rels", "commentAuthors.xml", "commentAuthors"),
]:
    relationship = f'<Relationship Id="s1944Comments" Type="{R}/{kind}" Target="{target}"/>'
    parts[part] = parts[part].replace(b"</Relationships>", relationship.encode() + b"</Relationships>")
for part, kind in [("ppt/comments/comment2.xml", "comments"), ("ppt/commentAuthors.xml", "commentAuthors")]:
    override = f'<Override PartName="/{part}" ContentType="application/vnd.openxmlformats-officedocument.presentationml.{kind}+xml"/>'
    parts["[Content_Types].xml"] = parts["[Content_Types].xml"].replace(b"</Types>", override.encode() + b"</Types>")
assert not any("/embeddings/" in name for name in parts)
result = io.BytesIO()
with zipfile.ZipFile(result, "w") as output:
    for name in sorted(parts):
        info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        output.writestr(info, parts[name])
path = ROOT / "SampleShow-WithComments.pptx"
path.write_bytes(result.getvalue())
print(path.name, hashlib.sha256(result.getvalue()).hexdigest())
