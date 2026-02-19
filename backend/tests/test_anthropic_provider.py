"""
Tests for Anthropic Claude LLM Provider (BQ-107)
=================================================

Covers: generate, stream, error mapping, system prompt handling,
factory integration, config literal, and model info.
"""

import os

os.environ["VECTORAIZ_AUTH_ENABLED"] = "false"

import pytest
from unittest.mock import AsyncMock, MagicMock, patch

from app.services.llm_providers.base import (
    LLMProviderError,
    RateLimitError,
    AuthenticationError,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_text_block(text: str):
    """Create a mock Anthropic text content block."""
    block = MagicMock()
    block.text = text
    return block


def _make_response(text: str = "Hello from Claude", model: str = "claude-sonnet-4-20250514"):
    """Create a mock Anthropic Messages response."""
    resp = MagicMock()
    resp.content = [_make_text_block(text)]
    resp.model = model
    resp.stop_reason = "end_turn"
    resp.usage = MagicMock(input_tokens=10, output_tokens=5)
    return resp


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(autouse=True)
def _set_anthropic_key(monkeypatch):
    """Ensure anthropic_api_key is set for all tests."""
    monkeypatch.setenv("VECTORAIZ_ANTHROPIC_API_KEY", "sk-ant-test-key")
    monkeypatch.setenv("VECTORAIZ_LLM_PROVIDER", "anthropic")


@pytest.fixture
def provider(monkeypatch):
    """Create an AnthropicProvider with mocked settings."""
    # Reset settings singleton to pick up env changes
    from app.config import Settings
    test_settings = Settings()
    monkeypatch.setattr("app.services.llm_providers.anthropic.settings", test_settings)

    from app.services.llm_providers.anthropic import AnthropicProvider
    return AnthropicProvider()


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestAnthropicGenerate:
    """Test non-streaming generation."""

    @pytest.mark.asyncio
    async def test_generate_returns_text(self, provider):
        """generate() should return the text from the API response."""
        mock_response = _make_response("Test response from Claude")
        provider.client.messages.create = AsyncMock(return_value=mock_response)

        result = await provider.generate("Hello Claude")

        assert result == "Test response from Claude"
        provider.client.messages.create.assert_awaited_once()

    @pytest.mark.asyncio
    async def test_generate_passes_params(self, provider):
        """generate() should pass model, max_tokens, temperature, messages."""
        mock_response = _make_response()
        provider.client.messages.create = AsyncMock(return_value=mock_response)

        await provider.generate("Hi", temperature=0.5, max_tokens=512)

        call_kwargs = provider.client.messages.create.call_args[1]
        assert call_kwargs["temperature"] == 0.5
        assert call_kwargs["max_tokens"] == 512
        assert call_kwargs["messages"] == [{"role": "user", "content": "Hi"}]


class TestAnthropicStream:
    """Test streaming generation."""

    @pytest.mark.asyncio
    async def test_stream_yields_chunks(self, provider):
        """generate_stream() should yield text chunks from text_stream."""
        chunks = ["Hello", " ", "world"]

        # Build an async iterator for text_stream
        async def mock_text_stream():
            for chunk in chunks:
                yield chunk

        mock_stream = AsyncMock()
        mock_stream.text_stream = mock_text_stream()
        mock_stream.__aenter__ = AsyncMock(return_value=mock_stream)
        mock_stream.__aexit__ = AsyncMock(return_value=False)

        provider.client.messages.stream = MagicMock(return_value=mock_stream)

        result = []
        async for text in provider.generate_stream("Tell me a story"):
            result.append(text)

        assert result == ["Hello", " ", "world"]


class TestAnthropicErrors:
    """Test error mapping to base exceptions."""

    @pytest.mark.asyncio
    async def test_auth_error(self, provider):
        """Invalid API key should raise AuthenticationError."""
        from anthropic import AuthenticationError as AnthropicAuthError

        provider.client.messages.create = AsyncMock(
            side_effect=AnthropicAuthError(
                message="Invalid API Key",
                response=MagicMock(status_code=401),
                body=None,
            )
        )

        with pytest.raises(AuthenticationError) as exc_info:
            await provider.generate("Hello")

        assert exc_info.value.provider == "anthropic"

    @pytest.mark.asyncio
    async def test_rate_limit_error(self, provider):
        """429 response should raise RateLimitError."""
        from anthropic import RateLimitError as AnthropicRateLimitError

        provider.client.messages.create = AsyncMock(
            side_effect=AnthropicRateLimitError(
                message="Rate limit exceeded",
                response=MagicMock(status_code=429),
                body=None,
            )
        )

        with pytest.raises(RateLimitError) as exc_info:
            await provider.generate("Hello")

        assert exc_info.value.provider == "anthropic"

    @pytest.mark.asyncio
    async def test_api_error(self, provider):
        """Generic API errors should raise LLMProviderError."""
        from anthropic import APIError

        provider.client.messages.create = AsyncMock(
            side_effect=APIError(
                message="Server error",
                request=MagicMock(),
                body=None,
            )
        )

        with pytest.raises(LLMProviderError) as exc_info:
            await provider.generate("Hello")

        assert exc_info.value.provider == "anthropic"


class TestAnthropicSystemPrompt:
    """Test system prompt handling — must be separate param, NOT in messages."""

    @pytest.mark.asyncio
    async def test_system_prompt_passed_as_separate_param(self, provider):
        """system_prompt should be passed as 'system' kwarg, not in messages list."""
        mock_response = _make_response()
        provider.client.messages.create = AsyncMock(return_value=mock_response)

        await provider.generate("Hello", system_prompt="You are a helpful assistant.")

        call_kwargs = provider.client.messages.create.call_args[1]
        # System prompt is a separate kwarg
        assert call_kwargs["system"] == "You are a helpful assistant."
        # Messages should only contain the user message, no system role
        for msg in call_kwargs["messages"]:
            assert msg["role"] != "system"

    @pytest.mark.asyncio
    async def test_no_system_key_when_none(self, provider):
        """When system_prompt is None, 'system' key should not be in kwargs."""
        mock_response = _make_response()
        provider.client.messages.create = AsyncMock(return_value=mock_response)

        await provider.generate("Hello")

        call_kwargs = provider.client.messages.create.call_args[1]
        assert "system" not in call_kwargs


class TestAnthropicFactory:
    """Test LLM service factory creates AnthropicProvider."""

    def test_factory_creates_anthropic_provider(self, monkeypatch):
        """LLMService with 'anthropic' should create AnthropicProvider."""
        from app.services.llm_providers.anthropic import AnthropicProvider

        monkeypatch.setenv("VECTORAIZ_ANTHROPIC_API_KEY", "sk-ant-test-key")
        monkeypatch.setenv("VECTORAIZ_LLM_PROVIDER", "anthropic")

        # Re-create settings to pick up env vars
        from app.config import Settings
        test_settings = Settings()
        monkeypatch.setattr("app.services.llm_service.settings", test_settings)
        monkeypatch.setattr("app.services.llm_providers.anthropic.settings", test_settings)

        from app.services.llm_service import LLMService
        service = LLMService(provider_override="anthropic")

        assert isinstance(service.provider, AnthropicProvider)


class TestAnthropicConfig:
    """Test config accepts 'anthropic' literal."""

    def test_config_accepts_anthropic(self, monkeypatch):
        """Settings should accept 'anthropic' as llm_provider value."""
        monkeypatch.setenv("VECTORAIZ_LLM_PROVIDER", "anthropic")
        monkeypatch.setenv("VECTORAIZ_ANTHROPIC_API_KEY", "sk-ant-test")

        from app.config import Settings
        s = Settings()

        assert s.llm_provider == "anthropic"
        assert s.anthropic_api_key == "sk-ant-test"


class TestAnthropicModelInfo:
    """Test get_model_info returns correct metadata."""

    def test_get_model_info_default(self, provider):
        """get_model_info should return provider, model, and context window."""
        info = provider.get_model_info()

        assert info["provider"] == "anthropic"
        assert info["model"] == provider.model_name
        assert info["max_context_window"] == 200_000
        assert "generate" in info["capabilities"]
        assert "stream" in info["capabilities"]


class TestAnthropicMissingKey:
    """Test that missing API key raises AuthenticationError."""

    def test_missing_key_raises(self, monkeypatch):
        """AnthropicProvider() without key should raise AuthenticationError."""
        monkeypatch.delenv("VECTORAIZ_ANTHROPIC_API_KEY", raising=False)

        from app.config import Settings
        test_settings = Settings()
        monkeypatch.setattr("app.services.llm_providers.anthropic.settings", test_settings)

        from app.services.llm_providers.anthropic import AnthropicProvider

        with pytest.raises(AuthenticationError):
            AnthropicProvider()
