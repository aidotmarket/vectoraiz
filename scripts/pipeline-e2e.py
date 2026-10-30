#!/usr/bin/env python3
"""CPU-only acceptance: all fixture formats, real citations, pack hashes and query."""
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / ('target/debug/vectoraiz-cli.exe' if os.name == 'nt' else 'target/debug/vectoraiz-cli')
with tempfile.TemporaryDirectory(prefix='vz-pipeline-') as temp:
    state = Path(temp) / 'state'
    pack = Path(temp) / 'pack'
    def cli(*args):
        result = subprocess.run([str(CLI), '--cpu', '--state', str(state), *args], cwd=ROOT, text=True, encoding="utf-8", capture_output=True, timeout=600)
        print(result.stderr, file=sys.stderr)
        if result.returncode:
            print(result.stdout)
            raise RuntimeError(f'CLI {args[0]} failed: {result.returncode}')
        return json.loads(result.stdout)
    indexed = cli('index', 'spike/fixtures/documents')
    assert indexed['completed'] == 20 and indexed['failed'] == 0, indexed
    unchanged = cli('index', 'spike/fixtures/documents')
    assert unchanged['skipped'] == 20, unchanged
    query = 'What is the orbital archive retention policy?'
    hits = cli('search', query)
    assert hits and 'thirty days' in hits[0]['chunk']['text'], hits[:1]
    answer = cli('ask', query)
    body, separator, footer = answer['text'].rpartition('\n\nRetrieved sources: ')
    assert separator and len(body.split()) > 3, answer
    assert 'thirty days' in body.lower(), body
    numbers = [int(n) for n in re.findall(r'\[(\d+)\]', footer)]
    assert numbers and numbers == [c['number'] for c in answer['citations']], answer
    manifest = cli('pack', str(pack), '--query', query)
    docs = {doc['doc_id']: doc for doc in pq.read_table(pack / 'documents.parquet').to_pylist()}
    for hit in hits:
        chunk = hit['chunk']
        assert chunk['doc_id'] in docs
        assert 1 <= chunk['page'] <= docs[chunk['doc_id']]['pages']
    chunks = {c['chunk_id']: c for c in pq.read_table(pack / 'chunks.parquet').to_pylist()}
    for citation in answer['citations']:
        source = chunks[citation['chunk_id']]
        assert (source['doc_id'], source['page']) == (citation['doc_id'], citation['page'])
        assert citation['doc_id'] in docs
        assert 1 <= citation['page'] <= docs[citation['doc_id']]['pages']
    for name, digest in manifest['files'].items():
        assert hashlib.sha256((pack / name).read_bytes()).hexdigest() == digest
        if name.endswith(('.md', '.py', '.json')):
            first = (pack / name).read_text().splitlines()[0]
            assert ',' not in first and '\t' not in first, name
    subprocess.run([sys.executable, str(pack / 'examples/query.py'), str(pack), str(CLI)], check=True)
    if os.environ.get('VZ_MINIO_ENDPOINT'):
        subprocess.run([sys.executable, str(ROOT / 'scripts/upload-e2e.py'), str(pack)], check=True)
    # Same dimensions and model bytes, different model identity: dimensions alone
    # must never authorize queries or relabel an export.
    pins = json.loads((ROOT / 'spike/fixtures/models.json').read_text())
    pins['embedding']['name'] += '-different-model'
    alternate = Path(temp) / 'alternate-models.json'
    alternate.write_text(json.dumps(pins))
    for operation in [('search', query), ('ask', query), ('pack', str(Path(temp) / 'wrong-pack'))]:
        refused = subprocess.run([str(CLI), '--cpu', '--state', str(state),
                                  '--models', str(alternate), *operation],
                                 cwd=ROOT, text=True, encoding='utf-8', capture_output=True, timeout=600)
        assert refused.returncode != 0 and 're-index required' in refused.stderr, refused.stderr
    assert manifest['embedding_recipe'] == json.loads((state / 'recipe.json').read_text())

    # Migrate a legacy library through the same recipe-directory/pointer path
    # used by desktop Re-index; old storage stays byte-for-byte intact.
    import shutil
    library = Path(temp) / 'library'
    legacy = library / 'phase0-index'
    shutil.copytree(state, legacy)
    old_recipe = json.loads((legacy / 'recipe.json').read_text())
    old_recipe['model'] += '-old'
    old_bytes = json.dumps(old_recipe).encode()
    (legacy / 'recipe.json').write_bytes(old_bytes)
    explicit_state = state
    state = library
    rebuilt = cli('index', 'spike/fixtures/documents')
    assert rebuilt['completed'] == 20 and rebuilt['failed'] == 0, rebuilt
    pointer_bytes = (library / 'active-index.json').read_bytes()
    pointer = json.loads(pointer_bytes)
    active = library / pointer['dir']
    recipe = json.loads((active / 'recipe.json').read_text())
    recipe_id = hashlib.sha256(json.dumps(recipe, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()[:16]
    assert pointer == {'dir': f'indexes/{recipe_id}', 'recipe_id': recipe_id}, pointer
    assert cli('search', query), 'search after migration'
    assert cli('ask', query)['citations'], 'Ask after migration'
    assert cli('pack', str(Path(temp) / 'migrated-pack'), '--query', query)['embedding_recipe'] == recipe
    assert cli('index', 'spike/fixtures/documents')['skipped'] == 20
    assert (library / 'active-index.json').read_bytes() == pointer_bytes
    assert (legacy / 'recipe.json').read_bytes() == old_bytes
    state = explicit_state

    # Short sources used to make the model stop after a bare citation marker.
    small = Path(temp) / 'small'
    small.mkdir()
    for name, text in {
        'orbital.txt': 'Orbital archive retention is thirty days, after which the cold storage tier deletes the snapshot.',
        'invoice.txt': 'Invoice 4471 from Northwind Supplies is due on the fifteenth of next month.',
        'heron.txt': 'A grey heron was seen fishing at the north pond every morning in October.',
    }.items():
        (small / name).write_text(text, encoding='utf-8')
    state = Path(temp) / 'small-state'
    small_indexed = cli('index', str(small))
    assert small_indexed['completed'] == 3 and small_indexed['failed'] == 0, small_indexed
    small_answers = []
    for question in ['How long is the orbital archive kept?',
                     'What is the orbital archive retention policy?']:
        small_answer = cli('ask', question)
        body, separator, footer = small_answer['text'].rpartition('\n\nRetrieved sources: ')
        assert separator and len(body.split()) > 3, small_answer
        assert 'thirty days' in body.lower(), small_answer
        numbers = [int(n) for n in re.findall(r'\[(\d+)\]', footer)]
        assert numbers == [1, 2, 3] == [c['number'] for c in small_answer['citations']], small_answer
        small_answers.append({'question': question, 'answer': small_answer})

    # A real two-page PDF: text on page 1, an image XObject only on page 2.
    mixed = Path(temp) / 'mixed'
    mixed.mkdir()
    text = 'BT /F1 12 Tf 20 100 Td (Searchable mixed document) Tj ET'
    image = 'q 40 0 0 40 20 20 cm /Im1 Do Q'
    objects = [
        '<</Type/Catalog/Pages 2 0 R>>',
        '<</Type/Pages/Kids[3 0 R 4 0 R]/Count 2>>',
        '<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Resources<</Font<</F1 5 0 R>>>>/Contents 6 0 R>>',
        '<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Resources<</XObject<</Im1 8 0 R>>>>/Contents 7 0 R>>',
        '<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>',
        f'<</Length {len(text)}>>\nstream\n{text}\nendstream',
        f'<</Length {len(image)}>>\nstream\n{image}\nendstream',
        '<</Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceGray/BitsPerComponent 8/Filter/ASCIIHexDecode/Length 3>>\nstream\n80>\nendstream',
    ]
    pdf = '%PDF-1.4\n'
    offsets = []
    for n, obj in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf += f'{n} 0 obj\n{obj}\nendobj\n'
    xref = len(pdf)
    pdf += f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'
    pdf += ''.join(f'{offset:010} 00000 n \n' for offset in offsets)
    pdf += f'trailer\n<</Size {len(objects)+1}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n'
    (mixed / 'mixed.pdf').write_text(pdf, encoding='ascii', newline='\n')
    state = Path(temp) / 'mixed-state'
    mixed_result = cli('index', str(mixed))
    assert mixed_result['completed'] == 1 and mixed_result['failed'] == 0, mixed_result
    if os.name == 'nt':
        assert mixed_result['needs_ocr_pages'] == [2], mixed_result
        assert 'needs OCR' in mixed_result['reason'] and '2' in mixed_result['reason'], mixed_result
    # Every CLI call reopens SQLite and the committed generation.
    reopened = cli('index', str(mixed))
    assert reopened['skipped'] == 1, reopened
    assert reopened['needs_ocr_pages'] == mixed_result['needs_ocr_pages'], reopened
    mixed_hits = cli('search', 'Searchable mixed document')
    assert mixed_hits and mixed_hits[0]['chunk']['page'] == 1, mixed_hits
    import sqlite3
    with closing(sqlite3.connect(state / 'state.sqlite')) as db:
        generation = db.execute('SELECT generation FROM documents').fetchone()[0]
    stored = json.loads((state / 'generations' / generation / 'document.json').read_text())
    assert stored['pages'] == 2 and stored['needs_ocr_pages'] == reopened['needs_ocr_pages'], stored
    state = Path(temp) / 'state'
    missing = subprocess.run([str(CLI), '--cpu', '--state', str(state), 'index', str(Path(temp) / 'missing-source')], cwd=ROOT, capture_output=True, text=True, encoding="utf-8", timeout=120)
    assert missing.returncode != 0 and 'source unavailable' in missing.stderr
    assert cli('search', query) == [], 'unavailable source must revoke cached content'
    print(json.dumps({'source_revocation': 'OK', 'index': indexed, 'unchanged': unchanged, 'search_top': hits[0]['chunk']['doc_id'], 'answer': answer, 'small_answers': small_answers, 'pack_hashes': 'OK', 'example_query': 'OK'}))
