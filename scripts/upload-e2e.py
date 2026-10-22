#!/usr/bin/env python3
"""Start a pinned MinIO with throwaway credentials; verify every uploaded byte."""
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import boto3
from botocore.config import Config

ROOT = Path(__file__).resolve().parents[1]
pack = Path(sys.argv[1])
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
with tempfile.TemporaryDirectory(prefix='vz-minio-') as data, (ROOT / 'minio-test.log').open('w') as log:
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
        subprocess.run([str(cli), 'upload', str(pack), '--endpoint', endpoint, '--bucket', 'phase0-pack', '--prefix', 'vz/fixture-pack'], env=env, check=True)
        for name in [*manifest['files'], 'manifest.json']:
            body = s3.get_object(Bucket='phase0-pack', Key='vz/fixture-pack/' + name)['Body'].read()
            assert body == (pack / name).read_bytes(), name
        print(json.dumps({'MinIO': 'RELEASE.2025-04-22T22-12-26Z', 'upload_round_trip': 'OK', 'files': len(manifest['files']) + 1}))
    finally:
        child.terminate()
        try: child.wait(timeout=15)
        except subprocess.TimeoutExpired: child.kill(); child.wait()
