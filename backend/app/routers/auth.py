"""
BQ-127: Local Auth Router — Setup, Login, Key Management
=========================================================

Provides local authentication endpoints for standalone mode:
    POST /api/auth/setup   — First-run admin creation (C10)
    POST /api/auth/login   — Username/password → API key
    POST /api/auth/keys    — Create new API key for authenticated user
    GET  /api/auth/keys    — List user's API keys (masked)
    DELETE /api/auth/keys/{key_id} — Revoke a key
    GET  /api/auth/me      — Current user info

In connected mode, the legacy signup/onboarding endpoints are also available.

Phase: BQ-127 — Air-Gap Architecture
Created: S130 (2026-02-13)
"""

from __future__ import annotations

import json
import logging
import secrets
import string
from datetime import datetime, timezone
from typing import List, Optional
from uuid import uuid4

from fastapi import APIRouter, Depends, HTTPException, Request, status
from pydantic import BaseModel, Field

from app.config import settings
from app.auth.api_key_auth import (
    AuthenticatedUser,
    get_current_user,
    hmac_hash_secret,
)

logger = logging.getLogger(__name__)

router = APIRouter()

# ---------------------------------------------------------------------------
# Rate limiting (simple in-memory, per spec: 3/min for setup)
# ---------------------------------------------------------------------------
_setup_attempts: dict[str, list[float]] = {}
_SETUP_RATE_LIMIT = 3
_SETUP_RATE_WINDOW = 60  # seconds


def _check_rate_limit(client_ip: str) -> None:
    """BQ-127 (C10): Rate limit setup endpoint to 3 attempts/min per IP."""
    import time

    now = time.time()
    attempts = _setup_attempts.get(client_ip, [])
    # Prune old attempts
    attempts = [t for t in attempts if now - t < _SETUP_RATE_WINDOW]
    if len(attempts) >= _SETUP_RATE_LIMIT:
        raise HTTPException(
            status_code=status.HTTP_429_TOO_MANY_REQUESTS,
            detail="Too many setup attempts. Try again in 60 seconds.",
        )
    attempts.append(now)
    _setup_attempts[client_ip] = attempts


# ---------------------------------------------------------------------------
# Rate limiting for login (5 attempts per IP per 5 minutes)
# ---------------------------------------------------------------------------
_login_attempts: dict[str, list[float]] = {}
_LOGIN_RATE_LIMIT = 5
_LOGIN_RATE_WINDOW = 300  # 5 minutes


def _check_login_rate_limit(client_ip: str) -> None:
    """Rate limit login endpoint to 5 attempts per IP per 5 minutes."""
    import time

    now = time.time()
    attempts = _login_attempts.get(client_ip, [])
    attempts = [t for t in attempts if now - t < _LOGIN_RATE_WINDOW]
    if len(attempts) >= _LOGIN_RATE_LIMIT:
        raise HTTPException(
            status_code=status.HTTP_429_TOO_MANY_REQUESTS,
            detail="Too many login attempts. Try again later.",
            headers={"Retry-After": str(_LOGIN_RATE_WINDOW)},
        )
    attempts.append(now)
    _login_attempts[client_ip] = attempts


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _prepare_password(password: str) -> bytes:
    """Pre-hash password with SHA-256 to handle bcrypt's 72-byte limit (bcrypt >= 5.0)."""
    import base64
    import hashlib
    return base64.b64encode(hashlib.sha256(password.encode()).digest())


def _hash_password(password: str) -> str:
    """Hash a password with bcrypt (SHA-256 pre-hash for long password safety)."""
    import bcrypt
    return bcrypt.hashpw(_prepare_password(password), bcrypt.gensalt()).decode()


def _verify_password(password: str, password_hash: str) -> bool:
    """Verify a password against a bcrypt hash."""
    import bcrypt
    return bcrypt.checkpw(_prepare_password(password), password_hash.encode())


def _generate_key_id() -> str:
    """Generate an 8-char alphanumeric key_id for local API keys (C2)."""
    alphabet = string.ascii_lowercase + string.digits
    return "".join(secrets.choice(alphabet) for _ in range(8))


def _generate_key_secret() -> str:
    """Generate a 32-char random secret for local API keys."""
    alphabet = string.ascii_lowercase + string.digits
    return "".join(secrets.choice(alphabet) for _ in range(32))


def _create_api_key_for_user(
    user_id: str,
    label: str = "Default",
    scopes: Optional[List[str]] = None,
) -> dict:
    """BQ-127: Create a local API key, store HMAC hash in DB, return full key ONCE.

    Returns dict with: key_id, full_key, label, scopes, created_at
    """
    from app.core.database import get_session_context
    from app.models.local_auth import LocalAPIKey

    if scopes is None:
        scopes = ["read", "write", "admin"]

    key_id = _generate_key_id()
    secret = _generate_key_secret()
    full_key = f"vz_{key_id}_{secret}"
    key_hash = hmac_hash_secret(secret)

    now = datetime.now(timezone.utc)
    record = LocalAPIKey(
        id=str(uuid4()),
        user_id=user_id,
        key_id=key_id,
        key_hash=key_hash,
        label=label,
        scopes=json.dumps(scopes),
        created_at=now,
    )

    with get_session_context() as session:
        session.add(record)
        session.commit()

    logger.info("API key created: key_id=%s user_id=%s label=%s", key_id, user_id, label)

    return {
        "key_id": key_id,
        "full_key": full_key,
        "label": label,
        "scopes": scopes,
        "created_at": now.isoformat(),
    }


