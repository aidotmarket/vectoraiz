"""Offline minimal routes only; no application lifespan or live auth."""
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

if __name__ != "__main__":
    raise unittest.SkipTest("Run this isolated harness directly; it replaces product dependencies")

OWNED = tempfile.TemporaryDirectory(prefix="r305-role-")
for field in ("DATA_DIRECTORY", "UPLOAD_DIRECTORY", "PROCESSED_DIRECTORY", "RAW_FILE_IMPORT_DIRECTORY", "SERIAL_DATA_DIR", "KEYSTORE_PATH"):
    os.environ["VECTORAIZ_" + field] = str(Path(OWNED.name) / field.lower())
os.environ["VECTORAIZ_MODE"] = "connected"
os.environ["VECTORAIZ_AUTH_ENABLED"] = "true"
# Block network and protected data/credential roots before product imports.
import socket
socket.socket.connect = lambda *a, **k: (_ for _ in ()).throw(AssertionError("live network forbidden"))
import builtins, io
_real_open = builtins.open
def owned_open(file, *args, **kwargs):
    if isinstance(file, (str, bytes, os.PathLike)):
        value = os.fsdecode(file)
        if value.startswith(("/data/", "/Users/max/.", "/Users/max/Library/")):
            raise AssertionError("protected root forbidden: " + value)
    return _real_open(file, *args, **kwargs)
builtins.open = owned_open
io.open = owned_open

from fastapi import FastAPI, Depends, HTTPException
from fastapi.testclient import TestClient
from app.auth import api_key_auth as auth
from app.middleware import auth as gate

