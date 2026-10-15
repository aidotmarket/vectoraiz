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
        self.app = FastAPI()
        @self.app.get("/admin")
        async def admin(user=Depends(gate.role_required("admin"))):
            return {"id": user.user_id}
        @self.app.get("/any")
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

if __name__ == "__main__": unittest.main(verbosity=2)
