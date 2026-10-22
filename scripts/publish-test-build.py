#!/usr/bin/env python3
"""Sign and verify CI artifacts locally; publish only when explicitly requested."""
import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = 'aidotmarket/vectoraiz'
DEST = 'aidotmarket/vectoraiz-test-builds'


def command(*args, **kwargs):
    return subprocess.run(list(map(str, args)), check=True, **kwargs)


def sha256(file):
    with file.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def api(path):
    return json.loads(subprocess.check_output(['gh', 'api', path], text=True))


def docker_checks():
    latest = api(f'repos/{SOURCE}/releases/latest')
    assert latest['tag_name'].startswith('v'), 'Docker latest is no longer a v* release'
    releases = api(f'repos/{SOURCE}/releases?per_page=100')
    first = next((r for r in releases if r['prerelease'] and not r['draft']), None)
    assert first is None or first['tag_name'].startswith('v'), 'Docker prerelease tag changed'
    return {'latest': latest['tag_name'], 'first_prerelease': first['tag_name'] if first else None}


def rotated_secrets():
    # Run the approved Cloud helper in isolated Python. Suppress upstream errors
    # and never include credentials in exception diagnostics or command arguments.
    helper = Path('/Users/max/koskadeux-state/s1811/infisical_endpoint_841ebac.py')
    environment = dict(os.environ)
    environment.pop('INFISICAL_PROJECT_ID', None)
    environment.pop('INFISICAL_API_URL', None)
    values = []
    for name in ['VECTORAIZ_TAURI_UPDATER_PRIVATE_KEY', 'VECTORAIZ_TAURI_UPDATER_KEY_PASSWORD']:
        result = command('env', '-u', 'INFISICAL_PROJECT_ID', '-u', 'INFISICAL_API_URL',
                         'python3', '-I', helper, 'get', name, 'prod', '/koskadeux',
                         env=environment, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        values.append(result.stdout.decode())
    if not values[0]:
        raise SystemExit('Cloud updater key is empty')
    return values


def public_packet(public):
    lines = base64.b64decode(public, validate=True).decode().splitlines()
    return base64.b64decode(lines[1], validate=True)


def signing_environment(dry_run, embedded, temporary):
    secret = temporary / 'signing.key'
    public_file = temporary / 'signing.pub'
    if dry_run:
        command('node', ROOT / 'app/node_modules/.bin/tauri', 'signer', 'generate',
                '--ci', '--password', '', '--write-keys', secret,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        key = secret.read_text().strip()
        public_file.write_bytes(base64.b64decode(secret.with_suffix('.key.pub').read_text().strip(), validate=True))
        secret.with_suffix('.key.pub').unlink()
        password = ''
    else:
        key, password = rotated_secrets()
        # Recreate the public key from the actual secret, not from a sibling
        # public secret or caller-supplied assertion. TemporaryDirectory is 0700.
        with secret.open('wb') as stream:
            os.chmod(secret, 0o600)
            stream.write(base64.b64decode(key, validate=True))
        command('minisign', '-R', '-s', secret, '-p', public_file,
                input=(password + '\n').encode(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    public = base64.b64encode(public_file.read_bytes()).decode()
    if not dry_run and public_packet(public) != public_packet(embedded):
        raise SystemExit('Release blocked: embedded updater public key does not match Cloud signing key')
    secret.unlink()
    return dict(os.environ, TAURI_SIGNING_PRIVATE_KEY=key,
                TAURI_SIGNING_PRIVATE_KEY_PASSWORD=password), public


def sign_and_verify(file, environment, public, version):
    signature = file.with_name(file.name + '.sig')
    command('node', ROOT / 'app/node_modules/.bin/tauri', 'signer', 'sign',
            '--app-version', version, file, env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    with tempfile.TemporaryDirectory(prefix='vz-verify-') as verification:
        sigfile = Path(verification) / 'artifact.sig'
        pubfile = Path(verification) / 'updater.pub'
        sigfile.write_bytes(base64.b64decode(signature.read_text().strip(), validate=True))
        pubfile.write_bytes(base64.b64decode(public, validate=True))
        command('minisign', '-Vm', file, '-x', sigfile, '-p', pubfile)
    return signature.read_text().strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--dry-run', action='store_true')
    mode.add_argument('--publish', action='store_true')
    parser.add_argument('--version', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--run-id', required=True, type=int)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r'0\.0\.[1-9][0-9]*', args.version):
        parser.error('Expected stage 1 version 0.0.N (N > 0)')
    if not re.fullmatch(r'[0-9a-f]{40}', args.commit):
        parser.error('--commit must be a full SHA')
    config = json.loads((ROOT / 'app/src-tauri/tauri.conf.json').read_text())
    public = config['plugins']['updater']['pubkey']
    if args.publish and public.startswith('PLACEHOLDER-'):
        raise SystemExit('Release blocked: updater key rotation freeze; placeholder public key')
    run = api(f'repos/{SOURCE}/actions/runs/{args.run_id}')
    assert run['head_sha'] == args.commit, 'CI run does not match requested commit'
    assert run['path'] == '.github/workflows/native-ci.yml', 'Wrong CI workflow'
    assert run['conclusion'] == 'success', 'CI must be complete and green'
    if args.publish:
        assert run['event'] == 'push' and run['head_branch'] == 'native', 'Publish only a native push build'
        comparison = api(f'repos/{SOURCE}/compare/{args.commit}...native')
        assert comparison['status'] in ('ahead', 'identical'), 'Source is not on native'
    checks = docker_checks()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='vz-artifacts-') as temporary:
        command('gh', 'run', 'download', args.run_id, '--repo', SOURCE,
                '--pattern', 'native-installers-*', '--dir', temporary)
        manifests = list(Path(temporary).rglob('build-*.json'))
        assert len(manifests) == 2, 'Expected both runner artifact manifests'
        targets = set()
        for path in manifests:
            build = json.loads(path.read_text())
            assert build['version'] == args.version and build['source_sha'] == args.commit, 'Artifact provenance/version mismatch'
            assert build['updater_signed'] is False
            assert build['updater_public_key'] == public, 'Artifact embedded public key differs from app configuration'
            targets.add(build['target'])
            for name, digest in build['files'].items():
                assert Path(name).name == name, 'Unsafe artifact filename'
                file = path.parent / name
                assert sha256(file) == digest, f'Artifact hash mismatch: {name}'
                assert not (output / name).exists(), f'Duplicate asset: {name}'
                shutil.copy2(file, output / name)
        assert targets == {'aarch64-apple-darwin', 'x86_64-pc-windows-msvc'}
    mac, = output.glob('*.app.tar.gz')
    windows, = output.glob('*.msi')
    dmg, = output.glob('*.dmg')
    platforms = {}
    tag = f'native-v{args.version}'
    with tempfile.TemporaryDirectory(prefix='vz-signing-') as signing:
        environment, verification_public = signing_environment(args.dry_run, public, Path(signing))
        for target, file in [('darwin-aarch64', mac), ('windows-x86_64', windows)]:
            signature = sign_and_verify(file, environment, verification_public, args.version)
            platforms[target] = {'signature': signature,
                                 'url': f'https://github.com/{DEST}/releases/download/{tag}/{file.name}'}
    del environment
    manifest = {'version': args.version, 'notes': f'Source: {SOURCE}@{args.commit}',
                'pub_date': datetime.now(timezone.utc).isoformat(), 'platforms': platforms}
    (output / 'latest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (output / 'publish-evidence.json').write_text(json.dumps({
        'run_url': run['html_url'], 'source_sha': args.commit, 'docker_checks': checks,
        'dry_run': args.dry_run, 'signatures_verified': True,
        'verification_public_key': verification_public,
        'signature_status': 'verified with throwaway key' if args.dry_run else 'verified with embedded key'}, indent=2) + '\n')
    assets = sorted(p for p in output.iterdir() if p.is_file())
    (output / 'SHA256SUMS').write_text(''.join(
        f'{sha256(p)}  {p.name}\n' for p in assets))
    assets.append(output / 'SHA256SUMS')
    print(f'{"Would upload" if args.dry_run else "Uploading"} full release {DEST}:{tag}')
    for file in assets:
        print(f'  {file.name}')
    if args.dry_run:
        print('Signed dry run: throwaway key signatures verified; no real key accessed; no GitHub writes.')
        return
    branch = api(f'repos/{DEST}')['default_branch']
    with tempfile.NamedTemporaryFile(mode='w', suffix='.md') as notes:
        notes.write(f'Stage 1 test build {args.version}\n\nSource commit: https://github.com/{SOURCE}/commit/{args.commit}\n\nCI: {run["html_url"]}\n')
        notes.flush()
        command('gh', 'release', 'create', tag, *assets, '--repo', DEST, '--target', branch,
                '--title', f'vectorAIz {args.version} test build', '--notes-file', notes.name, '--latest')
    assert api(f'repos/{DEST}/releases/latest')['tag_name'] == tag
    docker_checks()


if __name__ == '__main__':
    main()
