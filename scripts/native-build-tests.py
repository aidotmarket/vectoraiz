#!/usr/bin/env python3
"""Negative acceptance tests: version drift and release freeze must fail closed."""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
from pathlib import Path
import sys
import tempfile
import tomllib
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
        self.workspace_manifest = '[workspace]\nmembers = ["crates/*", "app/src-tauri"]\n[workspace.package]\nversion = "0.0.1"\n'
        (self.root / 'Cargo.toml').write_text(self.workspace_manifest)
        (self.root / 'crates/example').mkdir(parents=True)
        for member, name in [('crates/example', 'example'), ('app/src-tauri', 'app')]:
            (self.root / member / 'Cargo.toml').write_text(f'[package]\nname = "{name}"\nversion.workspace = true\n')
        self.lockfile = ('# Preserve lockfile formatting and registry versions.\nversion = 4\n'
                         '[[package]]\nname = "example"\nversion = "0.0.1"\n'
                         '[[package]]\nname = "app"\nversion = "0.0.1"\n'
                         '[[package]]\nname = "example"\nversion = "0.0.1"\nsource = "registry+https://example.test"\n')
        (self.root / 'Cargo.lock').write_text(self.lockfile)
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
        (self.root / 'Cargo.toml').write_text(self.workspace_manifest.replace('0.0.1', '0.0.7'))
        self.invoke('--write')
        self.invoke()
        for name in self.files:
            self.assertEqual(json.loads((self.root / name).read_text())['version'], '0.0.7')

        packages = tomllib.loads((self.root / 'Cargo.lock').read_text())['package']
        self.assertEqual([p['version'] for p in packages], ['0.0.7', '0.0.7', '0.0.1'])

    def test_cargo_lock_workspace_mismatch_fails_and_write_repairs_only_members(self):
        path = self.root / 'Cargo.lock'
        for name in ['example', 'app']:
            with self.subTest(member=name):
                drift = self.lockfile.replace(f'name = "{name}"\nversion = "0.0.1"',
                                              f'name = "{name}"\nversion = "0.0.2"', 1)
                path.write_text(drift)
                with self.assertRaisesRegex(SystemExit, f'Cargo.lock: {name} version differs'):
                    self.invoke()
                self.invoke('--write')
                self.assertEqual(path.read_text(), self.lockfile)
                self.invoke()

    def test_cargo_lock_missing_member_fails(self):
        (self.root / 'Cargo.lock').write_text(self.lockfile.replace(
            '[[package]]\nname = "app"\nversion = "0.0.1"\n', ''))
        for args in [(), ('--write',)]:
            with self.assertRaisesRegex(SystemExit, 'missing workspace members: app'):
                self.invoke(*args)

    def test_placeholder_cannot_release(self):
        with self.assertRaisesRegex(SystemExit, 'placeholder updater public key'):
            self.invoke('--release')


