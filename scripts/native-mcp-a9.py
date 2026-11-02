#!/usr/bin/env python3
"""Installed-app A9: continuous capture, proven attribution, zero loss, no fallback."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import signal
import socket
import subprocess
import threading
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('mcp_protocol', ROOT / 'scripts/native-mcp-protocol.py')
protocol = importlib.util.module_from_spec(spec); spec.loader.exec_module(protocol)


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


def enumerate_sockets(pids, evidence):
    if platform.system() == 'Windows':
        script = '$ErrorActionPreference="Stop"; $owners=@(' + ','.join(map(str,pids)) + '); '
        script += '$tcp=@(Get-NetTCPConnection | Where-Object {$_.OwningProcess -in $owners}); '
        script += '$udp=@(Get-NetUDPEndpoint | Where-Object {$_.OwningProcess -in $owners}); '
        script += '@{tcp=@($tcp | Select-Object OwningProcess,State,RemoteAddress,RemotePort);udp=@($udp | Select-Object OwningProcess,LocalPort)} | ConvertTo-Json -Depth 5 -Compress'
        data = json.loads(run(['powershell.exe','-NoProfile','-Command',script], evidence))
        assert not data['udp'], 'UDP endpoint in app or bridge'
        assert not any(t['State'] in [2,'Listen'] for t in data['tcp']), 'TCP listener in app or bridge'
        assert not data['tcp'], 'Internet socket in app or bridge during tool calls'
    else:
        for pid in pids:
            output = run(['lsof','-nP','-i','-a','-p',pid], evidence, accepted=(0,1))
            assert not output.strip(), f'Internet socket owned by app/bridge pid {pid}'


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
            elif not list(field) and tag not in ['Data']: fields.setdefault(tag,field.text or '')
        if 'EventID' in fields: events.append((int(fields['EventID']),fields))
    assert events, 'STOP: tracerpt emitted no decodable events'
    return events


def check_windows(events, app_pid, bridge_pids, port):
    process_owners={}; endpoint_owners={}; connections=[]; sends=[]
    def number(value): return int(value,16) if value.lower().startswith('0x') else int(value)
    for event_id,fields in events:
        if event_id==1 and all(k in fields for k in ['Process','Endpoint','UserModePid']):
            owner=number(fields['UserModePid']); process_owners[fields['Process']]=owner; endpoint_owners[fields['Endpoint']]=owner
        # AFD's application Process / Endpoint fields correlate to socket creation.
        owner=process_owners.get(fields.get('Process'),endpoint_owners.get(fields.get('Endpoint')))
        if event_id in [4,5] and owner in [app_pid,*bridge_pids]: connections.append((owner,fields))
        if event_id==18 and owner in [app_pid,*bridge_pids]: sends.append((owner,fields))
    app_connections=[fields for owner,fields in connections if owner==app_pid]
    assert len(app_connections)==1, 'Negative control missing, or extra app connection'
    negative=app_connections[0]
    assert number(negative['Port']) in [port, socket.htons(port)], 'Negative control port not attributed to app'
    endpoint=negative['Endpoint']
    assert any(owner==app_pid and f.get('Endpoint')==endpoint for owner,f in sends), 'Asynchronous negative send missing'
    assert all(owner==app_pid and f.get('Endpoint')==endpoint for owner,f in sends), 'App/bridge sent network data during tools'
    assert not any(owner in bridge_pids for owner,_ in connections), 'Bridge opened a network connection'


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--bridge',type=Path,required=True);parser.add_argument('--control',type=Path,required=True)
    args=parser.parse_args();control=args.control
    wait(control/'ready.json',1000);ready=json.loads((control/'ready.json').read_text())
    windows=platform.system()=='Windows'
    # DNS is outside the capture/tool window and never runs in either product process.
    addresses={host:sorted({r[4][0] for r in socket.getaddrinfo(host,443)}) for host in ['ai.market','api.ai.market']}
    Path('a9-resolved.json').write_text(json.dumps(addresses,indent=2))
    listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen(1)
    port=listener.getsockname()[1]
    def negative():
        listener.settimeout(30)
        conn,_=listener.accept()
        with conn: assert conn.recv(1024)==b'M3 asynchronous negative control'
    negative_thread=threading.Thread(target=negative,daemon=True);negative_thread.start()
    (control/'target').write_text(f'127.0.0.1:{port}')
    bridges=[];bridge_pids=[];capture=None;started=False
    with Path('a9-commands.log').open('w',encoding='utf-8') as evidence, Path('a9-tcpdump.log').open('w',encoding='utf-8') as capture_log:
        try:
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
                capture=subprocess.Popen(['sudo','-n','tcpdump','-i','pktap,all','-k','NP','-w',str(Path('a9.pcapng').resolve())],stdout=capture_log,stderr=subprocess.STDOUT)
                time.sleep(.5)
                assert capture.poll() is None, 'STOP: approved tcpdump command failed; see a9-tcpdump.log'
            (control/'capture.started').touch();wait(control/'negative.done',30);negative_thread.join(timeout=5)
            assert not negative_thread.is_alive(), 'Negative control did not reach test address'
            for era in ['modern','legacy']:
                bridge=protocol.Client(args.bridge,ready['client'],era,evidence);bridges.append(bridge);bridge_pids.append(bridge.child.pid)
                bridge.startup();bridge.tools_run(True)
            enumerate_sockets([ready['pid'],*bridge_pids],evidence)
            (control/'granted.done').touch();wait(control/'removed',30)
            for bridge in bridges: bridge.tools_run(False)  # Same open stdio process after revocation.
            enumerate_sockets([ready['pid'],*bridge_pids],evidence)
        finally:
            for bridge in bridges: bridge.close()
            listener.close()
            if windows and started:
                query=run(['logman','query','a9','-ets'],evidence)
                try:
                    losses=re.findall(r'Events Lost\s*:\s*(\d+)',query,re.I)
                    assert losses and all(int(n)==0 for n in losses), 'STOP: capture loss or absent Events Lost statistic'
                finally: run(['logman','stop','a9','-ets'],evidence)
                run(['tracerpt',Path('a9.etl').resolve(),'-of','CSV','-o',Path('a9.csv').resolve(),'-y'],evidence)
                run(['tracerpt',Path('a9.etl').resolve(),'-of','XML','-o',Path('a9.xml').resolve(),'-y'],evidence)
            elif capture is not None:
                run(['sudo','-n','kill','-INT',capture.pid],evidence)
                assert capture.wait(timeout=15)==0, 'tcpdump failed'
                capture_log.flush()
                losses=re.findall(r'(\d+) packets dropped by kernel',Path('a9-tcpdump.log').read_text())
                assert losses and all(int(n)==0 for n in losses), 'STOP: packet loss or absent tcpdump loss statistic'
        if windows:
            check_windows(windows_events('a9.xml'),ready['pid'],bridge_pids,port)
        else:
            packets=run(['tcpdump','-r',Path('a9.pcapng').resolve(),'-k','NP','-nn'],evidence)
            Path('a9-packets.txt').write_text(packets)
            app=[];bridge=[]
            for line in packets.splitlines():
                if re.search(r'vectoraiz[:.\s]'+str(ready['pid'])+r'\b',line,re.I): app.append(line)
                if any(re.search(r'vectoraiz-mcp[:.\s]'+str(pid)+r'\b',line,re.I) for pid in bridge_pids): bridge.append(line)
            assert app, 'STOP: negative control not captured with the app name and PID'
            assert any(re.search(r'length [1-9]\d*',l) for l in app), 'Asynchronous negative send missing'
            assert all(re.search(r'127\.0\.0\.1\.'+str(port)+r'\b',l) for l in app), 'App made a network connection during tools'
            assert not bridge, 'Bridge made a network connection'
        (control/'done').touch()
        evidence.write('A9 PASS: zero loss, negative control attributed, granted + removed, both eras, no network sockets\n')
    print('A9 continuous capture PASS',flush=True)


if __name__=='__main__': main()
