#!/usr/bin/env python3
"""Installed-app A9: continuous capture, proven attribution, zero loss, no fallback."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import ipaddress
import struct
from datetime import datetime, timezone
import socket
import subprocess
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
PRE_MESSAGE = b'M3 pre-capture control send'
NEGATIVE_MESSAGE = b'M3 asynchronous negative control'


def etl_losses(header):
    # Amendment 2 pins the runner's TRACE_LOGFILE_HEADER layout. Never guess.
    base = 0x68
    logger = base + 0x118
    if len(header) < logger + 6 or header[logger:logger + 6] != 'a9\0'.encode('utf-16le'):
        raise ValueError('STOP: truncated ETL header or unexpected a9 logger-name layout')
    return struct.unpack_from('<I', header, base + 48)[0], struct.unpack_from('<I', header, base + 276)[0]


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def instant(value):
    return datetime.fromisoformat(value.replace('Z', '+00:00')).timestamp()


def ip(value):
    address = ipaddress.ip_address(value)
    return str(address.ipv4_mapped if isinstance(address, ipaddress.IPv6Address) and address.ipv4_mapped else address)


def number(value):
    return int(value, 16) if str(value).lower().startswith('0x') else int(value)


def run(argv, evidence, timeout=30, accepted=(0,)):
    result = subprocess.run(list(map(str, argv)), capture_output=True, text=True, timeout=timeout)
    evidence.write(subprocess.list2cmdline(list(map(str, argv))) + '\n' + result.stdout + result.stderr + '\n'); evidence.flush()
    if result.returncode not in accepted:
        raise RuntimeError(f'STOP: approved A9 command failed ({result.returncode}): {argv[0]}; see a9-commands.log')
    return result.stdout


def wait(path, timeout=150):
    deadline = time.monotonic() + timeout
    while not path.exists():
        if time.monotonic() > deadline:
            raise TimeoutError(f'A9 marker missing: {path.name}')
        time.sleep(.05)


def enumerate_sockets(pids, addresses, evidence):
    resolved = {ip(address) for values in addresses.values() for address in values}
    if platform.system() == 'Windows':
        script = '$ErrorActionPreference="Stop"; $owners=@(' + ','.join(map(str,pids)) + '); '
        script += '$tcp=@(Get-NetTCPConnection | Where-Object {$_.OwningProcess -in $owners}); '
        script += '$udp=@(Get-NetUDPEndpoint | Where-Object {$_.OwningProcess -in $owners}); '
        script += '@{tcp=@($tcp | Select-Object OwningProcess,State,LocalAddress,LocalPort,RemoteAddress,RemotePort,@{n="CreatedUtc";e={if ($_.CreationTime) {$_.CreationTime.ToUniversalTime().ToString("o")} else {$null}}});udp=@($udp | Select-Object OwningProcess,LocalPort)} | ConvertTo-Json -Depth 5 -Compress'
        data = json.loads(run(['powershell.exe','-NoProfile','-Command',script], evidence))
        assert not data['udp'], 'UDP endpoint in app or bridge'
        assert not any(t['State'] in [2,'Listen'] for t in data['tcp']), 'TCP listener in app or bridge'
        assert not any(t['State'] in [5,'Established'] and ip(t['RemoteAddress']) in resolved for t in data['tcp']), 'Established ai.market connection in app or bridge'
        return data['tcp']
    for pid in pids:
        output = run(['lsof','-nP','-i','-a','-p',pid], evidence, accepted=(0,1))
        for line in output.splitlines()[1:]:
            assert ' UDP ' not in line, f'UDP endpoint owned by pid {pid}'
            assert '(LISTEN)' not in line, f'TCP listener owned by pid {pid}'
            if '(ESTABLISHED)' in line:
                remote = line.split('->', 1)[1].split()[0].rsplit(':', 1)[0].strip('[]')
                assert ip(remote) not in resolved, f'Established ai.market connection owned by pid {pid}'
    return []


def windows_events(path):
    # tracerpt XML retains named provider fields; never use the header execution PID.
    root = ET.parse(path).getroot()
    events=[]
    for event in root.iter():
        if event.tag.split('}')[-1] != 'Event': continue
        fields={}
        for field in event.iter():
            tag=field.tag.split('}')[-1]
            if tag == 'Data' and 'Name' in field.attrib: fields[field.attrib['Name']]=field.text or ''
            elif tag == 'TimeCreated': fields['TimeCreated'] = field.attrib['SystemTime']
            elif not list(field) and tag not in ['Data']: fields.setdefault(tag,field.text or '')
        if 'EventID' in fields: events.append((int(fields['EventID']),fields))
    assert events, 'STOP: tracerpt emitted no decodable events'
    return events


def check_windows(events, app_pid, bridge_pids, port, window, evidence):
    # The in-window create control supplies the app's kernel EPROCESS identity.
    # Use it in a second pass so pre-existing endpoint sends can precede it.
    # https://learn.microsoft.com/windows/win32/winsock/winsock-tracing-event-details
    process_owners = {}
    for event_id, fields in events:
        if event_id == 1 and all(k in fields for k in ['Process', 'UserModePid']):
            process_owners[number(fields['Process'])] = number(fields['UserModePid'])
    connections = []; sends = []
    for event_id, fields in events:
        owner = process_owners.get(number(fields['Process'])) if 'Process' in fields else None
        if owner not in [app_pid, *bridge_pids]:
            continue
        in_window = window[0] <= instant(fields['TimeCreated']) <= window[1]
        if event_id in [4, 5] or event_id in [18, 21, 22, 24, 25, 28]:
            evidence.write(json.dumps({'pid': owner, 'event': event_id, 'in_window': in_window, 'fields': fields}) + '\n')
        if not in_window:
            continue
        if event_id in [4, 5]: connections.append((owner, fields))
        if event_id in [18, 21, 22, 24, 25, 28]: sends.append((owner, event_id, fields))
    evidence.flush()
    app_connections = [f for owner, f in connections if owner == app_pid]
    assert len(app_connections) == 1, 'Negative control missing, or extra app connection in window'
    negative = app_connections[0]
    address = negative['Address']
    assert address == '127.0.0.1' or number(address) in [0x0100007f, 0x7f000001], 'Negative control address not loopback'
    assert number(negative['Port']) in [port, socket.htons(port)], 'Negative control port not attributed to app'
    # The listener independently receives the exact pre-control payload on the
    # accepted pre-capture tuple. Demand one distinct app-owned send endpoint of
    # precisely that size, with no in-window connect, and no other posted data.
    negative_endpoint = number(negative['Endpoint'])
    pre_endpoints = {number(f['Endpoint']) for owner, event_id, f in sends
                     if owner == app_pid and event_id == 18
                     and number(f['Endpoint']) != negative_endpoint
                     and number(f['BufferLength']) == len(PRE_MESSAGE)}
    assert len(pre_endpoints) == 1, 'STOP: pre-capture send not attributed to app PID'
    pre_endpoint = pre_endpoints.pop()
    assert any(owner == app_pid and event_id == 18 and number(f['Endpoint']) == negative_endpoint
               and number(f['BufferLength']) == len(NEGATIVE_MESSAGE) for owner, event_id, f in sends), 'Asynchronous negative send missing'
    for owner, event_id, fields in sends:
        expected = {negative_endpoint: len(NEGATIVE_MESSAGE), pre_endpoint: len(PRE_MESSAGE)}
        assert owner == app_pid and number(fields['Endpoint']) in expected, 'App/bridge sent network data during tools'
        assert number(fields['BufferLength']) == expected[number(fields['Endpoint'])], 'Extra data on control connection'
    assert not any(owner in bridge_pids for owner, _ in connections), 'Bridge opened a network connection'
    evidence.write(f'Pre-capture control attributed to app PID {app_pid}, endpoint {pre_endpoint:#x}\n')


def check_macos(packets, app_pid, bridge_pids, port, pre_port, negative_port, window, evidence):
    app = []; bridge = []
    for line in packets.splitlines():
        if not window[0] <= float(line.split()[0]) <= window[1]: continue
        if re.search(r'vectoraiz[:.\s]' + str(app_pid) + r'\b', line, re.I): app.append(line)
        if any(re.search(r'vectoraiz-mcp[:.\s]' + str(pid) + r'\b', line, re.I) for pid in bridge_pids): bridge.append(line)
    evidence.write('In-window app packets:\n' + '\n'.join(app) + '\nIn-window bridge packets:\n' + '\n'.join(bridge) + '\n'); evidence.flush()
    def outgoing(line, source_port):
        return re.search(r'127\.0\.0\.1\.' + str(source_port) + r' > 127\.0\.0\.1\.' + str(port) + r':', line)
    assert any(outgoing(line, pre_port) and f'length {len(PRE_MESSAGE)}' in line for line in app), 'STOP: pre-capture send not attributed to app PID'
    assert any(outgoing(line, negative_port) and 'Flags [S]' in line for line in app), 'Negative connection missing'
    assert any(outgoing(line, negative_port) and f'length {len(NEGATIVE_MESSAGE)}' in line for line in app), 'Asynchronous negative send missing'
    for line in app:
        # pktap may attribute received packets to the app as well. Only the two
        # exact loopback tuples are controls, in either direction.
        allowed = any(outgoing(line, source_port) or re.search(
            r'127\.0\.0\.1\.' + str(port) + r' > 127\.0\.0\.1\.' + str(source_port) + r':', line)
            for source_port in [pre_port, negative_port])
        assert allowed or 'in,' in line or ', in)' in line, 'App made a network connection or sent during tools'
    assert not bridge, 'Bridge made a network connection'


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--bridge',type=Path,required=True);parser.add_argument('--control',type=Path,required=True)
    args=parser.parse_args();control=args.control
    spec = importlib.util.spec_from_file_location('mcp_protocol', ROOT / 'scripts/native-mcp-protocol.py')
    protocol = importlib.util.module_from_spec(spec); spec.loader.exec_module(protocol)
    wait(control/'ready.json',1000);ready=json.loads((control/'ready.json').read_text())
    windows=platform.system()=='Windows'
    # DNS is outside the capture/tool window and never runs in either product process.
    addresses={host:sorted({r[4][0] for r in socket.getaddrinfo(host,443)}) for host in ['ai.market','api.ai.market']}
    Path('a9-resolved.json').write_text(json.dumps(addresses,indent=2))
    listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen(2);listener.settimeout(30)
    port=listener.getsockname()[1]
    (control/'target.tmp').write_text(f'127.0.0.1:{port}')
    (control/'target.tmp').replace(control/'target')
    # Accept first, before either capture command. This socket must predate ETW.
    pre_connection, pre_address = listener.accept()
    wait(control/'pre.connected',30)
    assert (control/'pre.connected').read_text().strip() == f'{pre_address[0]}:{pre_address[1]}'
    pre_connection.settimeout(5)
    negative_address = None
    window = [None, None]
    snapshots = []
    capture_path = Path('a9.pcapng').resolve()
    pid_path = Path('a9-tcpdump.pid').resolve()
    assert capture_path.parent == Path.cwd().resolve() and pid_path.parent == Path.cwd().resolve()
    tcpdump_pid = None
    bridges=[];bridge_pids=[];capture=None;started=False
    with Path('a9-commands.log').open('w',encoding='utf-8') as evidence, Path('a9-tcpdump.log').open('w',encoding='utf-8') as capture_log:
        evidence.write(f'Pre-capture accepted tuple {pre_address} -> 127.0.0.1:{port} at {utc_now()}\n'); evidence.flush()
        try:
            snapshots += enumerate_sockets([ready['pid']], addresses, evidence)
            if windows:
                run(['logman','create','trace','a9','-o',Path('a9.etl').resolve(),'-ets','-p','Microsoft-Windows-Winsock-AFD','0xFFFFFFFFFFFFFFFF','0xff'],evidence)
                started=True
            else:
                channel = Path.home() / 'Library/Application Support/vectorAIz/mcp.sock'
                assert channel.parent.stat().st_mode & 0o777 == 0o700, 'Installed channel directory is not 0700'
                assert channel.stat().st_mode & 0o777 == 0o600, 'Installed socket is not 0600'
                nobody = run(['sudo', '-n', '-u', 'nobody', '/usr/bin/id', '-u'], evidence)
                assert int(nobody.strip()) != os.getuid(), 'Directory probe did not change uid'
                run(['sudo', '-n', '-u', 'nobody', '/bin/test', '-x', channel.parent], evidence, accepted=(1,))
                evidence.write('Channel security PASS: other uid cannot traverse the installed 0700 directory\n'); evidence.flush()
                pid_path.unlink(missing_ok=True)
                command=['sudo','-n','sh','-c','echo $$ > "$1" || exit 1; exec tcpdump -i pktap,all -k NP -w "$2"','sh',str(pid_path),str(capture_path)]
                evidence.write(subprocess.list2cmdline(command) + '\n'); evidence.flush()
                capture=subprocess.Popen(command,stdout=capture_log,stderr=subprocess.STDOUT)
                wait(pid_path,5)
                tcpdump_pid=int(pid_path.read_text().strip())
                assert tcpdump_pid > 1, 'STOP: invalid tcpdump PID file'
                evidence.write(f'Read tcpdump PID {tcpdump_pid} before capture.started\n'); evidence.flush()
                time.sleep(.5)
                assert capture.poll() is None, 'STOP: approved tcpdump command failed; see a9-tcpdump.log'
            window[0] = time.time()
            Path('a9-window.json').write_text(json.dumps({'start': window[0], 'start_utc': utc_now()}))
            (control/'capture.started').touch()
            negative_connection, negative_address = listener.accept()
            with negative_connection:
                negative_connection.settimeout(5)
                assert negative_connection.recv(1024) == NEGATIVE_MESSAGE, 'Negative control did not reach test address'
            wait(control/'negative.done',30)
            for era in ['modern','legacy']:
                bridge=protocol.Client(args.bridge,ready['client'],era,evidence);bridges.append(bridge);bridge_pids.append(bridge.child.pid)
                bridge.startup();bridge.tools_run(True)
            snapshots += enumerate_sockets([ready['pid'],*bridge_pids],addresses,evidence)
            (control/'granted.done').touch();wait(control/'removed',30)
            for bridge in bridges: bridge.tools_run(False)  # Same open stdio process after revocation.
            snapshots += enumerate_sockets([ready['pid'],*bridge_pids],addresses,evidence)
        finally:
            window[1] = time.time()
            Path('a9-window.json').write_text(json.dumps({'start': window[0], 'end': window[1], 'start_utc': datetime.fromtimestamp(window[0], timezone.utc).isoformat() if window[0] else None, 'end_utc': utc_now()}, indent=2))
            Path('a9-sockets.json').write_text(json.dumps(snapshots, indent=2))
            (control/'capture.finished').touch()
            for bridge in bridges: bridge.close()
            listener.close()
            if windows and started:
                run(['logman','stop','a9','-ets'],evidence)
                losses = etl_losses(Path('a9.etl').read_bytes())
                evidence.write(f'ETL final EventsLost={losses[0]} BuffersLost={losses[1]}\n'); evidence.flush()
                assert losses == (0, 0), 'STOP: ETL EventsLost or BuffersLost is nonzero'
                run(['tracerpt',Path('a9.etl').resolve(),'-of','CSV','-o',Path('a9.csv').resolve(),'-y'],evidence)
                run(['tracerpt',Path('a9.etl').resolve(),'-of','XML','-o',Path('a9.xml').resolve(),'-y'],evidence)
            elif capture is not None:
                processes=subprocess.run(['ps','-axo','pid,ppid,user,command'],capture_output=True,text=True,check=True).stdout
                evidence.write('ps -axo pid,ppid,user,command (sudo/tcpdump):\n' + '\n'.join(line for line in processes.splitlines() if re.search(r'\b(sudo|tcpdump)\b',line)) + '\n')
                sudo_version=subprocess.run(['sudo','-V'],capture_output=True,text=True,check=True).stdout
                evidence.write('sudo -V | head -1: ' + sudo_version.splitlines()[0] + '\n'); evidence.flush()
                assert tcpdump_pid is not None, 'STOP: tcpdump PID file was not readable'
                run(['sudo','-n','kill','-INT',tcpdump_pid],evidence)
                assert capture.wait(timeout=15)==0, 'tcpdump failed'
                capture_log.flush()
                losses=re.findall(r'(\d+) packets dropped by kernel',Path('a9-tcpdump.log').read_text())
                assert losses and all(int(n)==0 for n in losses), 'STOP: packet loss or absent tcpdump loss statistic'
        if windows:
            for snapshot in snapshots:
                if snapshot['State'] in [5, 'Established']:
                    snapshot['created_in_window'] = window[0] <= instant(snapshot['CreatedUtc']) <= window[1]
            Path('a9-sockets.json').write_text(json.dumps(snapshots, indent=2))
            check_windows(windows_events('a9.xml'),ready['pid'],bridge_pids,port,window,evidence)
            assert not any(s.get('created_in_window') and not (s['OwningProcess'] == ready['pid'] and s['RemoteAddress'] == '127.0.0.1' and s['RemotePort'] == port and s['LocalPort'] in [pre_address[1], negative_address[1]]) for s in snapshots), 'App/bridge connection created inside tool-call window'
        else:
            packets=run(['tcpdump','-r',capture_path,'-k','NPD','-nn','-tt'],evidence)
            Path('a9-packets.txt').write_text(packets)
            check_macos(packets,ready['pid'],bridge_pids,port,pre_address[1],negative_address[1],window,evidence)
        with pre_connection:
            assert pre_connection.recv(1024) == PRE_MESSAGE, 'STOP: pre-capture write did not reach accepted control socket'
        (control/'done').touch()
        evidence.write('A9 PASS: zero loss, both controls attributed, granted + removed, both eras, no non-control connection created or sent in window\n')
    print('A9 continuous capture PASS',flush=True)


if __name__=='__main__':
    try:
        main()
    except Exception as error:
        Path('a9-result.json').write_text(json.dumps({'status': 'STOPPED', 'reason': str(error)}, indent=2))
        raise
    Path('a9-result.json').write_text(json.dumps({'status': 'PASS'}))
