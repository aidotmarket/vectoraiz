#!/usr/bin/env python3
"""Generate notices with checked subprocesses and separate UTF-8 output files."""
from pathlib import Path
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    with tempfile.TemporaryDirectory(prefix='vz-notices-') as temporary:
        rust, native, npm = [Path(temporary) / name for name in ['rust.txt', 'native.txt', 'npm.txt']]
        subprocess.run(['cargo', 'about', 'generate', '--locked', 'about.hbs', '-o', str(rust)],
                       cwd=ROOT, check=True)
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
