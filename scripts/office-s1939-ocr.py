#!/usr/bin/env python3
"""Offline Tesseract prerequisite/build probe; never installs or downloads assets."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('common', ROOT / 'scripts/office-s1934.py')
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)
REFUSAL = 'Scanned pages cannot be read with the installed offline OCR.'


def main(a):
    root = a.evidence.resolve()
    if root == ROOT or ROOT in root.parents: raise ValueError('private evidence required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    source = root / 'tesseract-link-probe.cc'
    source.write_text('#include <tesseract/baseapi.h>\nint main() { tesseract::TessBaseAPI api; return api.Version() ? 0 : 1; }\n')
    command = c.run(['xcrun', 'clang++', '-std=c++17', source, '-ltesseract', '-llept', '-o', root / 'link-probe'],
                    root, 'tesseract-link-probe', timeout=60)
    candidates = [Path('/opt/homebrew/share/tessdata'), Path('/usr/local/share/tessdata')]
    models = {}
    for language in ('eng', 'deu', 'fra', 'spa', 'jpn'):
        files = [p / (language + '.traineddata') for p in candidates if (p / (language + '.traineddata')).is_file()]
        models[language] = {'status': 'blocked' if not files else 'unrun',
                            'sha256': c.digest(files[0]) if files else None,
                            'license': None, 'reason': 'no installed offline traineddata/license pin' if not files else 'license provenance not established'}
    result = {'status': 'blocked', 'engine_executable': shutil.which('tesseract'), 'engine_version': None,
              'engine_build_sha256': None, 'engine_license': None, 'models': models, 'compile_link': command,
              'reason': 'Tesseract cannot be built/linked from installed prerequisites: see preserved compiler diagnostic; no pinned offline engine/source or five-language traineddata/license bundle supplied',
              'download_attempted': False, 'confined_ocr_launched': False, 'refusal': REFUSAL,
              'scanned_pages_qualification': 'fail', 'partial_output_policy': 'discard complete scanned-document result; preserve old snapshot',
              'Windows': {'status': 'NOT RUN'}, 'recipe': 'engine change requires N1 extraction recipe requalification'}
    if command['exit_code'] == 0:
        result['reason'] = 'Link probe succeeded but engine build/version and all required model/license pins are not established; do not run unpinned OCR'
    (root / 'ocr-status.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'status': result['status'], 'link_exit_code': command['exit_code'],
                      'model_count': sum(v['sha256'] is not None for v in models.values())}))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evidence', type=Path, required=True)
    main(p.parse_args())
