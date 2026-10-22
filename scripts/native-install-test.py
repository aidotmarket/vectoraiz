#!/usr/bin/env python3
"""Fresh hosted-runner stand-in: install downloaded artifact, launch UI, uninstall."""
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import time


def run(*args):
    subprocess.run(list(map(str, args)), check=True)


artifacts = Path('artifacts').resolve()
data = Path.home() / 'vectorAIz'
assert not data.exists(), 'Install test requires a fresh user data folder'
windows = platform.system() == 'Windows'
report = Path('installed-self-test.json').resolve()
env = dict(os.environ, VZ_INSTALLED_SELF_TEST_REPORT=str(report))
if windows:
    installer, = artifacts.rglob('*.msi')
    installed = Path(os.environ['ProgramFiles']) / 'vectorAIz'
    run('msiexec.exe', '/i', installer, '/qn', '/norestart', 'ALLUSERS=1',
        f'INSTALLDIR={installed}', '/L*v', str(Path('msi-install.log').resolve()))
    binary = installed / 'vectoraiz.exe'
else:
    installer, = artifacts.rglob('*.dmg')
    mount = Path(tempfile.mkdtemp(prefix='vz-dmg-'))
    run('hdiutil', 'attach', installer, '-nobrowse', '-mountpoint', mount)
    installed = Path('/Applications/vectorAIz.app')
    try:
        run('ditto', mount / 'vectorAIz.app', installed)
    finally:
        run('hdiutil', 'detach', mount)
        mount.rmdir()
    run('codesign', '--verify', '--deep', '--strict', installed)
    binary = installed / 'Contents/MacOS/vectoraiz'
assert binary.is_file(), f'Installed executable missing: {binary}'
try:
    # No checkout/assets in this working directory: exercise installed paths.
    with tempfile.TemporaryDirectory(prefix='vz-first-run-') as cwd:
        child = subprocess.Popen([str(binary), '--smoke-test', '--cpu'], cwd=cwd, env=env)
        try:
            deadline = time.monotonic() + 120
            while not report.exists() and time.monotonic() < deadline and child.poll() is None:
                time.sleep(0.5)
            assert report.is_file(), 'Installed WebView self-test did not return a report'
            result = json.loads(report.read_text())
            assert result['status'] == 'OK', result
            assert child.wait(timeout=20) == 0
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
    sentinel = data / 'install-self-test.json'
    before = sentinel.read_bytes()
finally:
    if windows:
        run('msiexec.exe', '/x', installer, '/qn', '/norestart',
            '/L*v', str(Path('msi-uninstall.log').resolve()))
    else:
        shutil.rmtree(installed)
assert not binary.exists(), 'Uninstall left the executable installed'
assert sentinel.read_bytes() == before, 'Uninstall removed or changed user data'
print(f'PASS: {platform.platform()} hosted-runner stand-in: installed UI self-test; uninstall preserved {data}')
