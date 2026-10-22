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


def step(message):
    print(f'[{time.strftime("%H:%M:%S")}] {message}', flush=True)


def run(*args, timeout=90):
    step(f'Run (timeout {timeout}s): ' + subprocess.list2cmdline(list(map(str, args))))
    try:
        result = subprocess.run(list(map(str, args)), timeout=timeout, check=False)
    except subprocess.TimeoutExpired:
        step('TIMEOUT: collecting process diagnostics')
        if platform.system() == 'Windows':
            subprocess.run(['powershell.exe', '-NoProfile', '-Command',
                            'Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,CommandLine | Format-List'],
                           timeout=20, check=False)
        raise
    # Windows Installer may return success with reboot required/scheduled.
    assert result.returncode in (0, 1641, 3010), f'Command failed: {result.returncode}'
    step(f'Completed: exit {result.returncode}')


step('Starting fresh hosted-runner install stand-in')
artifacts = Path('artifacts').resolve()
data = Path.home() / 'vectorAIz'
assert not data.exists(), 'Install test requires a fresh user data folder'
windows = platform.system() == 'Windows'
report = Path('installed-self-test.json').resolve()
env = dict(os.environ, VZ_INSTALLED_SELF_TEST_REPORT=str(report))
if windows:
    installer, = artifacts.rglob('*.msi')
    installed = Path(os.environ['ProgramFiles']) / 'vectorAIz'
    # Use the native system binary, with an explicit log created before launching.
    # Do not resolve msiexec through runner PATH or use shell elevation/UI prompts.
    msiexec = Path(os.environ['SystemRoot']) / 'System32/msiexec.exe'
    log = Path('msi-install.log').resolve()
    log.touch()
    # Use the MSI's per-machine default destination. Python quotes a spaced
    # INSTALLDIR=... argument as a whole; msiexec requires quoting its value
    # and can otherwise open its usage dialog even when /qn was supplied.
    run(msiexec, '/i', installer, '/qn', '/norestart', '/l*v', log,
        'ALLUSERS=1', timeout=240)
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
step(f'Installed executable: {binary}')
try:
    # No checkout/assets in this working directory: exercise installed paths.
    with tempfile.TemporaryDirectory(prefix='vz-first-run-') as cwd:
        # A per-machine app cannot write WebView2's default profile next to its
        # executable in Program Files. Match Tauri's user-scoped production profile.
        env['WEBVIEW2_USER_DATA_FOLDER'] = str(data / 'webview2')
        step('Launch installed WebView self-test (120s report deadline)')
        ui_log = Path('installed-ui.log').resolve().open('w')
        child = subprocess.Popen([str(binary), '--smoke-test', '--cpu'], cwd=cwd, env=env,
                                 stdout=ui_log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 120
            while not report.exists() and time.monotonic() < deadline and child.poll() is None:
                time.sleep(0.5)
            assert report.is_file(), 'Installed WebView self-test did not return a report'
            step('Self-test report received')
            result = json.loads(report.read_text())
            assert result['status'] == 'OK', result
            assert child.wait(timeout=20) == 0
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=10)
            ui_log.close()
    step('Check persistent data sentinel before uninstall')
    sentinel = data / 'install-self-test.json'
    before = sentinel.read_bytes()
finally:
    step('Uninstall; preserve user data')
    if windows:
        run(msiexec, '/x', installer, '/qn', '/norestart',
            '/l*v', str(Path('msi-uninstall.log').resolve()), timeout=120)
    else:
        shutil.rmtree(installed)
assert not binary.exists(), 'Uninstall left the executable installed'
assert sentinel.read_bytes() == before, 'Uninstall removed or changed user data'
print(f'PASS: {platform.platform()} hosted-runner stand-in: installed UI self-test; uninstall preserved {data}')
