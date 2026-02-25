"""
LLM Settings Models
===================

SQLModel tables for LLM provider configuration and usage tracking.
- LLMSettings: Encrypted API key storage per provider.
- LLMUsageLog: Append-only usage records for token/request tracking.

Phase: BQ-125 — Connect Your LLM
Created: 2026-02-12
"""

import uuid
from datetime import datetime, timezone
from typing import Optional

from sqlalchemy import Column, LargeBinary
from sqlmodel import Field, SQLModel


class LLMSettings(SQLModel, table=True):
    """Encrypted LLM provider configuration."""

    __tablename__ = "llm_settings"

    id: str = Field(default_factory=lambda: str(uuid.uuid4()), primary_key=True, max_length=36)
    scope: str = Field(default="instance", max_length=16)
    user_id: Optional[str] = Field(default=None, nullable=True, max_length=36)

    # Provider config
    provider: str = Field(max_length=32)
    model: str = Field(max_length=64)
    display_name: Optional[str] = Field(default=None, nullable=True, max_length=128)

    # Encrypted API key (binary fields need sa_column)
    encrypted_key: bytes = Field(sa_column=Column(LargeBinary, nullable=False))
    key_iv: bytes = Field(sa_column=Column(LargeBinary, nullable=False))
    key_tag: bytes = Field(sa_column=Column(LargeBinary, nullable=False))
    key_version: int = Field(default=1)
    key_hint: Optional[str] = Field(default=None, nullable=True, max_length=16)

    # Status
    is_active: bool = Field(default=True, index=True)
    last_tested_at: Optional[datetime] = Field(default=None, nullable=True)
    last_test_ok: Optional[bool] = Field(default=None, nullable=True)

    # Usage counters
    total_requests: int = Field(default=0)
    total_tokens: int = Field(default=0)

    # Metadata
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    updated_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    created_by: str = Field(default="admin", max_length=64)


class LLMUsageLog(SQLModel, table=True):
    """Append-only usage record for LLM requests."""

    __tablename__ = "llm_usage_log"

    id: Optional[int] = Field(default=None, primary_key=True)
    settings_id: str = Field(max_length=36, index=True)
    ts: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    operation: str = Field(max_length=32)
    model: str = Field(max_length=64)
    input_tokens: int = Field(default=0)
    output_tokens: int = Field(default=0)
    latency_ms: Optional[int] = Field(default=None, nullable=True)
    success: bool = Field(default=True)
    error_code: Optional[str] = Field(default=None, nullable=True, max_length=32)
