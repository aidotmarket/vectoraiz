#!/usr/bin/env python3
"""CPU-only acceptance: all fixture formats, real citations, pack hashes and query."""
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
        result = subprocess.run([str(CLI), '--cpu', '--state', str(state), *args], cwd=ROOT, text=True, capture_output=True, timeout=600)
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
    assert len(answer['text'].split()) > 3, answer
    assert answer['citations'] and re.search(r'\[[1-3]\]', answer['text']), answer
    manifest = cli('pack', str(pack), '--query', query)
    docs = {doc['doc_id']: doc for doc in pq.read_table(pack / 'documents.parquet').to_pylist()}
    for hit in hits:
        chunk = hit['chunk']
        assert chunk['doc_id'] in docs
        assert 1 <= chunk['page'] <= docs[chunk['doc_id']]['pages']
    for citation in answer['citations']:
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
    print(json.dumps({'index': indexed, 'unchanged': unchanged, 'search_top': hits[0]['chunk']['doc_id'], 'answer': answer, 'pack_hashes': 'OK', 'example_query': 'OK'}))
