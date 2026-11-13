#!/usr/bin/env python3
"""Download the pinned OFL harness bundle beside vz-office-worker before use."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', type=Path, required=True)
    args = parser.parse_args()
    target = args.worker.resolve().parent / 'fonts'
    if target == ROOT or ROOT in target.parents:
        raise ValueError('font assets must stay in private evidence outside checkout')
    target.mkdir(parents=True, exist_ok=True, mode=0o700)
    pins = json.loads((ROOT / 'scripts/office-font-pins.json').read_text())
    for name, row in pins['files'].items():
        path = target / name
        if not path.exists():
            subprocess.run(['rtk', 'proxy', 'curl', '--fail', '--location', '--silent',
                            '--show-error', row['url'], '-o', str(path)], check=True)
        if hashlib.sha256(path.read_bytes()).hexdigest() != row['sha256']:
            raise ValueError('font/license SHA256 mismatch: ' + name)
    (target / 'bundle.json').write_text(json.dumps(pins, indent=2) + '\n')
    print(f"Verified {len(pins['files'])} pinned font/license files in {target}")


if __name__ == '__main__':
    main()
