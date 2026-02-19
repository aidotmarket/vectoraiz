"""
Anthropic Claude LLM Provider
==============================

Anthropic implementation of BaseLLMProvider.
Uses official anthropic SDK with async support.

Phase: BQ-107
Created: 2026-02-12
"""

from anthropic import (
    AsyncAnthropic,
    APIError,
    RateLimitError as AnthropicRateLimitError,
    AuthenticationError as AnthropicAuthError,
)
from typing import AsyncGenerator, Optional, Dict, Any

from app.config import settings
from .base import BaseLLMProvider, LLMProviderError, RateLimitError, AuthenticationError


SUPPORTED_MODELS = {
    "claude-sonnet-4-20250514": {"context": 200_000},
    "claude-opus-4-20250514": {"context": 200_000},
    "claude-haiku-3-20240307": {"context": 200_000},
}

DEFAULT_MODEL = "claude-sonnet-4-20250514"


class AnthropicProvider(BaseLLMProvider):
    """
    Anthropic Claude LLM provider using official SDK.

    Supports:
    - claude-sonnet-4-20250514 (default, best balance)
    - claude-opus-4-20250514 (highest capability)
    - claude-haiku-3-20240307 (fastest / cheapest)
    - System prompt via separate `system` parameter (Anthropic-native)
    """

    def __init__(self):
        if not settings.anthropic_api_key:
            raise AuthenticationError(
                "Anthropic API key not found. Set VECTORAIZ_ANTHROPIC_API_KEY in environment.",
                provider="anthropic",
            )

        self.client = AsyncAnthropic(api_key=settings.anthropic_api_key)
        self.model_name = settings.llm_model or DEFAULT_MODEL

    async def generate(
        self,
        prompt: str,
        system_prompt: Optional[str] = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs,
    ) -> str:
        """Generate a complete response using Anthropic Messages API."""
        try:
            api_kwargs = self._build_request_kwargs(
                prompt, system_prompt, temperature, max_tokens
            )

            response = await self.client.messages.create(**api_kwargs)
            return self._extract_text(response)

        except AnthropicRateLimitError as e:
            raise RateLimitError(
                "Anthropic API rate limit exceeded. Please try again later.",
                provider="anthropic",
                original_error=e,
            )
        except AnthropicAuthError as e:
            raise AuthenticationError(
                "Anthropic API key is invalid.",
                provider="anthropic",
                original_error=e,
            )
        except APIError as e:
            raise LLMProviderError(
                f"Anthropic API error: {str(e)}",
                provider="anthropic",
                original_error=e,
            )
        except Exception as e:
            raise LLMProviderError(
                f"Anthropic generation failed: {str(e)}",
                provider="anthropic",
                original_error=e,
            )

    async def generate_stream(
        self,
        prompt: str,
        system_prompt: Optional[str] = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs,
    ) -> AsyncGenerator[str, None]:
        """Stream response chunks from Anthropic."""
        try:
            api_kwargs = self._build_request_kwargs(
                prompt, system_prompt, temperature, max_tokens
            )

            async with self.client.messages.stream(**api_kwargs) as stream:
                async for text in stream.text_stream:
                    yield text

        except AnthropicRateLimitError as e:
            raise RateLimitError(
                "Anthropic API rate limit exceeded during streaming.",
                provider="anthropic",
                original_error=e,
            )
        except APIError as e:
            raise LLMProviderError(
                f"Anthropic streaming error: {str(e)}",
                provider="anthropic",
                original_error=e,
            )
        except Exception as e:
            raise LLMProviderError(
                f"Anthropic streaming failed: {str(e)}",
                provider="anthropic",
                original_error=e,
            )

    def get_model_info(self) -> Dict[str, Any]:
        """Return Anthropic model metadata."""
        model_info = SUPPORTED_MODELS.get(self.model_name, {"context": 200_000})

        return {
            "provider": "anthropic",
            "model": self.model_name,
            "capabilities": ["generate", "stream", "chat"],
            "max_context_window": model_info["context"],
        }

    def _build_request_kwargs(
        self,
        prompt: str,
        system_prompt: Optional[str],
        temperature: float,
        max_tokens: int,
    ) -> dict:
        """Build keyword arguments for messages.create / messages.stream."""
        kwargs: dict = {
            "model": self.model_name,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "messages": [{"role": "user", "content": prompt}],
        }
        if system_prompt:
            kwargs["system"] = system_prompt
        return kwargs

    @staticmethod
    def _extract_text(response) -> str:
        """Extract concatenated text from Anthropic Messages response."""
        parts = []
        for block in response.content:
            if hasattr(block, "text"):
                parts.append(block.text)
        return "".join(parts)
