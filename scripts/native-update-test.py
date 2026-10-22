#!/usr/bin/env python3
"""Manual published-update acceptance, with a release-independent CI dry check."""
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
REPO = 'aidotmarket/vectoraiz-test-builds'
ENDPOINT = f'https://github.com/{REPO}/releases/latest/download/latest.json'


def run(*args, timeout=240, **kwargs):
    print('Run: ' + subprocess.list2cmdline(list(map(str, args))), flush=True)
    return subprocess.run(list(map(str, args)), timeout=timeout, check=True, **kwargs)


def main():
    before, after = os.environ['FROM_VERSION'], os.environ['TO_VERSION']
    for version in (before, after):
        if not re.fullmatch(r'0\.0\.[1-9][0-9]*', version):
            raise SystemExit('Expected stage 1 version 0.0.N (N > 0)')
    assert int(after.split('.')[-1]) > int(before.split('.')[-1]), 'to_version must be newer'
    config = json.loads((ROOT / 'app/src-tauri/tauri.conf.json').read_text())
    assert config['plugins']['updater']['endpoints'] == [ENDPOINT], 'Unexpected updater endpoint'
    assert not config['plugins']['updater']['pubkey'].startswith('PLACEHOLDER-')
    dry = os.environ.get('DRY_CHECK', 'true').lower() == 'true'
    evidence = {'from_version': before, 'to_version': after, 'latest_url': ENDPOINT,
                'runner': platform.platform(), 'stand_in': True, 'dry_check': dry}
    if dry:
        evidence['status'] = 'PASS: inputs and configured latest.json URL validated; no install attempted'
    else:
        exercise(before, after)
        evidence['status'] = 'PASS: published install, signed in-app update, self-test and preserved data'
    Path('update-evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(evidence['status'], flush=True)


def exercise(before, after):
    windows = platform.system() == 'Windows'
    data = Path.home() / 'vectorAIz'
    assert not data.exists(), 'Requires fresh hosted runner'
    artifacts = Path('update-artifacts').resolve()
    run('gh', 'release', 'download', f'native-v{before}', '--repo', REPO,
        '--pattern', '*.msi' if windows else '*.dmg', '--dir', artifacts)
    # The updater feed must be the expected published to_version before changing anything.
    import urllib.request
    with urllib.request.urlopen(ENDPOINT, timeout=30) as response:
        assert json.load(response)['version'] == after, 'Latest is not to_version'
    if windows:
        installer, = artifacts.glob('*.msi')
        msiexec = Path(os.environ['SystemRoot']) / 'System32/msiexec.exe'
        installed = Path(os.environ['ProgramFiles']) / 'vectorAIz'
        run(msiexec, '/i', installer, '/qn', '/norestart', '/l*v',
            Path('update-install.log').resolve(), 'ALLUSERS=1')
        binary = installed / 'vectoraiz.exe'
    else:
        installer, = artifacts.glob('*.dmg')
        with tempfile.TemporaryDirectory(prefix='vz-update-mount-') as mount:
            run('hdiutil', 'attach', installer, '-nobrowse', '-mountpoint', mount)
            installed = Path('/Applications/vectorAIz.app')
            try:
                run('ditto', Path(mount) / installed.name, installed)
            finally:
                run('hdiutil', 'detach', mount)
        binary = installed / 'Contents/MacOS/vectoraiz'
    env = dict(os.environ, WEBVIEW2_USER_DATA_FOLDER=str(data / 'webview2'))

    def self_test(version):
        report = Path(f'update-self-test-{version}.json').resolve()
        report.unlink(missing_ok=True)
        with Path(f'update-self-test-{version}.log').open('w') as log:
            run(binary, '--smoke-test', '--cpu', timeout=140,
                env=dict(env, VZ_INSTALLED_SELF_TEST_REPORT=str(report)), stdout=log, stderr=subprocess.STDOUT)
        result = json.loads(report.read_text())
        assert result['status'] == 'OK' and result['version'] == version, result

    try:
        self_test(before)
        sentinel = data / 'update-preserve.txt'
        sentinel.write_text('Must survive the in-app update and uninstall\n')
        preserved = sentinel.read_bytes()
        with Path('update-updater.log').open('w') as log:
            run(binary, '--update-test', timeout=300, env=dict(env, VZ_UPDATE_TO_VERSION=after),
                stdout=log, stderr=subprocess.STDOUT)
        # Windows updater launches msiexec asynchronously and exits the old app.
        deadline = time.monotonic() + 180
        while True:
            try:
                self_test(after)
                break
            except (AssertionError, OSError, subprocess.SubprocessError):
                if time.monotonic() >= deadline:
                    raise
                time.sleep(5)
        assert sentinel.read_bytes() == preserved
    finally:
        if windows:
            # ProductCode changes across releases; uninstall using the new MSI.
            run('gh', 'release', 'download', f'native-v{after}', '--repo', REPO,
                '--pattern', '*.msi', '--dir', artifacts / 'updated')
            updated, = (artifacts / 'updated').glob('*.msi')
            run(msiexec, '/x', updated, '/qn', '/norestart', '/l*v', Path('update-uninstall.log').resolve())
        else:
            import shutil
            shutil.rmtree(installed)
    assert not binary.exists()
    assert sentinel.read_bytes() == preserved


if __name__ == '__main__':
    main()
