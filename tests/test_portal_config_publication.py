"""Owned, offline store/router regression harness; run directly, outside app conftest.

rtk proxy python -B -s tests/test_portal_config_publication.py
PORTAL_SOURCE_ROOT can point to an isolated source export for baseline comparison.
No app lifespan, real JWT/crypto/provider, database or search engine is imported.
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import types
import unittest
from unittest.mock import patch

if __name__ != '__main__':
    raise unittest.SkipTest("Run this isolated harness directly; it replaces product dependencies")

# Install boundaries before any product import. All persistent writes are owned.
OWNED = Path(tempfile.mkdtemp(prefix="r306-publication-")).resolve()
SOURCE = Path(os.environ.get("PORTAL_SOURCE_ROOT", Path(__file__).resolve().parents[1])).resolve()
sys.dont_write_bytecode = True


def guard(event, args):
    if event.startswith(("socket.connect", "socket.bind", "socket.getaddrinfo", "socket.sendto", "socket.sendmsg")):
        raise AssertionError("network denied")
    if event in ("subprocess.Popen", "os.system"):
        raise AssertionError("subprocess denied")
    if event == "open" and isinstance(args[0], (str, bytes, os.PathLike)):
        path = Path(os.fsdecode(args[0])).resolve()
        mode, flags = args[1:3]
        writing = (isinstance(mode, str) and any(c in mode for c in "wax+")) or (isinstance(flags, int) and flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC))
        if writing and not path.is_relative_to(OWNED):
            raise AssertionError(f"non-owned write denied: {path}")
        if path.is_relative_to(Path.home()) or path.is_relative_to(Path('/data')):
            raise AssertionError(f"credential/primary root denied: {path}")
    if event in ("os.remove", "os.mkdir", "os.chmod", "os.rename"):
        paths = args[:2] if event == "os.rename" else args[:1]
        for value in paths:
            if isinstance(value, (str, bytes, os.PathLike)) and not Path(os.fsdecode(value)).resolve().is_relative_to(OWNED):
                raise AssertionError(f"non-owned mutation denied: {value}")


# Dependencies live in a preserved read-only venv under /var/tmp.
sys.addaudithook(guard)
from fastapi import HTTPException


def stub(name, **attrs):
    module = types.ModuleType(name)
    vars(module).update(attrs)
    sys.modules[name] = module
    return module


for package in ('app', 'app.models', 'app.schemas', 'app.routers', 'app.middleware', 'app.services', 'app.core'):
    stub(package, __path__=[])


def load(name, relative):
    spec = importlib.util.spec_from_file_location(name, SOURCE / relative)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def forbidden(*args, **kwargs):
    raise AssertionError('real dependency forbidden')


stub('bcrypt', hashpw=forbidden, gensalt=forbidden, checkpw=forbidden)
stub('app.config', settings=types.SimpleNamespace(data_directory=str(OWNED)))
schema = load('app.schemas.portal', 'app/schemas/portal.py')
store = load('app.models.portal', 'app/models/portal.py')


stub('app.middleware.portal_auth', get_portal_session=forbidden,
     create_portal_jwt=forbidden, check_dataset_acl=forbidden)
stub('app.core.async_utils', run_sync=forbidden)
stub('app.services.portal_service', get_portal_service=forbidden)
stub('app.services.processing_service', get_processing_service=lambda: types.SimpleNamespace(get_dataset=lambda _: object()))
OIDC_NAMES = ('fetch_oidc_discovery build_authorize_url validate_state exchange_code_for_tokens validate_id_token '
              'store_refresh_token store_id_token get_id_token clear_refresh_token log_portal_access get_access_logs '
              'get_end_session_url encrypt_client_secret clear_sessions_only').split()
stub('app.services.portal_oidc', **dict.fromkeys(OIDC_NAMES, forbidden), is_sso_enabled=lambda: True)
route = load('app.routers.portal', 'app/routers/portal.py')
stub('jwt', encode=forbidden, decode=forbidden)
acl = load('app.middleware.portal_auth', 'app/middleware/portal_auth.py')
stub('app.services.search_service', get_search_service=forbidden)
service = load('app.services.portal_service', 'app/services/portal_service.py')


class PublicationTests(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(dir=OWNED))
        store._PORTAL_CONFIG_PATH = self.directory / 'portal_config.json'
        store.reset_portal_config_cache()
        store.AccessCodeValidator.clear_rate_limits()
        config = schema.PortalConfig(enabled=True, tier='code', base_url='https://fixture.invalid',
            access_code_hash='owned-sentinel', portal_session_version=4,
            active_sessions={'existing': '2099-01-01T00:00:00+00:00'},
            datasets={'d': schema.DatasetPortalConfig(portal_visible=True, display_columns=['public'])})
        store.save_portal_config(config)
        self.before = config.model_dump()
        self.file_before = store._PORTAL_CONFIG_PATH.read_bytes()

    def run_async(self, coroutine):
        return asyncio.run(coroutine)

    def unchanged(self):
        self.assertEqual(store.get_portal_config().model_dump(), self.before)
        self.assertEqual(store._PORTAL_CONFIG_PATH.read_bytes(), self.file_before)
        self.assertEqual(list(self.directory.iterdir()), [store._PORTAL_CONFIG_PATH])
        self.assertEqual(json.loads(self.file_before), json.loads(store._PORTAL_CONFIG_PATH.read_text()))

    def test_rejected_admin_updates_preserve_all_state(self):
        for fields in ({'tier': 'open', 'access_code': '123456'}, {'tier': 'sso'},
                       {'base_url': '', 'enabled': True}, {'access_code': '', 'enabled': True},
                       {'access_code': 'NewCode123', 'base_url': '', 'enabled': True}):
            with self.subTest(fields=fields):
                store.save_portal_config(schema.PortalConfig(**self.before))
                with patch.object(store.AccessCodeValidator, 'hash_code', return_value='owned-new-hash'), self.assertRaises(HTTPException) as exc:
                    self.run_async(route.update_portal_settings(schema.PortalConfigUpdate(**fields)))
                self.assertEqual(exc.exception.status_code, 400)
                self.unchanged()

    def test_nested_get_and_saved_input_are_detached(self):
        for config in (store.get_portal_config(), schema.PortalConfig(**self.before)):
            store.save_portal_config(config)
            config.datasets['d'].display_columns.append('private')
            config.active_sessions.clear()
            config.enabled = False
            self.unchanged()

    def test_successful_save_reload_and_private_permissions(self):
        config = store.get_portal_config()
        config.datasets['d'].portal_visible = False
        config.active_sessions['other'] = 'fixture-expiry'
        store.save_portal_config(config)
        expected = config.model_dump()
        self.assertNotIn('_store_generation', expected)
        self.assertNotIn('_store_generation', schema.PortalConfig.model_json_schema()['properties'])
        self.assertEqual(store._PORTAL_CONFIG_PATH.stat().st_mode & 0o777, 0o600)
        store.reset_portal_config_cache()
        self.assertEqual(store.get_portal_config().model_dump(), expected)
        store._PORTAL_CONFIG_PATH.chmod(0o400)
        store.save_portal_config(store.get_portal_config())
        self.assertEqual(store._PORTAL_CONFIG_PATH.stat().st_mode & 0o777, 0o400)

    def test_write_flush_fsync_replace_failures(self):
        # File wrapper injects partial write and flush errors at actual I/O boundaries.
        original_fdopen = os.fdopen
        class BrokenStream:
            def __init__(self, fd, mode, stage):
                self.stream, self.stage = original_fdopen(fd, mode), stage
            def __enter__(self): return self
            def __exit__(self, *args): self.stream.close()
            def fileno(self): return self.stream.fileno()
            def write(self, value):
                if self.stage == 'write':
                    self.stream.write(value[:20])
                    raise OSError('owned partial write failure')
                return self.stream.write(value)
            def flush(self):
                if self.stage == 'flush': raise OSError('owned flush failure')
                self.stream.flush()
        for stage in ('write', 'flush', 'fsync', 'replace', 'fchmod'):
            with self.subTest(stage=stage):
                store.save_portal_config(schema.PortalConfig(**self.before))
                if stage in ('write', 'flush'):
                    target = patch.object(os, 'fdopen', side_effect=lambda fd, mode: BrokenStream(fd, mode, stage))
                else:
                    target = patch.object(os, stage, side_effect=OSError('owned '+stage+' failure'))
                with target, patch.object(store.AccessCodeValidator, 'hash_code', return_value='owned-new-hash'), self.assertRaises(OSError):
                    self.run_async(route.update_portal_settings(schema.PortalConfigUpdate(tier='open', access_code='NewCode123')))
                self.unchanged()

    def test_cache_published_only_after_file_replace(self):
        replace = os.replace
        def inspect_then_replace(source, destination):
            self.unchanged_except_temporary()
            replace(source, destination)
        with patch.object(os, 'replace', side_effect=inspect_then_replace) as replacement:
            self.run_async(route.update_portal_settings(schema.PortalConfigUpdate(tier='open')))
        replacement.assert_called_once()
        self.assertEqual(store.get_portal_config().tier, schema.PortalTier.open)
        self.assertEqual(json.loads(store._PORTAL_CONFIG_PATH.read_text())['tier'], 'open')

    def unchanged_except_temporary(self):
        self.assertEqual(store.get_portal_config().model_dump(), self.before)
        self.assertEqual(store._PORTAL_CONFIG_PATH.read_bytes(), self.file_before)

    def test_real_writer_partial_file_failure(self):
        # Also fails against the original direct Path.write_text implementation.
        original = Path.write_text
        def partial_then_fail(path, data, *args, **kwargs):
            original(path, data[:12], *args, **kwargs)
            raise OSError('owned direct-write failure')
        with patch.object(Path, 'write_text', partial_then_fail), patch.object(os, 'replace', side_effect=OSError('owned replacement failure')):
            with self.assertRaises(OSError):
                self.run_async(route.update_portal_settings(schema.PortalConfigUpdate(tier='open')))
        self.assertEqual(store._PORTAL_CONFIG_PATH.read_bytes(), self.file_before)
        self.unchanged()

    def test_stale_snapshot_cannot_restore_revoked_session(self):
        stale = store.get_portal_config()
        with patch.object(route, 'clear_refresh_token', return_value=None):
            self.run_async(route.revoke_portal_session('existing'))
        committed = store.get_portal_config().model_dump()
        stale.base_url = 'https://stale.invalid'
        with self.assertRaises(RuntimeError): store.save_portal_config(stale)
        self.assertEqual(store.get_portal_config().model_dump(), committed)

    def test_rotation_revoke_and_visibility(self):
        with patch.object(store.AccessCodeValidator, 'hash_code', return_value='owned-new-hash'):
            result = self.run_async(route.update_portal_settings(schema.PortalConfigUpdate(access_code='NewCode123')))
        self.assertEqual(result['portal_session_version'], 5)
        self.assertEqual(result['active_session_count'], 0)
        self.assertEqual(result['access_code_hash'], '***')
        body = schema.DatasetPortalConfig(portal_visible=False, display_columns=['new'])
        self.run_async(route.update_dataset_portal_config('d', body))
        body.portal_visible = True
        self.assertFalse(store.get_portal_config().datasets['d'].portal_visible)
        self.run_async(route.revoke_all_portal_sessions())
        self.assertEqual(store.get_portal_config().portal_session_version, 6)
        with patch.dict(sys.modules['app.services.portal_oidc'].__dict__, clear_sessions_only=lambda: None):
            self.run_async(route.revoke_all_portal_sessions_v2())
        self.assertEqual(store.get_portal_config().portal_session_version, 7)

    def test_acl_visible_reads_and_column_filtering(self):
        record = types.SimpleNamespace(metadata_json='{"row_count": 2}', original_filename='owned')
        processing = types.SimpleNamespace(get_dataset=lambda _: record)
        search = types.SimpleNamespace(search=lambda **kwargs: {'total': 1, 'results': [{'row_data': {'public': 1, 'private': 2}}]})
        portal = service.PortalService()
        acl.check_dataset_acl('d')
        with patch.object(service, 'get_processing_service', return_value=processing), patch.object(service, 'get_search_service', return_value=search):
            self.assertEqual([d.dataset_id for d in portal.get_visible_datasets()], ['d'])
            self.assertEqual(portal.search_dataset('d', 'owned').results[0]['row_data'], {'public': 1})
            detached = portal.get_dataset_portal_config('d')
            detached.portal_visible = False
            self.assertTrue(portal.is_dataset_visible('d'))
            self.run_async(route.update_dataset_portal_config('d', schema.DatasetPortalConfig(portal_visible=False)))
            self.assertEqual(portal.get_visible_datasets(), [])
            with self.assertRaises(HTTPException): acl.check_dataset_acl('d')
            with self.assertRaises(ValueError): portal.search_dataset('d', 'owned')

    def test_code_auth_tracks_session_without_real_crypto(self):
        request = types.SimpleNamespace(client=types.SimpleNamespace(host='owned-fixture'))
        with patch.object(store.AccessCodeValidator, 'verify_code', return_value=True), patch.object(route, 'create_portal_jwt', return_value='owned-not-a-jwt'):
            result = self.run_async(route.authenticate_with_code(schema.PortalAuthRequest(code='fixture'), request))
        self.assertEqual(result.token, 'owned-not-a-jwt')
        self.assertEqual(len(store.get_portal_config().active_sessions), 2)

    def test_sso_provider_interleavings(self):
        for change in ('visibility', 'ttl', 'revocation', 'disable', 'issuer', 'client', 'secret', 'base_url'):
            with self.subTest(change=change):
                config = schema.PortalConfig(**self.before)
                config.tier = schema.PortalTier.sso
                config.oidc_issuer, config.oidc_client_id, config.oidc_client_secret = 'fixture', 'fixture', 'sentinel'
                store.save_portal_config(config)
                async def exchange(*args):
                    if change == 'revocation':
                        await route.revoke_all_portal_sessions()
                    elif change == 'visibility':
                        await route.update_dataset_portal_config('d', schema.DatasetPortalConfig(portal_visible=False))
                    else:
                        fields = {'ttl': {'session_ttl_minutes': 60}, 'disable': {'enabled': False},
                            'issuer': {'oidc_issuer': 'new'}, 'client': {'oidc_client_id': 'new'},
                            'secret': {'oidc_client_secret': 'new'}, 'base_url': {'base_url': 'https://new.invalid'}}[change]
                        await route.update_portal_settings(schema.PortalConfigUpdate(**fields))
                    return {'id_token': 'owned-not-a-token'}
                async def claims(*args): return {'sub': 'owned-subject'}
                with patch.multiple(route, exchange_code_for_tokens=exchange, validate_id_token=claims,
                    validate_state=lambda _: {'redirect_uri': 'fixture', 'nonce': 'fixture'},
                    create_portal_jwt=lambda _: 'owned-not-a-jwt', store_id_token=lambda *a: None,
                    log_portal_access=lambda *a: None, encrypt_client_secret=lambda _: 'new-sentinel'):
                    request = types.SimpleNamespace(client=None)
                    if change in ('visibility', 'ttl'):
                        self.assertEqual(self.run_async(route.sso_callback(request, 'fixture', 'fixture')).status_code, 302)
                        latest = store.get_portal_config()
                        self.assertIn('existing', latest.active_sessions)
                        self.assertEqual(len(latest.active_sessions), 2)
                        if change == 'visibility': self.assertFalse(latest.datasets['d'].portal_visible)
                        else: self.assertEqual(latest.session_ttl_minutes, 60)
                    else:
                        with self.assertRaises(HTTPException): self.run_async(route.sso_callback(request, 'fixture', 'fixture'))
                        latest = store.get_portal_config()
                        self.assertEqual(len(latest.active_sessions), 0 if change == 'revocation' else 1)

    def test_sso_logout_does_not_restore_concurrent_settings(self):
        session = types.SimpleNamespace(session_id='existing', oidc_subject=None)
        config = store.get_portal_config()
        config.oidc_issuer = 'fixture'
        store.save_portal_config(config)
        async def end_session(*args, **kwargs):
            await route.update_dataset_portal_config('d', schema.DatasetPortalConfig(portal_visible=False))
            return 'owned-endpoint'
        with patch.multiple(route, get_id_token=lambda _: None, clear_refresh_token=lambda _: None, get_end_session_url=end_session):
            result = self.run_async(route.sso_logout(session))
        self.assertEqual(result['end_session_url'], 'owned-endpoint')
        self.assertNotIn('existing', store.get_portal_config().active_sessions)
        self.assertFalse(store.get_portal_config().datasets['d'].portal_visible)

    def test_revocation_during_id_token_validation(self):
        config = store.get_portal_config()
        config.tier = schema.PortalTier.sso
        config.oidc_issuer, config.oidc_client_id, config.oidc_client_secret = 'fixture', 'fixture', 'sentinel'
        store.save_portal_config(config)
        async def exchange(*args): return {'id_token': 'owned-not-a-token'}
        async def validate(*args):
            await route.revoke_all_portal_sessions()
            return {'sub': 'owned-subject'}
        with patch.multiple(route, exchange_code_for_tokens=exchange, validate_id_token=validate,
            validate_state=lambda _: {'redirect_uri': 'fixture', 'nonce': 'fixture'},
            create_portal_jwt=lambda _: 'owned-not-a-jwt', store_id_token=lambda *a: None, log_portal_access=lambda *a: None):
            with self.assertRaises(HTTPException):
                self.run_async(route.sso_callback(types.SimpleNamespace(client=None), 'fixture', 'fixture'))
        self.assertEqual(store.get_portal_config().portal_session_version, 5)
        self.assertEqual(store.get_portal_config().active_sessions, {})

    def test_parallel_local_transactions_preserve_both_changes(self):
        # Both threads start together; each complete read/modify/save must serialize.
        barrier = threading.Barrier(2)
        errors = []
        def worker(key):
            try:
                barrier.wait(timeout=5)
                with store.portal_config_update() as config:
                    config.active_sessions[key] = 'owned'
            except BaseException as exc: errors.append(exc)
        threads = [threading.Thread(target=worker, args=(key,)) for key in ('a', 'b')]
        for thread in threads: thread.start()
        for thread in threads: thread.join(timeout=5)
        self.assertFalse(any(t.is_alive() for t in threads))
        self.assertEqual(errors, [])
        self.assertEqual(set(store.get_portal_config().active_sessions), {'existing', 'a', 'b'})


if __name__ == '__main__':
    print(f'SOURCE={SOURCE} OWNED={OWNED} PYTHON={sys.executable}', flush=True)
    unittest.main(verbosity=2)
