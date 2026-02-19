"""
Billing Models
==============

SQLModel tables for persistent billing state:
- BillingUsage: Append-only usage records (token metering).
- BillingSubscription: Stripe subscription info per user.

Phase: BQ-111 — Persistent State
Created: 2026-02-12
"""

from datetime import datetime, timezone
from typing import Optional

from sqlmodel import Field, SQLModel


class BillingUsage(SQLModel, table=True):
    """Append-only usage record for token metering."""

    __tablename__ = "billing_usage"

    id: Optional[int] = Field(default=None, primary_key=True)
    user_id: str = Field(index=True, max_length=128)
    service: str = Field(max_length=128)
    amount_cents: int = Field(default=0)
    idempotency_key: str = Field(unique=True, max_length=255)
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))


class BillingSubscription(SQLModel, table=True):
    """Stripe subscription state for a user."""

    __tablename__ = "billing_subscriptions"

    id: Optional[int] = Field(default=None, primary_key=True)
    user_id: str = Field(unique=True, index=True, max_length=128)
    stripe_subscription_id: Optional[str] = Field(default=None, nullable=True, max_length=255)
    plan: str = Field(default="metered", max_length=64)
    status: str = Field(default="active", max_length=32)
    balance_cents: int = Field(default=0)
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    updated_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
