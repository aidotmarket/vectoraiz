"""
BQ-VZ-SHARED-SEARCH: Portal Models — Access Code Validator & Config Store
==========================================================================

Mandate M5: Access code hardening (min 6 chars alphanumeric, bcrypt, rate limiting).
Config persisted to /data/portal_config.json (same pattern as serial_store.py).
"""

import json
import logging
import os
import stat
import tempfile
from contextlib import contextmanager
from threading import RLock
from datetime import datetime, timedelta
from pathlib import Path
from typing import Optional, Dict, List

import bcrypt

from app.config import settings
from app.schemas.portal import PortalConfig

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Rate limiting (in-memory, resets on restart — acceptable for local VZ)
# ---------------------------------------------------------------------------
_access_attempts: Dict[str, List[datetime]] = {}
MAX_ATTEMPTS = 5
WINDOW_MINUTES = 15


class AccessCodeValidator:
    """Shared access code validation and hashing (Mandate M5)."""

    @staticmethod
    def validate_strength(code: str) -> bool:
        """Min 6 chars, alphanumeric. No pure-numeric PINs."""
        if len(code) < 6:
            return False
        if code.isdigit():
            return False  # Must include at least one letter
        return code.isalnum()

    @staticmethod
    def hash_code(code: str) -> str:
        return bcrypt.hashpw(code.encode(), bcrypt.gensalt()).decode()

    @staticmethod
    def verify_code(code: str, hashed: str) -> bool:
        return bcrypt.checkpw(code.encode(), hashed.encode())

    @staticmethod
    def check_rate_limit(ip: str) -> bool:
        """5 attempts per 15 minutes per IP. Returns True if allowed."""
        now = datetime.utcnow()
        cutoff = now - timedelta(minutes=WINDOW_MINUTES)
        attempts = _access_attempts.get(ip, [])
        attempts = [t for t in attempts if t > cutoff]
        _access_attempts[ip] = attempts
        return len(attempts) < MAX_ATTEMPTS

    @staticmethod
    def record_attempt(ip: str):
        now = datetime.utcnow()
        _access_attempts.setdefault(ip, []).append(now)

    @staticmethod
    def clear_rate_limits():
        """Clear all rate limit state (for testing)."""
        _access_attempts.clear()

    @staticmethod
    def invalidate_sessions_on_rotation(config: PortalConfig) -> PortalConfig:
        """Increment portal_session_version, clear active sessions (SS-C1)."""
        config.portal_session_version += 1
        config.active_sessions.clear()
        return config


# ---------------------------------------------------------------------------
# Portal Config Store — persisted JSON file
# ---------------------------------------------------------------------------
_PORTAL_CONFIG_PATH = Path(settings.data_directory) / "portal_config.json"
_cached_config: Optional[PortalConfig] = None


# Serializes local read/modify/publish operations; never held across an await.
_config_lock = RLock()
_config_generation = 0


class PortalConfigConflictError(RuntimeError):
    """A detached working copy predates another committed write."""


def get_portal_config() -> PortalConfig:
    """Return a detached working copy of the committed configuration."""
    global _cached_config
    with _config_lock:
        if _cached_config is None:
            if _PORTAL_CONFIG_PATH.exists():
                try:
                    data = json.loads(_PORTAL_CONFIG_PATH.read_text())
                    _cached_config = PortalConfig(**data)
                    logger.info("Loaded portal config from %s", _PORTAL_CONFIG_PATH)
                except Exception as e:
                    logger.warning("Failed to load portal config: %s", e)
            if _cached_config is None:
                _cached_config = PortalConfig()
        config = _cached_config.model_copy(deep=True)
        config._store_generation = _config_generation
        return config


def save_portal_config(config: PortalConfig) -> None:
    """Atomically persist a detached copy, then publish it to local readers."""
    global _cached_config, _config_generation
    with _config_lock:
        if config._store_generation is not None and config._store_generation != _config_generation:
            raise PortalConfigConflictError("Portal configuration changed; reload before saving")
        committed = config.model_copy(deep=True)
        payload = json.dumps(committed.model_dump(), indent=2, default=str)
        temporary_path = None
        try:
            _PORTAL_CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
            # mkstemp starts private; retain stricter owner permissions on replacement.
            mode = 0o600
            if _PORTAL_CONFIG_PATH.exists():
                mode &= stat.S_IMODE(_PORTAL_CONFIG_PATH.stat().st_mode)
            fd, temporary_path = tempfile.mkstemp(
                prefix=f".{_PORTAL_CONFIG_PATH.name}.", suffix=".tmp",
                dir=_PORTAL_CONFIG_PATH.parent,
            )
            with os.fdopen(fd, "w") as stream:
                os.fchmod(stream.fileno(), mode)
                stream.write(payload)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary_path, _PORTAL_CONFIG_PATH)
            temporary_path = None
        except OSError as e:
            logger.error("Failed to save portal config: %s", e)
            raise
        finally:
            if temporary_path is not None:
                Path(temporary_path).unlink(missing_ok=True)
        _config_generation += 1
        _cached_config = committed
        config._store_generation = _config_generation
        logger.info("Saved portal config to %s", _PORTAL_CONFIG_PATH)


@contextmanager
def portal_config_update():
    """Serialize synchronous updates against the latest committed state.

    Exceptions discard the working copy. Provider awaits must happen outside
    this context, followed by a fresh read and eligibility check inside it.
    """
    with _config_lock:
        config = get_portal_config()
        yield config
        save_portal_config(config)


def reset_portal_config_cache():
    """Reset in-memory cache (for testing); invalidate outstanding snapshots."""
    global _cached_config, _config_generation
    with _config_lock:
        _cached_config = None
        _config_generation += 1
