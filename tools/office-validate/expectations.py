"""Exact, hash-bound outcome comparison; raw diagnostics remain in the report."""
import hashlib
import json

FORMATS = {"Microsoft365", "Office2007"}


def load(root):
    expected = {}
    for manifest in sorted(root.rglob("EXPECTED.json")):
        data = json.loads(manifest.read_text(encoding="utf-8"))
        if data["version"] != 1:
            raise ValueError(f"Unsupported expectations: {manifest}")
        for name, outcome in data["files"].items():
            path = (manifest.parent / name).relative_to(root).as_posix()
            if path in expected:
                raise ValueError(f"Duplicate expectation: {path}")
            expected[path] = outcome
    return expected


def compare(root, files, sdk, renders, expected):
    errors = []
    actual = {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
              for p in files}
    opened = {f["path"]: f for f in sdk["files"]}
    rendered = {f["path"]: f for f in renders}
    if not actual or set(actual) != set(expected):
        errors.append("Input/expectation file set mismatch")
    if (set(opened) != set(actual) or len(opened) != len(sdk["files"]) or
            set(rendered) != set(actual) or len(rendered) != len(renders)):
        errors.append("Report/input file set mismatch")
    for path, digest in actual.items():
        try:
            exp, doc, pdf = expected[path], opened[path], rendered[path]
            if not digest == exp["sha256"] == doc["sha256"] == pdf["sha256"]:
                errors.append(f"{path}: SHA256 mismatch")
            if doc["openOk"] != exp["openOk"] or bool(doc.get("openError")) == doc["openOk"]:
                errors.append(f"{path}: SDK open result mismatch")
            formats = doc["formats"]
            if set(formats) != set(exp["formats"]) or (doc["openOk"] and set(formats) != FORMATS):
                errors.append(f"{path}: SDK format set mismatch")
            for name, result in formats.items():
                wanted = exp["formats"][name]
                if (result.get("error") is not None or result["errorCount"] != wanted["errorCount"] or
                        result["errorIds"] != wanted["errorIds"] or
                        sum(result["errorIds"].values()) != result["errorCount"]):
                    errors.append(f"{path}: {name} schema outcome mismatch")
            if (pdf["ok"] != exp["libreOfficeOk"] or
                    pdf.get("exitCode") != exp["libreOfficeExitCode"] or pdf.get("timedOut") or
                    (not pdf["ok"] and "Error: source file could not be loaded" not in pdf.get("stderr", ""))):
                errors.append(f"{path}: LibreOffice outcome mismatch")
        except (KeyError, TypeError, ValueError) as error:
            errors.append(f"{path}: invalid/missing outcome: {error}")
    return errors
