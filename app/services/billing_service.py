"""
Billing Service — Stripe Usage-Based Billing & Token Metering
==============================================================

PURPOSE:
    Integrates with Stripe for metered/usage-based billing:
    1. **create_subscription()** — Creates a Stripe customer + subscription
       with a metered price for token-based billing.
    2. **report_usage()** — Reports token consumption via Stripe Usage
       Records API (metered billing).
    3. **get_usage_summary()** — Returns usage history, tokens consumed,
       and cost for a user.
    4. **record_pipeline_usage()** — Called after pipeline steps to record
       tokens consumed with markup applied.

COST CALCULATION:
    Customer cost = tokens_consumed × cost_per_token × markup_rate
    Default markup: 3.0x over wholesale LLM costs.
    Stripe receives usage in "units" where 1 unit = 1 token.

CONFIGURATION (env vars with VECTORAIZ_ prefix):
    VECTORAIZ_STRIPE_SECRET_KEY        — Stripe secret API key
    VECTORAIZ_STRIPE_PRICE_ID          — Stripe metered price ID
    VECTORAIZ_STRIPE_WEBHOOK_SECRET    — Stripe webhook signing secret
    VECTORAIZ_BILLING_MARKUP_RATE      — Markup multiplier (default 3.0)

BQ-111: Usage and subscription data now persisted in SQL tables
       (billing_usage, billing_subscriptions) instead of in-memory dicts.

PHASE: BQ-098 — Stripe Usage-Based Billing
CREATED: S-BQ098 (2026-02-10)
UPDATED: BQ-111 (2026-02-12) — persistent state
"""

from __future__ import annotations

import json
import logging
import math
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional
from uuid import uuid4

from app.config import settings

logger = logging.getLogger(__name__)

__all__ = [
    "BillingService",
    "UsageRecord",
    "SubscriptionInfo",
    "billing_service",
]

# ---------------------------------------------------------------------------
# Stripe client — lazy-imported to avoid hard dependency when Stripe is not
# configured (e.g., local dev without billing).
# ---------------------------------------------------------------------------
_stripe = None


def _get_stripe():
    """Lazy-load the stripe module and configure API key."""
    global _stripe
    if _stripe is not None:
        return _stripe
    try:
        import stripe
        stripe.api_key = settings.stripe_secret_key
        _stripe = stripe
        return _stripe
    except ImportError:
        logger.warning(
            "stripe package not installed. Billing features disabled. "
            "Install with: pip install stripe"
        )
        return None


# ---------------------------------------------------------------------------
# Data classes (API-facing — unchanged from BQ-098)
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class UsageRecord:
    """A single usage record for a user."""
    record_id: str
    user_id: str
    tokens_input: int
    tokens_output: int
    total_tokens: int
    cost_cents: int
    pipeline_step: str
    dataset_id: Optional[str] = None
    timestamp: str = ""
    stripe_reported: bool = False


@dataclass(frozen=True)
class SubscriptionInfo:
    """Stripe subscription details."""
    customer_id: str
    subscription_id: str
    subscription_item_id: str
    status: str
    current_period_start: Optional[str] = None
    current_period_end: Optional[str] = None


@dataclass
class UsageSummary:
    """Aggregated usage summary for a user."""
    user_id: str
    total_tokens_input: int = 0
    total_tokens_output: int = 0
    total_tokens: int = 0
    total_cost_cents: int = 0
    record_count: int = 0
    records: List[Dict[str, Any]] = field(default_factory=list)
    period_start: Optional[str] = None
    period_end: Optional[str] = None


# ---------------------------------------------------------------------------
# DB helpers
# ---------------------------------------------------------------------------

def _get_db_session():
    from app.core.database import get_session_context
    return get_session_context()


