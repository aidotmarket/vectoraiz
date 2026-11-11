#!/usr/bin/env python3
"""Start a pinned MinIO with throwaway credentials; verify every uploaded byte."""
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import threading
import urllib.request
import boto3
from botocore.config import Config

ROOT = Path(__file__).resolve().parents[1]
RTK = ["rtk", "proxy"] if shutil.which("rtk") else []
pack = Path(sys.argv[1])
reference = Path(sys.argv[2])
prefix = "vz/" + json.loads((pack / "manifest.json").read_text())["pack_id"]
manifest = json.loads((pack / 'manifest.json').read_text())
# Test-only credentials never leave the local ephemeral server.
env = os.environ.copy()
env['MINIO_ROOT_USER'] = env['AWS_ACCESS_KEY_ID'] = 'phase0-fixture-user'
env['MINIO_ROOT_PASSWORD'] = env['AWS_SECRET_ACCESS_KEY'] = 'phase0-fixture-password'
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
endpoint = f'http://127.0.0.1:{port}'
server = ROOT / ('.tools/minio.exe' if os.name == 'nt' else '.tools/minio')
cli = ROOT / ('target/debug/vectoraiz-cli.exe' if os.name == 'nt' else 'target/debug/vectoraiz-cli')
with tempfile.TemporaryDirectory(prefix='vz-minio-') as data, (Path(os.environ.get('KD_BRIDGE_JOB_EVIDENCE', data)) / 'minio-test.log').open('w') as log:
    child = subprocess.Popen([*RTK, str(server), 'server', data, '--address', f'127.0.0.1:{port}', '--console-address', '127.0.0.1:0'], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=os.name != "nt")
    try:
        deadline = time.monotonic() + 60
        while True:
            try:
                urllib.request.urlopen(endpoint + '/minio/health/ready', timeout=1).close(); break
            except OSError:
                if time.monotonic() > deadline or child.poll() is not None: raise RuntimeError('MinIO failed to start; see minio-test.log')
                time.sleep(.2)
        s3 = boto3.client('s3', endpoint_url=endpoint, aws_access_key_id=env['AWS_ACCESS_KEY_ID'], aws_secret_access_key=env['AWS_SECRET_ACCESS_KEY'], region_name='us-east-1', config=Config(s3={'addressing_style': 'path'}))
        s3.create_bucket(Bucket='phase0-pack')
        subprocess.run([*RTK, str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'phase0-pack'], env=env, check=True)
        for name in [*manifest['files'], 'manifest.json']:
            body = s3.get_object(Bucket='phase0-pack', Key=prefix + '/' + name)['Body'].read()
            assert body == (pack / name).read_bytes(), name
        ref = json.loads(reference.read_text())
        generation = Path(ref['store']) / manifest['pack_id'] / ref['current']['generation']
        before_retry = {path.name for path in generation.glob('*.json')}
        retry = subprocess.run([*RTK, str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'phase0-pack', '--prefix', prefix], env=env, capture_output=True, text=True, encoding='utf-8', timeout=120)
        assert retry.returncode == 0, retry.stderr
        assert {path.name for path in generation.glob('*.json')} == before_retry, 'repeat created a new intent/receipt'
        for name in [*manifest['files'], 'manifest.json']:
            assert s3.get_object(Bucket='phase0-pack', Key=prefix + '/' + name)['Body'].read() == (pack / name).read_bytes()
        keys = {obj['Key'] for obj in s3.list_objects_v2(Bucket='phase0-pack').get('Contents', [])}
        assert keys == {prefix + '/' + name for name in [*manifest['files'], 'manifest.json']}, keys
        # A different completed manifest refuses before writes or a new intent.
        s3.create_bucket(Bucket='manifest-conflict-pack')
        s3.put_object(Bucket='manifest-conflict-pack', Key=prefix + '/manifest.json', Body=b'conflicting manifest')
        before_conflict = {path.name for path in generation.glob('*.json')}
        conflict = subprocess.run([*RTK, str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'manifest-conflict-pack'], env=env, capture_output=True, text=True, encoding='utf-8', timeout=120)
        assert conflict.returncode == 1 and 'readback mismatch' in conflict.stderr, conflict.stderr
        assert {path.name for path in generation.glob('*.json')} == before_conflict, 'conflict created an intent/receipt'
        assert s3.get_object(Bucket='manifest-conflict-pack', Key=prefix + '/manifest.json')['Body'].read() == b'conflicting manifest'
        assert {obj['Key'] for obj in s3.list_objects_v2(Bucket='manifest-conflict-pack').get('Contents', [])} == {prefix + '/manifest.json'}
        # A partial final namespace resumes only matching bytes, without staging.
        s3.create_bucket(Bucket='partial-pack')
        first = next(iter(manifest['files']))
        s3.put_object(Bucket='partial-pack', Key=prefix + '/' + first, Body=(pack / first).read_bytes())
        subprocess.run([*RTK, str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'partial-pack'], env=env, check=True)
        for name in [*manifest['files'], 'manifest.json']:
            assert s3.get_object(Bucket='partial-pack', Key=prefix + '/' + name)['Body'].read() == (pack / name).read_bytes()
        s3.create_bucket(Bucket='conflict-pack')
        s3.put_object(Bucket='conflict-pack', Key=prefix + '/' + first, Body=b'conflicting bytes')
        conflict = subprocess.run([*RTK, str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'conflict-pack'], env=env, capture_output=True, text=True, encoding='utf-8', timeout=120)
        assert conflict.returncode != 0 and 'readback mismatch' in conflict.stderr, conflict.stderr
        assert s3.get_object(Bucket='conflict-pack', Key=prefix + '/' + first)['Body'].read() == b'conflicting bytes'
        assert 'manifest.json' not in {obj['Key'].removeprefix(prefix + '/') for obj in s3.list_objects_v2(Bucket='conflict-pack').get('Contents', [])}
        # Fresh bucket permits a second completion boundary without overwriting
        # any prior pack. Lose only the private receipt after manifest readback.
        s3.create_bucket(Bucket='uncertain-pack')
        uncertain_env = dict(env, VZ_TEST_LOST_TRANSFER_RECEIPT='1')
        failed = subprocess.run([*RTK, str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'uncertain-pack'], env=uncertain_env, capture_output=True, text=True, encoding='utf-8', timeout=120)
        assert failed.returncode == 3 and 'injected lost transfer receipt' in failed.stderr, failed.stderr
        for name in [*manifest['files'], 'manifest.json']:
            assert s3.get_object(Bucket='uncertain-pack', Key=prefix + '/' + name)['Body'].read() == (pack / name).read_bytes()
        intents = [json.loads(path.read_text()) for path in generation.glob('transfer-*.json')]
        pending = [intent for intent in intents if '/uncertain-pack/' in (intent.get('destination') or '')]
        assert len(pending) == 1, pending
        assert not (generation / f"completed-{pending[0]['job_id']}.json").exists()
        recovered = subprocess.run([*RTK, str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'uncertain-pack'], env=env, capture_output=True, text=True, encoding='utf-8', timeout=120)
        assert recovered.returncode == 0, recovered.stderr
        assert (generation / f"completed-{pending[0]['job_id']}.json").is_file(), 'original intent remains uncertain'
        assert len([p for p in generation.glob('transfer-*.json') if '/uncertain-pack/' in (json.loads(p.read_text()).get('destination') or '')]) == 1

        # Model AWS's no-ListBucket missing-key 403 by injecting only the
        # preflight response, forwarding all remaining signed requests
        # (and their original Host) to the real MinIO conditional-write path.
        class ForbiddenPreflight(BaseHTTPRequestHandler):
            deny_next = False
            writes = []

            def log_message(self, *args):
                pass

            def forward(self):
                if self.command == 'GET' and self.path.endswith('/manifest.json') and ForbiddenPreflight.deny_next:
                    ForbiddenPreflight.deny_next = False
                    self.send_response(403)
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
                body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
                if self.command == 'PUT':
                    assert self.headers.get('If-None-Match') == '*', 'unconditional write'
                connection = http.client.HTTPConnection('127.0.0.1', port, timeout=120)
                try:
                    connection.request(self.command, self.path, body=body, headers=dict(self.headers))
                    response = connection.getresponse()
                    data = response.read()
                    if self.command == 'PUT':
                        ForbiddenPreflight.writes.append((self.path, response.status))
                    self.send_response(response.status)
                    self.send_header('Content-Length', str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                finally:
                    connection.close()

            do_GET = do_PUT = forward

        proxy = ThreadingHTTPServer(('127.0.0.1', 0), ForbiddenPreflight)
        worker = threading.Thread(target=proxy.serve_forever, daemon=True)
        worker.start()
        try:
            proxy_endpoint = f'http://127.0.0.1:{proxy.server_port}'
            def forbidden_upload(bucket, expected):
                ForbiddenPreflight.deny_next = True
                ForbiddenPreflight.writes = []
                result = subprocess.run([*RTK, str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', proxy_endpoint, '--bucket', bucket], env=env, capture_output=True, text=True, encoding='utf-8', timeout=120)
                assert result.returncode == expected, result.stderr
                assert not ForbiddenPreflight.deny_next, 'preflight not exercised'
                return result

            s3.create_bucket(Bucket='forbidden-preflight-pack')
            forbidden_upload('forbidden-preflight-pack', 0)
            assert all(status == 200 for _, status in ForbiddenPreflight.writes)
            forbidden_upload('forbidden-preflight-pack', 0)
            assert all(status == 412 for _, status in ForbiddenPreflight.writes)
            assert any(path.endswith('/manifest.json') for path, _ in ForbiddenPreflight.writes)
            for name in [*manifest['files'], 'manifest.json']:
                assert s3.get_object(Bucket='forbidden-preflight-pack', Key=prefix + '/' + name)['Body'].read() == (pack / name).read_bytes()
            # Existing payload bytes match, but the manifest must never be replaced.
            s3.put_object(Bucket='forbidden-preflight-pack', Key=prefix + '/manifest.json', Body=b'conflicting manifest')
            conflict = forbidden_upload('forbidden-preflight-pack', 3)
            assert 'readback mismatch' in conflict.stderr, conflict.stderr
            assert s3.get_object(Bucket='forbidden-preflight-pack', Key=prefix + '/manifest.json')['Body'].read() == b'conflicting manifest'
            assert all(status == 412 for _, status in ForbiddenPreflight.writes)
            proxy_pending = []
            for path in generation.glob('transfer-*.json'):
                intent = json.loads(path.read_text())
                receipt = generation / f"completed-{intent['job_id']}.json"
                if '/forbidden-preflight-pack/' in (intent.get('destination') or '') and not receipt.exists():
                    proxy_pending.append(intent)
            assert len(proxy_pending) == 1, proxy_pending
            pending_receipt = generation / f"completed-{proxy_pending[0]['job_id']}.json"
            # A payload mismatch also prevents a successful completed retry.
            s3.put_object(Bucket='forbidden-preflight-pack', Key=prefix + '/manifest.json', Body=(pack / 'manifest.json').read_bytes())
            s3.put_object(Bucket='forbidden-preflight-pack', Key=prefix + '/' + first, Body=b'conflicting bytes')
            conflict = forbidden_upload('forbidden-preflight-pack', 1)
            assert 'readback mismatch' in conflict.stderr, conflict.stderr
            assert not any(path.endswith('/manifest.json') for path, _ in ForbiddenPreflight.writes)
            assert not pending_receipt.exists(), 'mismatching bytes resolved an uncertain transfer'
            s3.put_object(Bucket='forbidden-preflight-pack', Key=prefix + '/' + first, Body=(pack / first).read_bytes())
            forbidden_upload('forbidden-preflight-pack', 0)
            assert pending_receipt.is_file(), '403 retry did not resolve the original intent'
        finally:
            proxy.shutdown()
            proxy.server_close()
            worker.join(timeout=5)
        print(json.dumps({'MinIO': 'RELEASE.2025-04-22T22-12-26Z', 'upload_round_trip': 'OK', 'completed_rerun_no_intent': 'OK', 'manifest_conflict_no_intent': 'OK', 'lost_receipt_recovered': 'OK', 'mocked_403_fresh_retry_conflict': 'OK', 'files': len(manifest['files']) + 1}))
    finally:
        if os.name == 'nt': child.terminate()
        else: os.killpg(child.pid, signal.SIGTERM)
        try: child.wait(timeout=15)
        except subprocess.TimeoutExpired:
            if os.name == 'nt': child.kill()
            else: os.killpg(child.pid, signal.SIGKILL)
            child.wait()