# ---------------------------------------------------------------------------
# Request / Response models
# ---------------------------------------------------------------------------

class SetupRequest(BaseModel):
    username: str = Field(..., min_length=3, max_length=255, description="Admin username")
    password: str = Field(..., min_length=8, max_length=255, description="Admin password (min 8 chars)")


class SetupResponse(BaseModel):
    user_id: str
    username: str
    api_key: str = Field(..., description="Full API key — shown ONCE, store it safely")
    message: str = "Admin account created. Save your API key — it cannot be retrieved later."


class LoginRequest(BaseModel):
    username: str
    password: str


class LoginResponse(BaseModel):
    user_id: str
    username: str
    api_key: str = Field(..., description="Full API key — shown ONCE")
    message: str = "Login successful. Save your API key."


class CreateKeyRequest(BaseModel):
    label: str = Field(default="Untitled", max_length=255, description="Human-readable key label")
    scopes: List[str] = Field(default=["read", "write"], description="Scopes for this key")


class CreateKeyResponse(BaseModel):
    key_id: str
    full_key: str = Field(..., description="Full API key — shown ONCE")
    label: str
    scopes: List[str]
    created_at: str


class KeyInfo(BaseModel):
    key_id: str
    label: Optional[str]
    scopes: List[str]
    created_at: str
    last_used_at: Optional[str]
    revoked: bool


class UserInfo(BaseModel):
    user_id: str
    username: str
    role: str
    is_active: bool
    created_at: str


# ---------------------------------------------------------------------------
# GET /api/auth/setup — Check if first-run setup is still available
# ---------------------------------------------------------------------------

@router.get(
    "/setup",
    summary="Check setup availability",
    description="Returns whether first-run setup is available (no admin exists yet).",
)
async def check_setup():
    """Returns {available: true/false} so the frontend can decide whether to show the setup form."""
    from app.core.database import get_session_context
    from app.models.local_auth import LocalUser
    from sqlmodel import select, func

    with get_session_context() as session:
        count = session.exec(select(func.count()).select_from(LocalUser)).one()

    return {"available": count == 0}


# ---------------------------------------------------------------------------
# POST /api/auth/setup — First-run admin creation (C10)
# ---------------------------------------------------------------------------

@router.post(
    "/setup",
    response_model=SetupResponse,
    status_code=status.HTTP_201_CREATED,
    summary="First-run setup — create admin account",
    description=(
        "Creates the first admin user. Only available when local_users table is empty. "
        "Transaction-safe check. Rate-limited 3/min per IP. (BQ-127 C10)"
    ),
)
async def setup(body: SetupRequest, request: Request):
    """BQ-127 (C10): First-run setup endpoint.

    Only works when local_users table is empty. Uses a transaction-safe
    check — SELECT COUNT + INSERT in the same transaction to prevent races.
    """
    _check_rate_limit(request.client.host if request.client else "unknown")

    from app.core.database import get_session_context
    from app.models.local_auth import LocalUser
    from sqlmodel import select, func

    with get_session_context() as session:
        # Transaction-safe check: count users inside the transaction
        count = session.exec(select(func.count()).select_from(LocalUser)).one()
        if count > 0:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail="Setup is no longer available.",
            )

        # Create admin user
        user = LocalUser(
            id=str(uuid4()),
            username=body.username.strip(),
            password_hash=_hash_password(body.password),
            role="admin",
            is_active=True,
        )
        session.add(user)
        session.commit()
        session.refresh(user)

    # Generate initial API key
    key_info = _create_api_key_for_user(
        user_id=user.id,
        label="Admin (setup)",
        scopes=["read", "write", "admin"],
    )

    logger.info("First-run setup complete: user=%s", user.username)

    return SetupResponse(
        user_id=user.id,
        username=user.username,
        api_key=key_info["full_key"],
    )


# ---------------------------------------------------------------------------
# POST /api/auth/login — Username/password → API key
# ---------------------------------------------------------------------------

@router.post(
    "/login",
    response_model=LoginResponse,
    summary="Login with username/password",
    description="Validates credentials and returns a new API key.",
)
async def login(body: LoginRequest, request: Request):
    """BQ-127: Login endpoint — validates credentials, creates and returns a new API key."""
    _check_login_rate_limit(request.client.host if request.client else "unknown")

    from app.core.database import get_session_context
    from app.models.local_auth import LocalUser
    from sqlmodel import select

    with get_session_context() as session:
        stmt = select(LocalUser).where(LocalUser.username == body.username.strip())
        user = session.exec(stmt).first()

    if not user or not user.is_active:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Invalid username or password.",
        )

    if not _verify_password(body.password, user.password_hash):
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Invalid username or password.",
        )

    # Generate a new API key for this login
    key_info = _create_api_key_for_user(
        user_id=user.id,
        label=f"Login ({datetime.now(timezone.utc).strftime('%Y-%m-%d %H:%M')})",
        scopes=["read", "write", "admin"],
    )

    return LoginResponse(
        user_id=user.id,
        username=user.username,
        api_key=key_info["full_key"],
    )


