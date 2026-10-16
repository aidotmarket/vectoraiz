"""Isolated stdlib regression; run directly with python -I, never via pytest.

Extract only the native handler and its stdlib imports. Owned decorator/dependency
fixtures preserve execution of the route declaration without importing the app.
No HTTP/auth integration claim, app startup, collectors, or shared process access.
Run in a throwaway process with a minimal environment; full dumps are never logged.
"""

import argparse
import ast
import faulthandler
import hashlib
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest


def deny_network(event, args):
    if event.startswith("socket."):
        raise AssertionError("Socket/DNS access forbidden in isolated regression")


sys.addaudithook(deny_network)

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--source", type=Path, default=(
    Path(__file__).resolve().parents[1] / "app/routers/diagnostics.py"
))
parser.add_argument("--native-only", action="store_true")
options = parser.parse_args()
source = options.source.read_bytes()
print("handler_source_sha256=" + hashlib.sha256(source).hexdigest(), flush=True)
print("test_source_sha256=" + hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
      flush=True)
tree = ast.parse(source)
handler = next(node for node in tree.body
               if isinstance(node, ast.AsyncFunctionDef) and node.name == "dump_threads")


def load_handler(temporary_file, dump_traceback):
    routes = []
    dependency = object()

    def get(path):
        def register(function):
            routes.append((path, function))
            return function
        return register

    def depends(function):
        if function is not dependency:
            raise AssertionError("Authentication dependency changed")
        return dependency

    # Execute the actual stdlib imports used by either baseline or repaired body.
    imports = [node for node in tree.body if isinstance(node, ast.Import)
               and all(alias.name in {"io", "tempfile", "faulthandler"}
                       for alias in node.names)]
    namespace = {"router": SimpleNamespace(get=get), "Depends": depends,
                 "AuthenticatedUser": object, "get_current_user": dependency}
    exec(compile(ast.Module(body=imports, type_ignores=[]), str(options.source), "exec"),
         namespace)
    namespace["tempfile"] = SimpleNamespace(TemporaryFile=temporary_file)
    namespace["faulthandler"] = SimpleNamespace(dump_traceback=dump_traceback)
    exec(compile(ast.Module(body=[handler], type_ignores=[]), str(options.source), "exec"),
         namespace)
    function = namespace["dump_threads"]
    if routes != [("/diagnostics/threads", function)]:
        raise AssertionError("Route declaration changed")
    if function.__defaults__ != (dependency,):
        raise AssertionError("Authentication default changed")
    return function


def invoke(function):
    # This handler has no awaits. Drive its native coroutine without an event loop
    # (and thus without asyncio's internal socketpair or any background threads).
    coroutine = function()
    try:
        coroutine.send(None)
    except StopIteration as result:
        return result.value
    finally:
        coroutine.close()
    raise AssertionError("Handler unexpectedly suspended")


class ThreadDumpRegression(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="thread-regression-")
        self.addCleanup(self.directory.cleanup)
        self.files = []
        self.descriptors = []

    def temporary_file(self, *args, **kwargs):
        file = tempfile.TemporaryFile(*args, dir=self.directory.name, **kwargs)
        self.files.append(file)
        self.descriptors.append(file.fileno())
        return file

    def assert_cleanup(self):
        import os

        self.assertEqual(len(self.files), 1)
        self.assertTrue(self.files[0].closed)
        with self.assertRaises(OSError):
            os.fstat(self.descriptors[0])
        self.assertEqual(list(Path(self.directory.name).iterdir()), [])

    def test_native_traceback_and_success_cleanup(self):
        function = load_handler(self.temporary_file, faulthandler.dump_traceback)
        result = invoke(function)
        self.assertEqual(set(result), {"thread_dump"})
        dump = result["thread_dump"]
        self.assertIsInstance(dump, str)
        # Assert without including the captured dump in failure output.
        self.assertTrue(bool(dump.strip()))
        self.assertTrue("dump_threads" in dump and "Current thread" in dump)
        self.assert_cleanup()

    def test_faulthandler_exception_cleanup(self):
        failure = RuntimeError("owned faulthandler failure")

        def failing_dump(*, file, all_threads):
            self.assertIs(all_threads, True)
            file.write(b"owned partial capture")
            raise failure

        function = load_handler(self.temporary_file, failing_dump)
        with self.assertRaises(RuntimeError) as caught:
            invoke(function)
        self.assertIs(caught.exception, failure)
        self.assert_cleanup()

    def test_rewind_decode_and_all_threads(self):
        def owned_dump(*, file, all_threads):
            self.assertIs(all_threads, True)
            file.write(b"owned traceback\xff\n")

        result = invoke(load_handler(self.temporary_file, owned_dump))
        self.assertEqual(result, {"thread_dump": "owned traceback\ufffd\n"})
        self.assert_cleanup()


if __name__ == "__main__":
    suite = (unittest.TestSuite([ThreadDumpRegression(
        "test_native_traceback_and_success_cleanup")]) if options.native_only
        else unittest.defaultTestLoader.loadTestsFromTestCase(ThreadDumpRegression))
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    sys.exit(0 if result.wasSuccessful() else 1)
