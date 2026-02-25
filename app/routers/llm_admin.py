"""
LLM Admin Router
================

API endpoints for LLM provider configuration management.
Prefix: /api/admin/llm

Phase: BQ-125 — Connect Your LLM
Created: 2026-02-12
"""

import logging
import time
from collections import defaultdict
from typing import Dict, List, Optional

from fastapi import APIRouter, Depends, HTTPException, Request, status

from app.auth.api_key_auth import get_current_user, AuthenticatedUser
from app.schemas.llm_settings import (
    LLMSettingsCreate,
    LLMSettingsListResponse,
    LLMSettingsResponse,
    LLMTestRequest,
    LLMTestResponse,
    LLMProvidersResponse,
    LLMUsageSummary,
)
from app.services.llm_settings_service import (
    VALID_PROVIDERS,
    get_settings,
    put_settings,
    delete_settings,
    test_connection,
    get_providers,
    get_usage,
    get_status,
)

logger = logging.getLogger(__name__)

router = APIRouter()

# --- Rate limiter for /test endpoint (3/min) ---
_test_rate_limits: Dict[str, List[float]] = defaultdict(list)
_RATE_LIMIT_MAX = 3
_RATE_LIMIT_WINDOW_S = 60


def _check_rate_limit(key: str) -> None:
    """Raise 429 if >3 calls/min for this key."""
    now = time.time()
    cutoff = now - _RATE_LIMIT_WINDOW_S
    # Clean old entries
    _test_rate_limits[key] = [t for t in _test_rate_limits[key] if t > cutoff]
    if len(_test_rate_limits[key]) >= _RATE_LIMIT_MAX:
        raise HTTPException(
            status_code=status.HTTP_429_TOO_MANY_REQUESTS,
            detail="Rate limit exceeded. Max 3 test requests per minute.",
        )
    _test_rate_limits[key].append(now)


# --- Endpoints ---

@router.get(
    "/settings",
    response_model=LLMSettingsListResponse,
    summary="Get LLM settings",
    description="List all configured LLM providers with masked keys.",
)
async def get_llm_settings():
    return get_settings()


@router.put(
    "/settings",
    response_model=LLMSettingsResponse,
    summary="Set LLM settings",
    description="Create or update LLM provider configuration. Encrypts the API key.",
)
async def put_llm_settings(
    body: LLMSettingsCreate,
    user: AuthenticatedUser = Depends(get_current_user),
):
    if body.provider not in VALID_PROVIDERS:
        raise HTTPException(
            status_code=status.HTTP_400_BAD_REQUEST,
            detail=f"Invalid provider '{body.provider}'. Must be one of: {', '.join(sorted(VALID_PROVIDERS))}",
        )
    try:
        result = put_settings(body)
        logger.info("LLM settings saved: provider=%s user=%s", body.provider, user.user_id)
        return result
    except ValueError as e:
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=str(e))


@router.delete(
    "/settings/{provider}",
    status_code=status.HTTP_200_OK,
    summary="Delete LLM settings",
    description="Remove a provider configuration.",
)
async def delete_llm_settings(
    provider: str,
    user: AuthenticatedUser = Depends(get_current_user),
):
    try:
        delete_settings(provider)
        logger.info("LLM settings deleted: provider=%s user=%s", provider, user.user_id)
        return {"message": f"Provider '{provider}' configuration deleted."}
    except ValueError as e:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail=str(e))


@router.post(
    "/test",
    response_model=LLMTestResponse,
    summary="Test LLM connection",
    description="Test connection to a configured provider (rate-limited: 3/min).",
)
async def test_llm_connection(
    body: LLMTestRequest,
    request: Request,
    user: AuthenticatedUser = Depends(get_current_user),
):
    # Rate limit by user ID
    rate_key = f"test:{user.user_id}"
    _check_rate_limit(rate_key)

    if body.provider not in VALID_PROVIDERS:
        raise HTTPException(
            status_code=status.HTTP_400_BAD_REQUEST,
            detail=f"Invalid provider '{body.provider}'.",
        )

    return test_connection(body.provider)


@router.get(
    "/providers",
    response_model=LLMProvidersResponse,
    summary="List LLM providers",
    description="Return supported providers and their available models.",
)
async def get_llm_providers():
    return get_providers()


@router.get(
    "/usage",
    response_model=List[LLMUsageSummary],
    summary="Get usage summary",
    description="Aggregate usage statistics per provider.",
)
async def get_llm_usage(provider: Optional[str] = None):
    return get_usage(provider)


@router.get(
    "/status",
    summary="Get LLM status",
    description="Check if any provider is configured and active.",
)
async def get_llm_status():
    return get_status()
