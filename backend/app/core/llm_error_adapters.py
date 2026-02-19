"""
LLM Error Adapters
==================

Provider-specific error normalization for LLM connection testing.
Maps provider exceptions to standard error codes with user-friendly messages.

Phase: BQ-125 — Connect Your LLM
Created: 2026-02-12
"""

from __future__ import annotations

import logging

logger = logging.getLogger(__name__)


class BaseErrorAdapter:
    """Normalize provider-specific errors to standard codes."""

    ERROR_MAP = {
        "auth_failed": "Authentication failed. Check your API key.",
        "rate_limited": "Provider rate limit hit. Try again shortly.",
        "quota_exceeded": "API quota exceeded. Check your provider account.",
        "connection_failed": "Could not reach provider. Check network.",
        "invalid_model": "Model not available. Check model name.",
        "unknown_error": "Connection test failed. Check logs for details.",
    }

    def normalize(self, exc: Exception) -> tuple[str, str]:
        """Returns (error_code, user_message). Override per provider."""
        logger.error("LLM error (unhandled): %s", exc)
        return "unknown_error", self.ERROR_MAP["unknown_error"]


class OpenAIErrorAdapter(BaseErrorAdapter):
    def normalize(self, exc: Exception) -> tuple[str, str]:
        exc_str = str(exc).lower()
        if "401" in exc_str or "invalid_api_key" in exc_str or "incorrect api key" in exc_str:
            return "auth_failed", self.ERROR_MAP["auth_failed"]
        if "429" in exc_str:
            if "quota" in exc_str:
                return "quota_exceeded", self.ERROR_MAP["quota_exceeded"]
            return "rate_limited", self.ERROR_MAP["rate_limited"]
        if "model" in exc_str and ("not found" in exc_str or "does not exist" in exc_str):
            return "invalid_model", self.ERROR_MAP["invalid_model"]
        if "connection" in exc_str or "timeout" in exc_str:
            return "connection_failed", self.ERROR_MAP["connection_failed"]
        return super().normalize(exc)


class AnthropicErrorAdapter(BaseErrorAdapter):
    def normalize(self, exc: Exception) -> tuple[str, str]:
        exc_str = str(exc).lower()
        if "401" in exc_str or "invalid x-api-key" in exc_str or "authentication" in exc_str:
            return "auth_failed", self.ERROR_MAP["auth_failed"]
        if "429" in exc_str:
            if "quota" in exc_str or "credit" in exc_str:
                return "quota_exceeded", self.ERROR_MAP["quota_exceeded"]
            return "rate_limited", self.ERROR_MAP["rate_limited"]
        if "model" in exc_str and "not found" in exc_str:
            return "invalid_model", self.ERROR_MAP["invalid_model"]
        if "connection" in exc_str or "timeout" in exc_str:
            return "connection_failed", self.ERROR_MAP["connection_failed"]
        return super().normalize(exc)


class GeminiErrorAdapter(BaseErrorAdapter):
    def normalize(self, exc: Exception) -> tuple[str, str]:
        exc_str = str(exc).lower()
        if "403" in exc_str or "api_key_invalid" in exc_str or "api key not valid" in exc_str:
            return "auth_failed", self.ERROR_MAP["auth_failed"]
        if "429" in exc_str or "resource_exhausted" in exc_str:
            if "quota" in exc_str:
                return "quota_exceeded", self.ERROR_MAP["quota_exceeded"]
            return "rate_limited", self.ERROR_MAP["rate_limited"]
        if "model" in exc_str and ("not found" in exc_str or "not supported" in exc_str):
            return "invalid_model", self.ERROR_MAP["invalid_model"]
        if "connection" in exc_str or "timeout" in exc_str:
            return "connection_failed", self.ERROR_MAP["connection_failed"]
        return super().normalize(exc)


ERROR_ADAPTERS: dict[str, BaseErrorAdapter] = {
    "openai": OpenAIErrorAdapter(),
    "anthropic": AnthropicErrorAdapter(),
    "gemini": GeminiErrorAdapter(),
}