class BillingService:
    """
    Manages Stripe usage-based billing and persistent usage tracking.

    BQ-111: Usage records and subscriptions are now stored in SQL tables
    instead of in-memory dicts. All data survives process restarts.
    """

    def __init__(self):
        self.markup_rate: float = getattr(settings, "billing_markup_rate", 3.0)
        self.price_id: str = getattr(settings, "stripe_price_id", "")

    @property
    def stripe_configured(self) -> bool:
        """Check if Stripe is properly configured."""
        return bool(getattr(settings, "stripe_secret_key", None)) and _get_stripe() is not None

    # ------------------------------------------------------------------
    # Subscription Management
    # ------------------------------------------------------------------

    async def create_subscription(
        self,
        user_id: str,
        email: str,
        payment_method_id: Optional[str] = None,
        metadata: Optional[Dict[str, str]] = None,
    ) -> SubscriptionInfo:
        """
        Create a Stripe customer and metered subscription.

        Returns:
            SubscriptionInfo with customer/subscription IDs.
        """
        stripe = _get_stripe()
        if not stripe:
            raise RuntimeError(
                "Stripe is not configured. Set VECTORAIZ_STRIPE_SECRET_KEY."
            )

        if not self.price_id:
            raise RuntimeError(
                "Stripe price ID not configured. Set VECTORAIZ_STRIPE_PRICE_ID."
            )

        # Check if user already has a subscription in DB
        existing = self._get_subscription_from_db(user_id)
        if existing:
            logger.info("User %s already has a subscription", user_id)
            return existing

        try:
            # Create Stripe customer
            customer_params: Dict[str, Any] = {
                "email": email,
                "metadata": {
                    "user_id": user_id,
                    **(metadata or {}),
                },
            }
            if payment_method_id:
                customer_params["payment_method"] = payment_method_id
                customer_params["invoice_settings"] = {
                    "default_payment_method": payment_method_id,
                }

            customer = stripe.Customer.create(**customer_params)
            logger.info(
                "Created Stripe customer %s for user %s", customer.id, user_id
            )

            # Create metered subscription
            subscription = stripe.Subscription.create(
                customer=customer.id,
                items=[{"price": self.price_id}],
                metadata={"user_id": user_id},
            )

            # Extract subscription item ID (needed for usage records)
            sub_item_id = subscription["items"]["data"][0]["id"]

            info = SubscriptionInfo(
                customer_id=customer.id,
                subscription_id=subscription.id,
                subscription_item_id=sub_item_id,
                status=subscription.status,
                current_period_start=datetime.fromtimestamp(
                    subscription.current_period_start, tz=timezone.utc
                ).isoformat(),
                current_period_end=datetime.fromtimestamp(
                    subscription.current_period_end, tz=timezone.utc
                ).isoformat(),
            )

            # Persist to DB
            self._save_subscription_to_db(user_id, info)

            logger.info(
                "Created metered subscription %s for user %s",
                subscription.id,
                user_id,
            )
            return info

        except Exception as exc:
            logger.error(
                "Failed to create Stripe subscription for user %s: %s",
                user_id,
                exc,
            )
            raise

    def _get_subscription_from_db(self, user_id: str) -> Optional[SubscriptionInfo]:
        """Load a subscription from the DB, return None if absent."""
        from app.models.billing import BillingSubscription
        from sqlmodel import select

        with _get_db_session() as session:
            stmt = select(BillingSubscription).where(BillingSubscription.user_id == user_id)
            row = session.exec(stmt).first()
            if row is None:
                return None
            if not row.stripe_subscription_id:
                return None
            # Reconstruct SubscriptionInfo (subscription_item_id stored in plan field as JSON)
            try:
                extra = json.loads(row.plan) if row.plan.startswith("{") else {}
            except (json.JSONDecodeError, AttributeError):
                extra = {}
            return SubscriptionInfo(
                customer_id=extra.get("customer_id", ""),
                subscription_id=row.stripe_subscription_id or "",
                subscription_item_id=extra.get("subscription_item_id", ""),
                status=row.status,
                current_period_start=extra.get("current_period_start"),
                current_period_end=extra.get("current_period_end"),
            )

    def _save_subscription_to_db(self, user_id: str, info: SubscriptionInfo) -> None:
        """Persist subscription info to the DB."""
        from app.models.billing import BillingSubscription
        from sqlmodel import select

        plan_json = json.dumps({
            "customer_id": info.customer_id,
            "subscription_item_id": info.subscription_item_id,
            "current_period_start": info.current_period_start,
            "current_period_end": info.current_period_end,
        })

        with _get_db_session() as session:
            stmt = select(BillingSubscription).where(BillingSubscription.user_id == user_id)
            existing = session.exec(stmt).first()
            if existing:
                existing.stripe_subscription_id = info.subscription_id
                existing.plan = plan_json
                existing.status = info.status
                existing.updated_at = datetime.now(timezone.utc)
                session.add(existing)
            else:
                row = BillingSubscription(
                    user_id=user_id,
                    stripe_subscription_id=info.subscription_id,
                    plan=plan_json,
                    status=info.status,
                    balance_cents=0,
                    created_at=datetime.now(timezone.utc),
                    updated_at=datetime.now(timezone.utc),
                )
                session.add(row)
            session.commit()

    # ------------------------------------------------------------------
    # Usage Reporting
    # ------------------------------------------------------------------

    def calculate_cost_cents(
        self,
        tokens_input: int,
        tokens_output: int,
        input_cost_per_million: float = 300.0,
        output_cost_per_million: float = 1500.0,
    ) -> int:
        """
        Calculate customer cost in cents from token counts.

        Uses wholesale LLM pricing × markup rate.
        Returns cost in cents (integer, minimum 1 cent).
        """
        input_cost = (tokens_input / 1_000_000) * input_cost_per_million
        output_cost = (tokens_output / 1_000_000) * output_cost_per_million
        wholesale_cost = input_cost + output_cost
        customer_cost = wholesale_cost * self.markup_rate
        return max(1, math.ceil(customer_cost))

    async def report_usage(
        self,
        user_id: str,
        tokens_input: int,
        tokens_output: int,
        pipeline_step: str = "unknown",
        dataset_id: Optional[str] = None,
    ) -> UsageRecord:
        """
        Report token usage — stores in DB and reports to Stripe if configured.
        """
        total_tokens = tokens_input + tokens_output
        cost_cents = self.calculate_cost_cents(tokens_input, tokens_output)
        now = datetime.now(timezone.utc).isoformat()
        record_id = str(uuid4())

        record = UsageRecord(
            record_id=record_id,
            user_id=user_id,
            tokens_input=tokens_input,
            tokens_output=tokens_output,
            total_tokens=total_tokens,
            cost_cents=cost_cents,
            pipeline_step=pipeline_step,
            dataset_id=dataset_id,
            timestamp=now,
            stripe_reported=False,
        )

        # Persist to DB (idempotency_key = record_id)
        self._save_usage_to_db(record)

        # Report to Stripe if configured (BQ-113: pass idempotency_key)
        stripe_reported = False
        if self.stripe_configured:
            sub_info = self._get_subscription_from_db(user_id)
            if sub_info and sub_info.subscription_item_id:
                stripe_reported = await self._report_to_stripe(
                    sub_info.subscription_item_id, total_tokens,
                    idempotency_key=record_id,
                )

        if stripe_reported:
            record = UsageRecord(
                record_id=record.record_id,
                user_id=record.user_id,
                tokens_input=record.tokens_input,
                tokens_output=record.tokens_output,
                total_tokens=record.total_tokens,
                cost_cents=record.cost_cents,
                pipeline_step=record.pipeline_step,
                dataset_id=record.dataset_id,
                timestamp=record.timestamp,
                stripe_reported=True,
            )

        logger.info(
            "Usage recorded: user=%s step=%s tokens=%d cost=%d cents stripe=%s",
            user_id,
            pipeline_step,
            total_tokens,
            cost_cents,
            stripe_reported,
        )
        return record

    def _save_usage_to_db(self, record: UsageRecord) -> None:
        """Insert a usage record into billing_usage."""
        from app.models.billing import BillingUsage

        row = BillingUsage(
            user_id=record.user_id,
            service=json.dumps({
                "record_id": record.record_id,
                "tokens_input": record.tokens_input,
                "tokens_output": record.tokens_output,
                "total_tokens": record.total_tokens,
                "pipeline_step": record.pipeline_step,
                "dataset_id": record.dataset_id,
                "stripe_reported": record.stripe_reported,
            }),
            amount_cents=record.cost_cents,
            idempotency_key=record.record_id,
            created_at=datetime.fromisoformat(record.timestamp) if record.timestamp else datetime.now(timezone.utc),
        )
        with _get_db_session() as session:
            session.add(row)
            try:
                session.commit()
            except Exception:
                session.rollback()
                # Idempotency: if record_id already exists, that's OK
                logger.debug("Usage record %s already exists (idempotent)", record.record_id)

    async def _report_to_stripe(
        self, subscription_item_id: str, quantity: int,
        idempotency_key: Optional[str] = None,
    ) -> bool:
        """Report usage to Stripe Usage Records API.

        BQ-113: Includes Idempotency-Key header to prevent double-charge
        on crash/retry.
        """
        stripe = _get_stripe()
        if not stripe:
            return False

        try:
            kwargs: Dict[str, Any] = {
                "quantity": quantity,
                "timestamp": int(time.time()),
                "action": "increment",
            }
            if idempotency_key:
                kwargs["idempotency_key"] = idempotency_key
            stripe.SubscriptionItem.create_usage_record(
                subscription_item_id,
                **kwargs,
            )
            logger.info(
                "Reported %d tokens to Stripe (sub_item=%s idem=%s)",
                quantity,
                subscription_item_id,
                idempotency_key,
            )
            return True
        except Exception as exc:
            logger.error("Failed to report usage to Stripe: %s", exc)
            return False

    # ------------------------------------------------------------------
    # Usage Querying
    # ------------------------------------------------------------------

    async def get_usage_summary(
        self,
        user_id: str,
        limit: int = 100,
    ) -> UsageSummary:
        """Get usage summary for a user from the database."""
        from app.models.billing import BillingUsage
        from sqlmodel import select

        summary = UsageSummary(user_id=user_id)

        with _get_db_session() as session:
            stmt = (
                select(BillingUsage)
                .where(BillingUsage.user_id == user_id)
                .order_by(BillingUsage.created_at.asc())
            )
            rows = session.exec(stmt).all()

        for row in rows:
            try:
                svc = json.loads(row.service) if row.service else {}
            except (json.JSONDecodeError, TypeError):
                svc = {}

            ti = svc.get("tokens_input", 0)
            to = svc.get("tokens_output", 0)
            tt = svc.get("total_tokens", ti + to)

            summary.total_tokens_input += ti
            summary.total_tokens_output += to
            summary.total_tokens += tt
            summary.total_cost_cents += row.amount_cents
            summary.record_count += 1

        # Recent records (most recent first), limited
        recent_rows = rows[-limit:] if rows else []
        summary.records = [
            {
                "record_id": (json.loads(r.service) if r.service else {}).get("record_id", str(r.id)),
                "tokens_input": (json.loads(r.service) if r.service else {}).get("tokens_input", 0),
                "tokens_output": (json.loads(r.service) if r.service else {}).get("tokens_output", 0),
                "total_tokens": (json.loads(r.service) if r.service else {}).get("total_tokens", 0),
                "cost_cents": r.amount_cents,
                "pipeline_step": (json.loads(r.service) if r.service else {}).get("pipeline_step", "unknown"),
                "dataset_id": (json.loads(r.service) if r.service else {}).get("dataset_id"),
                "timestamp": r.created_at.isoformat() if r.created_at else "",
                "stripe_reported": (json.loads(r.service) if r.service else {}).get("stripe_reported", False),
            }
            for r in reversed(recent_rows)
        ]

        if rows:
            summary.period_start = rows[0].created_at.isoformat() if rows[0].created_at else None
            summary.period_end = rows[-1].created_at.isoformat() if rows[-1].created_at else None

        return summary

    # ------------------------------------------------------------------
    # Pipeline Integration
    # ------------------------------------------------------------------

    async def record_pipeline_usage(
        self,
        user_id: str,
        dataset_id: str,
        pipeline_step: str,
        tokens_input: int = 0,
        tokens_output: int = 0,
    ) -> UsageRecord:
        """Record usage from a pipeline step execution."""
        return await self.report_usage(
            user_id=user_id,
            tokens_input=tokens_input,
            tokens_output=tokens_output,
            pipeline_step=pipeline_step,
            dataset_id=dataset_id,
        )


# Module-level singleton
billing_service = BillingService()
