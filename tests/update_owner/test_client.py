"""Run directly with python3 -I; never pytest/conftest or application imports."""
import ast
import importlib.abc
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest

# unittest.mock imports asyncio/socket. A tiny local patcher keeps this suite
# strictly within stdlib filesystem/JSON/clock code, without that import chain.
class patch:
    @staticmethod
    def object(obj, name, value=None, side_effect=None):
        class Replacement:
            def start(self):
                self.previous = getattr(obj, name)
                if side_effect is not None:
                    iterator = iter(side_effect)
                    self.value = lambda *args, **kwargs: next(iterator)
                else:
                    self.value = value if value is not None else lambda *args, **kwargs: None
                setattr(obj, name, self.value)
                return self.value

            def stop(self):
                setattr(obj, name, self.previous)

            def __enter__(self):
                return self.start()

            def __exit__(self, *args):
                self.stop()
        return Replacement()

ROOT = Path(__file__).resolve().parents[2]
DENIED = ("app", "sqlalchemy", "psycopg", "redis", "docker", "socket", "httpx", "requests", "urllib.request", "security", "conftest")
EVENTS = []


class Guard(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if any(fullname == name or fullname.startswith(name + ".") for name in DENIED):
            EVENTS.append("DENIED_IMPORT:" + fullname)
            raise RuntimeError("ISOLATION_GUARD:" + fullname)


def audit(event, args):
    if event.startswith(("socket.", "subprocess.", "os.system", "os.posix_spawn")):
        EVENTS.append("DENIED_EVENT:" + event)
        raise RuntimeError("ISOLATION_GUARD:" + event)


sys.meta_path.insert(0, Guard())
sys.addaudithook(audit)
spec = importlib.util.spec_from_file_location("isolated_owner_client", ROOT / "app/services/update_owner_client.py")
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)


class OwnerClientTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="r374-isolated-")
        self.addCleanup(self.tmp.cleanup)
        self.box = Path(self.tmp.name)
        self.replace = patch.object(client, "MAILBOX", self.box)
        self.replace.start()
        self.addCleanup(self.replace.stop)
        self.pair = {"installation": "installation-00000000000", "epoch": "epoch-00000000000000",
                     "container": "container-0000000000000", "generation": 3, "key": "secret-pair-000000000000"}
        client._write("bootstrap.json", self.pair)
        self.snapshot = {**{k: v for k, v in self.pair.items() if k != "key"},
                         "heartbeat": int(time.time()), "ready": True, "reason": "", "current": None,
                         "history": [], "keys": {}, "last_verified": None, "recovery": "Host status remains readable."}
        self.publish()

    def publish(self):
        client._write("status.json", client._sign(self.pair, "status", self.snapshot))

    def test_actual_writer_echo_and_no_socket_readiness(self):
        self.assertTrue(client.heartbeat())
        ready = client._verify(self.pair, "ready", client._read("ready.json"))
        self.assertEqual(ready, self.pair)
        self.assertTrue(client.available())
        self.snapshot["heartbeat"] -= 20
        self.publish()
        self.assertFalse(client.available())
        self.assertTrue(client.snapshot()["stale"])

    def test_missing_owner_refuses_without_mutation(self):
        (self.box / "status.json").unlink()
        with self.assertRaises(client.OwnerError) as error:
            client.submit("v2.0.0", "stable-idempotency-000000", "authenticated-principal")
        self.assertEqual(error.exception.http_status, 503)
        self.assertFalse((self.box / "request.json").exists())

    def test_bounded_authenticated_request_and_durable_ack(self):
        original_write = client._write

        def owner_accept(name, data):
            original_write(name, data)
            if name == "request.json":
                message = client._verify(self.pair, "request", data)
                self.assertEqual(message["principal"], "authorized-admin")
                self.assertEqual(message["scope"], "update:submit")
                self.assertLessEqual((self.box / name).stat().st_size, 4096)
                ack = {"key": message["key"], "target": message["target"], "id": "durable-operation-id", "code": 202}
                original_write("ack.json", client._sign(self.pair, "ack", {"pair": self.pair, "ack": ack}))

        with patch.object(client, "_write", owner_accept):
            result = client.submit("v2.0.0", "stable-idempotency-000000", "authorized-admin")
        self.assertEqual(result["operation_id"], "durable-operation-id")
        self.assertNotIn("key", client.status())

    def test_partial_copy_wrong_mac_epoch_and_generation_never_202(self):
        for variant in ("partial", "wrong-mac", "wrong-epoch", "wrong-generation"):
            with self.subTest(variant=variant):
                p = dict(self.pair)
                ack = {"key": "stable-idempotency-000000", "target": "v2.0.0", "id": "op", "code": 202}
                if variant == "wrong-epoch":
                    p["epoch"] = "another-epoch-0000000"
                if variant == "wrong-generation":
                    p["generation"] += 1
                envelope = client._sign(self.pair, "ack", {"pair": p, "ack": ack})
                if variant == "wrong-mac":
                    envelope["mac"] = "00" * 32
                client._write("ack.json", envelope)
                if variant == "partial":
                    (self.box / "ack.json").write_bytes(b'{"payload":')
                # Fake monotonic clock bounds the loop without any native wait.
                with patch.object(client.time, "monotonic", side_effect=[0, 1, 21]), patch.object(client.time, "sleep"):
                    with self.assertRaises(client.OwnerError) as error:
                        client.submit("v2.0.0", "stable-idempotency-000000", "authorized-admin")
                self.assertEqual(error.exception.code, "acceptance_unknown")
                self.assertEqual(error.exception.http_status, 504)

    def test_lost_response_lookup_and_expired_key_no_resubmit(self):
        op = {"id": "persisted-op", "target": "v2.0.0", "phase": "recovery_required"}
        self.snapshot["current"] = op
        self.snapshot["keys"] = {"stable-idempotency-000000": {"id": op["id"], "target": op["target"], "code": 202}}
        self.snapshot["heartbeat"] -= 86400
        self.publish()
        result = client.submit(op["target"], "stable-idempotency-000000", "authorized-admin")
        self.assertEqual(result["operation_id"], op["id"])
        self.assertFalse((self.box / "request.json").exists())
        self.assertEqual(client.status("stable-idempotency-000000")["current"], op)
        with self.assertRaises(client.OwnerError) as error:
            client.submit("v3.0.0", "stable-idempotency-000000", "authorized-admin")
        self.assertEqual(error.exception.http_status, 409)

    def test_symlink_and_oversized_snapshot_refused(self):
        (self.box / "status.json").write_bytes(b'x' * 65537)
        self.assertFalse(client.available())
        (self.box / "status.json").unlink()
        (self.box / "status.json").symlink_to(self.box / "bootstrap.json")
        self.assertFalse(client.available())

    def test_registered_mailbox_wins_over_stale_other_bootstrap(self):
        # Real environment selector is an allowlisted path, never arbitrary input.
        with patch.object(client, "MAILBOX", client._DEFAULT_MAILBOX):
            original = os.environ.get("VECTORAIZ_OWNER_MAILBOX")
            try:
                os.environ["VECTORAIZ_OWNER_MAILBOX"] = "/opt/vectoraiz-owner-mailbox"
                self.assertEqual(client._location(), Path("/opt/vectoraiz-owner-mailbox"))
                os.environ["VECTORAIZ_OWNER_MAILBOX"] = "/customer/path"
                with self.assertRaises(client.OwnerError):
                    client._location()
            finally:
                if original is None:
                    os.environ.pop("VECTORAIZ_OWNER_MAILBOX", None)
                else:
                    os.environ["VECTORAIZ_OWNER_MAILBOX"] = original

    def test_actual_route_authorization_without_importing_app(self):
        # Execute only the actual leaf scope decision from its AST. Do not load
        # FastAPI, auth, settings, database models or any application package.
        tree = ast.parse((ROOT / "app/routers/version.py").read_text())
        decision = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "_authorize_update")
        class Denied(Exception):
            def __init__(self, status, detail):
                self.status = status
        class User:
            def __init__(self, scopes):
                self.scopes = scopes
        env = {"AuthenticatedUser": User, "HTTPException": Denied}
        exec(compile(ast.Module(body=[decision], type_ignores=[]), "actual-scope-decision", "exec"), env)
        for scopes in (["admin"], ["read", "write", "admin"], ["update:submit"]):
            env["_authorize_update"](User(scopes))
        for scopes in ([], ["read"], ["read", "write"], ["host:exec"]):
            with self.assertRaises(Denied) as refused:
                env["_authorize_update"](User(scopes))
            self.assertEqual(refused.exception.status, 403)
        for name in ("post_update", "update_operation"):
            route = next(n for n in tree.body if isinstance(n, ast.AsyncFunctionDef) and n.name == name)
            self.assertTrue(any(isinstance(default, ast.Call) and isinstance(default.func, ast.Name)
                                and default.func.id == "Depends" and ast.unparse(default.args[0]) == "get_current_user"
                                for default in route.args.defaults))
        # Check the existing auth model's actual field names without importing it.
        auth = ast.parse((ROOT / "app/auth/api_key_auth.py").read_text())
        model = next(n for n in auth.body if isinstance(n, ast.ClassDef) and n.name == "AuthenticatedUser")
        names = {n.target.id for n in model.body if isinstance(n, ast.AnnAssign)}
        self.assertTrue({"user_id", "scopes"} <= names)

    def test_actual_cookie_auth_branch_with_fake_session_dependencies(self):
        # Execute only the existing cookie branch and absent-header denial.
        # Auth service/JWT/FastAPI dependencies are fakes; no app imports.
        auth = ast.parse((ROOT / "app/auth/api_key_auth.py").read_text())
        current = next(n for n in auth.body if isinstance(n, ast.AsyncFunctionDef) and n.name == "get_current_user")
        index = next(i for i, n in enumerate(current.body) if isinstance(n, ast.Assign)
                     and ast.unparse(n).startswith("token = request.cookies.get"))
        leaf = ast.parse("async def cookie_leaf(request): pass").body[0]
        leaf.body = current.body[index:index + 4]
        class RemoveImports(ast.NodeTransformer):
            def visit_ImportFrom(self, node):
                return None
        leaf = ast.fix_missing_locations(RemoveImports().visit(leaf))
        class User:
            def __init__(self, **values):
                self.__dict__.update(values)
        class Denied(Exception):
            def __init__(self, status_code=None, detail=None):
                self.status = status_code
        class Status:
            HTTP_401_UNAUTHORIZED = 401
        class Service:
            async def get_user_by_id(self, user_id):
                return User(id=user_id, is_active=True)
        role = "admin"
        env = {"JWT_COOKIE_NAME": "vz_session", "decode_jwt_token": lambda token: {"sub": "synthetic", "role": role},
               "get_auth_service": Service, "AuthenticatedUser": User, "HTTPException": Denied, "status": Status}
        exec(compile(ast.Module(body=[leaf], type_ignores=[]), "actual-cookie-auth-branch-fake-dependencies", "exec"), env)
        routes = ast.parse((ROOT / "app/routers/version.py").read_text())
        authorize = next(n for n in routes.body if isinstance(n, ast.FunctionDef) and n.name == "_authorize_update")
        exec(compile(ast.Module(body=[authorize], type_ignores=[]), "actual-update-scope-leaf", "exec"), env)
        for cookie, role, expected in [(True, "admin", 202), (True, "user", 403), (False, "user", 401)]:
            with self.subTest(cookie=cookie, role=role):
                request = User(cookies={"vz_session": "synthetic-cookie"} if cookie else {}, headers={}, state=User())
                coroutine = env["cookie_leaf"](request)
                try:
                    try:
                        coroutine.send(None)
                        self.fail("fake cookie auth unexpectedly yielded external work")
                    except StopIteration as done:
                        user = done.value
                    env["_authorize_update"](user)
                    result = 202
                except Denied as error:
                    result = error.status
                finally:
                    coroutine.close()
                self.assertEqual(result, expected)
        print("FAKE_SCOPE_ONLY: actual extracted cookie/scopes/absent-auth branches; real FastAPI/JWT/browser NOT_RUN")

    def test_guards_close_application_and_native_paths(self):
        with self.assertRaises(RuntimeError):
            __import__("app")
        with self.assertRaises(RuntimeError):
            os.system("denied-before-shell")
        self.assertEqual(EVENTS, ["DENIED_IMPORT:app", "DENIED_EVENT:os.system"])
        self.assertNotIn("socket", sys.modules)
        self.assertNotIn("asyncio", sys.modules)
        self.assertNotIn("subprocess", sys.modules)


if __name__ == "__main__":
    print("ISOLATION: direct-file stdlib load; native/network/application import guards active", flush=True)
    result = unittest.main(verbosity=2, exit=False)
    print("GUARD_EVENTS: " + json.dumps(EVENTS), flush=True)
    sys.exit(not result.result.wasSuccessful())
