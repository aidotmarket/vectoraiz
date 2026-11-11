#!/usr/bin/env python3
"""CPU-only acceptance: all fixture formats, real citations, pack hashes and query."""
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parents[1]
RTK = ["rtk", "proxy"] if shutil.which("rtk") else []
CLI = ROOT / ('target/debug/vectoraiz-cli.exe' if os.name == 'nt' else 'target/debug/vectoraiz-cli')
# Preview is enabled only for this acceptance build, never by environment or
# user preference. Restore the default-off CLI even when the journey fails.
subprocess.run(["cargo", "build", "-p", "vz-cli", "-p", "vz-parser-worker", "--features", "vz-cli/listing-preview-harness"], cwd=ROOT, check=True)
try:
    with tempfile.TemporaryDirectory(prefix='vz-pipeline-') as temp:
        temp = str(Path(temp).resolve())
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
        def reviewed_pack(output):
            source = ROOT / "spike/fixtures/documents"
            changes = Path(temp) / "changes.json"
            changes.write_text(json.dumps({"edits": {"documents.parquet/column/2/name": "reviewed_title", "documents.parquet/column/5/name": "reviewed_language"}, "exclusions": [p.name for p in source.iterdir() if p.suffix not in (".txt", ".md", ".markdown")], "identity_terms": ["private-fixture-identity"]}))
            reference = cli("preview", "--store", str(Path(temp) / "reviews"), "--source", str(source), "--changes", str(changes), "--query", query)
            reference_path = Path(temp) / "reference.json"
            reference_path.write_text(json.dumps(reference))
            current_path = Path(temp) / "current.json"
            current_path.write_text(json.dumps(reference["current"]))
            review = cli("review", "--store", reference["store"], "--current", str(current_path))
            assert review["metadata_inventory"] and review["accepted_review_hash"] is None
            assert any(e["value"] is None for e in review["metadata_inventory"])
            accepted = cli("review", "--store", reference["store"], "--current", str(current_path), "--accept")
            assert accepted["accepted_review_hash"] == reference["current"]["review_hash"]
            manifest = cli("--review-reference", str(reference_path), "pack", str(output), "--query", query)
            for path in output.rglob("*"):
                if path.is_file():
                    assert b"private-fixture-identity" not in path.read_bytes()
            # Exercise the same reviewed bytes through the folder adapter.
            gateway = Path(temp) / ("gateway-" + manifest["pack_id"])
            staging = Path(temp) / ("staging-" + manifest["pack_id"])
            gateway.mkdir(); staging.mkdir()
            exported = cli("--review-reference", str(reference_path), "export", str(output), "--pack-id", manifest["pack_id"], "--folder", str(gateway), "--staging", str(staging))
            assert exported
            for path in output.rglob("*"):
                if path.is_file():
                    assert (gateway / "vz" / manifest["pack_id"] / path.relative_to(output)).read_bytes() == path.read_bytes()
            # Exercise the shared CLI error boundary for local transfers after
            # promotion: destination bytes survive, receipt remains uncertain.
            fault_env = dict(os.environ, VZ_TEST_LOST_TRANSFER_RECEIPT="1")
            lost_package = Path(temp) / ("lost-package-" + manifest["pack_id"])
            lost_folder = Path(temp) / ("lost-folder-" + manifest["pack_id"])
            lost_folder.mkdir()
            for command, destination in [
                (["--cpu", "--state", str(state), "pack", str(lost_package), "--query", query], lost_package),
                (["export", str(output), "--pack-id", manifest["pack_id"], "--folder", str(lost_folder), "--staging", str(staging)], lost_folder / "vz" / manifest["pack_id"]),
            ]:
                failed = subprocess.run([*RTK, str(CLI), "--review-reference", str(reference_path), *command], env=fault_env, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", timeout=600)
                assert failed.returncode == 3 and "injected lost transfer receipt" in failed.stderr, failed.stderr
                for path in output.rglob("*"):
                    if path.is_file():
                        assert (destination / path.relative_to(output)).read_bytes() == path.read_bytes()
                generation = Path(reference["store"]) / manifest["pack_id"] / reference["current"]["generation"]
                intents = [json.loads(path.read_text()) for path in generation.glob("transfer-*.json")]
                pending = [intent for intent in intents if str(lost_package if destination == lost_package else lost_folder) in (intent.get("destination") or "")]
                assert len(pending) == 1, pending
                assert not (generation / f"completed-{pending[0]['job_id']}.json").exists()
            # Live loopback SFTP uses the production reviewed entry point.
            remote = Path(temp) / ("sftp-" + manifest["pack_id"])
            remote.mkdir(); (remote / "source").mkdir(); (remote / "staging").mkdir()
            server = subprocess.Popen([sys.executable, str(ROOT / "scripts/sftp-test-server.py"), str(remote)], stdout=subprocess.PIPE, text=True, encoding="utf-8")
            try:
                ready = json.loads(server.stdout.readline())
                report = cli("--review-reference", str(reference_path), "export", str(output), "--pack-id", manifest["pack_id"], "--sftp", f"fixture@127.0.0.1:{ready['port']}:/source", "--sftp-staging", "/staging", "--identity", str(remote / "client.pem"), "--known-hosts", str(remote / "pins.json"))
                assert report["mode"] == "sftp"
                for path in output.rglob("*"):
                    if path.is_file():
                        assert (remote / "source/vz" / manifest["pack_id"] / path.relative_to(output)).read_bytes() == path.read_bytes()
                # The real reviewed path must retain completed bytes and a
                # destination-bound intent when the rename acknowledgement is lost.
                (remote / "source-lost-ack").mkdir()
                (remote / "lost-rename-ack").write_text("fixture fault")
                uncertain = subprocess.run([str(CLI), "--review-reference", str(reference_path), "export", str(output), "--pack-id", manifest["pack_id"], "--sftp", f"fixture@127.0.0.1:{ready['port']}:/source-lost-ack", "--sftp-staging", "/staging", "--identity", str(remote / "client.pem"), "--known-hosts", str(remote / "pins.json")], cwd=ROOT, capture_output=True, text=True, encoding="utf-8", timeout=120)
                print(uncertain.stderr, file=sys.stderr)
                assert uncertain.returncode == 3, uncertain.stderr
                assert (remote / "source-lost-ack/vz" / manifest["pack_id"] / "manifest.json").read_bytes() == (output / "manifest.json").read_bytes()
                generation = Path(reference["store"]) / manifest["pack_id"] / reference["current"]["generation"]
                intents = [json.loads(path.read_text()) for path in generation.glob("transfer-*.json")]
                lost = [intent for intent in intents if "/source-lost-ack/vz/" in (intent.get("destination") or "")]
                assert len(lost) == 1 and "|stage:/staging/" in lost[0]["destination"], lost
                assert not (generation / f"completed-{lost[0]['job_id']}.json").exists()
            finally:
                server.terminate(); server.wait(timeout=15)
            return manifest, reference_path
        manifest, reference_path = reviewed_pack(pack)
        # object_pairs_hook runs bottom-up, including objects nested inside arrays.
        def sorted_object(pairs):
            keys = [key for key, _ in pairs]
            assert keys == sorted(keys), f'JSON object keys are not sorted: {keys}'
            return dict(pairs)
        for name in ['examples/query.json', 'manifest.json']:
            json.loads((pack / name).read_text(encoding='utf-8'), object_pairs_hook=sorted_object)
        docs = {doc['doc_id']: doc for doc in pq.read_table(pack / 'documents.parquet').to_pylist()}
        for hit in hits:
            chunk = hit['chunk']
            if chunk['doc_id'] not in docs: continue  # explicitly excluded unsupported original
            assert 1 <= chunk['page'] <= docs[chunk['doc_id']]['pages']
        chunks = {c['chunk_id']: c for c in pq.read_table(pack / 'chunks.parquet').to_pylist()}
        for citation in answer['citations']:
            if citation['doc_id'] not in docs: continue
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
            subprocess.run([sys.executable, str(ROOT / 'scripts/upload-e2e.py'), str(pack), str(reference_path)], check=True)
        # Same dimensions and model bytes, different model identity: dimensions alone
        # must never authorize queries or relabel an export.
        pins = json.loads((ROOT / 'spike/fixtures/models.json').read_text())
        pins['embedding']['name'] += '-different-model'
        alternate = Path(temp) / 'alternate-models.json'
        alternate.write_text(json.dumps(pins))
        for operation in [('search', query), ('ask', query), ('pack', str(Path(temp) / 'wrong-pack'))]:
            refused = subprocess.run([str(CLI), '--review-reference', str(reference_path), '--cpu', '--state', str(state),
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
        assert reviewed_pack(Path(temp) / 'migrated-pack')[0]['embedding_recipe'] == recipe
        assert cli('index', 'spike/fixtures/documents')['skipped'] == 20
        assert (library / 'active-index.json').read_bytes() == pointer_bytes
        assert (legacy / 'recipe.json').read_bytes() == old_bytes
        # Gate 3: exercise real indexer failures, publication and retry, rather
        # than constructing only synthetic completion statistics.
        def migration_library(name):
            target = Path(temp) / name
            shutil.copytree(explicit_state, target / 'phase0-index')
            (target / 'phase0-index/recipe.json').write_bytes(old_bytes)
            old_id = hashlib.sha256(json.dumps(json.loads(old_bytes), sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()[:16]
            (target / 'active-index.json').write_text(json.dumps({'dir': 'phase0-index', 'recipe_id': old_id}))
            return target

        def refused_index(folder, assets=ROOT / '.assets'):
            result = subprocess.run([str(CLI), '--cpu', '--state', str(state), '--assets', str(assets),
                                     'index', str(folder)], cwd=ROOT, text=True, encoding='utf-8', capture_output=True, timeout=600)
            print(result.stderr, file=sys.stderr)
            assert result.returncode != 0 and 'Re-index did not finish' in result.stderr, result.stderr
            return result.stderr

        supported_only = Path(temp) / 'supported-only'
        supported_only.mkdir()
        (supported_only / 'good.txt').write_text('Orbital archive retention is thirty days.')
        (supported_only / '.DS_Store').write_bytes(b'ignored')
        (supported_only / 'photo.png').write_bytes(b'ignored')
        state = migration_library('supported-only-library')
        filtered = cli('index', str(supported_only))
        assert filtered['total'] == 1 and filtered['completed'] == 1, filtered

        content = Path(temp) / 'content-failures'
        content.mkdir()
        (content / 'good.txt').write_text('Orbital archive retention is thirty days.')
        (content / 'bad.docx').write_bytes(b'not a ZIP document')
        (content / 'unsupported.bin').write_bytes(b'unsupported')
        state = migration_library('mixed-content-library')
        mixed_result = cli('index', str(content))
        assert (mixed_result['completed'], mixed_result['failed'], mixed_result['infrastructure_failed']) == (1, 1, 0), mixed_result
        assert json.loads((state / 'active-index.json').read_text())['dir'].startswith('indexes/')

        (content / 'good.txt').unlink()
        state = migration_library('all-failed-library')
        before = (state / 'active-index.json').read_bytes()
        failure = refused_index(content)
        assert '0 indexed' in failure and '1 failed' in failure, failure
        assert (state / 'active-index.json').read_bytes() == before
        assert list((state / 'indexes').glob('*/recipe.json')), 'candidate retained'

        infrastructure = Path(temp) / 'infrastructure-input'
        infrastructure.mkdir()
        (infrastructure / 'a-good.txt').write_text('Orbital archive retention is thirty days.')
        shutil.copyfile(ROOT / 'spike/fixtures/documents/access.pdf', infrastructure / 'b-access.pdf')
        incomplete_assets = Path(temp) / 'assets-without-pdfium'
        incomplete_assets.mkdir()
        for kind in ['embedding', 'chat']:
            model_file = pins[kind]['file']
            # Runner temp directories can be on a different Windows volume.
            shutil.copyfile(ROOT / '.assets' / model_file, incomplete_assets / model_file)
        state = migration_library('infrastructure-library')
        before = (state / 'active-index.json').read_bytes()
        failure = refused_index(infrastructure, incomplete_assets)
        assert '1 indexed' in failure and '1 failed' in failure and '1 infrastructure' in failure, failure
        assert 'b-access.pdf' in failure and 'PDFium asset missing' in failure, failure
        assert failure.count('Re-index did not finish') == 1, failure
        assert (state / 'active-index.json').read_bytes() == before
        resumed = cli('index', str(infrastructure))
        assert (resumed['completed'], resumed['skipped'], resumed['failed']) == (1, 1, 0), resumed
        assert (state / 'active-index.json').read_bytes() != before
        assert cli('search', query), 'search after infrastructure retry'

        # A matching legacy recipe is adopted without re-embedding or a new directory.
        state = Path(temp) / 'matching-legacy-library'
        shutil.copytree(explicit_state, state / 'phase0-index')
        assert cli('index', 'spike/fixtures/documents')['skipped'] == 20
        assert json.loads((state / 'active-index.json').read_text())['dir'] == 'phase0-index'
        assert not (state / 'indexes').exists()
        assert cli('search', query)

        state = Path(temp) / 'interrupted-first-library'
        (state / 'indexes').mkdir(parents=True)
        missing = subprocess.run([str(CLI), '--cpu', '--state', str(state), 'search', query],
                                 cwd=ROOT, text=True, encoding='utf-8', capture_output=True, timeout=600)
        assert missing.returncode != 0 and 're-index required' in missing.stderr
        assert not (state / 'phase0-index').exists(), 'read must not create legacy state'
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
        # A PDF whose second page object is missing: PDFium loads the document but not
        # page 2. It must be a document failure, never a silently truncated success.
        broken = Path(temp) / 'broken'
        broken.mkdir()
        (broken / 'ok.md').write_text('Healthy companion document about damaged page handling.\n', encoding='utf-8')
        bad_objects = [
            '<</Type/Catalog/Pages 2 0 R>>',
            '<</Type/Pages/Kids[3 0 R 99 0 R]/Count 2>>',
            '<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Resources<</Font<</F1 4 0 R>>>>/Contents 5 0 R>>',
            '<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>',
            f'<</Length {len(text)}>>\nstream\n{text}\nendstream',
        ]
        bad = '%PDF-1.4\n'
        bad_offsets = []
        for n, obj in enumerate(bad_objects, 1):
            bad_offsets.append(len(bad))
            bad += f'{n} 0 obj\n{obj}\nendobj\n'
        bad_xref = len(bad)
        bad += f'xref\n0 {len(bad_objects)+1}\n0000000000 65535 f \n'
        bad += ''.join(f'{offset:010} 00000 n \n' for offset in bad_offsets)
        bad += f'trailer\n<</Size {len(bad_objects)+1}/Root 1 0 R>>\nstartxref\n{bad_xref}\n%%EOF\n'
        (broken / 'broken.pdf').write_text(bad, encoding='ascii', newline='\n')
        state = Path(temp) / 'broken-state'
        broken_result = cli('index', str(broken))
        assert broken_result['completed'] == 1 and broken_result['failed'] == 1, broken_result
        assert broken_result.get('infrastructure_failed', 0) == 0, broken_result
        state = Path(temp) / 'state'
        missing = subprocess.run([str(CLI), '--cpu', '--state', str(state), 'index', str(Path(temp) / 'missing-source')], cwd=ROOT, capture_output=True, text=True, encoding="utf-8", timeout=120)
        assert missing.returncode != 0 and 'source unavailable' in missing.stderr
        assert cli('search', query) == [], 'unavailable source must revoke cached content'
        print(json.dumps({'source_revocation': 'OK', 'index': indexed, 'unchanged': unchanged, 'search_top': hits[0]['chunk']['doc_id'], 'answer': answer, 'small_answers': small_answers, 'pack_hashes': 'OK', 'example_query': 'OK'}))
finally:
    subprocess.run(["cargo", "build", "-p", "vz-cli"], cwd=ROOT, check=True)
