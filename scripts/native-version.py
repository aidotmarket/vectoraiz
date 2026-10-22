#!/usr/bin/env python3
"""Derive UI/installer versions from workspace.package.version; default is check-only."""
import argparse
import json
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    parser.add_argument('--release', action='store_true', help='Reject the rotation placeholder for real releases')
    args = parser.parse_args()
    if args.release:
        config = json.loads((ROOT / 'app/src-tauri/tauri.conf.json').read_text())
        if config['plugins']['updater']['pubkey'].startswith('PLACEHOLDER-'):
            raise SystemExit('Release blocked: placeholder updater public key; rotation required')
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    for name in ['app/package.json', 'app/src-tauri/tauri.conf.json', 'app/package-lock.json']:
        path = ROOT / name
        data = json.loads(path.read_text())
        versions = [data]
        if name.endswith('package-lock.json'):
            versions.append(data['packages'][''])
        if args.write:
            for entry in versions:
                entry['version'] = version
            path.write_text(json.dumps(data, indent=2) + '\n')
        elif any(entry.get('version') != version for entry in versions):
            raise SystemExit(f'{name}: version differs from workspace {version}; run scripts/native-version.py --write')
    print(f'Workspace, UI, lockfile and installer versions agree: {version}')


if __name__ == '__main__':
    main()
