"""
LLM Service
===========

Main entry point for LLM interactions in vectorAIz.
Acts as a factory and facade for specific providers.

Supports "Bring Your Own Key" (BYO-Key) model where users
provide their own Gemini or OpenAI API keys.

BQ-125: Async lazy-load with 60s TTL cache for DB-backed config.
Resolution order: DB active settings -> ENV vars -> "not configured" error.

Phase: 3.V.3
Created: 2026-01-25
Updated: 2026-02-12 (BQ-125)
"""

import logging
import time
from datetime import datetime
from typing import AsyncGenerator, Optional, Dict, Any, List

from app.config import settings
from app.services.llm_providers.base import BaseLLMProvider, LLMProviderError, AuthenticationError

logger = logging.getLogger(__name__)


class LLMService:
    """
    Main service for LLM interactions.
    Acts as a factory and facade for specific providers.

    BQ-125: Supports DB-backed config with async lazy initialization.
    Resolution order:
      1. DB llm_settings (active instance config)
      2. ENV vars (existing fallback)
      3. Error: "LLM not configured"

    Cache: 60s TTL + DB updated_at check for multi-worker consistency.
    """

    # Class-level cache shared across instances
    _cached_provider: Optional[BaseLLMProvider] = None
    _cache_expires_at: float = 0
    _cache_db_updated_at: Optional[datetime] = None
    _cached_settings_id: Optional[str] = None
    CACHE_TTL_S: int = 60

    def __init__(self, provider_override: Optional[str] = None):
        """
        Initialize LLM service.

        Args:
            provider_override: Optional provider to use instead of config default.
                             When set, skips DB lookup and uses ENV config directly.
        """
        self._provider_override = provider_override
        # If override specified, initialize immediately from ENV (legacy path)
        if provider_override:
            self._override_provider: Optional[BaseLLMProvider] = self._initialize_provider_from_env(provider_override)
        else:
            self._override_provider = None

    @property
    def provider(self) -> Optional[BaseLLMProvider]:
        """Public accessor for the current provider (override or cached)."""
        if self._override_provider:
            return self._override_provider
        return LLMService._cached_provider

    def _initialize_provider_from_env(self, provider_name: Optional[str] = None) -> BaseLLMProvider:
        """Initialize provider from ENV vars (legacy path)."""
        from app.services.llm_providers.gemini import GeminiProvider
        from app.services.llm_providers.openai import OpenAIProvider
        from app.services.llm_providers.anthropic import AnthropicProvider

        provider_type = (provider_name or settings.llm_provider).lower()

        if provider_type == "gemini":
            return GeminiProvider()
        elif provider_type == "openai":
            return OpenAIProvider()
        elif provider_type == "anthropic":
            return AnthropicProvider()
        else:
            raise ValueError(f"Unsupported LLM provider: {provider_type}. Use 'gemini', 'openai', or 'anthropic'.")

    async def _resolve_provider(self) -> BaseLLMProvider:
        """Lazily resolve provider from DB or ENV. Cached with TTL."""
        # If override was set at init, always use it
        if self._override_provider:
            return self._override_provider

        now = time.time()

        # Check if class-level cache is still valid
        if LLMService._cached_provider and now < LLMService._cache_expires_at:
            return LLMService._cached_provider

        # Try DB settings first
        db_settings = self._load_active_settings()
        if db_settings:
            # Compare updated_at to detect changes from other workers
            if (LLMService._cached_provider
                    and LLMService._cache_db_updated_at == db_settings.updated_at):
                # Same config, refresh TTL
                LLMService._cache_expires_at = now + self.CACHE_TTL_S
                return LLMService._cached_provider

            # New or changed config — rebuild provider
            from app.core.llm_key_crypto import decrypt_with_fallback

            api_key = decrypt_with_fallback(
                db_settings.encrypted_key,
                db_settings.key_iv,
                db_settings.key_tag,
                settings.get_secret_key(),
                settings.previous_secret_key,
                db_settings.key_version,
                provider_id=db_settings.provider,
                scope=db_settings.scope,
            )

            provider = self._create_provider_with_key(
                db_settings.provider, api_key, db_settings.model,
            )
            LLMService._cached_provider = provider
            LLMService._cache_db_updated_at = db_settings.updated_at
            LLMService._cached_settings_id = db_settings.id
            LLMService._cache_expires_at = now + self.CACHE_TTL_S
            logger.info("LLMService: loaded provider=%s model=%s from DB", db_settings.provider, db_settings.model)
            return provider

        # Fall back to ENV vars
        try:
            provider = self._initialize_provider_from_env()
            LLMService._cached_provider = provider
            LLMService._cache_db_updated_at = None
            LLMService._cached_settings_id = None
            LLMService._cache_expires_at = now + self.CACHE_TTL_S
            return provider
        except (AuthenticationError, ValueError) as exc:
            raise LLMProviderError(
                "LLM not configured. Add a provider via Settings or set environment variables.",
                provider="none",
                original_error=exc,
            )

    def _load_active_settings(self):
        """Load active instance settings from DB (sync, for use in async context)."""
        try:
            from app.core.database import get_session_context
            from app.models.llm_settings import LLMSettings
            from sqlmodel import select

            with get_session_context() as session:
                stmt = (
                    select(LLMSettings)
                    .where(LLMSettings.scope == "instance")
                    .where(LLMSettings.is_active == True)  # noqa: E712
                )
                row = session.exec(stmt).first()
                if row:
                    # Detach from session by reading all needed attributes
                    session.refresh(row)
                return row
        except Exception as exc:
            logger.debug("DB settings lookup failed (falling back to ENV): %s", exc)
            return None

    def _create_provider_with_key(
        self, provider_name: str, api_key: str, model: str,
    ) -> BaseLLMProvider:
        """Create a provider instance with an explicit API key and model."""
        from app.services.llm_providers.openai import OpenAIProvider
        from app.services.llm_providers.anthropic import AnthropicProvider
        from app.services.llm_providers.gemini import GeminiProvider

        if provider_name == "openai":
            from openai import AsyncOpenAI
            provider = OpenAIProvider.__new__(OpenAIProvider)
            provider.client = AsyncOpenAI(api_key=api_key)
            provider.model_name = model
            return provider

        elif provider_name == "anthropic":
            import anthropic
            provider = AnthropicProvider.__new__(AnthropicProvider)
            provider.client = anthropic.AsyncAnthropic(api_key=api_key)
            provider.model_name = model
            return provider

        elif provider_name == "gemini":
            import google.generativeai as genai
            genai.configure(api_key=api_key)
            provider = GeminiProvider.__new__(GeminiProvider)
            provider.model_name = model
            provider.model = genai.GenerativeModel(model)
            return provider

        raise ValueError(f"Unsupported provider: {provider_name}")

    @classmethod
    def invalidate(cls) -> None:
        """Invalidate the provider cache. Called by settings service after PUT/DELETE."""
        cls._cached_provider = None
        cls._cache_expires_at = 0
        cls._cache_db_updated_at = None
        cls._cached_settings_id = None
        logger.info("LLMService cache invalidated")

    @classmethod
    def get_cached_settings_id(cls) -> Optional[str]:
        """Return the settings_id of the currently cached provider, if any."""
        return cls._cached_settings_id

    async def generate(
        self,
        prompt: str,
        system_prompt: Optional[str] = None,
        temperature: Optional[float] = None,
        max_tokens: Optional[int] = None,
        **kwargs
    ) -> str:
        """Generate a response using the configured provider."""
        provider = await self._resolve_provider()
        # Clamp max_tokens to configured ceiling
        effective_max_tokens = min(max_tokens, settings.llm_max_tokens) if max_tokens else settings.llm_max_tokens
        start = time.time()
        try:
            result = await provider.generate(
                prompt=prompt,
                system_prompt=system_prompt,
                temperature=temperature or settings.llm_temperature,
                max_tokens=effective_max_tokens,
                **kwargs
            )
            latency_ms = int((time.time() - start) * 1000)
            self._log_usage_background("generate", latency_ms, True)
            return result
        except Exception as exc:
            latency_ms = int((time.time() - start) * 1000)
            self._log_usage_background("generate", latency_ms, False, str(type(exc).__name__))
            raise

    async def generate_stream(
        self,
        prompt: str,
        system_prompt: Optional[str] = None,
        temperature: Optional[float] = None,
        max_tokens: Optional[int] = None,
        **kwargs
    ) -> AsyncGenerator[str, None]:
        """Stream a response using the configured provider."""
        provider = await self._resolve_provider()
        # Clamp max_tokens to configured ceiling
        effective_max_tokens = min(max_tokens, settings.llm_max_tokens) if max_tokens else settings.llm_max_tokens
        start = time.time()
        try:
            async for chunk in provider.generate_stream(
                prompt=prompt,
                system_prompt=system_prompt,
                temperature=temperature or settings.llm_temperature,
                max_tokens=effective_max_tokens,
                **kwargs
            ):
                yield chunk
            latency_ms = int((time.time() - start) * 1000)
            self._log_usage_background("generate_stream", latency_ms, True)
        except Exception as exc:
            latency_ms = int((time.time() - start) * 1000)
            self._log_usage_background("generate_stream", latency_ms, False, str(type(exc).__name__))
            raise

    async def generate_with_context(
        self,
        question: str,
        context_chunks: List[str],
        system_prompt: Optional[str] = None,
        **kwargs
    ) -> str:
        """RAG Helper: Constructs a prompt with context and generates a response."""
        numbered_context = []
        for i, chunk in enumerate(context_chunks, 1):
            numbered_context.append(f"[{i}] {chunk}")

        context_text = "\n\n".join(numbered_context)

        rag_prompt = f"""Context information is below.
---------------------
{context_text}
---------------------
Given the context information and not prior knowledge, answer the query.
If the context doesn't contain relevant information, say "I don't have information about that in the provided context."
When citing information, reference the source number in brackets like [1], [2], etc.

Query: {question}
Answer:"""

        default_system = """You are a helpful assistant that answers questions based on provided context.
Always cite your sources using the numbered references [1], [2], etc.
Be accurate and concise. Only use information from the provided context."""

        return await self.generate(
            prompt=rag_prompt,
            system_prompt=system_prompt or default_system,
            **kwargs
        )

    def get_model_info(self) -> Dict[str, Any]:
        """Get metadata about the current LLM provider and model."""
        if self._override_provider:
            info = self._override_provider.get_model_info()
        elif LLMService._cached_provider:
            info = LLMService._cached_provider.get_model_info()
        else:
            return {"provider": "not_configured", "model": "none"}
        info["configured_temperature"] = settings.llm_temperature
        info["configured_max_tokens"] = settings.llm_max_tokens
        return info

    def is_configured(self) -> bool:
        """Check if the LLM service is properly configured."""
        if self._override_provider:
            return True
        if LLMService._cached_provider:
            return True
        # Try DB
        db_settings = self._load_active_settings()
        if db_settings:
            return True
        # Try ENV
        try:
            self._initialize_provider_from_env()
            return True
        except Exception:
            return False

    def _log_usage_background(
        self,
        operation: str,
        latency_ms: int,
        success: bool,
        error_code: Optional[str] = None,
    ) -> None:
        """Log usage to llm_usage_log if we have a settings_id."""
        settings_id = LLMService._cached_settings_id
        if not settings_id:
            return
        try:
            from app.services.llm_settings_service import log_usage
            model_info = LLMService._cached_provider.get_model_info() if LLMService._cached_provider else {}
            log_usage(
                settings_id=settings_id,
                operation=operation,
                model=model_info.get("model", "unknown"),
                latency_ms=latency_ms,
                success=success,
                error_code=error_code,
            )
        except Exception as exc:
            logger.debug("Usage logging failed (non-fatal): %s", exc)


# Singleton instance
_llm_service: Optional[LLMService] = None


def get_llm_service() -> LLMService:
    """Get the singleton LLM service instance."""
    global _llm_service
    if _llm_service is None:
        _llm_service = LLMService()
    return _llm_service


def reset_llm_service():
    """Reset the singleton (useful for testing or config changes)."""
    global _llm_service
    LLMService.invalidate()
    _llm_service = None
