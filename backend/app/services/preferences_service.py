"""
User Preferences Service
========================

Manages user preferences for LLM settings.
Singleton pattern - one preferences record per installation.

Phase: 3.W.5
Created: 2026-01-25
"""

import logging
from typing import Optional, Dict, Any

from sqlmodel import Session as DBSession

from app.models.state import UserPreferences, UserPreferencesUpdate, UserPreferencesRead

logger = logging.getLogger(__name__)


class UserPreferencesService:
    """
    Service for managing user LLM preferences.
    
    Uses singleton pattern - only one UserPreferences record (id=1).
    Preferences stored locally, never sent to ai.market.
    """

    def __init__(self, db: DBSession):
        """Initialize with database session."""
        self.db = db

    def get_preferences(self) -> UserPreferences:
        """
        Get current user preferences.
        Creates default if not exists.
        
        Returns:
            UserPreferences object
        """
        prefs = self.db.get(UserPreferences, 1)
        if prefs is None:
            prefs = UserPreferences(id=1)
            self.db.add(prefs)
            self.db.commit()
            self.db.refresh(prefs)
            logger.info("Created default UserPreferences")
        return prefs

    def get_preferences_safe(self) -> UserPreferencesRead:
        """
        Get preferences with API keys masked.
        Safe for returning to frontend.
        
        Returns:
            UserPreferencesRead with key visibility flags
        """
        prefs = self.get_preferences()
        return UserPreferencesRead(
            id=prefs.id,
            llm_provider=prefs.llm_provider,
            llm_model=prefs.llm_model,
            temperature=prefs.temperature,
            max_tokens=prefs.max_tokens,
            system_prompt_override=prefs.system_prompt_override,
            gemini_api_key_set=bool(prefs.gemini_api_key),
            openai_api_key_set=bool(prefs.openai_api_key),
            updated_at=prefs.updated_at
        )

    def update_preferences(self, update: UserPreferencesUpdate) -> UserPreferences:
        """
        Update user preferences.
        Only updates provided fields (partial update).
        
        Args:
            update: Fields to update
            
        Returns:
            Updated UserPreferences
        """
        prefs = self.get_preferences()
        
        update_data = update.model_dump(exclude_unset=True)
        
        for field, value in update_data.items():
            if value is not None:
                setattr(prefs, field, value)
        
        self.db.add(prefs)
        self.db.commit()
        self.db.refresh(prefs)
        
        logger.info(f"Updated preferences: {list(update_data.keys())}")
        return prefs

    def set_llm_provider(self, provider: str, model: Optional[str] = None) -> UserPreferences:
        """
        Switch LLM provider and optionally model.
        
        Args:
            provider: 'gemini' or 'openai'
            model: Optional model name
            
        Returns:
            Updated UserPreferences
        """
        if provider not in ('gemini', 'openai'):
            raise ValueError(f"Invalid provider: {provider}. Use 'gemini' or 'openai'")
        
        prefs = self.get_preferences()
        prefs.llm_provider = provider
        
        if model:
            prefs.llm_model = model
        elif provider == 'gemini' and prefs.llm_model.startswith('gpt'):
            # Auto-switch model when changing provider
            prefs.llm_model = 'gemini-1.5-flash'
        elif provider == 'openai' and prefs.llm_model.startswith('gemini'):
            prefs.llm_model = 'gpt-4-turbo'
        
        self.db.add(prefs)
        self.db.commit()
        self.db.refresh(prefs)
        
        logger.info(f"Switched to provider={provider}, model={prefs.llm_model}")
        return prefs

    def set_api_key(self, provider: str, api_key: str) -> bool:
        """
        Set API key for a provider.
        
        Args:
            provider: 'gemini' or 'openai'
            api_key: The API key value
            
        Returns:
            True if successful
        """
        if provider not in ('gemini', 'openai'):
            raise ValueError(f"Invalid provider: {provider}")
        
        prefs = self.get_preferences()
        
        if provider == 'gemini':
            prefs.gemini_api_key = api_key
        else:
            prefs.openai_api_key = api_key
        
        self.db.add(prefs)
        self.db.commit()
        
        logger.info(f"Updated API key for {provider}")
        return True

    def clear_api_key(self, provider: str) -> bool:
        """
        Clear API key for a provider.
        
        Args:
            provider: 'gemini' or 'openai'
            
        Returns:
            True if successful
        """
        if provider not in ('gemini', 'openai'):
            raise ValueError(f"Invalid provider: {provider}")
        
        prefs = self.get_preferences()
        
        if provider == 'gemini':
            prefs.gemini_api_key = None
        else:
            prefs.openai_api_key = None
        
        self.db.add(prefs)
        self.db.commit()
        
        logger.info(f"Cleared API key for {provider}")
        return True

    def get_active_api_key(self) -> Optional[str]:
        """
        Get the API key for the currently active provider.
        
        Returns:
            API key string or None
        """
        prefs = self.get_preferences()
        
        if prefs.llm_provider == 'gemini':
            return prefs.gemini_api_key
        else:
            return prefs.openai_api_key

    def has_api_key(self, provider: Optional[str] = None) -> bool:
        """
        Check if API key is configured.
        
        Args:
            provider: Check specific provider, or active provider if None
            
        Returns:
            True if API key is set
        """
        prefs = self.get_preferences()
        
        if provider is None:
            provider = prefs.llm_provider
        
        if provider == 'gemini':
            return bool(prefs.gemini_api_key)
        else:
            return bool(prefs.openai_api_key)

    def get_llm_config(self) -> Dict[str, Any]:
        """
        Get LLM configuration for use by LLMService.
        
        Returns:
            Dict with provider, model, temperature, max_tokens, api_key
        """
        prefs = self.get_preferences()
        
        return {
            "provider": prefs.llm_provider,
            "model": prefs.llm_model,
            "temperature": prefs.temperature,
            "max_tokens": prefs.max_tokens,
            "system_prompt": prefs.system_prompt_override,
            "api_key": self.get_active_api_key(),
        }

    def reset_to_defaults(self) -> UserPreferences:
        """
        Reset all preferences to defaults.
        Does NOT clear API keys for safety.
        
        Returns:
            Reset UserPreferences
        """
        prefs = self.get_preferences()
        
        prefs.llm_provider = "gemini"
        prefs.llm_model = "gemini-1.5-flash"
        prefs.temperature = 0.2
        prefs.max_tokens = 1024
        prefs.system_prompt_override = None
        # Keep API keys intact
        
        self.db.add(prefs)
        self.db.commit()
        self.db.refresh(prefs)
        
        logger.info("Reset preferences to defaults (API keys preserved)")
        return prefs


# =============================================================================
# Factory Function
# =============================================================================

def get_preferences_service(db: DBSession) -> UserPreferencesService:
    """Create a UserPreferencesService instance."""
    return UserPreferencesService(db)
