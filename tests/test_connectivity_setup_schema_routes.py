"""Generated schema guidance exercised against the source external REST router.

Run with unittest discovery to avoid the full app/DB conftest and ML dependencies:
python -m unittest discover -s tests -p test_connectivity_setup_schema_routes.py -v
Only orchestrator/metrics peripherals are private fakes; FastAPI, httpx, SQLModel,
response models, router handlers, and setup generator are real source contracts.
"""

import asyncio
import importlib.util
import re
import sys
import types
import unittest
from pathlib import Path
from unittest.mock import patch

import httpx
from fastapi import FastAPI

from app.models.connectivity import ColumnInfo, SchemaResponse
from app.services.connectivity_setup_generator import (
    DEFAULT_BASE_URL,
    ConnectivitySetupGenerator,
)


TOKEN = "<YOUR_CONNECTIVITY_TOKEN>"
BASE_URLS = (DEFAULT_BASE_URL, "https://custom-instance.example:9443")
DATASET_IDS = ("sales2026", "orders_archive", "region-west-42")


class _FixtureOrchestrator:
    def __init__(self):
        self.calls = []
        self.rate_limiter = types.SimpleNamespace(check_ip_blocked=lambda ip: False)
        self.token = object()

    def validate_token(self, raw_token):
        assert raw_token == TOKEN
        return self.token

    async def get_schema(self, token, dataset_id, client_ip):
        assert token is self.token
        self.calls.append(dataset_id)
        return SchemaResponse(
            dataset_id=dataset_id,
            table_name=f"dataset_{dataset_id}",
            row_count=2,
            columns=[ColumnInfo(name="revenue", type="DOUBLE", nullable=False)],
        )


