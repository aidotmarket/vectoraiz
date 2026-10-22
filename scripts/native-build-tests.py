#!/usr/bin/env python3
"""Negative acceptance tests: version drift and release freeze must fail closed."""
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('version', Path(__file__).with_name('native-version.py'))
version = importlib.util.module_from_spec(spec)
spec.loader.exec_module(version)
publish_spec = importlib.util.spec_from_file_location('publisher', Path(__file__).with_name('publish-test-build.py'))
publisher = importlib.util.module_from_spec(publish_spec)
publish_spec.loader.exec_module(publisher)


class VersionGate(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / 'app/src-tauri').mkdir(parents=True)
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.0.1"\n')
        self.files = ['app/package.json', 'app/src-tauri/tauri.conf.json', 'app/package-lock.json']
        for name in self.files:
            value = {'version': '0.0.1', 'packages': {'': {'version': '0.0.1'}}}
            if 'tauri.conf' in name:
                value['plugins'] = {'updater': {'pubkey': 'PLACEHOLDER-ROTATION-REQUIRED-DO-NOT-RELEASE'}}
            (self.root / name).write_text(json.dumps(value))

    def invoke(self, *args):
        with patch.object(version, 'ROOT', self.root), patch.object(sys, 'argv', ['native-version.py', *args]), contextlib.redirect_stdout(io.StringIO()):
            version.main()

    def test_agreeing_versions_pass(self):
        self.invoke()

    def test_each_derived_version_mismatch_fails(self):
        for name in self.files:
            with self.subTest(file=name):
                path = self.root / name
                original = path.read_text()
                value = json.loads(original)
                value['version'] = '0.0.2'
                path.write_text(json.dumps(value))
                with self.assertRaises(SystemExit):
                    self.invoke()
                path.write_text(original)

    def test_write_derives_versions_and_lockfile(self):
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.0.7"\n')
        self.invoke('--write')
        self.invoke()
        for name in self.files:
            self.assertEqual(json.loads((self.root / name).read_text())['version'], '0.0.7')

    def test_placeholder_cannot_release(self):
        with self.assertRaisesRegex(SystemExit, 'placeholder updater public key'):
            self.invoke('--release')


class PublisherGate(unittest.TestCase):
    def invoke(self, *args):
        with patch.object(sys, 'argv', ['publish-test-build.py', *args]), contextlib.redirect_stdout(io.StringIO()):
            publisher.main()

    def test_signing_key_mismatch_refuses_publish(self):
        import base64
        embedded = base64.b64encode(b'untrusted comment: test\n' + base64.b64encode(b'A' * 42) + b'\n').decode()
        secret = base64.b64encode(b'fixture secret').decode()
        def recreate(*args, **kwargs):
            Path(args[args.index('-p') + 1]).write_bytes(b'untrusted comment: test\n' + base64.b64encode(b'B' * 42) + b'\n')
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(publisher, 'rotated_secrets', return_value=(secret, '')), \
                patch.object(publisher, 'command', side_effect=recreate):
            with self.assertRaisesRegex(SystemExit, 'does not match Cloud signing key'):
                publisher.signing_environment(False, embedded, Path(temporary))

    def test_dry_run_assembles_artifacts_with_verified_throwaway_signatures_and_no_github_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'out'

            def api(path):
                self.assertEqual(path, f'repos/{publisher.SOURCE}/actions/runs/42')
                return {'head_sha': 'a' * 40, 'path': '.github/workflows/native-ci.yml',
                        'conclusion': 'success', 'html_url': 'https://example.test/run/42'}

            def download(*args, **kwargs):
                self.assertEqual(args[:3], ('gh', 'run', 'download'))
                folder = Path(args[-1])
                for target, names in [
                    ('aarch64-apple-darwin', ['test.app.tar.gz', 'test.dmg', 'mac-notices.txt']),
                    ('x86_64-pc-windows-msvc', ['test.msi', 'windows-notices.txt']),
                ]:
                    dest = folder / target
                    dest.mkdir()
                    hashes = {}
                    for name in names:
                        payload = f'fixture:{name}'.encode()
                        (dest / name).write_bytes(payload)
                        hashes[name] = hashlib.sha256(payload).hexdigest()
                    (dest / f'build-{target}.json').write_text(json.dumps({
                        'source_sha': 'a' * 40, 'version': '0.0.1', 'target': target,
                        'updater_signed': False, 'updater_public_key': json.loads((publisher.ROOT / 'app/src-tauri/tauri.conf.json').read_text())['plugins']['updater']['pubkey'], 'files': hashes}))

            with patch.object(publisher, 'api', side_effect=api), \
                    patch.object(publisher, 'docker_checks', return_value={'latest': 'v1.2.3'}), \
                    patch.object(publisher, 'command', side_effect=download) as command, \
                    patch.object(publisher, 'signing_environment', return_value=({}, 'throwaway-public')) as signing, \
                    patch.object(publisher, 'sign_and_verify', side_effect=lambda f, *args: (f.with_name(f.name + '.sig').write_text('verified-fixture'), 'verified-fixture')[1]), \
                    patch.object(publisher, 'rotated_secrets') as secrets:
                self.invoke('--dry-run', '--version', '0.0.1', '--commit', 'a' * 40,
                            '--run-id', '42', '--output', str(output))
                self.assertEqual(command.call_count, 1)
                self.assertTrue(signing.call_args.args[0])
                secrets.assert_not_called()
            manifest = json.loads((output / 'latest.json').read_text())
            self.assertEqual(manifest['version'], '0.0.1')
            for value in manifest['platforms'].values():
                self.assertEqual(value['signature'], 'verified-fixture')
            self.assertTrue(json.loads((output / 'publish-evidence.json').read_text())['signatures_verified'])
            for line in (output / 'SHA256SUMS').read_text().splitlines():
                digest, name = line.split('  ', 1)
                self.assertEqual(hashlib.sha256((output / name).read_bytes()).hexdigest(), digest)


