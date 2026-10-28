#!/usr/bin/env python3
"""Explicit model downloader; execution scripts never download."""
import argparse
from pathlib import Path
from common import HERE, read_json, fetch_all

if __name__ == "__main__":
    p = argparse.ArgumentParser(__doc__)
    p.add_argument("folder", type=Path)
    p.add_argument("--candidate", action="append")
    a = p.parse_args()
    pins = read_json(HERE / "candidates.json")
    if a.candidate:
        unknown = set(a.candidate) - {c["name"] for c in pins}
        if unknown:
            p.error(f"unknown candidates: {unknown}")
        pins = [c for c in pins if c["name"] in a.candidate]
    fetch_all(pins, a.folder, resume=True)
