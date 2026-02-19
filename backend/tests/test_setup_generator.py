"""
Tests for ConnectivitySetupGenerator — platform-specific config generation.

Covers:
- Each platform generates valid config structure
- Claude Desktop has correct mcpServers format
- Generic LLM prompt includes dataset info
- Token is properly inserted into configs
- Troubleshooting tips present for each platform

PHASE: BQ-MCP-RAG Phase 2 Tests
CREATED: S136
"""

import json

import pytest

from app.services.connectivity_setup_generator import (
    ConnectivitySetupGenerator,
    SUPPORTED_PLATFORMS,
)

TEST_TOKEN = "vzmcp_testAbCd_0123456789abcdef0123456789abcdef"
TEST_BASE_URL = "http://localhost:8100"
TEST_DATASETS = [
    {
        "id": "ds001",
        "name": "sales_data.csv",
        "row_count": 5000,
        "column_count": 12,
        "description": "Monthly sales records",
    },
    {
        "id": "ds002",
        "name": "customers.parquet",
        "row_count": 10000,
        "column_count": 8,
        "description": "",
    },
]


@pytest.fixture
def generator():
    return ConnectivitySetupGenerator()


# =====================================================================
# General structure tests
# =====================================================================

class TestGeneralStructure:
    """Every platform output has required keys."""

    @pytest.mark.parametrize("platform", sorted(SUPPORTED_PLATFORMS))
    def test_output_has_required_keys(self, generator, platform):
        result = generator.generate(
            platform=platform, token=TEST_TOKEN, base_url=TEST_BASE_URL,
            datasets=TEST_DATASETS,
        )
        assert "platform" in result
        assert "title" in result
        assert "steps" in result
        assert "troubleshooting" in result
        assert "notes" in result

    @pytest.mark.parametrize("platform", sorted(SUPPORTED_PLATFORMS))
    def test_steps_are_numbered(self, generator, platform):
        result = generator.generate(
            platform=platform, token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        steps = result["steps"]
        assert len(steps) >= 2
        for i, step in enumerate(steps, 1):
            assert step["step"] == i

    @pytest.mark.parametrize("platform", sorted(SUPPORTED_PLATFORMS))
    def test_troubleshooting_tips_present(self, generator, platform):
        result = generator.generate(
            platform=platform, token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        assert len(result["troubleshooting"]) >= 1

    @pytest.mark.parametrize("platform", sorted(SUPPORTED_PLATFORMS))
    def test_notes_present(self, generator, platform):
        result = generator.generate(
            platform=platform, token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        assert len(result["notes"]) >= 1


# =====================================================================
# Claude Desktop
# =====================================================================

class TestClaudeDesktop:
    def test_config_has_mcp_servers(self, generator):
        result = generator.generate(
            platform="claude_desktop", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        config = result["config"]
        assert "mcpServers" in config
        assert "vectoraiz" in config["mcpServers"]

    def test_config_has_docker_command(self, generator):
        result = generator.generate(
            platform="claude_desktop", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        server = result["config"]["mcpServers"]["vectoraiz"]
        assert server["command"] == "docker"
        assert "exec" in server["args"]

    def test_token_in_config(self, generator):
        result = generator.generate(
            platform="claude_desktop", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        config_str = json.dumps(result["config"])
        assert TEST_TOKEN in config_str

    def test_config_path_present(self, generator):
        result = generator.generate(
            platform="claude_desktop", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        assert "config_path" in result
        assert "macos" in result["config_path"]
        assert "windows" in result["config_path"]

    def test_has_validation_checkpoints(self, generator):
        result = generator.generate(
            platform="claude_desktop", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        validations = [s for s in result["steps"] if "validation" in s]
        assert len(validations) >= 2, "Expected at least 2 validation checkpoints"


# =====================================================================
# ChatGPT Desktop
# =====================================================================

class TestChatGPTDesktop:
    def test_config_has_mcp_servers(self, generator):
        result = generator.generate(
            platform="chatgpt_desktop", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        config = result["config"]
        assert "mcpServers" in config

    def test_token_in_config(self, generator):
        result = generator.generate(
            platform="chatgpt_desktop", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        config_str = json.dumps(result["config"])
        assert TEST_TOKEN in config_str


# =====================================================================
# Cursor
# =====================================================================

class TestCursor:
    def test_config_has_mcp_servers(self, generator):
        result = generator.generate(
            platform="cursor", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        assert "mcpServers" in result["config"]

    def test_token_in_config(self, generator):
        result = generator.generate(
            platform="cursor", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        assert TEST_TOKEN in json.dumps(result["config"])


# =====================================================================
# Generic REST
# =====================================================================

class TestGenericRest:
    def test_curl_examples_present(self, generator):
        result = generator.generate(
            platform="generic_rest", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        steps_str = json.dumps(result["steps"])
        assert "curl" in steps_str

    def test_all_endpoints_documented(self, generator):
        result = generator.generate(
            platform="generic_rest", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        config = result["config"]
        endpoints = config["endpoints"]
        assert "list_datasets" in endpoints
        assert "search" in endpoints
        assert "sql" in endpoints
        assert "schema" in endpoints

    def test_token_in_auth_header(self, generator):
        result = generator.generate(
            platform="generic_rest", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        assert TEST_TOKEN in result["config"]["auth_header"]


# =====================================================================
# Generic LLM (System Prompt)
# =====================================================================

class TestGenericLLM:
    def test_system_prompt_generated(self, generator):
        result = generator.generate(
            platform="generic_llm", token=TEST_TOKEN, base_url=TEST_BASE_URL,
            datasets=TEST_DATASETS,
        )
        config = result["config"]
        assert "system_prompt" in config
        assert len(config["system_prompt"]) > 100

    def test_system_prompt_includes_dataset_names(self, generator):
        result = generator.generate(
            platform="generic_llm", token=TEST_TOKEN, base_url=TEST_BASE_URL,
            datasets=TEST_DATASETS,
        )
        prompt = result["config"]["system_prompt"]
        assert "sales_data.csv" in prompt
        assert "customers.parquet" in prompt

    def test_system_prompt_includes_dataset_details(self, generator):
        result = generator.generate(
            platform="generic_llm", token=TEST_TOKEN, base_url=TEST_BASE_URL,
            datasets=TEST_DATASETS,
        )
        prompt = result["config"]["system_prompt"]
        assert "5000" in prompt  # row count
        assert "dataset_ds001" in prompt  # table name

    def test_system_prompt_includes_token(self, generator):
        result = generator.generate(
            platform="generic_llm", token=TEST_TOKEN, base_url=TEST_BASE_URL,
            datasets=TEST_DATASETS,
        )
        prompt = result["config"]["system_prompt"]
        assert TEST_TOKEN in prompt

    def test_system_prompt_includes_api_url(self, generator):
        result = generator.generate(
            platform="generic_llm", token=TEST_TOKEN, base_url=TEST_BASE_URL,
            datasets=TEST_DATASETS,
        )
        prompt = result["config"]["system_prompt"]
        assert f"{TEST_BASE_URL}/api/v1/ext" in prompt

    def test_system_prompt_includes_endpoints(self, generator):
        result = generator.generate(
            platform="generic_llm", token=TEST_TOKEN, base_url=TEST_BASE_URL,
            datasets=TEST_DATASETS,
        )
        prompt = result["config"]["system_prompt"]
        assert "/datasets" in prompt
        assert "/search" in prompt
        assert "/sql" in prompt
        assert "/schema" in prompt

    def test_system_prompt_no_datasets_message(self, generator):
        result = generator.generate(
            platform="generic_llm", token=TEST_TOKEN, base_url=TEST_BASE_URL,
            datasets=[],
        )
        prompt = result["config"]["system_prompt"]
        assert "No datasets currently available" in prompt

    def test_system_prompt_includes_description(self, generator):
        result = generator.generate(
            platform="generic_llm", token=TEST_TOKEN, base_url=TEST_BASE_URL,
            datasets=TEST_DATASETS,
        )
        prompt = result["config"]["system_prompt"]
        assert "Monthly sales records" in prompt


# =====================================================================
# Unknown platform
# =====================================================================

class TestUnknownPlatform:
    def test_unknown_platform_returns_error(self, generator):
        result = generator.generate(
            platform="unknown_platform", token=TEST_TOKEN, base_url=TEST_BASE_URL,
        )
        assert result["platform"] == "unknown_platform"
        assert "not supported" in json.dumps(result["steps"]).lower()
