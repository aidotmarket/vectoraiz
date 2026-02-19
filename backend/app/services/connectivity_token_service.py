"""
Connectivity Token Service — CRUD + HMAC verification for external LLM tokens.

Token format: vzmcp_<8-char-alphanum>_<32-char-hex>
Storage: HMAC-SHA256(pepper, secret) — raw secret never stored.

Uses the same VECTORAIZ_APIKEY_HMAC_SECRET pepper as app/auth/api_key_auth.py (M19).

Phase: BQ-MCP-RAG — Universal LLM Connectivity
Created: S136
"""

import hashlib
import hmac
import json
import logging
import re
import secrets
import string
from datetime import datetime, timezone
from typing import List, Optional, Tuple

from sqlmodel import select

from app.auth.api_key_auth import _get_hmac_secret
from app.core.database import get_session_context
from app.models.connectivity import ConnectivityToken, ConnectivityTokenRecord, VALID_SCOPES

logger = logging.getLogger(__name__)

# Token format constants
TOKEN_PREFIX = "vzmcp_"
TOKEN_ID_LENGTH = 8
TOKEN_SECRET_LENGTH = 32  # hex chars (16 bytes)
TOKEN_ID_CHARS = string.ascii_letters + string.digits
TOKEN_SECRET_PATTERN = re.compile(r"^[a-f0-9]{32}$")
TOKEN_ID_PATTERN = re.compile(r"^[a-zA-Z0-9]{8}$")


class ConnectivityTokenError(Exception):
    """Raised for token operation failures."""

    def __init__(self, code: str, message: str):
        self.code = code
        self.message = message
        super().__init__(message)


def _hmac_hash(secret: str) -> str:
    """HMAC-SHA256 hash a token secret using the shared pepper."""
    pepper = _get_hmac_secret().encode()
    return hmac.new(pepper, secret.encode(), hashlib.sha256).hexdigest()


def parse_token(raw_token: str) -> Tuple[str, str]:
    """Parse a raw token string into (token_id, secret).

    Validates format strictly BEFORE any DB lookup (M19).
    Raises ConnectivityTokenError if format is invalid.
    """
    if not isinstance(raw_token, str):
        raise ConnectivityTokenError("auth_invalid", "Token must be a string")

    # Must start with prefix
    if not raw_token.startswith(TOKEN_PREFIX):
        raise ConnectivityTokenError("auth_invalid", "Invalid token format")

    # Split — must yield exactly 3 parts: "vzmcp", token_id, secret
    parts = raw_token.split("_")
    if len(parts) != 3:
        raise ConnectivityTokenError("auth_invalid", "Invalid token format")

    token_id = parts[1]
    secret = parts[2]

    # Validate token_id: 8 alphanumeric chars
    if not TOKEN_ID_PATTERN.match(token_id):
        raise ConnectivityTokenError("auth_invalid", "Invalid token format")

    # Validate secret: 32 hex chars
    if not TOKEN_SECRET_PATTERN.match(secret):
        raise ConnectivityTokenError("auth_invalid", "Invalid token format")

    return token_id, secret


def verify_token(raw_token: str) -> ConnectivityToken:
    """Parse, verify HMAC, check revoked/expired. Returns validated token.

    Order of checks (M19):
      1. Parse format (reject before DB lookup)
      2. DB lookup by token_id
      3. HMAC verification (constant-time via hmac.compare_digest)
      4. Check revoked (reject BEFORE any work)
      5. Check expired
    """
    token_id, secret = parse_token(raw_token)

    with get_session_context() as session:
        record = session.get(ConnectivityTokenRecord, token_id)
        if record is None:
            raise ConnectivityTokenError("auth_invalid", "Unknown token")

        # HMAC verification — constant-time comparison mandatory
        expected_hash = _hmac_hash(secret)
        if not hmac.compare_digest(record.hmac_hash, expected_hash):
            raise ConnectivityTokenError("auth_invalid", "Invalid token")

        # Check revoked BEFORE any query processing
        if record.is_revoked:
            raise ConnectivityTokenError("auth_revoked", "Token has been revoked")

        # Check expiration
        if record.expires_at is not None:
            now = datetime.now(timezone.utc)
            expires = record.expires_at
            if expires.tzinfo is None:
                expires = expires.replace(tzinfo=timezone.utc)
            if now > expires:
                raise ConnectivityTokenError("auth_expired", "Token has expired")

        # Update last_used_at and request_count
        record.last_used_at = datetime.now(timezone.utc)
        record.request_count += 1
        session.add(record)
        session.commit()

        # Parse scopes
        try:
            scopes = json.loads(record.scopes)
        except (json.JSONDecodeError, TypeError):
            scopes = list(VALID_SCOPES)

        return ConnectivityToken(
            id=record.id,
            label=record.label,
            scopes=scopes,
            secret_last4=record.secret_last4,
            created_at=record.created_at,
            expires_at=record.expires_at,
            last_used_at=record.last_used_at,
            request_count=record.request_count,
        )


