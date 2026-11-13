#!/usr/bin/env python3
"""Parent raster validation and separately confined offline OCR experiment."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[1]
def module(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m
d = module('descriptor', 'scripts/office-s1939-workers.py')
png = module('png', 'scripts/office-png.py')
w = d.w


def main(a):
    root, assets = a.evidence.resolve(), a.assets.resolve()
    if root == ROOT or ROOT in root.parents:
        raise ValueError('private evidence required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    def run(argv, label):
        row = w.common.run(argv, root, label)
        if row['exit_code']: raise RuntimeError(row)
    prefix = assets / 'install'
    run(['xcrun', 'clang', '-c', '-fblocks', '-Wno-deprecated-declarations',
         ROOT / 'crates/vz-parser-worker/src/office-bootstrap.c', '-o', root / 'bootstrap.o'], 'build-bootstrap')
    helper = root / 'vz-tesseract-ocr'
    run(['xcrun', 'clang++', '-std=c++17', ROOT / 'scripts/office-tesseract-helper.cc', root / 'bootstrap.o',
         '-I' + str(prefix / 'include'), prefix / 'lib/libtesseract.a', prefix / 'lib/libleptonica.a',
         '-framework', 'CoreServices', '-lsandbox', '-o', helper], 'build-helper')
    run(['codesign', '--force', '--sign', '-', helper], 'sign-helper')
    helper_hash = w.common.digest(helper)
    worker, pdfium = a.worker.resolve(), a.pdfium.resolve()
    drafts = json.loads(a.drafts.read_text())
    result = {'helper_sha256': helper_hash, 'worker_sha256': w.common.digest(worker),
              'pdfium_sha256': w.common.digest(pdfium), 'bundle_sha256': w.common.digest(assets / 'bundle.json'),
              'sampler_enabled': False, 'roles': {}, 'memory': {}, 'Windows': 'NOT RUN',
              'content_comparison': 'entire UTF-8 output from identical raw raster; no prefixes; models/locale initialized before confinement',
              'qualification': 'regression only; human relevance grading DEFERRED; hard resident bound remains FAIL',
              'snapshot_promoted': False}
    scratch = root / 'scratch'
    scratch.mkdir(exist_ok=True)
    sb = root / 'ocr.sb'
    sb.write_text(w.profile(helper, helper, pdfium, scratch))
    # No tessdata or non-system library path grants. Static engine/codecs disabled.
    for role in ('ingest', 'render'):
        target = root / role
        target.mkdir(exist_ok=True)
        rows = {}
        for fixture in (x for x in drafts if x['format'] == 'pdf'):
            name, path = fixture['id'], Path(fixture['path'])
            if w.common.digest(path) != fixture['sha256']: raise ValueError('fixture changed')
            render_profile = target / (name + '.sb')
            render_profile.write_text(w.profile(worker, path, pdfium, scratch))
            render = w.execute([worker, render_profile, role, pdfium, path, scratch / (role + name)],
                               target, name + '-raster', d.frame({'path': str(path), 'pdfium': str(pdfium)}), readiness=True)
            response = render.pop('response', None)
            row = {'fixture_sha256': fixture['sha256'], 'language': fixture['language'],
                   'scanned': fixture['scanned'], 'raster': render, 'pages': []}
            for page in (response or {}).get('pages', []):
                decoded = png.decode(bytes(page['png']))
                width, height = decoded[:2]
                if (width, height) != (page['width'], page['height']): raise ValueError('dimensions mismatch')
                payload = struct.pack('<II', width, height) + png.rgb(decoded)
                if w.common.digest(helper) != helper_hash: raise ValueError('helper identity changed')
                label = name + '-' + str(page['page'])
                baseline = w.execute([helper, sb, 'control', assets], target, label + '-control', payload, readiness=True)
                confined = w.execute([helper, sb, 'ocr', assets], target, label + '-confined', payload, readiness=True)
                before = (target / baseline['stdout']).read_bytes()
                after = (target / confined['stdout']).read_bytes()
                equal = bool(before.strip()) and before == after and baseline['exit_code'] == confined['exit_code'] == 0
                row['pages'].append({'page': page['page'], 'generation': fixture['sha256'],
                    'raw_request_sha256': hashlib.sha256(payload).hexdigest(), 'width': width, 'height': height,
                    'full_content_equal': equal, 'utf8_bytes': len(after), 'control': baseline, 'confined': confined})
            row['status'] = 'pass' if row['pages'] and all(p['full_content_equal'] for p in row['pages']) else 'fail'
            rows[name] = row
            result['roles'][role] = rows
            (root / 'ocr.json').write_text(json.dumps(result, indent=2) + '\n')
            print(role, name, row['status'], flush=True)
    if a.content_only:
        result['memory'] = {'status': 'NOT RERUN; prior failure unchanged; content-only follow-up'}
        (root / 'ocr.json').write_text(json.dumps(result, indent=2) + '\n')
        return
    # Real initialized OCR executable, pre-input limits, no sampler.
    for kind in ('anonymous', 'mapped-file', 'mach', 'many-small', 'malloc', 'burst', 'raise', 'reserved-dirty'):
        result['memory'][kind] = w.execute([helper, sb, 'memory', assets, kind, scratch / kind], root, 'memory-' + kind)
    positive = w.execute([helper, sb, 'control-sockets', assets], root, 'control-sockets')
    confined = w.execute([helper, sb, 'sockets', assets], root, 'confined-sockets')
    before = positive.get('probe', {}).get('checks', [])
    after = confined.get('probe', {}).get('checks', [])
    result['strict_sockets'] = {'control': positive, 'confined': confined,
        'status': 'pass' if len(before) == len(after) == 12 and all(
            p['descriptor_returned'] and not q['descriptor_returned'] and q['rc'] == -1 and q['errno'] in (1, 13, 24)
            for p, q in zip(before, after)) else 'fail'}
    (root / 'ocr.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for key in ('evidence', 'assets', 'worker', 'pdfium', 'drafts'):
        p.add_argument('--' + key, type=Path, required=True)
    p.add_argument('--content-only', action='store_true', help='rerun affected raster/OCR content without memory probes')
    main(p.parse_args())
