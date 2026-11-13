#!/usr/bin/env python3
"""Audit the font/URL follow-up; prior failures are provenance, never reruns."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import time

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    with Path(path).open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def frames(path):
    payload = path.read_bytes()
    result = []
    while payload:
        if len(payload) < 4:
            raise ValueError('short frame')
        size = struct.unpack('<I', payload[:4])[0]
        if size > len(payload)-4:
            raise ValueError('short payload')
        result.append(json.loads(payload[4:4+size]))
        payload = payload[4+size:]
    return result


def main(a):
    e = a.evidence.resolve()
    if e == ROOT or ROOT in e.parents:
        raise ValueError('private evidence required')
    def read(name):
        return json.loads((e/name).read_text())
    def reference(path):
        return {'path': str(path.resolve()), 'sha256': sha(path)}
    pdf = read('pdf-final/pdf-differences.json')
    content = read('content-final/content.json')
    workers = read('workers-final/descriptor-workers.json')
    brokers = read('brokers-stage2/brokers.json')
    earlier_brokers = read('brokers-stage2-before-preload/brokers.json')
    ocr = read('ocr-final/ocr.json')
    hashes = {row.get('worker_sha256', row.get('executable_sha256'))
              for row in (pdf, content, workers, brokers, ocr)}
    if len(hashes) != 1:
        raise ValueError('worker hashes differ')
    pdf_summary = {}
    for role, rows in pdf['roles'].items():
        pages = [p for row in rows.values() for p in row['control_vs_confined']['pages']]
        traces = []
        run_successes = 0
        for label, row in rows.items():
            for mode, run in row['runs'].items():
                run_successes += run['exit_code'] == 0 and not run['deadline_kill'] and run['readiness'].get('ready') is True
            for mode in ('control', 'confined'):
                trace = row[mode+'_trace']
                directory = e/'pdf-final'/role
                actual = frames(directory/trace['stdout'])
                expected = frames(directory/row['runs'][mode+'-1']['stdout'])
                traces.append({'fixture': label, 'mode': mode,
                    'exit_code': trace['exit_code'],
                    'readiness_marker_found': trace['readiness_marker_found'],
                    'post_readiness_path_attempts': trace['post_readiness_path_attempts'],
                    'matches_uninjected_response': len(actual) == 2 and actual[0].get('ready') is True
                        and len(expected) == 1 and actual[1] == expected[0]})
        summary = {
            'documents': len(rows), 'pages': len(pages), 'qualified_runs': len(rows)*4,
            'qualified_run_successes': run_successes,
            'changed_documents': sum(not row['control_vs_confined']['all_fields_equal'] for row in rows.values()),
            'changed_pages': sum(not p['pixels_equal'] for p in pages),
            'changed_pixels': sum(p['changed_pixels'] for p in pages),
            'text_equal_pages': sum(p['text_equal'] for p in pages),
            'non_raster_equal_pages': sum(p['non_raster_fields_equal'] for p in pages),
            'stable_control_repeats': sum(row['control_repeat']['all_fields_equal'] for row in rows.values()),
            'stable_confined_repeats': sum(row['confined_repeat']['all_fields_equal'] for row in rows.values()),
            'diagnostic_traces': traces,
            'remaining_difference_cause': 'none observed against the identical bundled-font control',
        }
        summary['status'] = 'pass' if (summary['documents'] == 10 and summary['pages'] == 18
            and run_successes == 40 and summary['changed_documents'] == 0
            and summary['stable_control_repeats'] == summary['stable_confined_repeats'] == 10
            and all(t['exit_code'] == 0 and t['readiness_marker_found'] and
                    t['post_readiness_path_attempts'] == [] and t['matches_uninjected_response'] for t in traces)) else 'fail'
        pdf_summary[role] = summary
    ocr_rows = [row for rows in ocr['roles'].values() for row in rows.values()]
    ocr_pages = [p for row in ocr_rows for p in row['pages']]
    scanned = [row for row in ocr_rows if row['scanned']]
    confinement = {role: {'offered': len(row['probes']),
        'pass': sum(p['status'] == 'pass' for p in row['probes'].values()),
        'failed': [name for name, p in row['probes'].items() if p['status'] != 'pass'],
        'strict_socket_status': row['strict_creation']['status']}
        for role, row in workers['roles'].items()}
    contents = {role: {'fixtures': sum(not row['expect_refusal'] for row in rows.values()),
        'complete_serialized_matches': sum(row['full_content_equal'] for row in rows.values() if not row['expect_refusal']),
        'non_pdf_matches': sum(row['full_content_equal'] for row in rows.values() if row['format'] != 'pdf' and not row['expect_refusal']),
        'hostile_refusals': sum(row['status'] == 'pass' for row in rows.values() if row['expect_refusal']),
        'scan_handling': 'raster equality here; separately confined OCR results below; no snapshot promotion'}
        for role, rows in content['roles'].items()}
    broker_rows = [row for rows in brokers['roles'].values() for row in rows.values()]
    for row in workers['roles'].values():
        url = row['probes']['open_url']
        if url['status'] == 'pass' and url.get('probe', {}).get('explicit_launch_denial') is not True:
            raise ValueError('generic Open-URL error was incorrectly counted as denial')
    crashes = {}
    for role in ('ingest', 'render'):
        path = e/('url-launch-spec-'+role+'.ips')
        report = json.loads(path.read_text().split('\n', 1)[1])
        crashes[role] = {**reference(path), 'exception': report['exception'],
            'provenance': 'stage-2 retry before trusted raster warmup; final binary returns -10661 instead of trapping',
            'diagnostic_worker_sha256': earlier_brokers['worker_sha256'],
            'termination': report['termination'], 'application_specific_information': report.get('asi'),
            'triggered_frames': [f.get('symbol') for t in report['threads'] if t.get('triggered') for f in t['frames']],
            'main_frames': [f.get('symbol') for t in report['threads'] if t.get('name') == 'main' for f in t['frames']],
            'reason': 'The stage-2 crash shows LaunchServices preference loading entering CFNetwork error localization and trapping at CFRetain(NULL). '
                      'No launch OSStatus returned in that earlier run. This is not an explicit denial. '
                      'The underlying failed-resource operation is not established by the crash stack alone.'}
    sources = [*ROOT.glob('scripts/office-*'),
               ROOT/'crates/vz-parser-worker/build.rs', *ROOT.glob('crates/vz-parser-worker/src/office*'),
               ROOT/'tests/office/fixtures/hostile-worker.c', ROOT/'Cargo.lock']
    result = {
        'schema': 'office-C1-S1939-v6', 'created_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'base_sha': 'a4b8b929e26a3c43bdc6df2baf3827b776ac5f16',
        'measured_native_source_commit': a.measured_source,
        'source_pins': {str(p.relative_to(ROOT)): sha(p) for p in sorted(set(sources)) if p.is_file()},
        'prior_manifest': {**reference(a.prior), 'carried_sections_are_not_reruns': True},
        'private_evidence_root': str(e), 'host_os': (e/'os.txt').read_text(),
        'disposition': 'RETURN_TO_DESIGN; C1 FAIL; harness only; no enabled distribution',
        'worker_sha256': next(iter(hashes)), 'font_bundle': pdf['font_bundle'],
        'asset_provenance': {'pdfium_sha256': pdf['pdfium_sha256'],
            'ocr_helper_sha256': ocr['helper_sha256'], 'ocr_asset_bundle_sha256': ocr['bundle_sha256'],
            'ocr_asset_bundle_path': str(Path('/var/tmp/koskadeux/private-evidence/a1faf60504d5/assets/bundle.json')),
            'no_asset_redownload_or_new_memory_qualification': True},
        'font_provider': {'bytes': 38340252, 'files': 10, 'max_font_bytes': 24*1024*1024,
            'max_bundle_bytes': 64*1024*1024, 'interface': 'FPDF_SetSystemFontInfo; FPDF_SYSFONTINFO version 1',
            'policy': 'SHA256-checked owned bytes before confinement/baseline/readiness; same mandatory bundle in both arms; no system mapper delegation; no font path grants; callbacks only memcpy preloaded bytes',
            'font_refusal_controls': read('font-refusals.json'),
            'glyph_control': read('font-glyph-control.json'),
            'limits': 'fixture-scoped substitution policy; same-font parity does not establish original-font metrics or general document/structural fidelity'},
        'pdf': pdf_summary, 'content': contents, 'confinement': confinement,
        'ocr': {'fixtures': len(ocr_rows), 'matches': sum(row['status'] == 'pass' for row in ocr_rows),
            'pages': len(ocr_pages), 'page_matches': sum(p['full_content_equal'] for p in ocr_pages),
            'scanned_controls': len(scanned), 'scanned_matches': sum(row['status'] == 'pass' for row in scanned),
            'qualification': 'full nonempty UTF-8 equality on identical validated rasters; human accuracy/relevance not graded; memory not rerun'},
        'brokers': {'controls': len(broker_rows), 'positive_control_successes': sum(row['positive_control_succeeded'] for row in broker_rows),
            'explicit_denial_rows': sum(row['status'] == 'pass' for row in broker_rows),
            'final_open_url_reason': 'Both positive controls return OSStatus 0. Both final-profile LSOpenFromURLSpec calls return -10661 (kLSExecutableIncorrectFormat: no compatible executable found), with explicit_launch_denial=false. errno=EPERM alone does not qualify an OSStatus API. The profile/descriptor ceiling prevents launch success but this generic resolution/format result is not an explicit sandbox denial; the exact failed internal lookup is not established.',
            'url_api': 'LSOpenFromURLSpec', 'cases': brokers['roles'], 'open_url_crash_diagnosis': crashes},
        'independent_pixel_decoder_check': read('pixel-decoder-check.json'),
        'validation': read('validation.json'),
        'unchanged_obligations': {'memory_design': 'UNCHANGED; awaiting Max decision; prior hard resident guarantee FAIL; NOT RERUN',
            'Windows': 'NOT RUN', 'human_relevance_grading': 'DEFERRED', 'multi_address_flood_rig': 'DEFERRED',
            'other_prior_gaps': 'unchanged; see pinned v5 manifest; no qualification claimed'},
    }
    result['evidence_files'] = {str(p.relative_to(e)): sha(p) for p in sorted(e.rglob('*'))
        if p.is_file() and p.name not in ('manifest-v6.json', 'manifest-v6.sha256', 'manifest-generation.log')
        and not any(part in ('target', 'font-refusal-controls', 'pycache') for part in p.relative_to(e).parts)}
    path = e/'manifest-v6.json'
    path.write_text(json.dumps(result, indent=2)+'\n')
    (e/'manifest-v6.sha256').write_text(sha(path)+'  manifest-v6.json\n')
    print(json.dumps({key: result[key] for key in ('pdf', 'content', 'confinement', 'ocr')}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--prior', type=Path, required=True)
    parser.add_argument('--measured-source', required=True)
    main(parser.parse_args())
