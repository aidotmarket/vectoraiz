"""
Gemini LLM Provider
===================

Google Gemini implementation of BaseLLMProvider.
Uses google-generativeai SDK with async support.

Phase: 3.V.3
Created: 2026-01-25
"""

import google.generativeai as genai
from google.api_core import exceptions as google_exceptions
from typing import AsyncGenerator, Optional, Dict, Any

from app.config import settings
from .base import BaseLLMProvider, LLMProviderError, RateLimitError, AuthenticationError


class GeminiProvider(BaseLLMProvider):
    """
    Gemini LLM provider using google-generativeai SDK.
    
    Supports:
    - Gemini 1.5 Flash (default, fast & cost-effective)
    - Gemini 1.5 Pro (higher quality)
    - Gemini 2.0 Flash (experimental)
    - System instructions via system_instruction parameter
    """
    
    def __init__(self):
        if not settings.gemini_api_key:
            raise AuthenticationError(
                "Gemini API key not found. Set VECTORAIZ_GEMINI_API_KEY in environment.",
                provider="gemini"
            )
        
        genai.configure(api_key=settings.gemini_api_key)
        self.model_name = settings.llm_model or "gemini-1.5-flash"
        
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
            model = self._get_model(system_prompt)
            config = genai.GenerationConfig(
                temperature=temperature,
                max_output_tokens=max_tokens
            )
            
            response = await model.generate_content_async(
                prompt,
                generation_config=config
            )
            
            # Handle blocked responses
            if not response.candidates:
                block_reason = "Unknown"
                if response.prompt_feedback and hasattr(response.prompt_feedback, 'block_reason'):
                    block_reason = response.prompt_feedback.block_reason.name
                raise LLMProviderError(
                    f"Generation blocked by safety filters: {block_reason}",
                    provider="gemini"
                )
            
            return response.text
            
        except google_exceptions.ResourceExhausted as e:
            raise RateLimitError(
                "Gemini API quota exceeded. Please try again later.",
                provider="gemini",
                original_error=e
            )
        except google_exceptions.InvalidArgument as e:
            raise LLMProviderError(
                f"Invalid request to Gemini: {str(e)}",
                provider="gemini",
                original_error=e
            )
        except LLMProviderError:
            raise
        except Exception as e:
            raise LLMProviderError(
                f"Gemini generation failed: {str(e)}",
                provider="gemini",
                original_error=e
            )

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
            model = self._get_model(system_prompt)
            config = genai.GenerationConfig(
                temperature=temperature,
                max_output_tokens=max_tokens
            )
            
            response = await model.generate_content_async(
                prompt,
                generation_config=config,
                stream=True
            )
            
            async for chunk in response:
                if chunk.text:
                    yield chunk.text
                    
        except google_exceptions.ResourceExhausted as e:
            raise RateLimitError(
                "Gemini API quota exceeded during streaming.",
                provider="gemini",
                original_error=e
            )
        except Exception as e:
            raise LLMProviderError(
                f"Gemini streaming failed: {str(e)}",
                provider="gemini",
                original_error=e
            )

    def _get_model(self, system_prompt: Optional[str] = None):
        """Get GenerativeModel instance with optional system instruction."""
        if system_prompt:
            return genai.GenerativeModel(
                model_name=self.model_name,
                system_instruction=system_prompt
            )
        return genai.GenerativeModel(model_name=self.model_name)

    def get_model_info(self) -> Dict[str, Any]:
        """Return Gemini model metadata."""
        return {
            "provider": "gemini",
            "model": self.model_name,
            "capabilities": ["generate", "stream", "system_instruction"],
            "max_context_window": 1_000_000 if "1.5" in self.model_name else 128_000,
        }
