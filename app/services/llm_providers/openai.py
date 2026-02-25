"""
OpenAI LLM Provider
===================

OpenAI implementation of BaseLLMProvider.
Uses official openai SDK with async support.

Phase: 3.V.3
Created: 2026-01-25
"""

from openai import AsyncOpenAI, APIError, RateLimitError as OpenAIRateLimitError, AuthenticationError as OpenAIAuthError
from typing import AsyncGenerator, Optional, Dict, Any

from app.config import settings
from .base import BaseLLMProvider, LLMProviderError, RateLimitError, AuthenticationError


class OpenAIProvider(BaseLLMProvider):
    """
    OpenAI LLM provider using official SDK.
    
    Supports:
    - GPT-4o-mini (default, fast & cost-effective)
    - GPT-4o (higher quality)
    - GPT-4 Turbo
    - System messages via chat completion API
    """
    
    def __init__(self):
        if not settings.openai_api_key:
            raise AuthenticationError(
                "OpenAI API key not found. Set VECTORAIZ_OPENAI_API_KEY in environment.",
                provider="openai"
            )
        
        self.client = AsyncOpenAI(api_key=settings.openai_api_key)
        self.model_name = settings.llm_model or "gpt-4o-mini"

    async def generate(
        self,
        prompt: str,
        system_prompt: Optional[str] = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs
    ) -> str:
        """Generate a complete response using OpenAI Chat Completions."""
        try:
            messages = self._build_messages(prompt, system_prompt)
            
            response = await self.client.chat.completions.create(
                model=self.model_name,
                messages=messages,
                temperature=temperature,
                max_tokens=max_tokens,
                stream=False
            )
            
            return response.choices[0].message.content or ""
            
        except OpenAIRateLimitError as e:
            raise RateLimitError(
                "OpenAI API rate limit exceeded. Please try again later.",
                provider="openai",
                original_error=e
            )
        except OpenAIAuthError as e:
            raise AuthenticationError(
                "OpenAI API key is invalid.",
                provider="openai",
                original_error=e
            )
        except APIError as e:
            raise LLMProviderError(
                f"OpenAI API error: {str(e)}",
                provider="openai",
                original_error=e
            )
        except Exception as e:
            raise LLMProviderError(
                f"OpenAI generation failed: {str(e)}",
                provider="openai",
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
        """Stream response chunks from OpenAI."""
        try:
            messages = self._build_messages(prompt, system_prompt)
            
            stream = await self.client.chat.completions.create(
                model=self.model_name,
                messages=messages,
                temperature=temperature,
                max_tokens=max_tokens,
                stream=True
            )
            
            async for chunk in stream:
                if chunk.choices and chunk.choices[0].delta.content:
                    yield chunk.choices[0].delta.content
                    
        except OpenAIRateLimitError as e:
            raise RateLimitError(
                "OpenAI API rate limit exceeded during streaming.",
                provider="openai",
                original_error=e
            )
        except APIError as e:
            raise LLMProviderError(
                f"OpenAI streaming error: {str(e)}",
                provider="openai",
                original_error=e
            )
        except Exception as e:
            raise LLMProviderError(
                f"OpenAI streaming failed: {str(e)}",
                provider="openai",
                original_error=e
            )

    def _build_messages(self, prompt: str, system_prompt: Optional[str]) -> list:
        """Build chat messages list with optional system prompt."""
        messages = []
        if system_prompt:
            messages.append({"role": "system", "content": system_prompt})
        messages.append({"role": "user", "content": prompt})
        return messages

    def get_model_info(self) -> Dict[str, Any]:
        """Return OpenAI model metadata."""
        context_windows = {
            "gpt-4o": 128_000,
            "gpt-4o-mini": 128_000,
            "gpt-4-turbo": 128_000,
            "gpt-4": 8_192,
            "gpt-3.5-turbo": 16_385,
        }
        
        return {
            "provider": "openai",
            "model": self.model_name,
            "capabilities": ["generate", "stream", "chat", "function_calling"],
            "max_context_window": context_windows.get(self.model_name, 128_000),
        }
