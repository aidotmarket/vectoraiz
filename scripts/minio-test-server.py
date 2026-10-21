#!/usr/bin/env python3
"""Build a fixed MinIO source revision without a container or image publication."""
from pathlib import Path
import os
import subprocess
import sys
import tarfile
from importlib.machinery import SourceFileLoader

ROOT = Path(__file__).resolve().parents[1]
fetch = SourceFileLoader('assets', str(ROOT / 'scripts/fetch-assets.py')).load_module().fetch
source = ROOT / '.tools/minio-src'
archive = ROOT / '.tools/minio-src.tar.gz'
fetch('https://codeload.github.com/minio/minio/tar.gz/f19c534b9f457773dcd043d977433e1a71525c3b', 'ac3bc23e5d91981a89921800a931a59aad27146434f952e9a79076b919d8a661', archive)
source.mkdir(parents=True, exist_ok=True)
with tarfile.open(archive) as bundle:
    bundle.extractall(source, filter='data')
folder = next(source.iterdir())
binary = ROOT / ('.tools/minio.exe' if os.name == 'nt' else '.tools/minio')
subprocess.run(['go', 'build', '-o', str(binary), '.'], cwd=folder, check=True)
print(binary)
