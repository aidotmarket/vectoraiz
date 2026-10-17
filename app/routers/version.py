"""
BQ-VZ-AUTO-UPDATE: Version check and auto-update endpoints.
BQ-VZ-SERIAL-CLIENT: Billing status endpoint.

- GET  /api/version                    — current version + latest available from GHCR
- POST /api/version/update             — submit chosen release to host owner (auth required)
- GET  /api/v1/system/billing-status   — serial billing state for frontend
"""

import logging

from fastapi import APIRouter, Depends, Query, HTTPException
from fastapi.responses import JSONResponse
from pydantic import BaseModel, Field

from app.auth.api_key_auth import get_current_user, AuthenticatedUser
from app.services.update_service import check_for_updates, trigger_update, owner_status
from app.services.update_owner_client import OwnerError

logger = logging.getLogger(__name__)

router = APIRouter()


@router.get("/version")
async def get_version(force: bool = Query(False, description="Bypass cache and check GHCR now")):
    """Return current and latest version info. Public endpoint (no auth)."""
    return await check_for_updates(force=force)


class UpdateRequest(BaseModel):
    target: str = Field(min_length=5, max_length=80, pattern=r"^v?[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$")
    idempotency_key: str = Field(min_length=16, max_length=128, pattern=r"^[a-zA-Z0-9_-]+$")


def _authorize_update(user: AuthenticatedUser):
    # Existing authentication supplies the principal; a request cannot assert it.
    if "admin" not in user.scopes and "update:submit" not in user.scopes:
        raise HTTPException(403, detail="Update permission required.")


@router.post("/version/update")
async def post_update(body: UpdateRequest, user: AuthenticatedUser = Depends(get_current_user)):
    _authorize_update(user)
    try:
        result = await trigger_update(body.target, body.idempotency_key, str(user.user_id))
        return JSONResponse(status_code=202, content=result)
    except OwnerError as error:
        return JSONResponse(status_code=error.http_status, content={
            "status": error.code, "idempotency_key": body.idempotency_key,
            "message": "Keep this request key for lookup. Use the original host launcher or owner status CLI if the app is unreachable.",
        })


@router.get("/version/update/operations/{identifier}")
async def update_operation(identifier: str, user: AuthenticatedUser = Depends(get_current_user)):
    # Read permission does not authorize submission or reveal pairing material.
    try:
        return await owner_status(identifier)
    except OwnerError as error:
        raise HTTPException(error.http_status, detail=error.code) from None


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
