#!/usr/bin/env python3
"""Development/CI only. Fetch pinned assets and verify before atomically installing.
The desktop has its own consented model downloader; the CLI stays offline.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import tarfile
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
PDFIUM = {
    'Darwin': ('mac-arm64', '46c674298c60123f1195c2a03c65ee08c4783c84874ce4f5b3a054abb3ecaf30'),
    'Windows': ('win-x64', '3452d11031050ec65e57f180215b57b9e6d465e3fbedafa8a77ad90885c7d23e'),
}

def fetch(url, digest, output, size=None):
    if output.exists() and (size is None or output.stat().st_size == size) and hashlib.file_digest(output.open('rb'), 'sha256').hexdigest() == digest:
        return
    output.parent.mkdir(parents=True, exist_ok=True)
    partial = output.with_suffix(output.suffix + '.part')
    try:
        for attempt in range(5):
            try:
                request = urllib.request.Request(url, headers={'User-Agent': 'vectoraiz-phase0-assets/1'})
                with urllib.request.urlopen(request, timeout=120) as response, partial.open('wb') as dest:
                    while data := response.read(1024 * 1024):
                        dest.write(data)
                break
            except (urllib.error.URLError, TimeoutError, ConnectionError) as error:
                print(f'{output.name}: download attempt {attempt + 1} failed: {type(error).__name__}', flush=True)
                if attempt == 4:
                    raise
                time.sleep(2 ** attempt)
        if size is not None and partial.stat().st_size != size:
            raise ValueError(f'{output.name}: size mismatch: expected {size}, got {partial.stat().st_size}')
        actual = hashlib.file_digest(partial.open('rb'), 'sha256').hexdigest()
        if actual != digest:
            raise ValueError(f'{output.name}: sha256 mismatch: {actual}')
        os.replace(partial, output)
    finally:
        partial.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', type=Path, default=ROOT / '.assets')
    args = parser.parse_args()
    for pin in json.loads((ROOT / 'spike/fixtures/models.json').read_text()).values():
        fetch(pin['url'], pin['sha256'], args.out / pin['file'], pin['size'])
    target, digest = PDFIUM[platform.system()]
    archive = args.out / f'pdfium-{target}.tgz'
    fetch(f'https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/8111/pdfium-{target}.tgz', digest, archive)
    with tarfile.open(archive) as bundle:
        bundle.extractall(args.out / 'pdfium', filter='data')
    print(f'Verified models and PDFium chromium/8111 in {args.out}')

if __name__ == '__main__':
    main()
