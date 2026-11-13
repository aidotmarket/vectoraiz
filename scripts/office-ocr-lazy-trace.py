#!/usr/bin/env python3
"""Segregated diagnostic: detect post-readiness tessdata/locale file attempts."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]
def module(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m
d = module('descriptor', 'scripts/office-s1939-workers.py')
png = module('png', 'scripts/office-png.py')


def main(a):
    root = a.evidence.resolve()
    if root == ROOT or ROOT in root.parents: raise ValueError('private evidence required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    helper = a.ocr.resolve() / 'vz-tesseract-ocr'
    profile = a.ocr.resolve() / 'ocr.sb'
    result = {'helper_sha256': d.w.common.digest(helper), 'tracer_sha256': d.w.common.digest(a.tracer),
              'diagnostic_only': True, 'rows': {}}
    for name in ('ORBIT-00-00', 'ORBIT-00-04'):
        response = json.loads((a.ocr / 'ingest' / (name+'-raster.stdout')).read_bytes()[4:])
        page = response['pages'][0]
        decoded = png.decode(bytes(page['png']))
        request = struct.pack('<II', *decoded[:2]) + png.rgb(decoded)
        for mode in ('control', 'ocr'):
            label = name+'-'+mode
            env = {'PATH': os.environ['PATH'], 'HOME': str(root), 'DYLD_INSERT_LIBRARIES': str(a.tracer.resolve())}
            with (root/(label+'.stderr')).open('wb') as error:
                p = subprocess.Popen(['rtk','proxy',str(helper),str(profile),mode,str(a.assets.resolve())],
                                     stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=error,env=env)
                prefix = p.stdout.read(4)
                if len(prefix)!=4: raise ValueError('no readiness')
                size = struct.unpack('<I',prefix)[0]
                if size>4096: raise ValueError('readiness bound')
                ready = json.loads(p.stdout.read(size))
                if not ready.get('ready'): raise ValueError('not ready')
                boundary = error.tell()
                output,_ = p.communicate(request, timeout=30)
            (root/(label+'.stdout')).write_bytes(output)
            log = (root/(label+'.stderr')).read_bytes()
            after = log[boundary:].decode(errors='replace').splitlines()
            attempts = [line for line in after if line.startswith('FILE_TRACE') and any(
                token in line.lower() for token in ('traineddata','tessdata','locale','lc_ctype','lc_numeric'))]
            expected = (a.ocr/'ingest'/(name+'-1-confined.stdout')).read_bytes()
            result['rows'][label] = {'exit_code': p.returncode, 'readiness_stderr_offset': boundary,
                'post_readiness_model_locale_attempts': attempts, 'full_output_equal_to_final_uninjected_run': output==expected,
                'pre_readiness_model_opens': [line for line in log[:boundary].decode(errors='replace').splitlines() if 'traineddata' in line]}
    (root/'lazy-opens.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:(v['exit_code'],len(v['post_readiness_model_locale_attempts']),v['full_output_equal_to_final_uninjected_run']) for k,v in result['rows'].items()}))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    for key in ('evidence','ocr','assets','tracer'):p.add_argument('--'+key,type=Path,required=True)
    main(p.parse_args())
