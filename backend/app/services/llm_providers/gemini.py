"""
Gemini LLM Provider
===================

Google Gemini implementation of BaseLLMProvider.
Uses google-genai SDK (v1.0+) with Vertex AI support.

Phase: AG-002
Updated: 2026-02-20
"""

import logging
from typing import AsyncGenerator, Optional, Dict, Any
from google import genai
from google.genai import types

from app.config import settings
from .base import BaseLLMProvider, LLMProviderError, RateLimitError, AuthenticationError

logger = logging.getLogger(__name__)


class GeminiProvider(BaseLLMProvider):
    """
    Gemini LLM provider using google-genai SDK.
    
    Supports:
    - Gemini 1.5 Flash (default)
    - Gemini 1.5 Pro
    - Gemini 2.0 Flash
    - Vertex AI (GCA) mode
    """
    
    def __init__(self):
        self.use_gca = settings.google_genai_use_gca
        self.api_key = settings.gemini_api_key
        self.model_name = settings.llm_model or "gemini-1.5-flash"
        
        if not self.use_gca and not self.api_key:
            raise AuthenticationError(
                "Gemini API key not found. Set VECTORAIZ_GEMINI_API_KEY or use GCA.",
                provider="gemini"
            )
        
        if self.use_gca:
            logger.info("🛡️ GeminiProvider: Initializing with Vertex AI (GCA)")
            self.client = genai.Client(vertexai=True)
        else:
            self.client = genai.Client(api_key=self.api_key)
            
    async def generate(
        self,
        prompt: str,
        system_prompt: Optional[str] = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs
    ) -> str:
        """Generate a complete response using Gemini."""
        try:
            config = types.GenerateContentConfig(
                temperature=temperature,
                max_output_tokens=max_tokens,
                system_instruction=system_prompt,
            )
            
            response = await self.client.aio.models.generate_content(
                model=self.model_name,
                contents=prompt,
                config=config
            )
            
            if not response.candidates:
                raise LLMProviderError(
                    "Generation blocked by safety filters or no candidates returned.",
                    provider="gemini"
                )
            
            return response.text
            
        except Exception as e:
            self._handle_exception(e)

    async def generate_stream(
        self,
        prompt: str,
        system_prompt: Optional[str] = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs
    ) -> AsyncGenerator[str, None]:
        """Stream response chunks from Gemini."""
        try:
            config = types.GenerateContentConfig(
                temperature=temperature,
                max_output_tokens=max_tokens,
                system_instruction=system_prompt,
            )
            
            stream = await self.client.aio.models.generate_content(
                model=self.model_name,
                contents=prompt,
                config=config,
                stream=True
            )
            
            async for chunk in stream:
                if chunk.text:
                    yield chunk.text
                    
        except Exception as e:
            self._handle_exception(e)

    def _handle_exception(self, e: Exception):
        """Map SDK errors to standard provider errors."""
        err_str = str(e).lower()
        
        if "429" in err_str or "resource_exhausted" in err_str or "quota" in err_str:
            raise RateLimitError(
                f"Gemini quota exceeded: {str(e)}",
                provider="gemini",
                original_error=e
            )
        
        if "401" in err_str or "403" in err_str or "auth" in err_str:
            raise AuthenticationError(
                f"Gemini authentication failed: {str(e)}",
                provider="gemini",
                original_error=e
            )
            
        raise LLMProviderError(
            f"Gemini provider error: {str(e)}",
            provider="gemini",
            original_error=e
        )

    def get_model_info(self) -> Dict[str, Any]:
        """Return Gemini model metadata."""
        return {
            "provider": "gemini",
            "model": self.model_name,
            "capabilities": ["generate", "stream", "system_instruction"],
            "max_context_window": 1_000_000 if "1.5" in self.model_name else 128_000,
            "auth_mode": "gca" if self.use_gca else "api_key"
        }
