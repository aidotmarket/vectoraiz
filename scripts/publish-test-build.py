#!/usr/bin/env python3
"""Assemble CI artifacts; publishing stays blocked until the replacement key is configured."""
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
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
SOURCE = 'aidotmarket/vectoraiz'
DEST = 'aidotmarket/vectoraiz-test-builds'
PLACEHOLDER = 'UNSIGNED-DRY-RUN-NO-KEY-ACCESSED'


def command(*args, **kwargs):
    return subprocess.run(list(map(str, args)), check=True, **kwargs)


def api(path):
    return json.loads(subprocess.check_output(['gh', 'api', path], text=True))


def docker_checks():
    latest = api(f'repos/{SOURCE}/releases/latest')
    assert latest['tag_name'].startswith('v'), 'Docker latest is no longer a v* release'
    releases = api(f'repos/{SOURCE}/releases?per_page=100')
    # Match the first page consumed by the existing /rc Worker.
    first = next((r for r in releases if r['prerelease'] and not r['draft']), None)
    assert first is None or first['tag_name'].startswith('v'), 'Docker /rc prerelease tag changed'
    return {'latest': latest['tag_name'], 'first_prerelease': first['tag_name'] if first else None}


def rotated_secrets(key_name, password_name):
    # This path is unreachable in dry-run and while the placeholder is configured.
    # Names must come from the post-incident rotation dispatch, never the exposed pair.
    old = {'VECTORAIZ_TAURI_UPDATER_PRIVATE_KEY', 'VECTORAIZ_TAURI_UPDATER_KEY_PASSWORD'}
    if not key_name or not password_name or {key_name, password_name} & old:
        raise SystemExit('Supply replacement secret names from the rotation dispatch')
    token = (Path.home() / '.config/infisical/sysadmin-token').read_text().strip()
    values = []
    for name in [key_name, password_name]:
        query = urllib.parse.urlencode({'workspaceId': '0943f641-faee-4324-b337-0d50c276e4a9',
                                       'environment': 'prod', 'secretPath': '/'})
        request = urllib.request.Request(
            f'https://secrets.ai.market/api/v3/secrets/raw/{urllib.parse.quote(name, safe="")}?{query}',
            headers={'Authorization': f'Bearer {token}', 'User-Agent': 'vectoraiz-test-build-publisher/1'})
        with urllib.request.urlopen(request, timeout=30) as response:
            values.append(json.load(response)['secret']['secretValue'])
    assert values[0], 'Replacement key is empty'
    return values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--dry-run', action='store_true')
    mode.add_argument('--publish', action='store_true')
    parser.add_argument('--version', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--run-id', required=True, type=int)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--key-secret-name')
    parser.add_argument('--password-secret-name')
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
            targets.add(build['target'])
            for name, digest in build['files'].items():
                assert Path(name).name == name, 'Unsafe artifact filename'
                file = path.parent / name
                assert hashlib.file_digest(file.open('rb'), 'sha256').hexdigest() == digest, f'Artifact hash mismatch: {name}'
                assert not (output / name).exists(), f'Duplicate asset: {name}'
                shutil.copy2(file, output / name)
        assert targets == {'aarch64-apple-darwin', 'x86_64-pc-windows-msvc'}
    mac, = output.glob('*.app.tar.gz')
    windows, = output.glob('*.msi')
    dmg, = output.glob('*.dmg')
    platforms = {}
    tag = f'native-v{args.version}'
    environment = None
    if args.publish:
        key, password = rotated_secrets(args.key_secret_name, args.password_secret_name)
        environment = dict(os.environ, TAURI_SIGNING_PRIVATE_KEY=key, TAURI_SIGNING_PRIVATE_KEY_PASSWORD=password)
    for target, file in [('darwin-aarch64', mac), ('windows-x86_64', windows)]:
        signature = file.with_name(file.name + '.sig')
        if args.dry_run:
            signature.write_text(PLACEHOLDER + '\n')
        else:
            # Tauri prints signing-key hints; suppress all signer output, including on failure.
            command('node', ROOT / 'app/node_modules/.bin/tauri', 'signer', 'sign',
                    '--app-version', args.version, file, env=environment,
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            # Tauri signatures/public keys are base64-encoded minisign text.
            with tempfile.TemporaryDirectory(prefix='vz-verify-') as verification:
                sigfile = Path(verification) / 'artifact.sig'
                pubfile = Path(verification) / 'updater.pub'
                sigfile.write_bytes(base64.b64decode(signature.read_text().strip(), validate=True))
                pubfile.write_bytes(base64.b64decode(public, validate=True))
                command('minisign', '-Vm', file, '-x', sigfile, '-p', pubfile)
        platforms[target] = {'signature': signature.read_text().strip(),
                             'url': f'https://github.com/{DEST}/releases/download/{tag}/{file.name}'}
    manifest = {'version': args.version, 'notes': f'Source: {SOURCE}@{args.commit}',
                'pub_date': datetime.now(timezone.utc).isoformat(), 'platforms': platforms}
    (output / 'latest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (output / 'publish-evidence.json').write_text(json.dumps({
        'run_url': run['html_url'], 'source_sha': args.commit, 'docker_checks': checks,
        'dry_run': args.dry_run, 'signatures_verified': not args.dry_run,
        'signature_status': PLACEHOLDER if args.dry_run else 'verified'}, indent=2) + '\n')
    assets = sorted(p for p in output.iterdir() if p.is_file())
    (output / 'SHA256SUMS').write_text(''.join(
        f'{hashlib.file_digest(p.open("rb"), "sha256").hexdigest()}  {p.name}\n' for p in assets))
    assets.append(output / 'SHA256SUMS')
    print(f'{"Would upload" if args.dry_run else "Uploading"} full release {DEST}:{tag}')
    for file in assets:
        print(f'  {file.name}')
    if args.dry_run:
        print('UNSIGNED dry run: no signing key accessed; signature verification NOT performed; no GitHub writes.')
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
