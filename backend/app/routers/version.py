"""
BQ-VZ-AUTO-UPDATE: Version check and auto-update endpoints.
BQ-VZ-SERIAL-CLIENT: Billing status endpoint.

- GET  /api/version                    — current version + latest available from GHCR
- POST /api/version/update             — trigger Docker-based auto-update (auth required)
- GET  /api/v1/system/billing-status   — serial billing state for frontend
"""

import logging

from fastapi import APIRouter, Depends, Query

from app.auth.api_key_auth import get_current_user, AuthenticatedUser
from app.services.update_service import check_for_updates, trigger_update

logger = logging.getLogger(__name__)

router = APIRouter()


@router.get("/version")
async def get_version(force: bool = Query(False, description="Bypass cache and check GHCR now")):
    """Return current and latest version info. Public endpoint (no auth)."""
    return await check_for_updates(force=force)


@router.post("/version/update")
async def post_update(_user: AuthenticatedUser = Depends(get_current_user)):
    """Trigger auto-update via Docker socket. Requires authentication."""
    return await trigger_update()


@router.get("/v1/system/billing-status")
async def billing_status():
    """
    Return serial billing state for the frontend.

    BQ-VZ-SERIAL-CLIENT: mode, serial, state, remaining credits, payment_enabled.
    Public endpoint (no auth) — only exposes billing metadata, not secrets.
    """
    from app.services.serial_store import get_serial_store, MIGRATED

    store = get_serial_store()
    state = store.state
    cached = state.last_status_cache or {}

    if state.state == MIGRATED:
        mode = "ledger"
    elif state.state in ("active", "degraded"):
        mode = "serial"
    else:
        mode = state.state  # unprovisioned, provisioned

    return {
        "mode": mode,
        "serial": state.serial[:16] + "..." if state.serial else None,
        "state": state.state,
        "setup_remaining_usd": cached.get("setup_remaining_usd"),
        "data_remaining_usd": cached.get("data_remaining_usd"),
        "payment_enabled": cached.get("payment_enabled", False),
        "last_status_at": state.last_status_at,
    }