class PackageGate(unittest.TestCase):
    def test_prepare_drops_cached_models_and_stale_resources(self):
        module_spec = importlib.util.spec_from_file_location('package', Path(__file__).with_name('native-package.py'))
        package = importlib.util.module_from_spec(module_spec)
        module_spec.loader.exec_module(package)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ['.assets/pdfium/bin/pdfium.dll', '.assets/huge.gguf',
                         'app/src-tauri/gen/bundle/assets/stale.gguf',
                         'spike/fixtures/models.json', 'THIRD_PARTY_NOTICES',
                         'target/debug/vz-parser-worker.exe', 'target/debug/vectoraiz-mcp.exe',
                         'target/debug/vectoraiz-cli.exe']:
                file = root / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(b'fixture')
            (root / 'Cargo.toml').write_text('[workspace.package]\nversion="0.0.1"\n')
            (root / 'app/src-tauri/tauri.stage1.conf.json').write_text('{"bundle":{}}')
            with patch.object(package, 'ROOT', root), patch.object(package.platform, 'system', return_value='Windows'), \
                    patch.object(sys, 'argv', ['native-package.py', 'prepare']):
                package.main()
            bundle = root / 'app/src-tauri/gen/bundle'
            self.assertEqual(list(bundle.rglob('*.gguf')), [])
            self.assertTrue((bundle / 'assets/pdfium/bin/pdfium.dll').is_file())

    def test_collect_replaces_cached_installers_and_provenance(self):
        module_spec = importlib.util.spec_from_file_location('package', Path(__file__).with_name('native-package.py'))
        package = importlib.util.module_from_spec(module_spec)
        module_spec.loader.exec_module(package)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ['target/stage1/old.msi', 'target/stage1/build-stale.json',
                         'target/debug/bundle/msi/current.msi', 'THIRD_PARTY_NOTICES']:
                file = root / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(b'fixture')
            (root / 'Cargo.toml').write_text('[workspace.package]\nversion="0.0.1"\n')
            config = root / 'app/src-tauri/tauri.conf.json'
            config.parent.mkdir(parents=True)
            config.write_text('{"plugins":{"updater":{"pubkey":"public-fixture"}}}')
            with patch.object(package, 'ROOT', root), patch.object(package.platform, 'system', return_value='Windows'), \
                    patch.dict(__import__('os').environ, {'SOURCE_SHA': 'a' * 40}), \
                    patch.object(sys, 'argv', ['native-package.py', 'collect']):
                package.main()
                package.main()
            output = root / 'target/stage1'
            self.assertFalse((output / 'old.msi').exists())
            self.assertFalse((output / 'build-stale.json').exists())
            manifest = json.loads((output / 'build-x86_64-pc-windows-msvc.json').read_text())
            self.assertEqual(set(manifest['files']), {'vectorAIz_0.0.1_x64.msi', 'THIRD_PARTY_NOTICES-x86_64-pc-windows-msvc.txt'})


if __name__ == '__main__':
    unittest.main()
