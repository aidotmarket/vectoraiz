#!/usr/bin/env python3
"""Setup-only pinned private OCR assets. The helper never downloads anything."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('common', ROOT / 'scripts/office-s1934.py')
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)


def main(a):
    root = a.assets.resolve()
    if root == ROOT or ROOT in root.parents:
        raise ValueError('private assets outside checkout required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    pins = json.loads((ROOT / 'scripts/office-tesseract-pins.json').read_text())
    def run(argv, label):
        row = c.run(argv, root, label)
        if row['exit_code']:
            raise RuntimeError(row)
    for name, pin in pins['downloads'].items():
        path = root / name
        if not path.exists():
            run(['curl', '-fL', '--retry', '2', pin['url'], '-o', path], 'fetch-' + name)
        if c.digest(path) != pin['sha256']:
            raise ValueError('hash mismatch: ' + name)
    prefix = root / 'install'
    for name, version, options in (
        ('leptonica', pins['leptonica'], ['-DENABLE_ZLIB=OFF', '-DENABLE_PNG=OFF', '-DENABLE_JPEG=OFF',
          '-DENABLE_TIFF=OFF', '-DENABLE_WEBP=OFF', '-DENABLE_OPENJPEG=OFF', '-DENABLE_GIF=OFF', '-DBUILD_PROG=OFF']),
        ('tesseract', pins['engine'], ['-DBUILD_TRAINING_TOOLS=OFF', '-DBUILD_TESTS=OFF', '-DDISABLE_CURL=ON',
          '-DDISABLE_ARCHIVE=ON', '-DOPENMP_BUILD=OFF', '-DGRAPHICS_DISABLED=ON'])):
        source = root / (name + '-' + version)
        if not source.exists():
            with tarfile.open(root / (name + '-' + version + '.tar.gz')) as archive:
                archive.extractall(root, filter='data')
        build = root / (name + '-build')
        run(['cmake', '-S', source, '-B', build, '-DCMAKE_BUILD_TYPE=Release', '-DBUILD_SHARED_LIBS=OFF',
             '-DCMAKE_INSTALL_PREFIX=' + str(prefix), '-DCMAKE_PREFIX_PATH=' + str(prefix), *options], 'configure-' + name)
        run(['cmake', '--build', build, '-j', '4'], 'build-' + name)
        run(['cmake', '--install', build], 'install-' + name)
    records = {str(p.relative_to(root)): c.digest(p) for p in prefix.rglob('*') if p.is_file()}
    licenses = {str(p.relative_to(root)): c.digest(p) for pattern in ('tesseract-*/LICENSE', 'leptonica-*/leptonica-license.txt', 'LICENSE') for p in root.glob(pattern)}
    (root / 'bundle.json').write_text(json.dumps({'pins': pins, 'installed_sha256': records,
        'licenses': licenses, 'image_codecs': 'disabled: parent supplies bounded raw RGB',
        'status': 'built; OCR and confinement qualification pending'}, indent=2) + '\n')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--assets', type=Path, required=True)
    main(p.parse_args())
