"""
BQ-MCP-RAG Phase 3: Cross-Platform Smoke Tests.

Verifies that `ConnectivitySetupGenerator.generate()` returns valid,
well-structured config for every supported platform.

PHASE: BQ-MCP-RAG Phase 3 Tests
CREATED: S136
"""

import json

import pytest

from app.services.connectivity_setup_generator import (
    ConnectivitySetupGenerator,
    SUPPORTED_PLATFORMS,
)


FAKE_TOKEN = "vzmcp_testABCD_0123456789abcdef0123456789abcdef"
BASE_URL = "http://localhost:8100"


@pytest.fixture
def generator():
    return ConnectivitySetupGenerator()


# =====================================================================
# Helpers
# =====================================================================

def _assert_common_fields(result: dict, platform: str):
    """Every platform result must have these top-level keys."""
    assert result["platform"] == platform
    assert isinstance(result["title"], str) and len(result["title"]) > 0
    assert isinstance(result["steps"], list) and len(result["steps"]) > 0
    for step in result["steps"]:
        assert "step" in step
        assert "instruction" in step


# =====================================================================
# Per-platform smoke tests
# =====================================================================

class TestClaudeDesktopSetup:
    def test_valid_json_config(self, generator):
        result = generator.generate("claude_desktop", token=FAKE_TOKEN, base_url=BASE_URL)
        _assert_common_fields(result, "claude_desktop")

        # Config must be valid JSON-serializable
        config = result["config"]
        assert config is not None
        json_str = json.dumps(config)
        parsed = json.loads(json_str)

        # Must have mcpServers.vectoraiz.command and args
        assert "mcpServers" in parsed
        assert "vectoraiz" in parsed["mcpServers"]
        server = parsed["mcpServers"]["vectoraiz"]
        assert "command" in server
        assert "args" in server
        assert isinstance(server["args"], list)

    def test_config_includes_token(self, generator):
        result = generator.generate("claude_desktop", token=FAKE_TOKEN, base_url=BASE_URL)
        config_str = json.dumps(result["config"])
        assert FAKE_TOKEN in config_str


class TestChatGPTDesktopSetup:
    def test_valid_json_config(self, generator):
        result = generator.generate("chatgpt_desktop", token=FAKE_TOKEN, base_url=BASE_URL)
        _assert_common_fields(result, "chatgpt_desktop")

        config = result["config"]
        assert config is not None
        parsed = json.loads(json.dumps(config))

        assert "mcpServers" in parsed
        assert "vectoraiz" in parsed["mcpServers"]
        server = parsed["mcpServers"]["vectoraiz"]
        assert "command" in server
        assert "args" in server

    def test_config_includes_token(self, generator):
        result = generator.generate("chatgpt_desktop", token=FAKE_TOKEN, base_url=BASE_URL)
        config_str = json.dumps(result["config"])
        assert FAKE_TOKEN in config_str


class TestCursorSetup:
    def test_valid_json_config(self, generator):
        result = generator.generate("cursor", token=FAKE_TOKEN, base_url=BASE_URL)
        _assert_common_fields(result, "cursor")

        config = result["config"]
        assert config is not None
        parsed = json.loads(json.dumps(config))

        assert "mcpServers" in parsed
        server = parsed["mcpServers"]["vectoraiz"]
        assert "command" in server

    def test_config_includes_token(self, generator):
        result = generator.generate("cursor", token=FAKE_TOKEN, base_url=BASE_URL)
        config_str = json.dumps(result["config"])
        assert FAKE_TOKEN in config_str


class TestOpenAICustomGPTSetup:
    def test_valid_json_config(self, generator):
        result = generator.generate("openai_custom_gpt", token=FAKE_TOKEN, base_url=BASE_URL)
        _assert_common_fields(result, "openai_custom_gpt")

        config = result["config"]
        assert config is not None
        parsed = json.loads(json.dumps(config))

        # Must have OpenAPI action config
        assert "openapi_schema_url" in parsed or "api_base_url" in parsed
        assert "auth_type" in parsed

    def test_config_includes_token(self, generator):
        result = generator.generate("openai_custom_gpt", token=FAKE_TOKEN, base_url=BASE_URL)
        config_str = json.dumps(result["config"])
        assert FAKE_TOKEN in config_str


