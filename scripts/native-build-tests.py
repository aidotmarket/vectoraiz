#!/usr/bin/env python3
"""Negative acceptance tests: version drift and release freeze must fail closed."""
import contextlib
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


if __name__ == '__main__':
    unittest.main()
