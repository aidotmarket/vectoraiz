#!/usr/bin/env python3
"""Fail early if checkout conversion changes byte-sensitive fixture files."""
import hashlib
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
BINARY = {".pdf", ".parquet", ".zip", ".docx", ".xlsx", ".pptx"}


def main():
    tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).split(b"\0")
    count = 0
    for raw in tracked:
        if not raw:
            continue
        name = raw.decode("utf-8")
        path = Path(name)
        if path.suffix.lower() not in BINARY and not (
            path.suffix.lower() == ".html" and "fixtures" in path.parts
        ):
            continue
        committed = subprocess.check_output(["git", "show", f"HEAD:{name}"], cwd=ROOT)
        expected = hashlib.sha256(committed).hexdigest()
        actual = hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
        if actual != expected:
            raise SystemExit(f"Fixture bytes changed on checkout: {name}: {actual} != {expected}")
        print(f"sha256 OK {name}: {actual}")
        count += 1
    if count == 0:
        raise SystemExit("No byte-sensitive fixtures checked")
    print(f"Verified {count} byte-sensitive fixtures against committed Git blobs")


if __name__ == "__main__":
    main()
