#!/usr/bin/env python3
"""S1934 real executable memory/confinement probes; private evidence only."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import selectors
import signal
import socket
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("s1934", ROOT / "scripts/office-s1934.py")
common = importlib.util.module_from_spec(spec)
spec.loader.exec_module(common)


def profile(binary, snapshot, pdfium, scratch):
    def literal(path):
        return '(literal ' + json.dumps(str(path)) + ')'
    keys = ('hw.ncpu', 'hw.logicalcpu', 'hw.physicalcpu', 'hw.pagesize',
            'hw.memsize', 'kern.osrelease', 'kern.osversion')
    return ('(version 1)(deny default)'
            '(deny network* system-socket mach-lookup mach-register appleevent-send process-fork process-exec)'
            '(allow file-read* (subpath "/System") (subpath "/usr/lib") '
            '(literal "/") (literal "/dev/null") ' + ' '.join(map(literal, [binary, snapshot, pdfium])) + ')'
            '(allow file-read* file-write* (subpath ' + json.dumps(str(scratch)) + '))'
            '(allow file-write-data (literal "/dev/null"))'
            '(allow sysctl-read ' + ' '.join('(sysctl-name ' + json.dumps(k) + ')' for k in keys) + ')')


def execute(argv, target, label, data=None, readiness=False, inherited=()):
    """No RSS sampler. Bounded IPC, wall deadline and process-group reaping."""
    start = time.monotonic()
    with (target / (label + '.stderr')).open('wb') as error:
        process = subprocess.Popen([common.RTK, 'proxy', *map(str, argv)], cwd=ROOT,
                                   env={'PATH': os.environ['PATH'], 'HOME': str(target)},
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=error,
                                   start_new_session=True, pass_fds=inherited)
        timed_out = False
        ready = None
        try:
            if readiness == 'stderr':
                deadline = time.monotonic()+10
                while time.monotonic() < deadline and process.poll() is None:
                    if b'"ready":true' in (target / (label + '.stderr')).read_bytes():
                        ready = {'ready': True, 'channel': 'stderr'}
                        break
                    time.sleep(.01)
                if not ready:
                    raise ValueError('OCR exited before readiness')
            elif readiness:
                selector = selectors.DefaultSelector()
                selector.register(process.stdout, selectors.EVENT_READ)
                if not selector.select(10):
                    raise TimeoutError('no readiness')
                prefix = process.stdout.read(4)
                if len(prefix) != 4:
                    raise ValueError('worker exited before readiness')
                size = struct.unpack('<I', prefix)[0]
                if size > 4096:
                    raise ValueError('oversize readiness')
                ready = json.loads(process.stdout.read(size))
                if not ready.get('ready'):
                    raise ValueError('worker not ready')
            output, _ = process.communicate(data, timeout=25)
        except (subprocess.TimeoutExpired, TimeoutError, ValueError) as exc:
            timed_out = isinstance(exc, (subprocess.TimeoutExpired, TimeoutError))
            os.killpg(process.pid, signal.SIGKILL)
            output, _ = process.communicate()
            ready = {'failure': str(exc), 'prior_readiness': ready}
        (target / (label + '.stdout')).write_bytes(output)
    result = {'exit_code': process.returncode, 'elapsed_s': time.monotonic() - start,
              'deadline_kill': timed_out, 'readiness': ready,
              'stdout': label + '.stdout', 'stderr': label + '.stderr', 'sampler_enabled': False}
    try:
        result['probe'] = json.loads(output)
    except (ValueError, UnicodeDecodeError):
        pass
    if readiness and len(output) >= 4:
        size = struct.unpack('<I', output[:4])[0]
        if size == len(output) - 4 and size <= 32*1024*1024:
            result['response'] = json.loads(output[4:])
    result['bootstrap'] = []
    for line in (target / (label + '.stderr')).read_text(errors='replace').splitlines():
        try:
            entry = json.loads(line)
            if 'baseline_mapped_bytes' in entry or 'writable_baseline_virtual_bytes' in entry or 'nofile_soft' in entry:
                result['bootstrap'].append(entry)
        except ValueError:
            pass
    result['profile_sha256'] = common.digest(Path(argv[1]))
    return result


def main(args):
    root = args.evidence.resolve()
    if root == ROOT or ROOT in root.parents:
        raise ValueError('private evidence required')
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    worker = args.target.resolve() / 'debug/vz-office-worker'
    ocr = root / 'vz-office-ocr'
    build = common.run(['xcrun', 'clang', '-fblocks', '-Wno-deprecated-declarations',
                        ROOT / 'scripts/office-ocr.m', ROOT / 'crates/vz-parser-worker/src/office-bootstrap.c',
                        '-framework', 'Foundation', '-framework', 'Vision', '-framework', 'CoreServices',
                        '-lsandbox', '-o', ocr], root, 'build-ocr')
    if build['exit_code']:
        raise RuntimeError(build)
    for binary in (worker, ocr):
        signed = common.run(['codesign', '--force', '--sign', '-', binary], root, 'sign-' + binary.name)
        if signed['exit_code']:
            raise RuntimeError(signed)
    inventory = json.loads(args.inventory.read_text())
    sentinel = root / 'host-sentinel.txt'
    sentinel.write_text('private host sentinel\n')
    snapshot = root / 'snapshot.txt'
    snapshot.write_text('supplied read-only snapshot\n')
    results = {'memory': {}, 'confinement': {}, 'controls': {}, 'Windows': {'status': 'NOT RUN'},
               'limits': ['only this macOS build; no hard resident-memory proof',
                          'inherited Mach ports and full rendered image/accessibility fidelity remain unqualified',
                          'routed LAN/internet positives and packet capture omitted: FAIL',
                          'OCR offline asset list not established: FAIL'],
               'executable_hashes': {str(p): common.digest(p) for p in (worker, ocr)}}
    for role in ('ingest', 'render', 'ocr'):
        target = root / role
        target.mkdir(exist_ok=True)
        scratch = target / 'scratch'
        scratch.mkdir(exist_ok=True)
        binary = ocr if role == 'ocr' else worker
        sb = target / 'profile.sb'
        sb.write_text(profile(binary, snapshot, args.pdfium.resolve(), scratch))
        def argv(mode, *extra):
            return [binary, sb, mode, *([] if role == 'ocr' else [args.pdfium.resolve()]), *extra]
        memory = {}
        for kind in (() if args.section == 'confinement' else ('anonymous', 'mapped-file', 'mach', 'many-small', 'malloc', 'burst', 'raise', 'reserved-dirty')):
            row = execute(argv('memory', kind, scratch / ('allocation-' + kind)), target, 'memory-' + kind)
            probe = row.get('probe', {})
            explicit = probe.get('rc') == -1 and (probe.get('errno') == (1 if kind == 'raise' else 12) or probe.get('mach_error') == 3)
            # N3: small allocations succeed; cumulative crossing must be refused.
            setup = next((b for b in row['bootstrap'] if 'baseline_mapped_bytes' in b), {})
            setup_ok = (setup.get('set_rc') == 0 and setup.get('requested_limit') == setup.get('soft') == setup.get('hard')
                        and setup.get('requested_limit', 0) == setup.get('baseline_mapped_bytes', 0) + 1536*1024*1024)
            row['kernel_refusal'] = bool(setup_ok and explicit and not row['deadline_kill'] and row['exit_code'] == 0)
            if kind in ('anonymous', 'many-small', 'malloc', 'mach') and not probe.get('cumulative_new_bytes', 0):
                row['kernel_refusal'] = False
                row['failure'] = 'no below-budget positive allocations proven'
            if kind == 'reserved-dirty':
                row['status'] = 'FAIL'
            memory[kind] = row
        results['memory'][role] = memory
        if args.section == 'memory':
            (root / 'workers-memory-partial.json').write_text(json.dumps(results, indent=2))
            continue
        probes = ['snapshot', 'scratch_create', 'host_read', 'host_stat', 'host_list', 'host_write',
                  'host_create', 'host_rename', 'host_unlink', 'snapshot_write', 'child_exec', 'child_fork',
                  'open_url', 'dns', 'mach_broker', 'mach_open_url', 'xpc_broker', 'unix_connect',
                  'system_socket', 'socketpair', 'inherited_handle', 'inherited_host_read']
        for family, destinations in ((4, ('127.0.0.1', '192.0.2.1', '1.1.1.1')),
                                     (6, ('::1', 'fe80::1%lo0', '2606:4700:4700::1111'))):
            for udp in (0, 1):
                for destination in destinations:
                    for operation in ('socket', 'connect', 'bind', 'listen', 'sendto', 'sendmsg', 'connectx'):
                        probes.append(f'{operation}:{family}:{udp}:{destination}')
                probes.append(f'multicast:{family}:{udp}:' + ('::1' if family == 6 else '127.0.0.1'))
        confined = {}
        for number, probe in enumerate(probes):
            # No probes may alter the sentinel used by another trial.
            host = scratch / ('created-' + str(number)) if probe == 'scratch_create' else sentinel
            if probe == 'host_create':
                host = root / ('forbidden-create-' + role)
            sentinel.write_text('private host sentinel\n')
            with sentinel.open('rb') as inherited:
                row = execute(argv('probe', probe, snapshot, host,
                                   str(inherited.fileno()) if probe == 'inherited_host_read' else '9'),
                              target, 'probe-' + str(number), inherited=(inherited.fileno(),))
            result = row.get('probe', {})
            positive = probe in ('snapshot', 'scratch_create')
            if positive:
                row['status'] = 'PASS' if result.get('rc', -1) >= 0 else 'FAIL'
            else:
                explicit = result.get('rc') == -1 and result.get('errno') in (1, 13)
                if probe.startswith('inherited'):
                    explicit = result.get('rc') == -1 and result.get('errno') == 9
                row['status'] = 'PASS' if explicit else 'FAIL'
            confined[probe] = row
        results['confinement'][role] = confined
        results['controls'][role] = {}
        if role != 'ocr':
            for extension in ('txt', 'md', 'html', 'docx', 'xlsx', 'pdf', 'pdf_scan'):
                fixture = next((Path(x['path']) for x in inventory if x['format'] == ('pdf' if extension == 'pdf_scan' else extension)
                                and (extension != 'pdf_scan' or 'scanned' in x['name'])), None)
                if fixture is None:
                    results['controls'][role][extension] = {'status': 'FAIL', 'reason': 'missing fixture'}
                    continue
                # Snapshot bytes are pinned and explicitly granted; no arbitrary broker.
                sb.write_text(profile(binary, fixture, args.pdfium.resolve(), scratch))
                request = json.dumps({'path': str(fixture), 'pdfium': str(args.pdfium.resolve())}).encode()
                row = execute(argv(role), target, 'control-' + extension,
                              struct.pack('<I', len(request)) + request, readiness=True)
                response = row.pop('response', None)
                row['pages'] = len(response.get('pages', [])) if response else 0
                row['status'] = 'PASS' if row['exit_code'] == 0 and row['pages'] else 'FAIL'
                row['full_content_qualified'] = False
                if response:
                    for page in response['pages']:
                        if page.get('ocr_required') and page.get('png'):
                            ocr_scratch = root / 'ocr/scratch'
                            ocr_scratch.mkdir(parents=True, exist_ok=True)
                            ocr_profile = root / 'ocr/profile.sb'
                            ocr_profile.write_text(profile(ocr, snapshot, args.pdfium.resolve(), ocr_scratch))
                            row['separate_ocr'] = execute([ocr, ocr_profile, 'ocr'], target,
                                'ocr-' + extension + '-' + str(page['page']), bytes(page['png']), readiness='stderr')
                results['controls'][role][extension] = row
        (root / 'workers-partial.json').write_text(json.dumps(results, indent=2))
    (root / ('workers-' + args.section + '.json')).write_text(json.dumps(results, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('evidence', 'target', 'inventory', 'pdfium'):
        parser.add_argument('--' + key, type=Path, required=True)
    parser.add_argument('--section', choices=('memory', 'confinement', 'all'), default='all')
    main(parser.parse_args())