class UiTestShippingGate(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('ui_gate', Path(__file__).with_name('native-ui-test-gate.py'))
        self.gate = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.gate)
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        # Use unchanged repository inputs rather than an invented safe config.
        for name in ['app/src-tauri/Cargo.toml', 'app/src-tauri/tauri.conf.json',
                     'app/src-tauri/tauri.stage1.conf.json', 'app/src-tauri/capabilities',
                     '.github/workflows']:
            source = self.gate.ROOT / name
            dest = self.root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            if source.is_dir():
                shutil.copytree(source, dest)
            else:
                shutil.copy2(source, dest)

    def invoke(self):
        # Dependency resolution is exercised by the real CI gate; these tests
        # isolate feature discovery and rejection without invoking Cargo.
        with patch.object(self.gate, 'ROOT', self.root), \
                patch.object(self.gate.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout='')) as cargo, \
                contextlib.redirect_stdout(io.StringIO()):
            self.gate.main()
        return cargo

    def add_features(self, name, features):
        path = self.root / 'app/src-tauri' / name
        path.parent.mkdir(parents=True, exist_ok=True)
        config = json.loads(path.read_text()) if path.exists() else {}
        config.setdefault('build', {})['features'] = features
        path.write_text(json.dumps(config))

    def test_unchanged_shipping_inputs_pass(self):
        self.assertGreater(self.invoke().call_count, 0)

    def test_base_config_ui_test_fails(self):
        self.add_features('tauri.conf.json', ['ui-test'])
        with self.assertRaisesRegex(AssertionError, 'Shipping config enables ui-test'):
            self.invoke()

    def test_stage1_config_ui_test_fails(self):
        self.add_features('tauri.stage1.conf.json', ['ui-test'])
        with self.assertRaisesRegex(AssertionError, 'Shipping config enables ui-test'):
            self.invoke()

    def test_generated_overlay_ui_test_fails(self):
        self.add_features('gen/stage1.conf.json', ['ui-test'])
        with self.assertRaisesRegex(AssertionError, 'Shipping config enables ui-test'):
            self.invoke()

    def test_workflow_ui_test_fails(self):
        path = self.root / '.github/workflows/native-ci.yml'
        original = path.read_text()
        mutant = original.replace('--features smoke-test,custom-protocol',
                                  '--features smoke-test,custom-protocol,ui-test')
        self.assertNotEqual(original, mutant)
        path.write_text(mutant)
        with self.assertRaisesRegex(AssertionError, 'Shipping workflow enables ui-test'):
            self.invoke()

    def test_effective_features_include_cli_base_stage1_and_generated(self):
        self.add_features('tauri.conf.json', ['base-feature'])
        self.add_features('tauri.stage1.conf.json', ['stage1-feature'])
        self.add_features('gen/stage1.conf.json', ['generated-feature'])
        cargo = self.invoke()
        resolved = [set(call.args[0][call.args[0].index('--features') + 1].split(','))
                    for call in cargo.call_args_list]
        for features in resolved:
            self.assertTrue({'base-feature', 'stage1-feature', 'generated-feature'} <= features)
        self.assertTrue(any({'smoke-test', 'custom-protocol'} <= features for features in resolved))
        self.assertFalse(any('vectoraiz/smoke-test' in features for features in resolved))


class ProtocInstaller(unittest.TestCase):
    def installer(self):
        spec = importlib.util.spec_from_file_location('protoc', Path(__file__).with_name('install-protoc.py'))
        protoc = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(protoc)
        return protoc

    def test_download_retry_is_bounded_and_preserves_failure(self):
        protoc = self.installer()
        failure = OSError('download fixture failure')
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(protoc.urllib.request, 'urlopen', side_effect=failure) as request, \
                patch.object(protoc.time, 'sleep') as sleep:
            with self.assertRaisesRegex(OSError, 'download fixture failure'):
                protoc.download('https://example.test/archive.zip', Path(temporary) / 'archive.zip')
            self.assertEqual(request.call_count, 4)
            self.assertEqual([call.args[0] for call in sleep.call_args_list], [2, 4, 8])

    def test_checksum_mismatch_stops_before_extraction(self):
        protoc = self.installer()
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(protoc, 'download', side_effect=lambda url, path: path.write_bytes(b'corrupt archive')), \
                patch.object(protoc.zipfile, 'ZipFile') as unzip:
            with self.assertRaisesRegex(SystemExit, 'SHA-256 mismatch'):
                protoc.install(Path(temporary), 'Windows', 'AMD64')
            unzip.assert_not_called()

    def test_unsupported_platform_fails_before_download(self):
        protoc = self.installer()
        with patch.object(protoc, 'download') as download:
            with self.assertRaisesRegex(SystemExit, 'Unsupported protoc platform'):
                protoc.install(Path('unused'), 'Windows', 'ARM64')
            download.assert_not_called()


class PublisherGate(unittest.TestCase):
    def invoke(self, *args):
        with patch.object(sys, 'argv', ['publish-test-build.py', *args]), contextlib.redirect_stdout(io.StringIO()):
            publisher.main()

    def test_signing_key_mismatch_refuses_publish(self):
        import base64
        embedded = base64.b64encode(b'untrusted comment: test\n' + base64.b64encode(b'A' * 42) + b'\n').decode()
        secret = base64.b64encode(b'fixture secret').decode()
        def recreate(*args, **kwargs):
            if os.name != 'nt':
                self.assertEqual(Path(args[args.index('-s') + 1]).stat().st_mode & 0o777, 0o600)
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


