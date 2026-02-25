"""
Billing & API Key Management Router
=====================================

BQ-098: Provides endpoints for:
1. API Key Management: POST/GET/DELETE /api/api-keys
2. Usage Tracking: GET /api/usage
3. Subscription Management: POST /api/billing/subscribe

BQ-111: API keys now stored in SQL (api_keys table) with
        HMAC-SHA256 hashing using a server pepper.

CREATED: S-BQ098 (2026-02-10)
UPDATED: BQ-111 (2026-02-12) — persistent API key storage
"""

import hashlib
import hmac
import json
import logging
import os
import secrets
from datetime import datetime, timezone
from typing import Dict, List, Optional, Any

from fastapi import APIRouter, Depends, HTTPException, status
from pydantic import BaseModel, Field

from app.auth.api_key_auth import get_current_user, AuthenticatedUser
from app.services.billing_service import billing_service

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Server pepper for HMAC-SHA256 key hashing
# ---------------------------------------------------------------------------
_SERVER_PEPPER: str = os.environ.get("VECTORAIZ_API_KEY_PEPPER", "vectoraiz-default-pepper")

# ---------------------------------------------------------------------------
# Pydantic models (unchanged — same API contract)
# ---------------------------------------------------------------------------

class CreateApiKeyRequest(BaseModel):
    name: str = Field(..., min_length=1, max_length=100, description="Human-readable key name")
    scopes: List[str] = Field(default=["read", "write"], description="Permission scopes")


class ApiKeyResponse(BaseModel):
    key_id: str
    name: str
    prefix: str
    scopes: List[str]
    created_at: str
    last_used: Optional[str] = None


class ApiKeyCreatedResponse(ApiKeyResponse):
    """Returned only on creation — includes the full key (shown once)."""
    api_key: str = Field(..., description="Full API key — store securely, shown only once")


class UsageResponse(BaseModel):
    user_id: str
    total_tokens_input: int
    total_tokens_output: int
    total_tokens: int
    total_cost_cents: int
    record_count: int
    records: List[Dict[str, Any]]
    period_start: Optional[str] = None
    period_end: Optional[str] = None


class SubscribeRequest(BaseModel):
    email: str
    payment_method_id: Optional[str] = None


class SubscribeResponse(BaseModel):
    customer_id: str
    subscription_id: str
    status: str
    current_period_start: Optional[str] = None
    current_period_end: Optional[str] = None


# ---------------------------------------------------------------------------
# Router
# ---------------------------------------------------------------------------
router = APIRouter()


def _hmac_key(raw_key: str) -> str:
    """HMAC-SHA256 of the raw API key with server pepper."""
    return hmac.new(
        _SERVER_PEPPER.encode(),
        raw_key.encode(),
        hashlib.sha256,
    ).hexdigest()


def _get_db_session():
    from app.core.database import get_session_context
    return get_session_context()


# ---------------------------------------------------------------------------
# API Key Management Endpoints
# ---------------------------------------------------------------------------

@router.post(
    "/api-keys",
    response_model=ApiKeyCreatedResponse,
    status_code=status.HTTP_201_CREATED,
    summary="Create a new API key",
    description="Generate a new API key for programmatic access. The full key is returned only once.",
)
async def create_api_key(
    body: CreateApiKeyRequest,
    user: AuthenticatedUser = Depends(get_current_user),
):
    """Create a new API key for the authenticated user."""
    from app.models.api_key import APIKey

    raw_key = f"aim_{secrets.token_urlsafe(32)}"
    key_prefix = raw_key[:8]
    key_hash = _hmac_key(raw_key)
    now = datetime.now(timezone.utc)

    db_key = APIKey(
        user_id=user.user_id,
        key_prefix=key_prefix,
        key_hash=key_hash,
        label=body.name,
        scopes=json.dumps(body.scopes),
        is_active=True,
        created_at=now,
    )

    with _get_db_session() as session:
        session.add(db_key)
        session.commit()
        session.refresh(db_key)
        key_id = str(db_key.id)

    logger.info("API key created: id=%s user=%s name=%s", key_id, user.user_id, body.name)

    return ApiKeyCreatedResponse(
        key_id=key_id,
        name=body.name,
        prefix=key_prefix + "...",
        scopes=body.scopes,
        created_at=now.isoformat(),
        api_key=raw_key,
    )


