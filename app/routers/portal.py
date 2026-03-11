"""
BQ-VZ-SHARED-SEARCH: Portal Router — /api/portal/* endpoints
==============================================================

All portal endpoints live here. Completely separate from admin routes (M2).
ACL enforced on every dataset access (M1).
"""

import logging
import secrets
from datetime import datetime, timezone, timedelta

from fastapi import APIRouter, Depends, HTTPException, Request, Query, status

from app.core.async_utils import run_sync
from app.middleware.portal_auth import (
    get_portal_session,
    create_portal_jwt,
    check_dataset_acl,
)
from app.models.portal import (
    AccessCodeValidator,
    get_portal_config,
    save_portal_config,
)
from app.schemas.portal import (
    DatasetPortalConfig,
    PortalAuthRequest,
    PortalAuthResponse,
    PortalConfigUpdate,
    PortalPublicConfig,
    PortalSearchQuery,
    PortalSession,
    PortalTier,
)
from app.services.portal_service import get_portal_service

logger = logging.getLogger(__name__)

router = APIRouter()


# ---------------------------------------------------------------------------
# Public portal endpoints (no admin auth)
# ---------------------------------------------------------------------------

@router.get("/config")
async def get_portal_public_config():
    """Public: returns tier type, portal name, branding. No secrets."""
    config = get_portal_config()
    return PortalPublicConfig(
        enabled=config.enabled,
        tier=config.tier,
        name="Search Portal",
    )


@router.post("/auth/code", response_model=PortalAuthResponse)
async def authenticate_with_code(body: PortalAuthRequest, request: Request):
    """Shared access code authentication. Rate-limited per IP (M5)."""
    config = get_portal_config()

    if not config.enabled:
        raise HTTPException(status_code=404, detail="Portal is not enabled")

    if config.tier != PortalTier.code:
        raise HTTPException(status_code=400, detail="Access code auth not available for this portal tier")

    client_ip = request.client.host if request.client else "unknown"

    # Rate limit check (M5)
    if not AccessCodeValidator.check_rate_limit(client_ip):
        raise HTTPException(
            status_code=status.HTTP_429_TOO_MANY_REQUESTS,
            detail="Too many attempts. Try again later.",
        )

    # Record attempt before verification
    AccessCodeValidator.record_attempt(client_ip)

    # Verify code
    if not config.access_code_hash:
        raise HTTPException(status_code=500, detail="Portal access code not configured")

    if not AccessCodeValidator.verify_code(body.code, config.access_code_hash):
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Invalid access code",
        )

    # Issue portal JWT (SS-C1)
    now = datetime.now(timezone.utc)
    session = PortalSession(
        session_id=secrets.token_hex(16),
        tier=PortalTier.code,
        ip_address=client_ip,
        created_at=now,
        expires_at=now + timedelta(minutes=config.session_ttl_minutes),
        portal_session_version=config.portal_session_version,
    )

    token = create_portal_jwt(session)

    # Track session (SS-C3)
    config.active_sessions[session.session_id] = session.expires_at.isoformat()
    save_portal_config(config)

    return PortalAuthResponse(
        token=token,
        expires_at=session.expires_at,
        tier=session.tier.value,
    )


@router.get("/datasets")
async def list_portal_datasets(session: PortalSession = Depends(get_portal_session)):
    """List datasets visible to portal users. ACL-enforced (M1)."""
    portal_svc = get_portal_service()
    datasets = await run_sync(portal_svc.get_visible_datasets)
    return {"datasets": [d.model_dump() for d in datasets]}


@router.post("/search")
async def search_portal(
    query: PortalSearchQuery,
    session: PortalSession = Depends(get_portal_session),
):
    """Search a portal-visible dataset. ACL-enforced (M1)."""
    # ACL check
    check_dataset_acl(query.dataset_id)

    portal_svc = get_portal_service()
    result = await run_sync(
        portal_svc.search_dataset,
        query.dataset_id,
        query.query,
        query.limit,
        query.offset,
    )
    return result.model_dump()