class NoticeGenerators(unittest.TestCase):
    def test_generator_failure_stops_before_combining_and_keeps_prior_output(self):
        spec = importlib.util.spec_from_file_location('notices', Path(__file__).with_name('native-generate-notices.py'))
        notices = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(notices)
        for failing in range(3):
            with self.subTest(generator=failing), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                output = root / 'THIRD_PARTY_NOTICES'
                output.write_text('prior output')
                calls = []

                def generate(argv, **kwargs):
                    calls.append(argv)
                    if len(calls) - 1 == failing:
                        raise subprocess.CalledProcessError(1, argv)
                    if '-o' in argv:
                        Path(argv[argv.index('-o') + 1]).write_text('Rust licence')
                    else:
                        kwargs['stdout'].write(b'licence')

                with patch.object(notices, 'ROOT', root), \
                        patch.object(notices.subprocess, 'run', side_effect=generate), \
                        self.assertRaises(subprocess.CalledProcessError):
                    notices.main()
                self.assertEqual(len(calls), failing + 1)
                self.assertEqual(output.read_text(), 'prior output')


class HelperDiagnostics(unittest.TestCase):
    def test_failed_helper_retains_private_stderr_without_printing_values(self):
        def fail(*args, **kwargs):
            kwargs['stderr'].write(b'private diagnostic fixture')
            raise subprocess.CalledProcessError(9, args, output=b'secret stdout fixture')
        with patch.object(publisher, 'command', side_effect=fail):
            with self.assertRaises(SystemExit) as caught:
                publisher.rotated_secrets()
        message = str(caught.exception)
        self.assertNotIn('fixture', message)
        path = Path(message.split('private stderr: ', 1)[1])
        self.addCleanup(path.unlink)
        self.assertEqual(path.read_bytes(), b'private diagnostic fixture')
        if os.name != 'nt':
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)


class PackageGate(unittest.TestCase):
    notices = ('THIRD_PARTY_NOTICES — Rust dependencies\nRust licence\n'
               'llama.cpp revision\nNative licence\n'
               'PDFium chromium/8111:\nPDFium licence\n'
               'Provisional model licences and source notices:\nModel licence\n'
               'UI npm dependencies\nnpm licence\n')

    def package_module(self):
        spec = importlib.util.spec_from_file_location('package', Path(__file__).with_name('native-package.py'))
        package = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(package)
        return package

    def test_prepare_refuses_missing_or_empty_notice_sections(self):
        package = self.package_module()
        headers = ['THIRD_PARTY_NOTICES — Rust dependencies', 'llama.cpp ',
                   'PDFium chromium/8111:', 'Provisional model licences and source notices:',
                   'UI npm dependencies']
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'Cargo.toml').write_text('[workspace.package]\nversion="0.0.1"\n')
            notices = root / 'THIRD_PARTY_NOTICES'
            notices.write_text(self.notices, encoding='utf-8')
            package.validate_notices(notices)
            for header in headers:
                with self.subTest(missing=header):
                    notices.write_text(self.notices.replace(header, ''), encoding='utf-8')
                    with patch.object(package, 'ROOT', root), \
                            patch.object(sys, 'argv', ['native-package.py', 'prepare']), \
                            self.assertRaisesRegex(SystemExit, 'Incomplete third-party notices'):
                        package.main()
            for body in ['Rust licence', 'PDFium licence', 'Model licence', 'npm licence']:
                with self.subTest(empty=body):
                    notices.write_text(self.notices.replace(body, ''), encoding='utf-8')
                    with self.assertRaisesRegex(SystemExit, 'empty'):
                        package.validate_notices(notices)

    def test_dmg_retry_is_bounded_and_preserves_failure(self):
        package = self.package_module()
        failure = subprocess.CalledProcessError(1, ['hdiutil', 'create'])
        with patch.object(package, 'run', side_effect=[failure, None]) as run, \
                patch.object(package.time, 'sleep') as sleep:
            package.create_dmg('stage', 'image.dmg')
            self.assertEqual(run.call_count, 2)
            sleep.assert_called_once_with(2)
        with patch.object(package, 'run', side_effect=failure) as run, \
                patch.object(package.time, 'sleep') as sleep:
            with self.assertRaises(subprocess.CalledProcessError):
                package.create_dmg('stage', 'image.dmg')
            self.assertEqual(run.call_count, 4)
            self.assertEqual(sleep.call_count, 3)

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
            (root / 'THIRD_PARTY_NOTICES').write_text(self.notices, encoding='utf-8')
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


def load_tests(loader, tests, pattern):
    # Keep the bake-off's offline scoring/download tests in the existing Python gate.
    tests.addTests(loader.discover(str(Path(__file__).resolve().parents[1] / 'spike/bakeoff'), pattern='test_*.py'))
    return tests


if __name__ == '__main__':
    unittest.main()
