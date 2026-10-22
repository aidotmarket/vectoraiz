#!/usr/bin/env python3
"""Derive UI/installer and lockfile versions from workspace.package.version; default is check-only."""
import argparse
import json
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def cargo_lock_versions(workspace, version, write):
    members = set()
    excluded = {path for pattern in workspace.get('exclude', []) for path in ROOT.glob(pattern)}
    for pattern in workspace['members']:
        for member in ROOT.glob(pattern):
            if member not in excluded:
                manifest = tomllib.loads((member / 'Cargo.toml').read_text())
                members.add(manifest['package']['name'])
    path = ROOT / 'Cargo.lock'
    original = path.read_text()
    blocks = re.split(r'(?m)(?=^\[\[package\]\]$)', original)
    found = set()
    for index, block in enumerate(blocks):
        if not block.startswith('[[package]]'):
            continue
        package = tomllib.loads(block)['package'][0]
        name = package['name']
        if name not in members or 'source' in package:
            continue
        if name in found:
            raise SystemExit(f'Cargo.lock: duplicate workspace member {name}')
        found.add(name)
        if package['version'] != version:
            if not write:
                raise SystemExit(f'Cargo.lock: {name} version differs from workspace {version}; run scripts/native-version.py --write')
            blocks[index] = re.sub(r'(?m)^version = "[^"]+"$', f'version = "{version}"', block, count=1)
    if found != members:
        raise SystemExit(f'Cargo.lock: missing workspace members: {", ".join(sorted(members - found))}')
    updated = ''.join(blocks)
    if write and updated != original:
        path.write_text(updated)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    parser.add_argument('--release', action='store_true', help='Reject the rotation placeholder for real releases')
    args = parser.parse_args()
    if args.release:
        config = json.loads((ROOT / 'app/src-tauri/tauri.conf.json').read_text())
        if config['plugins']['updater']['pubkey'].startswith('PLACEHOLDER-'):
            raise SystemExit('Release blocked: placeholder updater public key; rotation required')
    workspace = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']
    version = workspace['package']['version']
    cargo_lock_versions(workspace, version, args.write)
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
    print(f'Workspace, UI, Cargo/npm lockfiles and installer versions agree: {version}')


if __name__ == '__main__':
    main()
