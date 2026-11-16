#!/usr/bin/env python3
"""Generate notices with checked subprocesses and separate UTF-8 output files."""
from pathlib import Path
import os
import json
import re
import hashlib
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def complete_worker_notices(rust, tree):
    # Local workspace members include a path suffix and do not match this form.
    required = set()
    for line in tree.splitlines():
        match = re.match(r'^([\w-]+) v(\S+)', line)
        if match and not re.search(r'\((?:/|[A-Za-z]:[\\/])', line):
            required.add((match[1], match[2]))
    if not required:
        raise SystemExit('Worker dependency inventory is empty')
    listed = set(re.findall(r'^- ([\w-]+) (\S+)$', rust, re.MULTILINE))
    folder = ROOT / 'docs/native-worker-licenses'
    for entry in json.loads((folder / 'manifest.json').read_text()):
        key = (entry['name'], entry['version'])
        if key not in required - listed:
            continue
        # Git may check text out as CRLF on Windows; hashes use canonical LF.
        data = (folder / entry['file']).read_bytes().replace(b'\r\n', b'\n')
        if hashlib.sha256(data).hexdigest() != entry['license_sha256']:
            raise SystemExit(f'Worker licence text changed: {key}')
        rust += f"\nMIT (MIT)\nUsed by:\n- {key[0]} {key[1]}\n" + data.decode('utf-8')
        listed.add(key)
    if missing := sorted(required - listed):
        raise SystemExit(f'Worker dependencies missing from third-party notices: {missing}')
    return rust


def main():
    with tempfile.TemporaryDirectory(prefix='vz-notices-') as temporary:
        rust, native, npm = [Path(temporary) / name for name in ['rust.txt', 'native.txt', 'npm.txt']]
        subprocess.run(['cargo', 'about', 'generate', '--locked', 'about.hbs', '-o', str(rust)],
                       cwd=ROOT, check=True)
        tree = subprocess.run(['cargo', 'tree', '--locked', '-p', 'vz-parser-worker',
                               '--edges', 'normal,build', '--prefix', 'none', '--format', '{p}'],
                              cwd=ROOT, check=True, capture_output=True, text=True).stdout
        rust.write_text(complete_worker_notices(rust.read_text(encoding='utf-8-sig'), tree), encoding='utf-8')
        with native.open('wb') as output:
            subprocess.run([sys.executable, ROOT / 'scripts/native-notices.py'], cwd=ROOT,
                           env=os.environ | {'PYTHONIOENCODING': 'utf-8'},
                           stdout=output, check=True)
        with npm.open('wb') as output:
            subprocess.run([shutil.which('npm') or 'npm', 'run', '--silent', 'licenses'],
                           cwd=ROOT / 'app', stdout=output, check=True)
        # Publish only when every generator succeeded; never append to stale output.
        (ROOT / 'THIRD_PARTY_NOTICES').write_text(
            '\n'.join(path.read_text(encoding='utf-8-sig') for path in [rust, native, npm]),
            encoding='utf-8')


if __name__ == '__main__':
    main()
