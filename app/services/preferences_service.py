"""
User Preferences Service
========================

Manages user preferences for Allie personality settings.
Singleton pattern - one preferences record per installation.

Phase: 3.W.5
Created: 2026-01-25
"""

import logging

from sqlmodel import Session as DBSession

from app.models.state import UserPreferences, UserPreferencesUpdate, UserPreferencesRead

logger = logging.getLogger(__name__)


class UserPreferencesService:
    """
    Service for managing user preferences.

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
        Get preferences safe for returning to frontend.

        Returns:
            UserPreferencesRead
        """
        prefs = self.get_preferences()
        return UserPreferencesRead(
            id=prefs.id,
            user_id=prefs.user_id,
            system_prompt_override=prefs.system_prompt_override,
            tone_mode=prefs.tone_mode,
            quiet_mode=prefs.quiet_mode,
            has_seen_intro=prefs.has_seen_intro,
            updated_at=prefs.updated_at,
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

    def reset_to_defaults(self) -> UserPreferences:
        """
        Reset all preferences to defaults.

        Returns:
            Reset UserPreferences
        """
        prefs = self.get_preferences()

        prefs.system_prompt_override = None
        prefs.tone_mode = "friendly"
        prefs.quiet_mode = False

        self.db.add(prefs)
        self.db.commit()
        self.db.refresh(prefs)

        logger.info("Reset preferences to defaults")
        return prefs


# =============================================================================
# Factory Function
# =============================================================================

def get_preferences_service(db: DBSession) -> UserPreferencesService:
    """Create a UserPreferencesService instance."""
    return UserPreferencesService(db)