class TestGeneratedSchemaRoutes(unittest.TestCase):
    def setUp(self):
        self.generator = ConnectivitySetupGenerator()
        self.orchestrator = _FixtureOrchestrator()
        # Load the actual router under a private name without replacing app modules
        # for other tests. The boundary fakes exist only during this source import.
        orchestrator = types.ModuleType("app.services.query_orchestrator")
        orchestrator.ConnectivityError = type("FixtureConnectivityError", (Exception,), {})
        orchestrator.get_query_orchestrator = lambda: self.orchestrator
        metrics = types.ModuleType("app.services.connectivity_metrics")
        metrics.get_connectivity_metrics = lambda: types.SimpleNamespace()
        spec = importlib.util.spec_from_file_location(
            "_schema_contract_ext", Path(__file__).resolve().parents[1] / "app/routers/ext.py"
        )
        router_module = importlib.util.module_from_spec(spec)
        with patch.dict(sys.modules, {
            orchestrator.__name__: orchestrator,
            metrics.__name__: metrics,
        }):
            spec.loader.exec_module(router_module)
        self.router = router_module.router
        self.app = FastAPI()
        self.app.include_router(self.router)
        self.canonical_route = next(
            route for route in self.router.routes
            if route.response_model is SchemaResponse and "GET" in route.methods
        )

    def _get(self, url):
        async def request():
            async with httpx.AsyncClient(
                transport=httpx.ASGITransport(app=self.app), base_url=DEFAULT_BASE_URL
            ) as client:
                return await client.get(url, headers={"Authorization": f"Bearer {TOKEN}"})
        return asyncio.run(request())

    def _check_generated_route(self, template, dataset_id):
        url = template.format(dataset_id=dataset_id, id=dataset_id)
        response = self._get(url)
        self.assertEqual(response.status_code, 200, response.text)
        self.assertEqual(
            httpx.URL(url).path,
            self.canonical_route.path.format(dataset_id=dataset_id),
        )
        self.assertEqual(response.json()["dataset_id"], dataset_id)
        self.assertEqual(response.json()["table_name"], f"dataset_{dataset_id}")
        self.assertEqual(response.json()["columns"][0]["name"], "revenue")
        self.assertEqual(response.json()["columns"][0]["type"], "DOUBLE")
        self.assertEqual(self.orchestrator.calls[-1], dataset_id)

    def test_gemini_schema_url(self):
        for base in BASE_URLS:
            config = self.generator.generate("gemini", TOKEN, base)["config"]
            self.assertEqual(config["api_base_url"], f"{base}/api/v1/ext")
            for dataset_id in DATASET_IDS:
                with self.subTest(base=base, dataset_id=dataset_id):
                    self._check_generated_route(config["endpoints"]["schema"], dataset_id)

    def test_generic_rest_relative_schema_path(self):
        for base in BASE_URLS:
            config = self.generator.generate("generic_rest", TOKEN, base)["config"]
            self.assertEqual(config["api_base_url"], f"{base}/api/v1/ext")
            endpoint = config["endpoints"]["schema"]
            self.assertEqual(endpoint["method"], "GET")
            self.assertTrue(endpoint["path"].startswith("/"))
            for dataset_id in DATASET_IDS:
                with self.subTest(base=base, dataset_id=dataset_id):
                    self._check_generated_route(
                        config["api_base_url"] + endpoint["path"], dataset_id
                    )

    def test_generic_llm_heading_and_summary(self):
        for base in BASE_URLS:
            prompt = self.generator.generate("generic_llm", TOKEN, base)["config"]["system_prompt"]
            heading = re.findall(r"^### GET (\S*\{dataset_id\}\S*)$", prompt, re.M)
            summary = re.findall(r"^- \*\*Schema\*\* \(GET (\S+)\):", prompt, re.M)
            self.assertEqual(len(heading), 1)
            self.assertEqual(len(summary), 1)
            for dataset_id in DATASET_IDS:
                for label, template in (
                    ("heading", heading[0]),
                    ("summary", f"{base}/api/v1/ext{summary[0]}"),
                ):
                    with self.subTest(base=base, dataset_id=dataset_id, prompt=label):
                        self._check_generated_route(template, dataset_id)

    def test_legacy_path_is_404_canonical_control_is_200(self):
        for dataset_id in DATASET_IDS:
            with self.subTest(dataset_id=dataset_id):
                self.assertEqual(self._get(f"/api/v1/ext/schema/{dataset_id}").status_code, 404)
                self.assertEqual(self.orchestrator.calls, [])
                self._check_generated_route(
                    DEFAULT_BASE_URL + self.canonical_route.path, dataset_id
                )
                self.orchestrator.calls.clear()

    def test_other_api_paths_and_token_placeholder(self):
        for base in BASE_URLS:
            api = f"{base}/api/v1/ext"
            gemini = self.generator.generate("gemini", TOKEN, base)["config"]
            rest = self.generator.generate("generic_rest", TOKEN, base)["config"]
            for name, method, path in (
                ("list_datasets", "GET", "/datasets"),
                ("search", "POST", "/search"),
                ("sql", "POST", "/sql"),
            ):
                self.assertEqual(gemini["endpoints"][name], api + path)
                self.assertEqual(rest["endpoints"][name], {"method": method, "path": path})
            self.assertEqual(gemini["auth_header"], f"Bearer {TOKEN}")
            self.assertEqual(rest["auth_header"], f"Bearer {TOKEN}")
            prompt = self.generator.generate("generic_llm", TOKEN, base)["config"]["system_prompt"]
            self.assertIn(f"Authorization: Bearer {TOKEN}", prompt)
            for method, path in (("GET", "/datasets"), ("POST", "/search"), ("POST", "/sql")):
                self.assertIn(f"### {method} {api}{path}", prompt)
            gpt = self.generator.generate("openai_custom_gpt", TOKEN, base)["config"]
            self.assertEqual(gpt["openapi_schema_url"], api + "/openapi.json")
            self.assertEqual(gpt["auth_value"], f"Bearer {TOKEN}")
            for platform in ("claude_desktop", "chatgpt_desktop", "cursor", "vscode"):
                config = self.generator.generate(platform, TOKEN, base)["config"]
                self.assertEqual(config, {"mcpServers": {"vectoraiz": {
                    "command": "docker",
                    "args": ["exec", "-i", "vectoraiz", "python", "-m", "app.mcp_server", "--token", TOKEN],
                }}})
