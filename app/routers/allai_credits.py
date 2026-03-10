"""
allAI Credits Router — Balance + Stripe Checkout proxy
=======================================================

Proxies credit balance/usage and checkout requests to ai.market
via the SerialClient.
"""

import logging

from fastapi import APIRouter, HTTPException, Depends

from app.auth.api_key_auth import get_current_user, AuthenticatedUser
from app.services.serial_client import SerialClient
from app.services.serial_store import get_serial_store, ACTIVE, DEGRADED

logger = logging.getLogger(__name__)

router = APIRouter()


def _require_serial() -> tuple[str, str]:
    """Return (serial, install_token) or raise 409."""
    store = get_serial_store()
    state = store.state
    if state.state not in (ACTIVE, DEGRADED):
        raise HTTPException(
            status_code=409,
            detail="Serial not active. Connect to ai.market first.",
        )
    if not state.serial or not state.install_token:
        raise HTTPException(
            status_code=409,
            detail="Missing serial credentials.",
        )
    return state.serial, state.install_token


@router.get("/credits")
async def get_credits(user: AuthenticatedUser = Depends(get_current_user)):
    """Return allAI credit balance and recent usage."""
    serial, install_token = _require_serial()
    client = SerialClient()
    result = await client.credits_usage(serial, install_token)
    if not result.get("success"):
        raise HTTPException(
            status_code=result.get("status_code", 502),
            detail=result.get("error", "Failed to fetch credits"),
        )
    return result


@router.post("/credits/purchase")
async def purchase_credits(user: AuthenticatedUser = Depends(get_current_user)):
    """Create a Stripe Checkout session and return the checkout URL."""
    serial, install_token = _require_serial()
    client = SerialClient()
    result = await client.credits_checkout(serial, install_token)
    if not result.get("success"):
        raise HTTPException(
            status_code=result.get("status_code", 502),
            detail=result.get("error", "Failed to create checkout session"),
        )
    return result
