"""
LLM Provider Base Class
=======================

Abstract base class and exceptions for LLM providers.
Enforces a consistent interface for generation and streaming.

Phase: 3.V.3
Created: 2026-01-25
"""

from abc import ABC, abstractmethod
from typing import AsyncGenerator, Optional, Dict, Any


class LLMProviderError(Exception):
    """Base exception for LLM provider errors."""
    
    def __init__(self, message: str, provider: str = "unknown", original_error: Exception = None):
        self.message = message
        self.provider = provider
        self.original_error = original_error
        super().__init__(self.message)


class RateLimitError(LLMProviderError):
    """Raised when provider rate limit is exceeded."""
    pass


class AuthenticationError(LLMProviderError):
    """Raised when API key is invalid or missing."""
    pass


class BaseLLMProvider(ABC):
    """
    Abstract base class for LLM providers.
    Enforces a consistent interface for generation and streaming.
    
    Implementations must handle:
    - Non-streaming generation
    - Streaming generation
    - System prompts/instructions
    - Error mapping to common exceptions
    """
    
    @abstractmethod
    async def generate(
        self,
        prompt: str,
        system_prompt: Optional[str] = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs
    ) -> str:
        """
        Generate a complete response string.
        
        Args:
            prompt: The user prompt to respond to
            system_prompt: Optional system instructions
            temperature: Sampling temperature (0.0-1.0)
            max_tokens: Maximum tokens in response
            
        Returns:
            Generated text response
            
        Raises:
            LLMProviderError: On generation failure
        """
        pass

    @abstractmethod
    async def generate_stream(
        self,
        prompt: str,
        system_prompt: Optional[str] = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs
    ) -> AsyncGenerator[str, None]:
        """
        Yield response chunks as they are generated.
        
        Args:
            prompt: The user prompt to respond to
            system_prompt: Optional system instructions
            temperature: Sampling temperature (0.0-1.0)
            max_tokens: Maximum tokens in response
            
        Yields:
            Text chunks as they are generated
            
        Raises:
            LLMProviderError: On generation failure
        """
        pass
    
    @abstractmethod
    def get_model_info(self) -> Dict[str, Any]:
        """
        Return metadata about the configured model.
        
        Returns:
            Dict with provider, model name, and capabilities
        """
        pass