@router.get("/search/{dataset_id}")
async def search_dataset(
    dataset_id: str,
    q: str = Query(..., description="Search query"),
    limit: int = Query(20, ge=1, le=100),
    session: PortalSession = Depends(get_portal_session),
):
    """Search single dataset. ACL-enforced (M1)."""
    check_dataset_acl(dataset_id)

    portal_svc = get_portal_service()
    result = await run_sync(
        portal_svc.search_dataset,
        dataset_id,
        q,
        limit,
    )
    return result.model_dump()


# ---------------------------------------------------------------------------
# Admin portal management endpoints (require admin auth)
# ---------------------------------------------------------------------------

admin_router = APIRouter()


@admin_router.get("/settings/portal")
async def get_portal_settings():
    """Get current portal configuration (admin only)."""
    config = get_portal_config()
    # Return config but redact the access_code_hash
    data = config.model_dump()
    data["access_code_hash"] = "***" if config.access_code_hash else None
    data["active_session_count"] = len(config.active_sessions)
    return data


@admin_router.put("/settings/portal")
async def update_portal_settings(body: PortalConfigUpdate):
    """Enable/disable portal, set tier, set access code, set base URL."""
    config = get_portal_config()

    if body.tier is not None:
        config.tier = body.tier

    if body.base_url is not None:
        config.base_url = body.base_url.rstrip("/")

    if body.session_ttl_minutes is not None:
        config.session_ttl_minutes = body.session_ttl_minutes

    # Handle access code update (M5)
    if body.access_code is not None:
        if body.access_code == "":
            # Clear access code
            config.access_code_hash = None
        else:
            if not AccessCodeValidator.validate_strength(body.access_code):
                raise HTTPException(
                    status_code=400,
                    detail="Access code must be at least 6 alphanumeric characters and not purely numeric",
                )
            config.access_code_hash = AccessCodeValidator.hash_code(body.access_code)
            # Invalidate existing sessions on code rotation (SS-C1)
            config = AccessCodeValidator.invalidate_sessions_on_rotation(config)

    # Handle enable/disable
    if body.enabled is not None:
        if body.enabled:
            # M6: Reject enable without base_url
            if not config.base_url:
                raise HTTPException(
                    status_code=400,
                    detail="Cannot enable portal without setting a base URL (Mandate M6)",
                )
            # Validate tier=code has access code set
            if config.tier == PortalTier.code and not config.access_code_hash:
                raise HTTPException(
                    status_code=400,
                    detail="Cannot enable portal with 'code' tier without setting an access code",
                )
        config.enabled = body.enabled

    save_portal_config(config)

    data = config.model_dump()
    data["access_code_hash"] = "***" if config.access_code_hash else None
    data["active_session_count"] = len(config.active_sessions)
    return data


@admin_router.put("/settings/portal/datasets/{dataset_id}")
async def update_dataset_portal_config(dataset_id: str, body: DatasetPortalConfig):
    """Set per-dataset portal visibility and column restrictions."""
    # Verify dataset exists
    from app.services.processing_service import get_processing_service
    processing_svc = get_processing_service()
    record = processing_svc.get_dataset(dataset_id)
    if not record:
        raise HTTPException(status_code=404, detail=f"Dataset '{dataset_id}' not found")

    config = get_portal_config()
    config.datasets[dataset_id] = body
    save_portal_config(config)

    return {"dataset_id": dataset_id, **body.model_dump()}


@admin_router.post("/settings/portal/revoke-sessions")
async def revoke_all_portal_sessions():
    """Revoke all active portal sessions by incrementing portal_session_version."""
    config = get_portal_config()
    config = AccessCodeValidator.invalidate_sessions_on_rotation(config)
    save_portal_config(config)
    return {"message": "All portal sessions revoked", "portal_session_version": config.portal_session_version}
