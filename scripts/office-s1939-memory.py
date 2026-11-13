#!/usr/bin/env python3
"""Audit a dedicated small helper; missing actual-role proof remains a failure."""
import argparse
import importlib.util
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('workers', ROOT / 'scripts/office-s1934-workers.py')
w = importlib.util.module_from_spec(spec)
spec.loader.exec_module(w)


def main(a):
    root = a.evidence.resolve()
    if root == ROOT or ROOT in root.parents: raise ValueError('private evidence required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    helper = root / 'memory-helper'
    build = w.common.run(['xcrun', 'clang', '-O2', '-Wno-deprecated-declarations',
                          ROOT / 'scripts/office-memory-helper.c', '-lsandbox', '-o', helper], root, 'build-memory-helper')
    assert build['exit_code'] == 0
    sign = w.common.run(['codesign', '--force', '--sign', '-', helper], root, 'sign-memory-helper')
    assert sign['exit_code'] == 0
    results = {'status': 'fail', 'helper_sha256': w.common.digest(helper), 'build': build, 'sampler_enabled': False,
               'guarantee': 'address-space growth only; no hard resident cap established', 'roles': {},
               'Windows': {'status': 'NOT RUN'}, 'actual_worker_memory_qualification': 'unrun: dedicated helper diagnostic is not full parser/PDFium/OCR allocation proof'}
    for role in ('ingest', 'render', 'ocr'):
        target = root / role
        target.mkdir(exist_ok=True)
        profile = target / 'profile.sb'
        profile.write_text(w.profile(helper, helper, a.pdfium.resolve(), target))
        rows = {}
        for kind in ('anonymous', 'mapped-file', 'mach', 'many-small', 'malloc', 'burst', 'raise', 'reserved-dirty', 'replacement'):
            if role == 'ocr':
                rows[kind] = {'status': 'blocked', 'reason': 'offline OCR engine and assets unavailable'}; continue
            row = w.execute([helper, profile, kind, target / (kind + '.map'), role, a.pdfium.resolve()], target, kind)
            setup = next((x for x in row['bootstrap'] if 'baseline_mapped_bytes' in x), {})
            valid = (setup.get('set_rc') == 0 and setup.get('requested_limit') == setup.get('soft') == setup.get('hard')
                     and setup.get('requested_limit', 0) == setup.get('baseline_mapped_bytes', 0) + setup.get('remaining_budget_bytes', 0)
                     and setup.get('dirtyable_baseline_bytes', 2**64) + setup.get('remaining_budget_bytes', 0) == 1536*1024*1024)
            probe = row.get('probe', {})
            refusal = valid and row['exit_code'] == 0 and probe.get('rc') == -1 and (probe.get('errno') in (1, 12) or probe.get('mach_error') == 3)
            if kind in ('anonymous', 'many-small', 'malloc', 'mach') and not probe.get('cumulative_new_bytes', 0): refusal = False
            row.update(status='pass' if refusal else 'fail', setup_valid=valid, kernel_refusal=refusal)
            if kind == 'reserved-dirty':
                row['status'] = 'pass' if row['exit_code'] == 72 else 'fail'
                row['case'] = 'refuse startup when counted dirtyable baseline exceeds budget; not allocation proof'
            if kind == 'replacement': row['case'] = 'unmap excluded read-only reservation then map/touch writable replacement'
            rows[kind] = row
        results['roles'][role] = rows
        (root / 'memory.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for key in ('evidence', 'pdfium'): p.add_argument('--' + key, type=Path, required=True)
    main(p.parse_args())
