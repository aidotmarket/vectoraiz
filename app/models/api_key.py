"""
API Key Model
=============

SQLModel table for persistent API key storage.
Keys are stored as HMAC-SHA256 hashes — the raw key is shown once
at creation time and never persisted.

Phase: BQ-111 — Persistent State
Created: 2026-02-12
"""

from datetime import datetime, timezone
from typing import Optional

from sqlmodel import Field, SQLModel, Column, Text


class APIKey(SQLModel, table=True):
    """
    Persistent API key record.

    The raw key is never stored.  ``key_hash`` is
    HMAC-SHA256(raw_key, SERVER_PEPPER) and ``key_prefix`` holds the
    first 8 characters for display in the UI.
    """

    __tablename__ = "api_keys"

    id: Optional[int] = Field(default=None, primary_key=True)
    user_id: str = Field(index=True, max_length=128)
    key_prefix: str = Field(max_length=16)
    key_hash: str = Field(index=True, max_length=128)
    label: Optional[str] = Field(default=None, nullable=True, max_length=255)
    scopes: str = Field(default='["read","write"]', sa_column=Column(Text, default='["read","write"]'))
    is_active: bool = Field(default=True, index=True)
    last_used_at: Optional[datetime] = Field(default=None, nullable=True)
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    revoked_at: Optional[datetime] = Field(default=None, nullable=True)
