#!/usr/bin/env python3
"""Working external broker/URL controls and conservative native role verdicts."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('descriptor', ROOT / 'scripts/office-s1939-workers.py')
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)
w = d.w


def main(a):
    root = a.evidence.resolve()
    if root == ROOT or ROOT in root.parents: raise ValueError('private evidence required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    app = root / 'C1URLControl.app'
    binary = app / 'Contents/MacOS/C1URLControl'
    binary.parent.mkdir(parents=True,exist_ok=True)
    plist = {'CFBundleExecutable':'C1URLControl','CFBundleIdentifier':'invalid.koskadeux.c1-url-control.a1faf60504d5',
             'CFBundleName':'C1URLControl','CFBundleVersion':'1','CFBundlePackageType':'APPL','LSUIElement':True,
             'CFBundleURLTypes':[{'CFBundleURLName':'C1 private control','CFBundleURLSchemes':['c1-feasibility-probe']} ]}
    (app/'Contents/Info.plist').write_bytes(plistlib.dumps(plist))
    def run(argv,label):
        row=w.common.run(argv,root,label)
        if row['exit_code']: raise RuntimeError(row)
    run(['xcrun','clang','-fobjc-arc','-fblocks','-Wno-deprecated-declarations',ROOT/'scripts/office-url-control.m',
         '-framework','CoreServices','-o',binary],'build-url-control')
    run(['codesign','--force','--sign','-',app],'sign-url-control')
    run([binary,'register',app],'register-url-control')
    worker,pdfium=a.worker.resolve(),a.pdfium.resolve()
    result={'worker_sha256':w.common.digest(worker),'pdfium_sha256':w.common.digest(pdfium),
            'url_control_sha256':w.common.digest(binary),'roles':{},'Windows':'NOT RUN',
            'xpc_verdict':'generic XPC error alone never passes; require same-process kernel lookup denial for identical endpoint plus external XPC reply success',
            'control_endpoint':'com.apple.cfprefsd.daemon; read-only synthetic XPC dictionary',
            'url_control':'LSOpenCFURLRef OSStatus=0 using registered private handler; process observation diagnostic only; URL AppleEvent consumption not qualified; no network URL or browser launch'}
    snapshot=root/'supplied.txt';snapshot.write_text('supplied snapshot\n')
    host=root/'host-sentinel.txt';host.write_text('private sentinel\n')
    for role in ('ingest','render'):
        target=root/role;target.mkdir(exist_ok=True)
        scratch=target/'scratch';scratch.mkdir(exist_ok=True)
        sb=target/'profile.sb';sb.write_text(w.profile(worker,snapshot,pdfium,scratch))
        rows={}
        for name in ('open_url','mach_broker','mach_open_url','xpc_broker'):
            control=w.execute([worker,sb,'control-probe',pdfium,name,snapshot,host,'9'],target,'control-'+name)
            positive=control.get('probe',{}).get('rc')==0 and control['exit_code']==0
            if name=='open_url':
                deadline=time.monotonic()+3
                launched=[]
                while not launched and time.monotonic()<deadline:
                    listing=subprocess.check_output(['rtk','proxy','ps','-axo','pid=,comm='],text=True)
                    for line in listing.splitlines():
                        parts=line.strip().split(None,1)
                        if len(parts)==2 and parts[1]==str(binary):launched.append(int(parts[0]))
                    if not launched:time.sleep(.05)
                control['launched_private_control_pids']=launched
                (target/'url-control-processes.json').write_text(json.dumps({'pids':launched,'executable':str(binary)}))
                # LSOpenCFURLRef's own OSStatus is the positive API control.
                # Do not claim URL delivery from process-table observation.
                for pid in launched:
                    try:os.kill(pid,signal.SIGTERM)
                    except ProcessLookupError:pass
            confined=w.execute([worker,sb,'probe',pdfium,name,snapshot,host,'9'],target,'confined-'+name)
            p=confined.get('probe',{})
            if name.startswith('mach_'):denial=p.get('mach_error')==1100
            elif name=='xpc_broker':denial=p.get('xpc_error') is True and p.get('endpoint_lookup_error')==1100
            else:denial=p.get('explicit_launch_denial') is True
            passed=positive and denial and confined['exit_code']==0 and not confined['deadline_kill']
            rows[name]={'status':'pass' if passed else 'fail','positive_control_succeeded':positive,
                        'explicit_denial':denial,'control':control,'confined':confined}
            if not passed:
                rows[name]['reason']='Final descriptor-only LaunchServices call terminates before returning an OSStatus; a trap/generic error is not an explicit sandbox denial. Control success does not qualify this row.' if positive else 'External control did not succeed; see complete diagnostics.'
        result['roles'][role]=rows
        (root/'brokers.json').write_text(json.dumps(result,indent=2)+'\n')
        print(role,{k:(v['positive_control_succeeded'],v['explicit_denial'],v['status']) for k,v in rows.items()},flush=True)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    for key in ('evidence','worker','pdfium'):p.add_argument('--'+key,type=Path,required=True)
    main(p.parse_args())
