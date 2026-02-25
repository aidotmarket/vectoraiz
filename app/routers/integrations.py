"""
Stripe Connect Integration Router
===================================

PURPOSE:
    Proxy endpoints for Stripe Connect operations. Forwards requests
    to ai.market's Connect endpoints, keeping marketplace communication
    server-side.

ENDPOINTS:
    POST /api/integrations/stripe/onboarding → Initiate Stripe onboarding
    GET  /api/integrations/stripe/status      → Get Connect status
    POST /api/integrations/stripe/login-link  → Get Stripe dashboard info

AUTH:
    All endpoints require authenticated user (X-API-Key validated against
    ai.market). The same key is forwarded to ai.market Connect endpoints.

BQ-103 ST-1 (2026-02-11)
"""

import logging

from fastapi import APIRouter, Depends, HTTPException, Request, status

from app.auth.api_key_auth import AuthenticatedUser, get_current_user
from app.services.stripe_connect_proxy import (
    StripeConnectProxyError,
    stripe_connect_proxy,
)

logger = logging.getLogger(__name__)

router = APIRouter(tags=["integrations"])


def _extract_api_key(request: Request) -> str:
    """
    Extract the user's API key from the request.

    The key was already validated by get_current_user, so we know it's valid.
    We forward it to ai.market for user identification.
    """
    api_key = request.headers.get("X-API-Key")
    if not api_key:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="X-API-Key header required for Stripe operations.",
        )
    return api_key


@router.post("/stripe/onboarding")
async def initiate_onboarding(
    request: Request,
    user: AuthenticatedUser = Depends(get_current_user),
):
    """
    Initiate Stripe Connect onboarding for the authenticated seller.

    Creates a Stripe Standard account (if not exists) and returns a
    hosted onboarding URL. Redirect the user to this URL to complete
    Stripe verification.

    Returns:
        {
            "onboarding_url": "https://connect.stripe.com/...",
            "account_id": "acct_...",
            "type": "hosted"
        }
    """
    api_key = _extract_api_key(request)

    try:
        result = await stripe_connect_proxy.initiate_onboarding(api_key)
        logger.info(
            "Stripe onboarding initiated for user %s, account %s",
            user.user_id,
            result.get("account_id"),
        )
        return result

    except StripeConnectProxyError as exc:
        logger.error(
            "Stripe onboarding failed for user %s: %s",
            user.user_id, exc.detail,
        )
        raise HTTPException(status_code=exc.status_code, detail=exc.detail)


@router.get("/stripe/status")
async def get_stripe_status(
    request: Request,
    user: AuthenticatedUser = Depends(get_current_user),
):
    """
    Get the Stripe Connect status for the authenticated seller.

    Status values:
        - not_connected: No Stripe account linked
        - pending: Account created but onboarding incomplete
        - complete: Fully onboarded, can receive payments

    Returns:
        {
            "status": "not_connected" | "pending" | "complete",
            "account_id": "acct_..." | null,
            "details_submitted": bool,
            "payouts_enabled": bool,
            "charges_enabled": bool,
            "requirements": {...} | null
        }
    """
    api_key = _extract_api_key(request)

    try:
        return await stripe_connect_proxy.get_status(api_key)

    except StripeConnectProxyError as exc:
        logger.error(
            "Stripe status fetch failed for user %s: %s",
            user.user_id, exc.detail,
        )
        raise HTTPException(status_code=exc.status_code, detail=exc.detail)


@router.post("/stripe/login-link")
async def get_stripe_login_link(
    request: Request,
    user: AuthenticatedUser = Depends(get_current_user),
):
    """
    Get Stripe dashboard access for the authenticated seller.

    Note: Standard Connect accounts access their dashboard directly
    at dashboard.stripe.com. This endpoint returns the account_id
    for reference.

    Returns:
        {
            "message": str,
            "account_id": "acct_..."
        }
    """
    api_key = _extract_api_key(request)

    try:
        return await stripe_connect_proxy.get_login_link(api_key)

    except StripeConnectProxyError as exc:
        logger.error(
            "Stripe login link failed for user %s: %s",
            user.user_id, exc.detail,
        )
        raise HTTPException(status_code=exc.status_code, detail=exc.detail)
