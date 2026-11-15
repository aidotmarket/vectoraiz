"""Prove real report expectations reject fixture and expectation mutations."""
import copy
import json
from pathlib import Path
import shutil
import sys
import tempfile

import expectations
from leak_check import EXTENSIONS

root, report_path = map(Path, sys.argv[1:])
report = json.loads(report_path.read_text(encoding="utf-8"))
with tempfile.TemporaryDirectory() as temp:
    target = Path(temp) / "inputs"
    shutil.copytree(root, target)
    files = sorted(p for p in target.rglob("*") if p.suffix.lower() in EXTENSIONS)
    expected = expectations.load(target)
    def compare(wanted):
        return expectations.compare(target, files, report["openXml"], report["libreOffice"], wanted)
    if not report["ok"] or compare(expected):
        raise SystemExit("Self-test requires a green real validation report")
    for group in sorted({p.relative_to(target).parts[0] for p in files}):
        fixture = next(p for p in files if p.relative_to(target).parts[0] == group)
        original = fixture.read_bytes()
        changed = bytearray(original)
        changed[-1] ^= 1
        fixture.write_bytes(changed)
        if not compare(expected):
            raise SystemExit(f"Fixture-byte mutation escaped: {group}")
        fixture.write_bytes(original)
        altered = copy.deepcopy(expected)
        altered[fixture.relative_to(target).as_posix()]["openOk"] ^= True
        if not compare(altered):
            raise SystemExit(f"Expected-entry mutation escaped: {group}")
        print(f"{group}: baseline green; one-byte mutation RED; expected-entry mutation RED")
