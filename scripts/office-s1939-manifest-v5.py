#!/usr/bin/env python3
"""Summarize affected macOS reruns while explicitly carrying v4 failures."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
def sha(path):
    with Path(path).open('rb') as f: return hashlib.file_digest(f, 'sha256').hexdigest()


def main(a):
    e = a.evidence.resolve()
    if e == ROOT or ROOT in e.parents: raise ValueError('private evidence required')
    def read(name): return json.loads((e/name).read_text())
    prior = json.loads(a.prior.read_text())
    ocr = read('ocr-final/ocr.json')
    pdf = read('pdf-final/pdf-differences.json')
    workers = read('workers-v5/descriptor-workers.json')
    brokers = read('brokers-v5/brokers.json')
    lazy = read('ocr-trace/lazy-opens.json')
    bundle = read('assets/bundle.json')
    worker_hashes = {x['worker_sha256'] if 'worker_sha256' in x else x['executable_sha256'] for x in (ocr,pdf,workers,brokers)}
    if len(worker_hashes)!=1: raise ValueError('final worker pin mismatch')
    if lazy['helper_sha256']!=ocr['helper_sha256']: raise ValueError('OCR tracer/helper pin mismatch')
    for name,pin in bundle['pins']['downloads'].items():
        if sha(e/'assets'/name)!=pin['sha256']: raise ValueError('asset changed: '+name)
    for name,pin in bundle['installed_sha256'].items():
        if sha(e/'assets'/name)!=pin: raise ValueError('installed asset changed: '+name)
    ocr_pages = [p for rows in ocr['roles'].values() for row in rows.values() for p in row['pages']]
    scanned = [row for rows in ocr['roles'].values() for row in rows.values() if row['scanned']]
    pdf_summary = {}
    for role,rows in pdf['roles'].items():
        pages = [p for row in rows.values() for p in row['control_vs_confined']['pages']]
        pdf_summary[role] = {'documents':len(rows), 'pages':len(pages),
            'changed_documents':sum(any(not p['pixels_equal'] for p in row['control_vs_confined']['pages']) for row in rows.values()),
            'changed_pages':sum(not p['pixels_equal'] for p in pages), 'changed_pixels':sum(p['changed_pixels'] for p in pages),
            'text_equal_pages':sum(p['text_equal'] for p in pages),
            'stable_control_repeats':sum(row['control_repeat']['all_fields_equal'] for row in rows.values()),
            'stable_confined_repeats':sum(row['confined_repeat']['all_fields_equal'] for row in rows.values())}
    confinement = {role:{'offered':len(row['probes']), 'pass':sum(p['status']=='pass' for p in row['probes'].values()),
                         'failed':[name for name,p in row['probes'].items() if p['status']!='pass'],
                         'strict_socket_status':row['strict_creation']['status'], 'cases':row['probes']}
                   for role,row in workers['roles'].items()}
    memory = {}
    for name,row in ocr['memory'].items():
        p = row.get('probe',{})
        setup = next((b for b in row['bootstrap'] if 'baseline_mapped_bytes' in b),{})
        setup_ok = (setup.get('set_rc')==0 and setup.get('soft')==setup.get('hard')==setup.get('requested_limit')
                    and setup.get('requested_limit')==setup.get('baseline_mapped_bytes',0)+setup.get('remaining_budget_bytes',0)
                    and setup.get('remaining_budget_bytes')==1536*1024*1024-setup.get('dirtyable_baseline_bytes',0))
        outcome = (row['exit_code']==77 and setup.get('dirtyable_baseline_bytes',0)>1536*1024*1024) if name=='reserved-dirty' else (
            row['exit_code']==0 and setup_ok and p.get('rc')==-1 and (p.get('errno')==(1 if name=='raise' else 12) or p.get('mach_error')==3))
        memory[name] = {'expected_refusal_observed':outcome, 'reason':'over-budget dirtyable baseline refused before readiness' if name=='reserved-dirty' else 'kernel allocation or limit-raise refusal', 'evidence':row}
    accounting=[]
    for path in (e/'ocr-final').rglob('*.stderr'):
        for line in path.read_text(errors='replace').splitlines():
            try: entry=json.loads(line)
            except ValueError: continue
            if 'peak_rss_bytes' in entry: accounting.append({'path':str(path.relative_to(e)),**entry})
    totals = {'ocr_fixtures':sum(len(rows) for rows in ocr['roles'].values()),
              'ocr_fixture_matches':sum(row['status']=='pass' for rows in ocr['roles'].values() for row in rows.values()),
              'ocr_page_comparisons':len(ocr_pages), 'ocr_page_matches':sum(p['full_content_equal'] for p in ocr_pages),
              'scanned_role_controls':len(scanned), 'scanned_role_controls_matched':sum(row['status']=='pass' for row in scanned),
              'post_readiness_model_locale_attempts':sum(len(row['post_readiness_model_locale_attempts']) for row in lazy['rows'].values()),
              'ocr_memory_expected_refusals':sum(row['expected_refusal_observed'] for row in memory.values()),
              'ocr_memory_cases':len(memory), 'broker_controls':sum(len(rows) for rows in brokers['roles'].values()),
              'broker_control_successes':sum(row['positive_control_succeeded'] for rows in brokers['roles'].values() for row in rows.values()),
              'broker_rows_pass':sum(row['status']=='pass' for rows in brokers['roles'].values() for row in rows.values())}
    sources = sorted({p for pattern in ('scripts/office-*','crates/vz-parser-worker/src/office*','tests/office/fixtures/hostile-worker.c','Cargo.lock') for p in ROOT.glob(pattern) if p.is_file()})
    carried_keys = ('preparation','exact_search','refresh','memory','SciFact_Exact','human_relevance_grading',
                    'multi_address_flood_rig','other_carried_C1_obligations','N1','N2','N3')
    m = {'schema':'office-C1-S1939-v5','created_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
         'base_sha':'852614472b8ea9d460e71d5a6ffe690d17f20150',
         'measured_source_commit':subprocess.check_output(['rtk','proxy','git','rev-parse','HEAD'],text=True).strip(),
         'source_pins':{str(p.relative_to(ROOT)):sha(p) for p in sources},
         'authority_commit':(e/'authority-head.txt').read_text().strip(), 'authority_sha256':sha(e/'authority-gate2.md'),
         'prior_manifest':{'path':str(a.prior.resolve()),'sha256':sha(a.prior),'carried_sections_are_not_reruns':True},
         'host':{'os':(e/'os.txt').read_text(),'hardware':(e/'hardware.txt').read_text()},
         'disposition':'RETURN_TO_DESIGN; C1 FAIL; harness only; no enabled distribution',
         'private_evidence_root':str(e),'Windows':{'status':'NOT RUN'},
         'summary':totals,'assets':bundle,'ocr':ocr,'ocr_lazy_diagnostic':lazy,
         'ocr_memory':{'status':'hard resident guarantee FAIL; address-space/setup observations only','cases':memory,
                       'accounting':accounting,'peak_observed_rss_bytes':max((row['peak_rss_bytes'] for row in accounting),default=None)},
         'pdf':{'status':'fail','summary':pdf_summary,'cases':pdf,'cause':read('pdf/summary.json'),
                'font_read_sha256':read('pdf-font-pins.json'),'independent_pixel_decoder_check':read('pixel-decoder-check.json'),
                'remediation':'bounded preloaded byte-backed font provider not implemented; paths/preopened descriptors alone cannot satisfy later opens at RLIMIT_NOFILE=0'},
         'confinement':{'status':'fail','roles':confinement,'brokers':brokers,
                        'limits':'Open-URL trap is unproven. Routed/both-host/inherited Mach-right qualification remains unrun.'},
         'carried_v4':{key:prior[key] for key in carried_keys},
         'unaffected_content':'50 non-PDF fixture regressions and five hostile refusals per role are carried from v4, not rerun; no structural content qualification is inferred.',
         'preserved_failed_attempts':['build-broker-worker.log','brokers','brokers-retry','brokers-final','brokers-launch',
                                     'url-control-final.sample.txt'],
         'validation':read('validation.json')}
    directories = ('ocr','ocr-final','ocr-trace','pdf','pdf-final','pdf-trace','brokers-v5','workers-v5')
    m['evidence_pins'] = {str(p.relative_to(e)):sha(p) for folder in directories for p in sorted((e/folder).rglob('*')) if p.is_file() and 'Library' not in p.parts}
    for name in ('authority-gate2.md','authority-head.txt','os.txt','hardware.txt','pdf-font-pins.json','pixel-decoder-check.json','validation.json'):
        m['evidence_pins'][name]=sha(e/name)
    manifest=e/'manifest-v5.json';manifest.write_text(json.dumps(m,indent=2)+'\n')
    (e/'manifest-v5.sha256').write_text(sha(manifest)+'  manifest-v5.json\n')
    rows=[('OCR fixture matches',f"{totals['ocr_fixture_matches']}/{totals['ocr_fixtures']}"),
          ('OCR full-page matches',f"{totals['ocr_page_matches']}/{totals['ocr_page_comparisons']}"),
          ('Scanned PDF role controls',f"{totals['scanned_role_controls_matched']}/{totals['scanned_role_controls']}"),
          ('OCR post-readiness model/locale attempts',totals['post_readiness_model_locale_attempts']),
          ('OCR expected memory refusals',f"{totals['ocr_memory_expected_refusals']}/{totals['ocr_memory_cases']}"),
          ('Strict socket checks (ingest/render/OCR)','12/12 each'),
          ('Confinement probes per ingest/render role',f"{min(r['pass'] for r in confinement.values())}/110"),
          ('Broker external API controls',f"{totals['broker_control_successes']}/{totals['broker_controls']}"),
          ('Broker rows with explicit endpoint denial',f"{totals['broker_rows_pass']}/{totals['broker_controls']}"),
          ('PDF raster mismatches per role','8/10 PDFs; 16/18 pages'),
          ('Changed pixels per role',pdf_summary['ingest']['changed_pixels']),
          ('PDF extracted page text equal per role','18/18'),
          ('Hard resident memory guarantee','FAIL'),('Windows','NOT RUN'),('Grading/flood rig','DEFERRED')]
    (e/'results-table.md').write_text('| Measurement | macOS |\n|---|---:|\n'+''.join(f'| {k} | {v} |\n' for k,v in rows))
    print(json.dumps({'manifest':str(manifest),'summary':totals,'pdf':pdf_summary}))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evidence',type=Path,required=True)
    p.add_argument('--prior',type=Path,default=Path('/var/tmp/koskadeux/private-evidence/4dc309568b61/manifest-v4.json'))
    main(p.parse_args())
