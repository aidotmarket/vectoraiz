#!/usr/bin/env python3
"""Descriptor-only macOS feasibility; independent controls, never retries jobs."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import socket
import struct

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("workers", ROOT / "scripts/office-s1934-workers.py")
w = importlib.util.module_from_spec(spec)
spec.loader.exec_module(w)
UNPROVEN = ("open_url", "mach_broker", "mach_open_url", "xpc_broker", "unix_connect")


def probes():
    rows = ['snapshot', 'scratch_create', 'host_read', 'host_stat', 'host_list', 'host_write',
            'host_create', 'host_rename', 'host_unlink', 'snapshot_write', 'child_exec', 'child_fork',
            *UNPROVEN[:1], 'dns', *UNPROVEN[1:], 'system_socket', 'socketpair',
            'inherited_handle', 'inherited_host_read']
    for family, destinations in ((4, ('127.0.0.1', '192.0.2.1', '1.1.1.1')),
                                 (6, ('::1', 'fe80::1%lo0', '2606:4700:4700::1111'))):
        for udp in (0, 1):
            for destination in destinations:
                for operation in ('socket', 'connect', 'bind', 'listen', 'sendto', 'sendmsg', 'connectx'):
                    rows.append(f'{operation}:{family}:{udp}:{destination}')
            rows.append(f'multicast:{family}:{udp}:' + ('::1' if family == 6 else '127.0.0.1'))
    assert len(rows) == 110
    return rows


def frame(value):
    value = json.dumps(value).encode()
    return struct.pack('<I', len(value)) + value


def main(a):
    root = a.evidence.resolve()
    if root == ROOT or ROOT in root.parents:
        raise ValueError('private evidence outside checkout required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    worker = a.target.resolve() / 'release/vz-office-worker'
    pdfium = a.pdfium.resolve()
    signed = w.common.run(['codesign', '--force', '--sign', '-', worker], root, 'sign-worker')
    assert signed['exit_code'] == 0
    inventory = json.loads(a.inventory.read_text())
    sentinel = root / 'host-sentinel.txt'
    snapshot = root / 'supplied.txt'
    sentinel.write_text('private host sentinel\n')
    snapshot.write_text('supplied snapshot\n')
    result = {'executable_sha256': w.common.digest(worker), 'pdfium_sha256': w.common.digest(pdfium),
              'roles': {}, 'Windows': {'status': 'NOT RUN'}, 'sampler_enabled': False,
              'ocr': {'status': 'blocked', 'refusal': 'Scanned pages cannot be read with the installed offline OCR.'}}
    # A real AF_UNIX endpoint supplies a route-positive control outside the worker.
    unix_path = root / 'control.sock'
    if unix_path.exists(): unix_path.unlink()
    with socket.socket(socket.AF_UNIX) as listener:
        listener.bind(str(unix_path)); listener.listen(32)
        for role in ('ingest', 'render'):
            target = root / role
            target.mkdir(exist_ok=True)
            scratch = target / 'scratch'
            scratch.mkdir(exist_ok=True)
            sb = target / 'profile.sb'
            sb.write_text(w.profile(worker, snapshot, pdfium, scratch))
            def argv(mode, *extra): return [worker, sb, mode, pdfium, *extra]
            positive = {}
            for name in (*UNPROVEN, 'socketpair', 'system_socket'):
                host = unix_path if name == 'unix_connect' else sentinel
                row = w.execute(argv('control-probe', name, snapshot, host, '9'), target, 'positive-' + name)
                row['positive_control_succeeded'] = row.get('probe', {}).get('rc', -1) >= 0
                positive[name] = row
            sockets_control = w.execute(argv('control-sockets'), target, 'positive-sockets')
            checks = w.execute(argv('socket-checks'), target, 'socket-checks')
            controls = sockets_control.get('probe', {}).get('checks', [])
            strict = checks.get('probe', {}).get('checks', [])
            checks['status'] = 'pass' if len(strict) == len(controls) == 12 and all(
                x['rc'] == -1 and x['errno'] in (1, 13, 24) and not x['descriptor_returned'] and c['descriptor_returned']
                for x, c in zip(strict, controls)) else 'fail'
            offered = {}
            for number, name in enumerate(probes()):
                host = unix_path if name == 'unix_connect' else sentinel
                if name == 'scratch_create': host = scratch / f'preopened-{number}'
                if name == 'host_create': host = root / f'forbidden-{role}'
                with sentinel.open('rb') as inherited:
                    row = w.execute(argv('probe', name, snapshot, host, str(inherited.fileno())),
                                    target, f'probe-{number}', inherited=(inherited.fileno(),))
                probe = row.get('probe', {})
                if name in ('snapshot', 'scratch_create'):
                    passed = probe.get('rc', -1) >= 0
                    row['positive_semantics'] = 'supplied descriptor read/write; worker does not open paths'
                else:
                    passed = probe.get('rc') == -1 and probe.get('errno') in (1, 13, 24)
                    if name.startswith('inherited'): passed = probe.get('rc') == -1 and probe.get('errno') == 9
                row['status'] = 'pass' if passed else 'fail'
                if name in UNPROVEN:
                    row['E4_status'] = 'unproven: error without positive control'
                    row['positive_control_succeeded'] = positive[name]['positive_control_succeeded']
                    if name == 'unix_connect':
                        passed = positive[name]['positive_control_succeeded'] and probe.get('rc') == -1 and not probe.get('socket_created') and probe.get('errno') in (1, 13, 24)
                        row['status'] = 'pass' if passed else 'fail'
                    elif not positive[name]['positive_control_succeeded'] or not passed:
                        row['status'] = 'fail'
                        row['reason'] = 'unproven: error without positive control or explicit denial'
                offered[name] = row
            content = {}
            for extension in ('txt', 'md', 'html', 'docx', 'xlsx', 'pdf', 'pdf_scan'):
                candidates = [Path(x['path']) for x in inventory if x['format'] == ('pdf' if extension == 'pdf_scan' else extension)
                              and (extension != 'pdf_scan' or 'scanned' in x['name'])]
                fixture = min(candidates, key=lambda p:p.stat().st_size) if candidates else None
                if a.fixtures:
                    draft_candidates = list(a.fixtures.glob('*.' + ('pdf' if extension == 'pdf_scan' else extension)))
                    draft_candidates = [p for p in draft_candidates if ('scan' in p.name) == (extension == 'pdf_scan')]
                    if draft_candidates: fixture = min(draft_candidates,key=lambda p:p.stat().st_size)
                if fixture is None:
                    content[extension] = {'status': 'blocked', 'reason': 'fixture absent'}; continue
                sb.write_text(w.profile(worker, fixture, pdfium, scratch))
                request = frame({'path': str(fixture), 'pdfium': str(pdfium)})
                baseline = w.execute(argv('control-' + role, fixture, scratch / ('control-' + extension)), target,
                                     'content-positive-' + extension, request, readiness=True)
                confined = w.execute(argv(role, fixture, scratch / ('confined-' + extension)), target,
                                     'content-confined-' + extension, request, readiness=True)
                expected, actual = baseline.get('response'), confined.get('response')
                same = expected is not None and expected == actual and baseline['exit_code'] == confined['exit_code'] == 0
                scans = any(p.get('ocr_required') for p in (actual or {}).get('pages', []))
                content[extension] = {'status': 'pass' if same and not scans else 'fail',
                    'full_content_equal': same, 'comparison': 'all pages, text, HTML, dimensions and every PNG byte; catches font substitution/lazy resource differences',
                    'baseline': baseline, 'confined': confined,
                    'ocr_refusal': result['ocr']['refusal'] if scans else None,
                    'partial_output_discarded': scans, 'snapshot_promoted': False,
                    'limitation': 'equality is regression evidence; DOCX images/styles/accessibility completeness remains unqualified'}
            result['roles'][role] = {'probes': offered, 'positive_controls': positive, 'socket_positive': sockets_control,
                                     'strict_creation': checks, 'content': content}
            (root / 'descriptor-workers.json').write_text(json.dumps(result, indent=2))
    unix_path.unlink()


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for key in ('evidence', 'target', 'inventory', 'pdfium'): p.add_argument('--' + key, type=Path, required=True)
    p.add_argument('--fixtures',type=Path)
    main(p.parse_args())