@router.get(
    "/api-keys",
    response_model=List[ApiKeyResponse],
    summary="List API keys",
    description="List all API keys for the authenticated user (keys are masked).",
)
async def list_api_keys(
    user: AuthenticatedUser = Depends(get_current_user),
):
    """List all API keys belonging to the authenticated user."""
    from app.models.api_key import APIKey
    from sqlmodel import select

    with _get_db_session() as session:
        stmt = (
            select(APIKey)
            .where(APIKey.user_id == user.user_id)
            .where(APIKey.is_active == True)  # noqa: E712
            .order_by(APIKey.created_at.desc())
        )
        rows = session.exec(stmt).all()

    user_keys = []
    for row in rows:
        try:
            scopes = json.loads(row.scopes) if row.scopes else ["read", "write"]
        except (json.JSONDecodeError, TypeError):
            scopes = ["read", "write"]

        user_keys.append(
            ApiKeyResponse(
                key_id=str(row.id),
                name=row.label or "",
                prefix=row.key_prefix + "...",
                scopes=scopes,
                created_at=row.created_at.isoformat() if row.created_at else "",
                last_used=row.last_used_at.isoformat() if row.last_used_at else None,
            )
        )
    return user_keys


@router.delete(
    "/api-keys/{key_id}",
    status_code=status.HTTP_200_OK,
    summary="Delete an API key",
    description="Revoke and delete an API key by ID.",
)
async def delete_api_key(
    key_id: str,
    user: AuthenticatedUser = Depends(get_current_user),
):
    """Delete an API key belonging to the authenticated user."""
    from app.models.api_key import APIKey

    with _get_db_session() as session:
        try:
            row = session.get(APIKey, int(key_id))
        except (ValueError, TypeError):
            row = None

        if not row or not row.is_active:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"API key '{key_id}' not found.",
            )

        if row.user_id != user.user_id:
            raise HTTPException(
                status_code=status.HTTP_403_FORBIDDEN,
                detail="You can only delete your own API keys.",
            )

        # Soft-delete
        row.is_active = False
        row.revoked_at = datetime.now(timezone.utc)
        session.add(row)
        session.commit()

    logger.info("API key revoked: id=%s user=%s", key_id, user.user_id)
    return {"message": f"API key '{key_id}' deleted successfully."}


# ---------------------------------------------------------------------------
# Usage Endpoint
# ---------------------------------------------------------------------------

@router.get(
    "/usage",
    response_model=UsageResponse,
    summary="Get usage summary",
    description="Returns token consumption, cost, and processing history for the authenticated user.",
)
async def get_usage(
    limit: int = 100,
    user: AuthenticatedUser = Depends(get_current_user),
):
    """Get usage summary for the authenticated user."""
    summary = await billing_service.get_usage_summary(
        user_id=user.user_id,
        limit=limit,
    )

    return UsageResponse(
        user_id=summary.user_id,
        total_tokens_input=summary.total_tokens_input,
        total_tokens_output=summary.total_tokens_output,
        total_tokens=summary.total_tokens,
        total_cost_cents=summary.total_cost_cents,
        record_count=summary.record_count,
        records=summary.records,
        period_start=summary.period_start,
        period_end=summary.period_end,
    )


# ---------------------------------------------------------------------------
# Subscription Endpoint
# ---------------------------------------------------------------------------

@router.post(
    "/billing/subscribe",
    response_model=SubscribeResponse,
    summary="Create a metered billing subscription",
    description="Creates a Stripe customer and metered subscription for usage-based billing.",
)
async def create_subscription(
    body: SubscribeRequest,
    user: AuthenticatedUser = Depends(get_current_user),
):
    """Create a Stripe metered subscription for the authenticated user."""
    try:
        info = await billing_service.create_subscription(
            user_id=user.user_id,
            email=body.email,
            payment_method_id=body.payment_method_id,
        )
        return SubscribeResponse(
            customer_id=info.customer_id,
            subscription_id=info.subscription_id,
            status=info.status,
            current_period_start=info.current_period_start,
            current_period_end=info.current_period_end,
        )
    except RuntimeError as exc:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail=str(exc),
        )
    except Exception as exc:
        logger.error("Subscription creation failed: %s", exc)
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail="Failed to create subscription.",
        )
