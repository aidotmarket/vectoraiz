#!/usr/bin/env python3
"""Prepare unsigned updater artifacts. No updater key is used on hosted CI."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def run(*argv, **kwargs):
    return subprocess.run(list(map(str, argv)), check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('phase', choices=['prepare', 'collect'])
    args = parser.parse_args()
    windows = platform.system() == 'Windows'
    suffix = '.exe' if windows else ''
    triple = 'x86_64-pc-windows-msvc' if windows else 'aarch64-apple-darwin'
    generated = ROOT / 'app/src-tauri/gen'
    bundle = generated / 'bundle'
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    if args.phase == 'prepare':
        # Never copy the CI model cache into an installer (including stale inputs).
        shutil.rmtree(bundle, ignore_errors=True)
        bundle.mkdir(parents=True)
        runtime = bundle / ('assets/pdfium/bin' if windows else 'assets/pdfium/lib')
        runtime.mkdir(parents=True)
        library = 'pdfium.dll' if windows else 'libpdfium.dylib'
        shutil.copy2(ROOT / '.assets/pdfium' / ('bin' if windows else 'lib') / library, runtime / library)
        assert not list(bundle.rglob('*.gguf')), 'Models must be imported or downloaded separately'
        shutil.copy2(ROOT / 'spike/fixtures/models.json', bundle / 'models.json')
        shutil.copy2(ROOT / 'THIRD_PARTY_NOTICES', bundle / 'THIRD_PARTY_NOTICES')
        (generated / 'bin').mkdir(exist_ok=True)
        for name in ['vz-parser-worker', 'vectoraiz-mcp', 'vectoraiz-cli']:
            shutil.copy2(ROOT / f'target/debug/{name}{suffix}', generated / f'bin/{name}-{triple}{suffix}')
        if not windows:
            shutil.copy2(ROOT / 'target/debug/vz-vision-ocr', generated / f'bin/vz-vision-ocr-{triple}')
        config = json.loads((ROOT / 'app/src-tauri/tauri.stage1.conf.json').read_text())
        if not windows:
            config['bundle']['externalBin'].append('gen/bin/vz-vision-ocr')
        (generated / 'stage1.conf.json').write_text(json.dumps(config, indent=2) + '\n')
        return
    output = ROOT / 'target/stage1'
    # target/ is restored by rust-cache, including prior packaging outputs.
    # Never reuse stale installers/manifests or DMG symlinks.
    shutil.rmtree(output, ignore_errors=True)
    output.mkdir(parents=True)
    if windows:
        installers = list((ROOT / 'target/debug/bundle/msi').glob('*.msi'))
        assert len(installers) == 1, installers
        shutil.copy2(installers[0], output / f'vectorAIz_{version}_x64.msi')
    else:
        app = ROOT / 'target/debug/bundle/macos/vectorAIz.app'
        # Sign every Mach-O leaf, including resource libraries, then the container.
        for file in sorted(app.rglob('*')):
            if file.is_file() and not file.is_symlink():
                result = subprocess.check_output(['file', '-b', str(file)], text=True)
                if 'Mach-O' in result:
                    run('codesign', '--force', '--sign', '-', file)
        run('codesign', '--force', '--sign', '-', app)
        run('codesign', '--verify', '--deep', '--strict', '--verbose=2', app)
        run('tar', '-czf', output / f'vectorAIz_{version}_aarch64.app.tar.gz', '-C', app.parent, app.name)
        staging = ROOT / 'target/dmg-stage'
        shutil.rmtree(staging, ignore_errors=True)
        staging.mkdir()
        shutil.copytree(app, staging / app.name, symlinks=True, dirs_exist_ok=True)
        (staging / 'Applications').symlink_to('/Applications', target_is_directory=True)
        run('hdiutil', 'create', '-volname', 'vectorAIz', '-srcfolder', staging,
            '-ov', '-format', 'UDZO', output / f'vectorAIz_{version}_aarch64.dmg')
    shutil.copy2(ROOT / 'THIRD_PARTY_NOTICES', output / f'THIRD_PARTY_NOTICES-{triple}.txt')
    manifest = {'source_sha': __import__('os').environ['SOURCE_SHA'], 'version': version,
                'target': triple, 'profile': 'debug', 'updater_signed': False,
                'updater_public_key': json.loads((ROOT / 'app/src-tauri/tauri.conf.json').read_text())['plugins']['updater']['pubkey'],
                'sizes_bytes': {p.name: p.stat().st_size for p in sorted(output.iterdir()) if p.is_file()},
                'files': {p.name: hashlib.file_digest(p.open('rb'), 'sha256').hexdigest()
                          for p in sorted(output.iterdir()) if p.is_file()}}
    for name, size in manifest['sizes_bytes'].items():
        print(f'Packaged {name}: {size:,} bytes', flush=True)
    (output / f'build-{triple}.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
