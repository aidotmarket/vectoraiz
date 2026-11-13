#!/usr/bin/env python3
"""Release-profile C1 measurement coordinator; private evidence, no product wiring."""
import argparse
import csv
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('common', ROOT / 'scripts/office-s1934.py')
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)
E3 = Path('/var/tmp/koskadeux/private-evidence/280dc1c8a3a1')
E4 = Path('/var/tmp/koskadeux/private-evidence/8588b12d5b30')
RELEVANCE = Path('/var/tmp/koskadeux/private-evidence/e706de79d202/relevance')


def main(a):
    root = a.evidence.resolve()
    if root == ROOT or ROOT in root.parents: raise ValueError('private evidence required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    inventory = E3 / 'run-v2/public-inventory.json'
    entries = json.loads(inventory.read_text())
    assert len(entries) == 1000 and all(c.digest(x['path']) == x['sha256'] for x in entries)
    source = E3 / 'run-v2/public/public-state'
    env = dict(os.environ, CMAKE_DISABLE_FIND_PACKAGE_OpenSSL='TRUE', CARGO_TARGET_DIR=str(a.target.resolve()),
               VZ_S1934_SOURCE=str(source), VZ_OFFICE_ASSETS=str(E3 / 'run-v2/models'),
               VZ_OFFICE_PINS=str(E3 / 'run-v2/models/pins.json'))
    for key in ('LANCE_FTS_NUM_SHARDS', 'LANCE_FTS_PARTITION_SIZE', 'LANCE_FTS_FLUSH_SIZE', 'LANCE_CPU_THREADS', 'LANCE_IO_CORE_RESERVATION'):
        env.pop(key, None)
    build = c.run([c.CARGO, 'test', '--release', '-p', 'vz-core', '--features', 'office-feasibility',
                   '--test', 'office_s1934', '--no-run'], root, 'release-build', env)
    if build['exit_code']: raise RuntimeError(build)
    binaries = [p for p in (a.target / 'release/deps').glob('office_s1934-*') if p.is_file() and os.access(p, os.X_OK) and not p.suffix]
    assert len(binaries) == 1
    binary = binaries[0]
    pins = {'inventory_sha256': c.digest(inventory), 'prior_manifest_sha256': c.digest(E4 / 'manifest-v3.json'),
            'source_state': str(source), 'documents': 1000, 'chunks': 54099, 'pages': 7747,
            'binary_sha256': c.digest(binary), 'cargo_lock_sha256': c.digest(ROOT / 'Cargo.lock'),
            'model_pins_sha256': c.digest(E3 / 'run-v2/models/pins.json'), 'source_hashes_verified': True,
            'release_graph': 'cargo test --release; default opt-level=3 for whole dependency graph',
            'cache': 'first-open/repeated filesystem cache; no OS cache flush', 'runtime_worker_threads': 4}
    (root / 'source-pins.json').write_text(json.dumps(pins, indent=2))
    if a.loaded_control_reference:
        measured = c.run(['/usr/bin/time', '-l', binary, '--exact', 's1939_prepare_under_search', '--nocapture'],
                         root, 'loaded-e4-control', dict(env, VZ_S1934_ROOT=str(a.loaded_control_reference.resolve()),
                         VZ_S1939_UNDER_SEARCH=str(root), VZ_S1939_UNDER_SEARCH_E4='1'), timeout=600)
        (root / 'commands.json').write_text(json.dumps(measured, indent=2))
        return
    matrix = {}
    for name, knobs, test in (
        ('e4-release-control', {}, 's1939_e4_control'),
        ('host-default', {}, 's1934_spool'),
        ('host-default-partition32', {'LANCE_FTS_PARTITION_SIZE': '32', 'LANCE_FTS_FLUSH_SIZE': '16'}, 's1934_spool'),
        ('shards2', {'LANCE_FTS_NUM_SHARDS': '2', 'LANCE_FTS_PARTITION_SIZE': '32', 'LANCE_FTS_FLUSH_SIZE': '16'}, 's1934_spool'),
        ('shards4', {'LANCE_FTS_NUM_SHARDS': '4', 'LANCE_FTS_PARTITION_SIZE': '32', 'LANCE_FTS_FLUSH_SIZE': '16'}, 's1934_spool'),
        ('shards2-repeat', {'LANCE_FTS_NUM_SHARDS': '2', 'LANCE_FTS_PARTITION_SIZE': '32', 'LANCE_FTS_FLUSH_SIZE': '16'}, 's1934_spool')):
        trial = root / name
        trial.mkdir(exist_ok=True)
        trial_env = dict(env, **knobs, VZ_S1934_ROOT=str(trial), VZ_S1934_PREPARE='1')
        if test == 's1939_e4_control': trial_env['VZ_S1939_E4_CONTROL'] = str(trial)
        if not (trial / 'preparation.json').exists():
            command = c.run(['/usr/bin/time', '-l', binary, '--exact', test, '--nocapture'], trial, 'prepare', trial_env, timeout=600)
            if command['exit_code']:
                matrix[name] = {'status': 'fail', 'command': command}
                (root / 'matrix.json').write_text(json.dumps(matrix, indent=2)); continue
        value = json.loads((trial / 'preparation.json').read_text())
        matrix[name] = {'status': 'pass' if value['preparation_busy_ms'] <= 60000 else 'fail', 'knobs': knobs, **value,
                        'peak_rss_evidence': str(trial / 'prepare.log'), 'disk_bytes_after': sum(p.stat().st_blocks*512 for p in trial.rglob('*') if p.is_file())}
        (root / 'matrix.json').write_text(json.dumps(matrix, indent=2))
        print(name, round(value['preparation_busy_ms'], 2), flush=True)
    # Pin shards=2 as a diagnostic recipe; no auto-selection or both-host claim.
    trial = root / 'shards2'
    shutil.copyfile(E4 / 'search/queries.json', trial / 'queries.json')
    knobs = {'LANCE_FTS_NUM_SHARDS': '2', 'LANCE_FTS_PARTITION_SIZE': '32', 'LANCE_FTS_FLUSH_SIZE': '16'}
    selected_env = dict(env, **knobs, VZ_S1934_ROOT=str(trial), VZ_S1934_CANDIDATE='exact')
    if not (trial / 'exact').exists():
        copied = c.run(['/bin/cp', '-cR', trial / 'base', trial / 'exact'], trial, 'copy-exact')
        if copied['exit_code']: raise RuntimeError(copied)
    candidate = c.run(['/usr/bin/time', '-l', binary, '--exact', 's1934_candidate', '--nocapture'], trial, 'exact', selected_env, timeout=600)
    refresh = c.run(['/usr/bin/time', '-l', binary, '--exact', 's1934_refresh', '--nocapture'], trial, 'refresh',
                    dict(selected_env, VZ_S1934_REFRESH=str(trial)), timeout=600)
    under = root / 'under-search'
    under.mkdir(exist_ok=True)
    under_command = c.run(['/usr/bin/time', '-l', binary, '--exact', 's1939_prepare_under_search', '--nocapture'],
                          under, 'prepare-under-search', dict(selected_env, VZ_S1939_UNDER_SEARCH=str(under)), timeout=600)
    relevance = root / 'relevance'
    relevance.mkdir(exist_ok=True)
    relevance_env = dict(env, VZ_S1934_ROOT=str(relevance), VZ_S1939_SCIFACT=str(RELEVANCE))
    measured = c.run(['/usr/bin/time', '-l', binary, '--exact', 's1939_scifact_exact', '--nocapture'], relevance, 'exact', relevance_env, timeout=600)
    if measured['exit_code'] == 0:
        rows = json.loads((relevance / 'scifact-exact.json').read_text())['rows']
        qrels = {}
        with (RELEVANCE / 'scifact/qrels/test.tsv').open() as f:
            for r in csv.DictReader(f, delimiter='\t'): qrels.setdefault(r['query-id'], {})['scifact-' + r['corpus-id']] = int(r['score'])
        assert len(rows) == len(qrels) and {r['id'] for r in rows} == set(qrels)
        values = []
        for row in rows:
            rel = qrels[row['id']]; documents = list(dict.fromkeys(row['documents'])); metrics = {}
            for k in (8, 10):
                ideal = sorted(rel.values(), reverse=True)[:k]
                dcg = sum((2**rel.get(doc, 0)-1)/math.log2(i+2) for i, doc in enumerate(documents[:k]))
                idcg = sum((2**grade-1)/math.log2(i+2) for i, grade in enumerate(ideal))
                metrics[f'recall@{k}'] = sum(rel.get(doc, 0) > 0 for doc in documents[:k])/sum(v > 0 for v in rel.values())
                metrics[f'nDCG@{k}'] = dcg/idcg if idcg else None
            values.append(metrics)
        scores = {key: sum(v[key] for v in values)/len(values) for key in values[0]}
        prior = json.loads((RELEVANCE / 'scores.json').read_text())['SciFact']['exact']['mean']
        (relevance / 'scores.json').write_text(json.dumps({'queries': len(rows), 'mean': scores, 'E4_exact': prior,
            'status': 'pass' if all(scores[k] >= .98*prior[k] for k in scores) else 'fail',
            'scope': 'retained frozen E4 combined index; release query rerun; not re-extraction/index qualification',
            'pins': {str(p): c.digest(p) for p in [RELEVANCE / 'combined.sqlite', RELEVANCE / 'fixture-pins.json', RELEVANCE / 'scifact/qrels/test.tsv']}}, indent=2))
    (root / 'commands.json').write_text(json.dumps({'exact': candidate, 'refresh': refresh, 'relevance': measured, 'under_search': under_command}, indent=2))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evidence', type=Path, required=True)
    p.add_argument('--target', type=Path, required=True)
    p.add_argument('--loaded-control-reference', type=Path)
    main(p.parse_args())
