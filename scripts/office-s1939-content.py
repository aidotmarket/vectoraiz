#!/usr/bin/env python3
"""All 60 pinned multilingual fixtures plus hostile refusal controls; no judging."""
import argparse
import importlib.util
import json
from pathlib import Path
import zipfile
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('descriptor', ROOT / 'scripts/office-s1939-workers.py')
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)
w = d.w


def main(a):
    root = a.evidence.resolve()
    if root == ROOT or ROOT in root.parents: raise ValueError('private evidence required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    drafts = json.loads((a.fixtures.parent / 'office-drafts.json').read_text())
    assert len(drafts) == 60
    fixtures = []
    for x in drafts:
        path = Path(x['path'])
        assert w.common.digest(path) == x['sha256']
        fixtures.append({'id': x['id'], 'path': path, 'language': x['language'], 'format': x['format'],
                         'scanned': x['scanned'], 'expect_refusal': False, 'sha256': x['sha256']})
    hostile = root / 'hostile'
    hostile.mkdir(exist_ok=True)
    for extension in ('pdf', 'docx', 'xlsx'):
        path = hostile / ('malformed.' + extension)
        path.write_bytes(b'not a valid document\x00\xff')
        fixtures.append({'id': 'malformed-' + extension, 'path': path, 'language': None, 'format': extension,
                         'scanned': False, 'expect_refusal': True, 'sha256': w.common.digest(path)})
    for extension, part in (('docx', 'word/document.xml'), ('xlsx', 'xl/workbook.xml')):
        path = hostile / ('ratio-bomb.' + extension)
        with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED) as z: z.writestr(part, b'A'*250000)
        fixtures.append({'id': 'ratio-bomb-' + extension, 'path': path, 'language': None, 'format': extension,
                         'scanned': False, 'expect_refusal': True, 'sha256': w.common.digest(path)})
    worker = a.target.resolve() / 'release/vz-office-worker'
    pdfium = a.pdfium.resolve()
    results = {'roles': {}, 'worker_sha256': w.common.digest(worker), 'pdfium_sha256': w.common.digest(pdfium),
               'comparison': 'all serialized page fields and all PNG bytes; no text-prefix or page-count shortcut',
               'qualification': 'regression detector only; structural image/table/accessibility completeness not qualified',
               'OCR': {'status': 'blocked', 'locale_tessdata_full_content_comparison': 'unrun: engine/assets unavailable'},
               'Windows': {'status': 'NOT RUN'}}
    for role in ('ingest', 'render'):
        target = root / role
        target.mkdir(exist_ok=True)
        scratch = target / 'scratch'
        scratch.mkdir(exist_ok=True)
        rows = {}
        for fixture in fixtures:
            path = fixture['path']; label = fixture['id']
            sb = target / (label + '.sb')
            sb.write_text(w.profile(worker, path, pdfium, scratch))
            request = d.frame({'path': str(path), 'pdfium': str(pdfium)})
            def argv(mode): return [worker, sb, mode, pdfium, path, scratch / (mode + '-' + label)]
            baseline = w.execute(argv('control-' + role), target, 'positive-' + label, request, readiness=True)
            confined = w.execute(argv(role), target, 'confined-' + label, request, readiness=True)
            expected = baseline.pop('response', None); actual = confined.pop('response', None)
            complete_equal = expected is not None and expected == actual and baseline['exit_code'] == confined['exit_code'] == 0
            if fixture['expect_refusal']:
                passed = baseline['exit_code'] != 0 and confined['exit_code'] != 0 and not confined['deadline_kill'] and actual is None
            else:
                passed = complete_equal and not fixture['scanned']
            rows[label] = {**{k: str(v) if isinstance(v, Path) else v for k, v in fixture.items()},
                'status': 'pass' if passed else 'fail', 'full_content_equal': complete_equal,
                'baseline': baseline, 'confined': confined, 'parent_discarded': fixture['scanned'] or not passed,
                'refusal': 'Scanned pages cannot be read with the installed offline OCR.' if fixture['scanned'] else None,
                'snapshot_promoted': False}
            results['roles'][role] = rows
            (root / 'content.json').write_text(json.dumps(results, indent=2))
        print(role, sum(x['status'] == 'pass' for x in rows.values()), len(rows), flush=True)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for key in ('evidence', 'target', 'pdfium', 'fixtures'): p.add_argument('--' + key, type=Path, required=True)
    main(p.parse_args())
