#!/usr/bin/env python3
"""Repeat identical PDFium renders, compare pixels, and trace lazy file opens."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
def module(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m
d = module('descriptor', 'scripts/office-s1939-workers.py')
png = module('png', 'scripts/office-png.py')
w = d.w


def compare(before, after):
    result = {'all_fields_equal': before == after, 'pages': []}
    if not before or not after:
        result['failure'] = 'missing response'
        return result
    result['page_count_equal'] = len(before['pages']) == len(after['pages'])
    for a, b in zip(before['pages'], after['pages']):
        x, y = png.decode(bytes(a['png'])), png.decode(bytes(b['png']))
        dimensions_equal = x[:3] == y[:3]
        differences = [i for i, (p, q) in enumerate(zip(x[3], y[3])) if p != q] if dimensions_equal else []
        pixels = {i//x[2] for i in differences}
        bounds = ([min(i%x[0] for i in pixels), min(i//x[0] for i in pixels),
                   max(i%x[0] for i in pixels), max(i//x[0] for i in pixels)] if pixels else None)
        result['pages'].append({'page': a['page'], 'png_bytes_equal': a['png'] == b['png'],
            'text_equal': a['text'] == b['text'], 'non_raster_fields_equal': {k:v for k,v in a.items() if k!='png'} == {k:v for k,v in b.items() if k!='png'},
            'dimensions_equal': dimensions_equal, 'width': x[0], 'height': x[1],
            'pixels_equal': dimensions_equal and x[3] == y[3], 'changed_pixels': len(pixels),
            'changed_channels': len(differences), 'changed_bbox': bounds,
            'before_pixel_sha256': hashlib.sha256(x[3]).hexdigest(), 'after_pixel_sha256': hashlib.sha256(y[3]).hexdigest()})
    return result


def main(a):
    root = a.evidence.resolve()
    if root == ROOT or ROOT in root.parents: raise ValueError('private evidence required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    worker, pdfium = a.worker.resolve(), a.pdfium.resolve()
    tracer = root / 'file-trace.dylib'
    build = w.common.run(['xcrun', 'clang', '-dynamiclib', ROOT / 'scripts/office-file-trace.c', '-o', tracer], root, 'build-tracer')
    if build['exit_code']: raise RuntimeError(build)
    signed = w.common.run(['codesign', '--force', '--sign', '-', tracer], root, 'sign-tracer')
    if signed['exit_code']: raise RuntimeError(signed)
    drafts = json.loads(a.drafts.read_text())
    result = {'worker_sha256': w.common.digest(worker), 'pdfium_sha256': w.common.digest(pdfium),
              'diagnostic_tracer_sha256': w.common.digest(tracer), 'roles': {},
              'tracer_policy': 'diagnostic only; qualified comparisons run without loader override',
              'renderer_config': 'same pinned PDFium; target width 1600; same default render/color configuration',
              'Windows': 'NOT RUN'}
    for role in ('ingest', 'render'):
        target = root / role
        target.mkdir(exist_ok=True)
        scratch = target / 'scratch'
        scratch.mkdir(exist_ok=True)
        rows = {}
        for fixture in (x for x in drafts if x['format']=='pdf'):
            name, path = fixture['id'], Path(fixture['path'])
            if w.common.digest(path) != fixture['sha256']: raise ValueError('fixture changed')
            sb = target / (name+'.sb')
            sb.write_text(w.profile(worker, path, pdfium, scratch))
            request = d.frame({'path': str(path), 'pdfium': str(pdfium)})
            responses, runs = {}, {}
            for mode in (() if a.trace_only else ('control-1', 'control-2', 'confined-1', 'confined-2')):
                worker_mode = 'control-'+role if mode.startswith('control') else role
                row = w.execute([worker, sb, worker_mode, pdfium, path, scratch / (name+mode)], target, name+'-'+mode, request, readiness=True)
                responses[mode] = row.pop('response', None)
                runs[mode] = row
            row = {'fixture_sha256': fixture['sha256'], 'language': fixture['language'], 'scanned': fixture['scanned'], 'runs': runs}
            if not a.trace_only:
                row.update(control_repeat=compare(responses['control-1'], responses['control-2']),
                           confined_repeat=compare(responses['confined-1'], responses['confined-2']),
                           control_vs_confined=compare(responses['control-1'], responses['confined-1']))
            # Explicitly segregated diagnosis: never qualify an injected process.
            for mode in (() if a.skip_trace else ('control', 'confined')):
                label = name+'-trace-'+mode
                env = {'PATH': os.environ['PATH'], 'HOME': str(target), 'DYLD_INSERT_LIBRARIES': str(tracer)}
                argv = [worker, sb, 'control-'+role if mode=='control' else role, pdfium, path, scratch/label]
                with (target/(label+'.stderr')).open('wb') as err, (target/(label+'.stdout')).open('wb') as out:
                    completed = subprocess.run(['rtk','proxy',*map(str,argv)], input=request, stdout=out, stderr=err, env=env, timeout=30)
                row[mode+'_trace'] = {'exit_code': completed.returncode, 'stderr': label+'.stderr', 'stdout': label+'.stdout',
                    'file_opens': [line for line in (target/(label+'.stderr')).read_text(errors='replace').splitlines() if line.startswith('FILE_TRACE')]}
            rows[name] = row
            result['roles'][role] = rows
            (root/'pdf-differences.json').write_text(json.dumps(result,indent=2)+'\n')
            print(role, name, 'traced' if a.trace_only else [(p['changed_pixels'],p['text_equal']) for p in row['control_vs_confined']['pages']], flush=True)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for key in ('evidence', 'worker', 'pdfium', 'drafts'):
        p.add_argument('--'+key,type=Path,required=True)
    p.add_argument('--trace-only', action='store_true')
    p.add_argument('--skip-trace', action='store_true')
    main(p.parse_args())
