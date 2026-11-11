#!/usr/bin/env python3
"""Start a pinned MinIO with throwaway credentials; verify every uploaded byte."""
import hashlib
import json
import os
from pathlib import Path
import socket
import shutil
import subprocess
import sys
import tempfile
import time
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
    child = subprocess.Popen([str(server), 'server', data, '--address', f'127.0.0.1:{port}', '--console-address', '127.0.0.1:0'], env=env, stdout=log, stderr=subprocess.STDOUT)
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
        subprocess.run([str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'phase0-pack'], env=env, check=True)
        for name in [*manifest['files'], 'manifest.json']:
            body = s3.get_object(Bucket='phase0-pack', Key=prefix + '/' + name)['Body'].read()
            assert body == (pack / name).read_bytes(), name
        retry = subprocess.run([str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'phase0-pack', '--prefix', prefix], env=env, capture_output=True, text=True, encoding='utf-8', timeout=120)
        assert retry.returncode != 0 and 'conditional write failed' in retry.stderr, retry.stderr
        for name in [*manifest['files'], 'manifest.json']:
            assert s3.get_object(Bucket='phase0-pack', Key=prefix + '/' + name)['Body'].read() == (pack / name).read_bytes()
        assert not s3.list_objects_v2(Bucket='phase0-pack', Prefix='_vz-staging/').get('Contents'), 'job-owned staging must be cleaned'
        # Fresh bucket permits a second completion boundary without overwriting
        # any prior pack. Lose only the private receipt after manifest readback.
        s3.create_bucket(Bucket='uncertain-pack')
        uncertain_env = dict(env, VZ_TEST_LOST_TRANSFER_RECEIPT='1')
        failed = subprocess.run([*RTK, str(cli), '--review-reference', str(reference), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'uncertain-pack'], env=uncertain_env, capture_output=True, text=True, encoding='utf-8', timeout=120)
        assert failed.returncode == 3 and 'injected lost transfer receipt' in failed.stderr, failed.stderr
        for name in [*manifest['files'], 'manifest.json']:
            assert s3.get_object(Bucket='uncertain-pack', Key=prefix + '/' + name)['Body'].read() == (pack / name).read_bytes()
        ref = json.loads(reference.read_text())
        generation = Path(ref['store']) / manifest['pack_id'] / ref['current']['generation']
        intents = [json.loads(path.read_text()) for path in generation.glob('transfer-*.json')]
        pending = [intent for intent in intents if '/uncertain-pack/' in (intent.get('destination') or '')]
        assert len(pending) == 1, pending
        assert not (generation / f"completed-{pending[0]['job_id']}.json").exists()
        print(json.dumps({'MinIO': 'RELEASE.2025-04-22T22-12-26Z', 'upload_round_trip': 'OK', 'files': len(manifest['files']) + 1}))
    finally:
        child.terminate()
        try: child.wait(timeout=15)
        except subprocess.TimeoutExpired: child.kill(); child.wait()
