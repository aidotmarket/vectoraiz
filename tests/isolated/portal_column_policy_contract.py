"""Owned, offline contract tests; run directly, bypassing app conftest/lifespan.

rtk proxy python -B tests/isolated/portal_column_policy_contract.py

Compile actual schema and handler/service definitions, not implementation copies.
Only config persistence, processing, and search collaborators are owned mocks.
No auth/JWT/provider/index/database modules are imported or executed.
This standalone filename avoids installing process-wide guards during ordinary
pytest collection; run it in its own process using the command above.
"""
import ast
import asyncio
import json
import logging
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

# Establish owned configuration and deny external effects BEFORE library/product imports.
_OWNED = tempfile.TemporaryDirectory(prefix="r307-owned-")
for _key in list(os.environ):
    if any(word in _key.upper() for word in ("TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "API_KEY")):
        os.environ.pop(_key, None)
for _name in ("DATA_DIRECTORY", "UPLOAD_DIRECTORY", "PROCESSED_DIRECTORY"):
    os.environ["VECTORAIZ_" + _name] = _OWNED.name
os.environ["DATABASE_URL"] = "sqlite:///" + _OWNED.name + "/never-open.db"
_DENIED_ROOTS = tuple(str(p) for p in (
    Path("/data"), Path.home() / ".aws", Path.home() / ".ssh",
    Path.home() / ".azure", Path.home() / ".config/gcloud",
    Path.home() / "Library/Keychains",
))

def _deny_live_effects(event, args):
    if event in {"socket.connect", "socket.bind", "socket.getaddrinfo", "subprocess.Popen", "os.system"}:
        raise RuntimeError("R307 isolated test denied external effect: " + event)
    if event == "open" and isinstance(args[0], (str, bytes, os.PathLike)):
        path = str(Path(os.fsdecode(args[0])).resolve())
        if any(path == root or path.startswith(root + "/") for root in _DENIED_ROOTS):
            raise RuntimeError("R307 isolated test denied live data/credential root")

sys.addaudithook(_deny_live_effects)

from fastapi import APIRouter, HTTPException
from pydantic import ValidationError

ROOT = Path(__file__).resolve().parents[2]


def actual_definitions(path, names, namespace):
    tree = ast.parse((ROOT / path).read_text(), filename=path)
    nodes = [node for node in tree.body if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) and node.name in names]
    assert {node.name for node in nodes} == set(names)
    exec(compile(ast.Module(body=nodes, type_ignores=[]), path, "exec"), namespace)


class PortalColumnPolicyTests(unittest.TestCase):
    def setUp(self):
        self.ns = {"__name__": "owned_portal_schema"}
        exec(compile((ROOT / "app/schemas/portal.py").read_text(), "app/schemas/portal.py", "exec"), self.ns)
        self.Policy = self.ns["DatasetPortalConfig"]
        self.config = self.ns["PortalConfig"](datasets={"owned": self.Policy(
            portal_visible=False, search_columns=["public"], display_columns=["public"], max_results=3,
        )})
        self.save = Mock()
        self.processing = Mock()
        self.processing.get_dataset.return_value = types.SimpleNamespace(original_filename="owned.csv")
        self.search = Mock()
        self.ns.update(
            get_portal_config=lambda: self.config, save_portal_config=self.save,
            get_processing_service=lambda: self.processing,
            get_search_service=lambda: self.search,
            HTTPException=HTTPException, admin_router=APIRouter(), json=json,
            logger=logging.getLogger("r307-owned"),
        )
        actual_definitions("app/routers/portal.py", {"update_dataset_portal_config"}, self.ns)
        actual_definitions("app/services/portal_service.py", {"PortalService"}, self.ns)
        self.service = self.ns["PortalService"]()
        # Reuse the existing portal-search fixture builder without its lifespan imports.
        actual_definitions("tests/test_portal_search.py", {"_make_search_results"}, self.ns)
        module = types.ModuleType("app.services.processing_service")
        module.get_processing_service = lambda: self.processing
        self.modules = patch.dict(sys.modules, {"app.services.processing_service": module})
        self.modules.start()
        self.addCleanup(self.modules.stop)

    def update(self, fields, dataset_id="owned"):
        body = self.Policy.model_validate(fields)
        result = asyncio.run(self.ns["update_dataset_portal_config"](dataset_id, body))
        self.save.assert_called_once_with(self.config)
        self.assertEqual(result, {"dataset_id": dataset_id, **self.config.datasets[dataset_id].model_dump()})
        return self.config.datasets[dataset_id].model_dump()

    def test_visibility_only_preserves_all_policy(self):
        before = self.config.datasets["owned"].model_dump()
        self.assertEqual(self.update({"portal_visible": True}), {**before, "portal_visible": True})

    def test_empty_request_preserves_all_fields(self):
        before = self.config.datasets["owned"].model_dump()
        self.assertEqual(self.update({}), before)

    def test_partial_updates_preserve_every_omitted_field(self):
        for field, value in [("search_columns", ["other"]), ("display_columns", ["other"]), ("max_results", 7), ("portal_visible", True)]:
            with self.subTest(field=field):
                self.save.reset_mock()
                before = self.config.datasets["owned"].model_dump()
                self.assertEqual(self.update({field: value}), {**before, field: value})

    def test_explicit_empty_arrays_clear_restrictions(self):
        self.assertEqual(self.update({"search_columns": [], "display_columns": []}), dict(
            portal_visible=False, search_columns=[], display_columns=[], max_results=3,
        ))

    def test_explicit_full_update_retains_meaning(self):
        fields = dict(portal_visible=True, search_columns=["new"], display_columns=["new"], max_results=19)
        self.assertEqual(self.update(fields), fields)

    def test_explicit_defaults_are_updates(self):
        self.assertEqual(self.update(dict(portal_visible=False, search_columns=[], display_columns=[], max_results=100)),
                         self.Policy().model_dump())

    def test_new_config_uses_existing_defaults(self):
        for fields in ({}, {"portal_visible": True}, {"display_columns": ["public"]}):
            with self.subTest(fields=fields):
                self.save.reset_mock()
                self.config.datasets.pop("new", None)
                self.assertEqual(self.update(fields, "new"), self.Policy(**fields).model_dump())

    def test_invalid_explicit_fields_rejected_by_actual_schema(self):
        for fields in ({"max_results": 0}, {"max_results": 1001}, {"max_results": None}, {"display_columns": None}, {"search_columns": [None]}, {"portal_visible": None}):
            with self.subTest(fields=fields), self.assertRaises(ValidationError):
                self.update(fields)
        self.save.assert_not_called()

    def test_complete_merged_policy_validated_before_publication(self):
        previous = self.config.datasets["owned"]
        previous.max_results = 0  # Simulate invalid current in-memory state.
        with self.assertRaises(ValidationError):
            self.update({"portal_visible": True})
        self.assertIs(self.config.datasets["owned"], previous)
        self.save.assert_not_called()

    def test_missing_dataset_still_404_and_no_save(self):
        self.processing.get_dataset.return_value = None
        with self.assertRaises(HTTPException) as error:
            asyncio.run(self.ns["update_dataset_portal_config"]("missing", self.Policy()))
        self.assertEqual(error.exception.status_code, 404)
        self.save.assert_not_called()

    def test_http_contract_keeps_put_path_and_body_schema(self):
        route, = self.ns["admin_router"].routes
        self.assertEqual(route.path, "/settings/portal/datasets/{dataset_id}")
        self.assertEqual(route.methods, {"PUT"})
        self.assertIs(route.body_field.type_, self.Policy)
        # Inspect the real mount; do not import main or auth dependencies.
        tree = ast.parse((ROOT / "app/main.py").read_text())
        mounts = [n for n in ast.walk(tree) if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr == "include_router" and n.args and isinstance(n.args[0], ast.Name) and n.args[0].id == "portal_admin_router"]
        mount, = mounts
        keywords = {k.arg: k.value for k in mount.keywords}
        self.assertEqual(ast.literal_eval(keywords["prefix"]), "/api")
        self.assertEqual(keywords["dependencies"].id, "admin_route_dependency")
        self.assertIn("admin_route_dependency = [Depends(require_admin)]", (ROOT / "app/main.py").read_text())

    def result(self, rows, restricted=True, limit=20):
        self.config.datasets["owned"].portal_visible = True
        self.config.datasets["owned"].display_columns = ["public"] if restricted else []
        self.search.search.return_value = {"results": rows, "total": 41}
        response = self.service.search_dataset("owned", "owned query", limit)
        self.assertIsInstance(response, self.ns["PortalSearchResult"])
        return json.loads(response.model_dump_json())

    def test_restricted_serialized_text_uses_allowed_projection(self):
        row = {"score": 0.5, "row_data": {"public": "allowed", "private": "owned-hidden-sentinel"},
               "text_content": "public: allowed; private: owned-hidden-sentinel", "extra": "owned-hidden-sentinel"}
        data = self.result([row])
        self.assertNotIn("owned-hidden-sentinel", json.dumps(data))
        self.assertEqual(data["results"][0]["row_data"], {"public": "allowed"})
        self.assertIn("allowed", data["results"][0]["text_content"])
        self.assertEqual(data["results"][0]["score"], 0.5)
        self.assertEqual(data["dataset_name"], "owned.csv")
        self.assertEqual(data["total_count"], 41)
        self.assertEqual(set(data), {"dataset_id", "dataset_name", "total_count", "results", "query"})
        self.assertEqual(set(data["results"][0]), {"score", "row_data", "text_content"})
        self.assertEqual(row["row_data"]["private"], "owned-hidden-sentinel")

    def test_missing_empty_or_disallowed_only_rows_never_fall_back(self):
        for fields in ({}, {"row_data": {}}, {"row_data": None}, {"row_data": {"private": "owned-hidden-sentinel"}}):
            with self.subTest(fields=fields):
                data = self.result([{**fields, "text_content": "owned-hidden-sentinel"}])
                self.assertNotIn("owned-hidden-sentinel", json.dumps(data))
                self.assertEqual(data["results"][0]["row_data"], {})
                self.assertEqual(data["results"][0]["text_content"], "")

    def test_allowed_falsey_and_structured_values_survive(self):
        for value in (None, False, 0, "", ["allowed"], {"label": "allowed"}):
            with self.subTest(value=value):
                item = self.result([{"row_data": {"public": value, "private": "owned-hidden-sentinel"}, "text_content": "owned-hidden-sentinel"}])["results"][0]
                self.assertEqual(item["row_data"], {"public": value})
                self.assertNotIn("owned-hidden-sentinel", json.dumps(item))

    def test_existing_search_fixture_allowed_output(self):
        rows = self.ns["_make_search_results"](["public", "private"], n=2)["results"]
        items = self.result(rows)["results"]
        for i, item in enumerate(items):
            self.assertEqual(item["row_data"], {"public": f"val_public_{i}"})
            self.assertIn(f"val_public_{i}", item["text_content"])
            self.assertNotIn(f"val_private_{i}", json.dumps(item))

    def test_unrestricted_legacy_row_and_raw_text_unchanged(self):
        row = {"score": 0.8, "row_data": {"public": "allowed", "private": "legacy"}, "text_content": "legacy raw text"}
        self.assertEqual(self.result([row], restricted=False)["results"], [row])

    def test_unrestricted_missing_row_and_text_defaults(self):
        self.assertEqual(self.result([{}], restricted=False)["results"], [{"score": None, "row_data": {}, "text_content": ""}])

    def test_max_limit_and_search_arguments_unchanged(self):
        for limit, expected in [(1, 1), (3, 3), (50, 3)]:
            with self.subTest(limit=limit):
                self.result([], limit=limit)
                self.search.search.assert_called_with(query="owned query", dataset_id="owned", limit=expected)

    def test_nonvisible_still_rejected_without_search(self):
        with self.assertRaises(ValueError):
            self.service.search_dataset("owned", "owned query")
        self.search.search.assert_not_called()


if __name__ == "__main__":
    unittest.main(verbosity=2)
