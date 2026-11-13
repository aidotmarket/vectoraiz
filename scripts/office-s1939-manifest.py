#!/usr/bin/env python3
"""Reconcile private C1 v4 evidence without upgrading any omission to a pass."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time
ROOT = Path(__file__).resolve().parents[1]


def sha(p):
    with p.open('rb') as f: return hashlib.file_digest(f, 'sha256').hexdigest()


def main(a):
    e = a.evidence.resolve()
    if e == ROOT or ROOT in e.parents: raise ValueError('private evidence required')
    def read(p): return json.loads((e / p).read_text())
    def cmd(*argv): return subprocess.check_output(['rtk', 'proxy', *argv], text=True).strip()
    def status(value, why=None): return {'status': value, **({'reason': why} if why else {})}
    def peak(path):
        found = re.search(r'(?m)^\s*(\d+)\s+maximum resident set size', (e / path).read_text())
        return int(found[1]) if found else None
    search = a.search
    loaded = a.loaded
    matrix = read(search + '/matrix.json')
    exact = read(search + '/shards2/exact.json')
    refresh = read(search + '/shards2/refresh.json')
    under = read(search + '/under-search/under-search.json')
    control = read(loaded + '/under-search.json')
    prior = json.loads(a.prior.read_text())
    workers = read('workers-v4/descriptor-workers.json')
    content = read('content-v4/content.json')
    memory = read('memory-final/memory.json')
    ocr = read('ocr/ocr-status.json')
    scores = read(search + '/relevance/scores.json')
    for name, row in matrix.items():
        row['peak_rss_bytes'] = peak(search + '/' + name + '/prepare.log')
        row['temporary_disk_peak'] = status('unrun', 'allocated bytes after build recorded; peak temporary spill not sampled')
    busy = under['preparation']
    phases = {'writer_lookup_busy_ms': 18000, 'lance_append_busy_ms': 12000, 'fts_ms': 25000}
    phase_cases = {k: {'status': 'pass' if busy[k] <= limit else 'fail', 'measured_ms': busy[k], 'maximum_ms': limit} for k, limit in phases.items()}
    prior_busy = read('search-final/under-search/under-search.json')['preparation']
    for key, limit in phases.items():
        phase_cases[key]['prior_measured_ms'] = prior_busy[key]
        phase_cases[key]['observed_max_ms'] = max(busy[key], prior_busy[key])
        phase_cases[key]['status'] = 'pass' if max(busy[key], prior_busy[key]) <= limit else 'fail'
    phase_cases['validation_close_promotion_ms'] = {'status': 'pass' if busy['validation_close_ms'] + busy['promotion_ms'] <= 5000 else 'fail',
            'measured_ms': busy['validation_close_ms'] + busy['promotion_ms'], 'maximum_ms': 5000}
    old_owner = control['preparation']['owner_to_ready_ms']
    new_owner = busy['owner_to_ready_ms']
    previous_loaded = read('search-final/under-search/under-search.json')['preparation']
    previous_control = read('search-loaded-e4/under-search.json')['preparation']
    historical = {}
    for role, row in prior['items']['3_confinement']['roles'].items():
        failed = row['failed']
        assert len(failed) == 22
        cases = {}
        for name in failed:
            now = workers.get('roles', {}).get(role, {}).get('probes', {}).get(name, status('blocked', 'offline OCR executable/assets unavailable'))
            cases[name] = {'E4_status': 'fail', 'E4_interpretation': 'unproven: error without positive control' if name in ('open_url','mach_broker','mach_open_url','xpc_broker','unix_connect') else 'socket/socketpair/multicast creation succeeded',
                           'v4_status': now['status'], 'positive_control_succeeded': now.get('positive_control_succeeded'), 'current_evidence': now}
        historical[role] = {'failing_E4_rows_accounted': len(cases), 'cases': cases}
    confinement = {}
    for role, row in workers['roles'].items():
        confinement[role] = {'status': 'fail', 'pass': sum(p['status'] == 'pass' for p in row['probes'].values()), 'offered': len(row['probes']),
            'cases': row['probes'], 'strict_creation': row['strict_creation'], 'socket_positive': row['socket_positive'], 'broker_positive_controls': row['positive_controls']}
    confinement['ocr'] = {'status': 'blocked', 'offered': 110, 'reason': 'offline OCR engine/assets unavailable; no substitute capability binary counted as an OCR worker', 'cases': {name: status('blocked', 'offline OCR executable/assets unavailable') for name in workers['roles']['ingest']['probes']}}
    content_aggregate = {}
    for role, rows in content['roles'].items():
        ordinary = [r for r in rows.values() if not r['expect_refusal']]
        hostile = [r for r in rows.values() if r['expect_refusal']]
        content_aggregate[role] = {'status': 'fail', 'ordinary_offered': len(ordinary), 'ordinary_pass': sum(r['status'] == 'pass' for r in ordinary),
            'scanned_refused': sum(r['scanned'] for r in ordinary), 'raster_mismatches': sum(bool(r.get('differences')) for r in ordinary),
            'hostile_offered': len(hostile), 'hostile_pass': sum(r['status'] == 'pass' for r in hostile), 'cases': rows}
    tests = {}
    for name in ('fmt-final', 'python-final', 'clippy-retry', 'tests-complete'):
        file = e / (name + '.exit')
        code = int(file.read_text()) if file.exists() else None
        tests[name] = {'status': 'pass' if code == 0 else 'fail' if code is not None else 'unrun', 'exit_code': code, 'log': name + '.log'}
    sources = [p for pattern in ('scripts/office-s1939*', 'scripts/office-memory-helper.c', 'crates/vz-core/src/storage/office_s1934.rs',
               'crates/vz-core/src/office_bytes.rs', 'crates/vz-core/src/parsers.rs', 'crates/vz-core/tests/office_s1934.rs',
               'crates/vz-parser-worker/src/office*', 'tests/office/fixtures/hostile-worker.c', 'scripts/office-s1934*.py', 'Cargo.lock') for p in ROOT.glob(pattern) if p.is_file()]
    m = {'schema': 'office-C1-S1939-v4', 'source_commit': cmd('git', 'rev-parse', 'HEAD'), 'created_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
         'base_sha': 'b35c43f5eaa9c614fddb5b9ec7ea6e73035597aa', 'authority_sha': '8c8ba764e434786439456c8ea9d8df16ccf1f380',
         'authority_sha256': sha(e / 'authority-gate2.md'), 'review_sha256': sha(e / 'cc-response.md'), 'prior_manifest_sha256': sha(a.prior),
         'host': {'os': (e / 'os.txt').read_text(), 'hardware': (e / 'hardware.txt').read_text()},
         'disposition': 'RETURN_TO_DESIGN; C1 FAIL; no enabled distribution', 'private_evidence_root': str(e),
         'Windows': {'status': 'NOT RUN', 'qualification': 'fail: required other host missing'},
         'source_pins': {str(p.relative_to(ROOT)): sha(p) for p in sources},
         'measured_source_archives': {str(p.relative_to(e)): sha(p) for p in (e / 'sources-before-loaded-control').rglob('*') if p.is_file()},
         'dependency_upgrades': False, 'largest_measured_corpus': {'documents': 1000, 'chunks': 54099, 'pages': 7747}, 'largest_qualified_corpus': None,
         'preparation': {'status': 'fail', 'matrix': matrix, 'knobs_pinned_diagnostic': {'shards': 2, 'partition_mib': 32, 'flush_mib': 16, 'tokenizer': 'unchanged simple/English; stemming and stop words on; positions off'},
             'definition': 'sum writer/lookup, Lance append excluding producer receive wait, FTS and validation/promotion busy durations regardless of overlap; owner-to-ready independently measured',
             'under_25_search': {'preparation': busy, 'phases': phase_cases, 'identities': under['identities'], 'queries': len(under['samples']),
                   'max_search_ms': max(r['latency_ms'] for r in under['samples']), 'status': 'pass' if busy['preparation_busy_ms'] <= 60000 and max(r['latency_ms'] for r in under['samples']) <= 5000 else 'fail',
                   'peak_process_rss_bytes': peak(search + '/under-search/prepare-under-search.log'), 'baseline_rss_bytes': under['baseline_rss_bytes'], 'incremental_peak_rss_bytes': peak(search + '/under-search/prepare-under-search.log') - under['baseline_rss_bytes'], 'preparation_memory_status': 'pass' if peak(search + '/under-search/prepare-under-search.log') - under['baseline_rss_bytes'] <= 1536*1024*1024 else 'fail'},
             'recovery_owner_no_regression': {'status': 'pass' if new_owner <= old_owner and previous_loaded['owner_to_ready_ms'] <= previous_control['owner_to_ready_ms'] else 'fail', 'latest_pair_status': 'pass' if new_owner <= old_owner else 'fail', 'prior_pair_delta_ms': previous_loaded['owner_to_ready_ms']-previous_control['owner_to_ready_ms'], 'same_corpus': True, 'same_25_identity_queries': True,
                   'successor_ms': new_owner, 'E4_release_ms': old_owner, 'delta_ms': new_owner-old_owner,
                   'limitation': 'independent runs with shared host load/cache; no statistical guarantee'},
             'full_owner_extraction_embedding_to_ready': status('unrun', 'both controls recover identical already extracted/embedded generations; full owner extraction/embedding not repeated'),
             'promotion': 'private atomic successor directory rename and parent fsync; no product generation swap',
             'crash_recovery_repeated_retirement': status('unrun', 'private preparation and repeated build measured; crash/pointer/old-reader lifecycle not qualified')},
         'exact_search': {'status': 'pass' if max(r['latency_ms'] for r in exact['samples']) <= 5000 and exact['incremental_rss_bytes'] <= 512*1024*1024 else 'fail',
             'identities': 25, 'queries': len(exact['samples']), 'max_ms': max(r['latency_ms'] for r in exact['samples']),
             'absolute_rss_bytes': exact['one_snapshot_rss_bytes'], 'incremental_rss_bytes': exact['incremental_rss_bytes'], 'plan': exact['plan'],
             'recipe': exact['access_recipe'], 'measured_source_hashes': exact['source_hashes'], 'qualification': 'extraction regression only; human correctness and both-host A1 remain unqualified'},
         'refresh': {'status': 'pass' if refresh['no_op_ms'] <= 2000 and refresh['post_embedding_preparation_ms'] <= 10000 else 'fail', **refresh,
             'source_level_mutation_correctness': status('unrun', 'synthetic chunks, not source hash/extraction add/update/delete replay')},
         'confinement': {'status': 'fail', 'roles': confinement, 'E4_failure_accounting': historical,
             'inherited_Mach_ports': status('unrun', 'arbitrary inherited Mach-right closure not qualified'),
             'routed_LAN_internet_packet_capture_listener_enumeration': status('unrun', 'literal creation and local AF_UNIX positive controls do not establish all routed outcome evidence')},
         'full_content': {'status': 'fail', 'roles': content_aggregate, 'detector': 'full page fields plus every PNG byte; mismatched output discarded, no retry',
             'PDFium_font_resource_substitution': {'status': 'fail', 'reason': 'successful confined PDFs change PNG bytes while text/structure match; full comparison detects silent rendering degradation; exact lazy-open cause not traced'},
             'OCR_lazy_tessdata_locale_comparison': status('unrun', 'Tesseract engine/assets absent'), 'structural_images_tables_accessibility': status('unrun', 'equality is a regression detector, not complete structural qualification')},
         'ocr': ocr, 'memory': memory, 'SciFact_Exact': scores,
         'human_relevance_grading': status('unrun', 'DEFERRED (Mars #35415); no judges or invented qrels'),
         'multi_address_flood_rig': status('unrun', 'DEFERRED (Mars #35415); not built or run'),
         'other_carried_C1_obligations': {k: status('unrun', 'carried, outside this isolated redesign rerun') for k in ('tenant_signins','installed_hosts','power_network_PEM_trust','accepted_refusal_maximum','answers_desktop_cache_mutation_lease')},
         'N1': 'Any corpus/access-path/recipe change requires requalification; no both-host qualified corpus.',
         'N2': 'Exact has no vector index to update; FTS new-data indexing plus merge is timed independently.',
         'N3': 'Below-budget positives may succeed; count dirtyable baseline inside budget; sampler disabled; replacement failure keeps hard resident guarantee FAIL.',
         'validation': tests,
         'preserved_failed_attempts': {'missing_test_assets': 'tests-final.log', 'parallel_worker_signal': 'tests-assets.log',
                 'missing_legacy_Vision_test_helper': 'tests-serial.log', 'initial_Clippy_closure_lint': 'clippy-final.log',
                 'misdirected_build_cancelled': 'search/release-build-misdirected.log', 'OCR_link_failure': 'ocr/tesseract-link-probe.log'},
         'previous_observations': {'search': read('search-final/matrix.json'), 'loaded_successor': read('search-final/under-search/under-search.json')['preparation'],
                 'loaded_E4': read('search-loaded-e4/under-search.json')['preparation']}}
    m['evidence_pins'] = {str(p.relative_to(e)): sha(p) for folder in (search,loaded,'workers-v4','content-v4','memory-final','ocr') for p in (e / folder).rglob('*.json')}
    (e / 'manifest-v4.json').write_text(json.dumps(m, indent=2) + '\n')
    rows = [('Documents',1000),('Chunks',54099),('Pages',7747),('Preparation busy, ms',round(matrix['shards2']['preparation_busy_ms'],2)),
            ('Preparation under search, ms',round(busy['preparation_busy_ms'],2)),('Search max, ms',round(m['exact_search']['max_ms'],2)),
            ('Confinement passes per role',min(r['pass'] for role,r in confinement.items() if role != 'ocr')),('Confinement probes per role',110),('Strict socket checks per role',12),
            ('Writable replacement bytes',2147483648),('SciFact queries',scores['queries']),('SciFact recall@10',round(scores['mean']['recall@10'],6)),
            ('SciFact nDCG@10',round(scores['mean']['nDCG@10'],6))]
    table = '| Measurement | macOS |\n|---|---:|\n' + ''.join(f'| {name} | {value} |\n' for name,value in rows)
    (e / 'results-table.md').write_text(table)
    text = '# Office C1 S1939 feasibility rerun\n\n**RETURN_TO_DESIGN; C1 FAIL. No enabled distribution.**\n\n' + table
    text += f"\nRelease preparation uses a streamed typed successor, two batches at most, and separately charged writer/lookup, append, FTS and promotion work. The loaded FTS phase is {busy['fts_ms']/1000:.3f} s against 25 s. Loaded recovery owner-to-ready is {new_owner/1000:.3f} s versus {old_owner/1000:.3f} s for the E4 release control. Full owner extraction/embedding was not rerun.\n"
    text += f"\nPreserved earlier loaded observation: FTS {prior_busy['fts_ms']/1000:.3f} s; successor owner-to-ready {previous_loaded['owner_to_ready_ms']/1000:.3f} s versus E4 {previous_control['owner_to_ready_ms']/1000:.3f} s. Its phase/no-regression misses remain failures despite the faster repeat.\n"
    text += '\nIngest and render each retain four unproven/failing broker/open-URL rows. All 22 E4 failures per role are accounted for; the five originally lacking controls remain labelled in history. AF_UNIX now has a successful external listener control. Literal socket checks pass before and after closing a supplied descriptor; this does not qualify routed outcome confinement.\n'
    text += '\nEach role compares 60 multilingual office fixtures plus five hostile refusal controls. Fifty ordinary fixtures match; eight non-scanned PDFs change raster bytes and two scans are refused. OCR is blocked: the Tesseract header/link probe fails and all five offline traineddata/license pins are absent. No download or unconfined retry occurs.\n'
    text += '\nThe dedicated memory helper refuses an over-budget dirtyable baseline, but replacing excluded read-only mappings permits touching 2 GiB writable memory. The hard resident requirement remains FAIL. Real ingest/render startup and allocation probes are retained separately; OCR is blocked.\n'
    text += '\nSciFact Exact reruns all 300 frozen queries on the retained E4 combined index and exactly matches its baseline; this is not re-extraction/index qualification. Windows is NOT RUN. Human relevance grading and the multi-address flood rig are UNRUN, DEFERRED (Mars #35415). Peak temporary disk, full structural accessibility/content qualification, installed/routed/both-host evidence and remaining C1 obligations remain unrun.\n'
    (ROOT / 'docs/office-c1-s1939-rerun.md').write_text(text)
    print(json.dumps({'manifest': str(e / 'manifest-v4.json'), 'status': 'fail', 'table': rows}))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evidence', type=Path, required=True)
    p.add_argument('--prior', type=Path, default=Path('/var/tmp/koskadeux/private-evidence/8588b12d5b30/manifest-v3.json'))
    p.add_argument('--search', default='search-v4')
    p.add_argument('--loaded', default='search-loaded-e4-v4')
    main(p.parse_args())
