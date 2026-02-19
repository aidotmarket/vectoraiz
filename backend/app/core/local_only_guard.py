"""
Standalone Mode Guard (BQ-128)
==============================

Determines whether the system is running in standalone (air-gapped) mode.
In standalone mode, Allie is completely disabled — no ai.market = no Claude.

Usage:
    from app.core.local_only_guard import is_local_only

    if is_local_only():
        raise AllieDisabledError(...)
"""

from app.config import settings


def is_local_only() -> bool:
    """Return True if running in standalone (air-gapped) mode."""
    return settings.mode == "standalone"