class TestGenericRESTSetup:
    def test_valid_config(self, generator):
        result = generator.generate("generic_rest", token=FAKE_TOKEN, base_url=BASE_URL)
        _assert_common_fields(result, "generic_rest")

        config = result["config"]
        assert config is not None

        # Must have API base URL, auth header, endpoint list
        assert "api_base_url" in config
        assert "auth_header" in config
        assert "endpoints" in config
        assert isinstance(config["endpoints"], dict)

    def test_config_includes_token(self, generator):
        result = generator.generate("generic_rest", token=FAKE_TOKEN, base_url=BASE_URL)
        config_str = json.dumps(result["config"])
        assert FAKE_TOKEN in config_str

    def test_correct_base_url(self, generator):
        result = generator.generate("generic_rest", token=FAKE_TOKEN, base_url=BASE_URL)
        assert "localhost:8100" in result["config"]["api_base_url"]


class TestGenericLLMSetup:
    def test_valid_config(self, generator):
        result = generator.generate("generic_llm", token=FAKE_TOKEN, base_url=BASE_URL)
        _assert_common_fields(result, "generic_llm")

        config = result["config"]
        assert config is not None

        # Must have system prompt text with API details
        assert "system_prompt" in config
        prompt = config["system_prompt"]
        assert isinstance(prompt, str) and len(prompt) > 100

    def test_config_includes_token(self, generator):
        result = generator.generate("generic_llm", token=FAKE_TOKEN, base_url=BASE_URL)
        assert FAKE_TOKEN in result["config"]["system_prompt"]

    def test_correct_base_url(self, generator):
        result = generator.generate("generic_llm", token=FAKE_TOKEN, base_url=BASE_URL)
        assert "localhost:8100" in result["config"]["system_prompt"]


class TestVSCodeSetup:
    def test_valid_json_config(self, generator):
        result = generator.generate("vscode", token=FAKE_TOKEN, base_url=BASE_URL)
        _assert_common_fields(result, "vscode")

        config = result["config"]
        assert config is not None
        parsed = json.loads(json.dumps(config))

        assert "mcpServers" in parsed
        assert "vectoraiz" in parsed["mcpServers"]

    def test_config_includes_token(self, generator):
        result = generator.generate("vscode", token=FAKE_TOKEN, base_url=BASE_URL)
        config_str = json.dumps(result["config"])
        assert FAKE_TOKEN in config_str


# =====================================================================
# Invalid platform
# =====================================================================

class TestInvalidPlatform:
    def test_invalid_platform_returns_error_steps(self, generator):
        result = generator.generate("nonexistent_platform", token=FAKE_TOKEN, base_url=BASE_URL)
        assert result["platform"] == "nonexistent_platform"
        assert result["title"] == "Unknown Platform"
        assert "not supported" in result["steps"][0]["instruction"].lower()

    def test_empty_platform(self, generator):
        result = generator.generate("", token=FAKE_TOKEN, base_url=BASE_URL)
        assert result["title"] == "Unknown Platform"


# =====================================================================
# All platforms covered
# =====================================================================

class TestAllPlatformsCovered:
    """Ensure every platform in SUPPORTED_PLATFORMS is tested."""

    # Platforms that generate MCP-style config (mcpServers key)
    MCP_PLATFORMS = {"claude_desktop", "chatgpt_desktop", "cursor", "vscode"}

    # Platforms that generate REST/API config
    REST_PLATFORMS = {"openai_custom_gpt", "generic_rest", "generic_llm", "gemini"}

    def test_all_supported_platforms_generate_valid_output(self, generator):
        for platform in SUPPORTED_PLATFORMS:
            result = generator.generate(platform, token=FAKE_TOKEN, base_url=BASE_URL)
            assert result["platform"] == platform, f"Platform mismatch for {platform}"
            assert result["title"] != "Unknown Platform", f"Unknown platform: {platform}"
            assert len(result["steps"]) > 0, f"No steps for {platform}"
            assert result["config"] is not None, f"No config for {platform}"

    def test_all_platforms_include_token(self, generator):
        for platform in SUPPORTED_PLATFORMS:
            result = generator.generate(platform, token=FAKE_TOKEN, base_url=BASE_URL)
            config_str = json.dumps(result.get("config", {}))
            assert FAKE_TOKEN in config_str, f"Token not in config for {platform}"