class RoleGateTests(unittest.TestCase):
    def setUp(self):
        auth.api_key_cache.clear()
        # Fail closed if an owned mock is missing; no provider, key or crypto work.
        for module, name in [(auth, "_fetch_ai_market_me"), (auth, "_validate_local_key"),
                             (auth, "_validate_key_against_aimarket")]:
            guard = patch.object(module, name, AsyncMock(side_effect=AssertionError("unmocked auth effect")))
            guard.start()
            self.addCleanup(guard.stop)
        for module, name in [(auth, "_get_http_client"), (auth, "_upsert_ai_market_user"),
                             (gate, "decode_jwt_token"), (gate, "get_jwt_secret")]:
            guard = patch.object(module, name, side_effect=AssertionError("unmocked auth effect"))
            guard.start()
            self.addCleanup(guard.stop)
        self.app = FastAPI()
        @self.app.api_route("/admin", methods=["GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "TRACE"])
        async def admin(user=Depends(gate.role_required("admin"))):
            return {"id": user.user_id}
        @self.app.api_route("/any", methods=["GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "TRACE"])
        async def any_user(user=Depends(gate.role_required("admin", "user"))):
            return {"id": user.user_id}
        self.client = TestClient(self.app)
        self.addCleanup(self.client.close)

    def validator(self, role="admin", scopes=None, error=None):
        async def validate(request):
            if error: raise HTTPException(status_code=error, detail="owned refusal")
            user = auth.AuthenticatedUser(user_id="owned", key_id="fixture", scopes=scopes if scopes is not None else gate._role_to_scopes(role))
            request.state.user_role = role
            request.state.user = user
            return user
        return AsyncMock(side_effect=validate)

    def test_connected_validated_roles_and_refusals(self):
        for role, scopes, path, expected in [
            ("admin", ["read", "write", "admin"], "/admin", 200),
            ("admin", ["read"], "/admin", 403),
            ("user", ["read", "write"], "/admin", 403),
            ("user", ["read", "write"], "/any", 200),
            ("user", ["read"], "/admin", 403),
        ]:
            with self.subTest(role=role, scopes=scopes, path=path):
                validator = self.validator(role, scopes)
                with patch.object(auth.settings, "mode", "connected"), patch.object(auth, "get_current_user", validator):
                    self.assertEqual(self.client.get(path, headers={"Authorization": "Bearer owned-unvalidated-fixture"}).status_code, expected)
                    self.assertEqual(validator.await_count, 1)
        for label, code in [("invalid",401),("expired",401),("inactive",403),("unavailable",503)]:
            with self.subTest(label=label), patch.object(auth, "get_current_user", self.validator(error=code)):
                self.assertEqual(self.client.get("/admin", headers={"Authorization":"Bearer owned-fixture"}).status_code, code)

    def test_existing_market_mapping_is_not_promoted(self):
        for role in ("buyer", "seller", "model_provider", "admin", "pending", "unknown"):
            mapped = auth._authenticated_user_from_ai_market_user({"id":"owned", "role":role, "status":"active"})
            self.assertEqual("admin" in mapped.scopes, role == "admin")
        with self.assertRaises(HTTPException) as raised:
            auth._authenticated_user_from_ai_market_user({"id":"owned", "role":"admin", "status":"inactive"})
        self.assertEqual(raised.exception.status_code,403)

    def test_standalone_no_validator_or_remote_call(self):
        validator = self.validator()
        with patch.object(auth.settings, "mode", "standalone"), patch.object(auth, "get_current_user", validator):
            self.assertEqual(self.client.get("/admin", headers={"Authorization":"Bearer owned-fixture"}).status_code,401)
            validator.assert_not_awaited()

    def test_cookie_key_missing_and_development_controls(self):
        self.assertEqual(self.client.get("/admin").status_code,401)
        for mode in ("connected", "standalone"):
            with patch.object(auth.settings, "mode", mode):
                validator = self.validator()
                with patch.object(auth, "get_current_user", validator):
                    self.assertEqual(self.client.get("/admin", headers={"X-API-Key":"owned-mock-key"}).status_code,200)
                    validator.assert_awaited_once()
                import sys, types
                service = types.ModuleType("app.services.auth_service")
                service.get_auth_service = lambda: types.SimpleNamespace(get_user_by_id=AsyncMock(return_value=types.SimpleNamespace(id="owned",is_active=True)))
                with patch.dict(sys.modules, {"app.services.auth_service":service}), patch.object(gate,"decode_jwt_token",return_value={"sub":"owned","role":"admin"}):
                    self.assertEqual(self.client.get("/admin",headers={"Cookie":"vz_session=owned-mock-cookie"}).status_code,200)
                with patch.dict(sys.modules, {"app.services.auth_service":service}), patch.object(gate,"decode_jwt_token",return_value={"sub":"owned","role":"user"}):
                    self.assertEqual(self.client.get("/admin",headers={"Cookie":"vz_session=owned-mock-cookie"}).status_code,403)
        with patch.object(auth,"_is_auth_enabled",return_value=False), patch.object(auth,"get_current_user",self.validator()):
            self.assertEqual(self.client.get("/admin").status_code,200)


    def market_validation(self, role="pending", status="active", scopes=None):
        async def handle(token):
            data = {"id": "owned", "role": role, "status": status}
            user = auth._authenticated_user_from_ai_market_user(data)
            if scopes is not None:
                user.scopes = scopes
            return data, user
        return AsyncMock(side_effect=handle)

    def test_read_only_market_roles_by_method_and_route(self):
        for role in ("pending", "unknown"):
            for method in ("GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "TRACE"):
                for path in ("/any", "/admin"):
                    with self.subTest(role=role, method=method, path=path):
                        auth.api_key_cache.clear()
                        validator = self.market_validation(role)
                        with patch.object(auth, "_handle_ai_market_token", validator):
                            response = self.client.request(method, path, headers={
                                "Authorization": "Bearer  owned-fixture  ",
                                "X-User-Role": "admin", "X-Scopes": "read write admin",
                            })
                        expected = 200 if path == "/any" and method in ("GET", "HEAD", "OPTIONS") else 403
                        self.assertEqual(response.status_code, expected)
                        if expected == 403 and method != "HEAD":
                            self.assertEqual(response.json(), {"detail": "Insufficient permissions"})
                        validator.assert_awaited_once_with("owned-fixture")

    def test_market_write_roles_and_admin_requirements(self):
        for role, scopes in [("buyer", None), ("seller", None), ("model_provider", None),
                             ("admin", None), ("admin", ["read", "admin"]), ("admin", ["read", "write"])]:
            for method in ("GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE"):
                for path in ("/any", "/admin"):
                    with self.subTest(role=role, scopes=scopes, method=method, path=path):
                        auth.api_key_cache.clear()
                        validator = self.market_validation(role, scopes=scopes)
                        with patch.object(auth, "_handle_ai_market_token", validator):
                            response = self.client.request(method, path, headers={"Authorization": "Bearer owned-fixture"})
                        write_allowed = method in ("GET", "HEAD", "OPTIONS") or scopes != ["read", "admin"]
                        allowed = (path == "/any" or role == "admin" and scopes != ["read", "write"]) and write_allowed
                        self.assertEqual(response.status_code, 200 if allowed else 403)
                        if not allowed and method != "HEAD":
                            self.assertEqual(response.json(), {"detail": "Insufficient permissions"})
                        validator.assert_awaited_once_with("owned-fixture")

    def test_nonempty_bearer_refusal_precedes_cookie_and_key(self):
        for mode in ("connected", "standalone"):
            for label, code in [("invalid", 401), ("expired", 401), ("inactive", 403), ("unavailable", 503), ("invalid-user", 401)]:
                with self.subTest(mode=mode, label=label):
                    auth.api_key_cache.clear()
                    if label == "inactive":
                        validator = self.market_validation(status="inactive")
                    elif label == "invalid-user":
                        async def invalid(token, request):
                            return auth.AuthenticatedUser(user_id="owned", key_id="fixture", scopes=["read", "write", "admin"], valid=False)
                        validator = AsyncMock(side_effect=invalid)
                    else:
                        validator = AsyncMock(side_effect=HTTPException(status_code=code, detail="owned refusal"))
                    target = "_validate_ai_market_bearer" if label == "invalid-user" else "_handle_ai_market_token"
                    with patch.object(auth.settings, "mode", mode), patch.object(auth, target, validator):
                        response = self.client.post("/any", headers={"Authorization": "Bearer owned-fixture", "Cookie": "vz_session=owned-cookie", "X-API-Key": "vz_owned_fixture"})
                    self.assertEqual(response.status_code, code if mode == "connected" else 401)
                    self.assertEqual(set(response.json()), {"detail"})
                    if mode == "standalone": validator.assert_not_awaited()
                    else: self.assertEqual(validator.await_count, 1)

    def test_empty_bearer_uses_real_unified_cookie_key_fallback(self):
        import sys, types
        for mode in ("connected", "standalone"):
            for header in ("Bearer ", "Bearer    ", "Bearer \t ", "bearer owned-fixture", "Bearer"):
                for credential in ("cookie", "key"):
                    for method in ("GET", "POST"):
                        with self.subTest(mode=mode, header=repr(header), credential=credential, method=method):
                            auth.api_key_cache.clear()
                            bearer = AsyncMock(side_effect=AssertionError("empty token reached validator"))
                            headers = {"Authorization": header}
                            service = types.ModuleType("app.services.auth_service")
                            lookup = AsyncMock(return_value=types.SimpleNamespace(id="owned", is_active=True))
                            service.get_auth_service = lambda: types.SimpleNamespace(get_user_by_id=lookup)
                            key = AsyncMock(return_value=auth.AuthenticatedUser(user_id="owned", key_id="fixture", scopes=["read"]))
                            headers.update({"Cookie": "vz_session=owned-cookie"} if credential == "cookie" else {"X-API-Key": "vz_owned_fixture"})
                            with patch.object(auth.settings, "mode", mode), patch.object(auth, "_validate_ai_market_bearer", bearer), patch.object(auth, "_validate_local_key", key), patch.dict(sys.modules, {"app.services.auth_service": service}), patch.object(gate, "decode_jwt_token", return_value={"sub": "owned", "role": "admin"}):
                                response = self.client.request(method, "/admin", headers=headers)
                            self.assertEqual(response.status_code, 200)
                            bearer.assert_not_awaited()
                            if credential == "key":
                                key.assert_awaited_once_with("vz_owned_fixture")
                                lookup.assert_not_awaited()
                            else:
                                lookup.assert_awaited_once_with("owned")
                                key.assert_not_awaited()

if __name__ == "__main__": unittest.main(verbosity=2)