def create_token(
    label: str,
    scopes: Optional[List[str]] = None,
    expires_at: Optional[datetime] = None,
    max_tokens: int = 10,
) -> Tuple[str, ConnectivityToken]:
    """Create a new connectivity token.

    Returns (raw_token, ConnectivityToken). The raw_token is shown ONCE.

    Args:
        label: Human label for the token.
        scopes: List of scopes. Defaults to all scopes.
        expires_at: Optional expiration. None = no expiration.
        max_tokens: Max active (non-revoked) tokens allowed.

    Raises:
        ConnectivityTokenError: If max tokens exceeded or invalid scopes.
    """
    # Default scopes
    if scopes is None:
        scopes = list(VALID_SCOPES)

    # Validate scopes
    invalid = set(scopes) - VALID_SCOPES
    if invalid:
        raise ConnectivityTokenError(
            "scope_denied",
            f"Invalid scopes: {', '.join(sorted(invalid))}",
        )

    with get_session_context() as session:
        # Enforce max active tokens
        stmt = select(ConnectivityTokenRecord).where(
            ConnectivityTokenRecord.is_revoked == False  # noqa: E712
        )
        active_count = len(session.exec(stmt).all())
        if active_count >= max_tokens:
            raise ConnectivityTokenError(
                "rate_limited",
                f"Maximum {max_tokens} active tokens allowed. Revoke an existing token first.",
            )

        # Generate token_id (unique 8-char alphanumeric)
        for _ in range(10):
            token_id = "".join(secrets.choice(TOKEN_ID_CHARS) for _ in range(TOKEN_ID_LENGTH))
            existing = session.get(ConnectivityTokenRecord, token_id)
            if existing is None:
                break
        else:
            raise ConnectivityTokenError("internal_error", "Failed to generate unique token ID")

        # Generate secret (32-char hex = 16 bytes)
        secret = secrets.token_hex(16)

        # Build raw token
        raw_token = f"{TOKEN_PREFIX}{token_id}_{secret}"

        # Store HMAC hash (never the raw secret)
        hmac_hash = _hmac_hash(secret)
        secret_last4 = secret[-4:]

        record = ConnectivityTokenRecord(
            id=token_id,
            label=label,
            hmac_hash=hmac_hash,
            secret_last4=secret_last4,
            scopes=json.dumps(scopes),
            created_at=datetime.now(timezone.utc),
            expires_at=expires_at,
        )
        session.add(record)
        session.commit()
        session.refresh(record)

        logger.info(
            "Connectivity token created: id=%s label=%s scopes=%s",
            token_id, label, scopes,
        )

        token = ConnectivityToken(
            id=record.id,
            label=record.label,
            scopes=scopes,
            secret_last4=secret_last4,
            created_at=record.created_at,
            expires_at=record.expires_at,
        )

        return raw_token, token


def revoke_token(token_id: str) -> ConnectivityToken:
    """Revoke a connectivity token by ID.

    Raises ConnectivityTokenError if not found or already revoked.
    """
    with get_session_context() as session:
        record = session.get(ConnectivityTokenRecord, token_id)
        if record is None:
            raise ConnectivityTokenError("auth_invalid", "Token not found")

        if record.is_revoked:
            raise ConnectivityTokenError("auth_revoked", "Token is already revoked")

        record.is_revoked = True
        record.revoked_at = datetime.now(timezone.utc)
        session.add(record)
        session.commit()

        logger.info("Connectivity token revoked: id=%s label=%s", token_id, record.label)

        try:
            scopes = json.loads(record.scopes)
        except (json.JSONDecodeError, TypeError):
            scopes = list(VALID_SCOPES)

        return ConnectivityToken(
            id=record.id,
            label=record.label,
            scopes=scopes,
            secret_last4=record.secret_last4,
            created_at=record.created_at,
            expires_at=record.expires_at,
            last_used_at=record.last_used_at,
            request_count=record.request_count,
        )


def list_tokens() -> List[ConnectivityToken]:
    """List all connectivity tokens (active and revoked)."""
    with get_session_context() as session:
        stmt = select(ConnectivityTokenRecord).order_by(
            ConnectivityTokenRecord.created_at.desc()
        )
        records = session.exec(stmt).all()

        tokens = []
        for record in records:
            try:
                scopes = json.loads(record.scopes)
            except (json.JSONDecodeError, TypeError):
                scopes = list(VALID_SCOPES)

            tokens.append(ConnectivityToken(
                id=record.id,
                label=record.label,
                scopes=scopes,
                secret_last4=record.secret_last4,
                created_at=record.created_at,
                expires_at=record.expires_at,
                last_used_at=record.last_used_at,
                request_count=record.request_count,
            ))

        return tokens
