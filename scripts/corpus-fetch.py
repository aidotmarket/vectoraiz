#!/usr/bin/env python3
"""Fetch only explicitly manifested, verified corpus files."""
import argparse
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "spike/bakeoff"))
from common import ROOT, fetch_all, read_json

if __name__ == "__main__":
    p = argparse.ArgumentParser(__doc__)
    p.add_argument("--manifest", type=Path, default=ROOT / "spike/corpus/manifest.json")
    p.add_argument("--output", type=Path, default=ROOT / ".corpus/files")
    p.add_argument("--record", action="store_true")
    a = p.parse_args()
    manifest = read_json(a.manifest)
    fetch_all(manifest if isinstance(manifest, list) else manifest["documents"], a.output, record=a.record)