# ---------------------------------------------------------------------------
# POST /api/auth/keys — Create new API key
# ---------------------------------------------------------------------------

@router.post(
    "/keys",
    response_model=CreateKeyResponse,
    status_code=status.HTTP_201_CREATED,
    summary="Create a new API key",
    description="Creates a new API key for the authenticated user. The full key is returned ONCE.",
)
async def create_key(
    body: CreateKeyRequest,
    user: AuthenticatedUser = Depends(get_current_user),
):
    """BQ-127: Create a new API key for the authenticated user."""
    key_info = _create_api_key_for_user(
        user_id=user.user_id,
        label=body.label,
        scopes=body.scopes,
    )

    return CreateKeyResponse(**key_info)


# ---------------------------------------------------------------------------
# GET /api/auth/keys — List user's API keys (masked)
# ---------------------------------------------------------------------------

@router.get(
    "/keys",
    response_model=List[KeyInfo],
    summary="List API keys",
    description="Returns all API keys for the authenticated user with masked secrets.",
)
async def list_keys(user: AuthenticatedUser = Depends(get_current_user)):
    """BQ-127: List API keys for the authenticated user (secrets masked)."""
    from app.core.database import get_session_context
    from app.models.local_auth import LocalAPIKey
    from sqlmodel import select

    with get_session_context() as session:
        stmt = select(LocalAPIKey).where(LocalAPIKey.user_id == user.user_id)
        keys = session.exec(stmt).all()

    result = []
    for k in keys:
        try:
            scopes = json.loads(k.scopes)
        except (json.JSONDecodeError, TypeError):
            scopes = ["read", "write"]

        result.append(KeyInfo(
            key_id=k.key_id,
            label=k.label,
            scopes=scopes,
            created_at=k.created_at.isoformat() if k.created_at else "",
            last_used_at=k.last_used_at.isoformat() if k.last_used_at else None,
            revoked=k.revoked_at is not None,
        ))

    return result


# ---------------------------------------------------------------------------
# DELETE /api/auth/keys/{key_id} — Revoke a key
# ---------------------------------------------------------------------------

@router.delete(
    "/keys/{key_id}",
    status_code=status.HTTP_200_OK,
    summary="Revoke an API key",
    description="Soft-revokes an API key by setting revoked_at timestamp.",
)
async def revoke_key(
    key_id: str,
    user: AuthenticatedUser = Depends(get_current_user),
):
    """BQ-127: Revoke an API key by key_id."""
    from app.core.database import get_session_context
    from app.models.local_auth import LocalAPIKey
    from sqlmodel import select

    with get_session_context() as session:
        stmt = select(LocalAPIKey).where(
            LocalAPIKey.key_id == key_id,
            LocalAPIKey.user_id == user.user_id,
        )
        key_record = session.exec(stmt).first()
        if not key_record:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Key '{key_id}' not found.",
            )

        if key_record.revoked_at is not None:
            raise HTTPException(
                status_code=status.HTTP_409_CONFLICT,
                detail=f"Key '{key_id}' is already revoked.",
            )

        key_record.revoked_at = datetime.now(timezone.utc)
        session.add(key_record)
        session.commit()

    # Invalidate cache for this key
    from app.auth.api_key_auth import api_key_cache
    keys_to_remove = [k for k, v in api_key_cache.items() if v.key_id == key_id]
    for k in keys_to_remove:
        api_key_cache.pop(k, None)

    logger.info("API key revoked: key_id=%s by user=%s", key_id, user.user_id)
    return {"detail": f"Key '{key_id}' revoked."}


# ---------------------------------------------------------------------------
# GET /api/auth/me — Current user info
# ---------------------------------------------------------------------------

@router.get(
    "/me",
    response_model=UserInfo,
    summary="Current user info",
    description="Returns information about the currently authenticated user.",
)
async def get_me(user: AuthenticatedUser = Depends(get_current_user)):
    """BQ-127: Return current user info from local auth store."""
    from app.core.database import get_session_context
    from app.models.local_auth import LocalUser

    with get_session_context() as session:
        local_user = session.get(LocalUser, user.user_id)

    if not local_user:
        # Fallback for ai.market users in connected mode
        return UserInfo(
            user_id=user.user_id,
            username=user.user_id,
            role="user",
            is_active=True,
            created_at="",
        )

    return UserInfo(
        user_id=local_user.id,
        username=local_user.username,
        role=local_user.role,
        is_active=local_user.is_active,
        created_at=local_user.created_at.isoformat() if local_user.created_at else "",
    )
