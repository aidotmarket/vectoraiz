"""
Deduction Queue — Exactly-Once Billing State Machine
=====================================================

PURPOSE:
    Persistent PostgreSQL queue for credit deductions to ai-market-backend.
    Implements exactly-once delivery via idempotency keys, atomic claim,
    and a full state machine with lease-based processing.

STATE MACHINE:
    pending → processing (leased, worker_id + leased_at set)
    processing → completed (success)
    processing → failed_terminal (402 insufficient_funds, 400/401/403/404, hard decline)
    processing → failed_retryable (5xx, timeout, network error, JSON parse on 5xx)
    failed_retryable → pending (after backoff, if attempts < MAX_ATTEMPTS)
    failed_retryable → dead_letter (attempts >= MAX_ATTEMPTS)
    processing → pending (lease expired: leased_at + LEASE_TTL < now)

BQ-113: Exactly-Once Billing (P0)
CREATED: S120 (2026-02-12)
"""

import json
import logging
import random
import time
import uuid
from datetime import datetime, timedelta
import httpx
import sqlalchemy as sa
from sqlalchemy import (
    Column,
    DateTime,
    Integer,
    MetaData,
    String,
    Table,
    Text,
)
from sqlalchemy.exc import IntegrityError

from app.config import settings
from app.core.database import get_engine

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------
MAX_ATTEMPTS = 5
MAX_BACKOFF = 300  # seconds
LEASE_TTL = 300  # seconds
CLAIM_BATCH_SIZE = 10
MONITOR_INTERVAL = 60  # seconds
METRICS_INTERVAL = 60  # seconds

# ---------------------------------------------------------------------------
# SQLAlchemy Table definition
# ---------------------------------------------------------------------------
deductions_metadata = MetaData()

deductions_table = Table(
    "deductions",
    deductions_metadata,
    Column("id", Integer, primary_key=True, autoincrement=True),
    Column("user_id", String(128), nullable=False),
    Column("idempotency_key", String(255), nullable=False, unique=True),
    Column("payload", Text, nullable=False),
    Column("created_at", DateTime, nullable=False),
    Column("next_retry_at", DateTime, nullable=False),
    Column("attempt_count", Integer, nullable=False, server_default="0"),
    Column("last_error", Text, nullable=True),
    Column("status", String(32), nullable=False, server_default="pending"),
    Column("worker_id", String(64), nullable=True),
    Column("leased_at", DateTime, nullable=True),
)

# Indexes defined separately so they match the migration
sa.Index("idx_status_retry", deductions_table.c.status, deductions_table.c.next_retry_at)
sa.Index("idx_status_leased", deductions_table.c.status, deductions_table.c.leased_at)


def create_deductions_table() -> None:
    """Create the deductions table if it doesn't exist (startup safety net)."""
    engine = get_engine()
    deductions_metadata.create_all(engine)


def _calculate_backoff(attempt_count: int) -> float:
    """
    Jittered exponential backoff.

    Formula: min(300, 5 * 2^attempt) + random(0, attempt*2)
    """
    base = min(MAX_BACKOFF, 5 * (2 ** attempt_count))
    jitter = random.uniform(0, attempt_count * 2)
    return base + jitter


class DeductionQueue:
    """
    Persistent deduction queue backed by PostgreSQL via SQLAlchemy Core.

    Each public method acquires its own connection (per-operation isolation).
    Uses SELECT FOR UPDATE SKIP LOCKED for atomic claim.
    """

    def __init__(self) -> None:
        pass

    # ------------------------------------------------------------------
    # Enqueue
    # ------------------------------------------------------------------

    def enqueue(self, payload: dict) -> bool:
        """
        Insert a deduction into the queue.  Returns True on success,
        False if the idempotency_key already exists (duplicate).
        """
        user_id = payload["user_id"]
        idempotency_key = payload["idempotency_key"]
        now = datetime.utcnow()

        engine = get_engine()
        with engine.begin() as conn:
            try:
                conn.execute(
                    deductions_table.insert().values(
                        user_id=user_id,
                        idempotency_key=idempotency_key,
                        payload=json.dumps(payload),
                        created_at=now,
                        next_retry_at=now,
                        status="pending",
                        attempt_count=0,
                    )
                )
                return True
            except IntegrityError:
                logger.info("Duplicate deduction enqueue: %s", idempotency_key)
                return False

    # ------------------------------------------------------------------
    # Atomic claim (SELECT FOR UPDATE SKIP LOCKED)
    # ------------------------------------------------------------------

    def _claim_batch(self, conn, worker_id: str) -> list[dict]:
        """
        Atomically claim up to CLAIM_BATCH_SIZE pending rows using
        SELECT FOR UPDATE SKIP LOCKED (Postgres-native row locking).

        Returns list of dicts with id, payload, attempt_count, idempotency_key.
        """
        now = datetime.utcnow()
        t = deductions_table

        # Select rows to claim with row-level locking
        subq = (
            sa.select(t.c.id)
            .where(t.c.status == "pending")
            .where(t.c.next_retry_at <= now)
            .order_by(t.c.created_at.asc())
            .limit(CLAIM_BATCH_SIZE)
            .with_for_update(skip_locked=True)
        )
        rows = conn.execute(subq).fetchall()
        if not rows:
            return []

        claimed_ids = [r.id for r in rows]

        # Update claimed rows to processing
        conn.execute(
            t.update()
            .where(t.c.id.in_(claimed_ids))
            .values(status="processing", worker_id=worker_id, leased_at=now)
        )

        # Fetch full row data for processing
        result = conn.execute(
            sa.select(t.c.id, t.c.payload, t.c.attempt_count, t.c.idempotency_key)
            .where(t.c.id.in_(claimed_ids))
            .order_by(t.c.created_at.asc())
        )
        return [dict(r._mapping) for r in result.fetchall()]

    # ------------------------------------------------------------------
    # Process
    # ------------------------------------------------------------------

    async def process_all_pending(self) -> int:
        """
        Claim and process a batch of pending deductions.

        Returns the number of successfully completed items.
        """
        worker_id = uuid.uuid4().hex[:12]
        engine = get_engine()
        t = deductions_table

        # Claim phase — single transaction with row locks
        with engine.begin() as conn:
            items = self._claim_batch(conn, worker_id)

        if not items:
            return 0

        processed = 0
        for item in items:
            ded_id = item["id"]
            payload = json.loads(item["payload"])
            attempt_count = item["attempt_count"]
            idempotency_key = item["idempotency_key"]

            start_time = time.monotonic()
            sent, data, retryable, status_code = await self._attempt_send(
                payload, idempotency_key
            )
            elapsed = time.monotonic() - start_time

            with engine.begin() as conn:
                if sent:
                    # Success — mark completed
                    conn.execute(
                        t.update().where(t.c.id == ded_id).values(status="completed")
                    )
                    processed += 1
                    logger.info(
                        "Deduction completed: id=%d key=%s elapsed=%.2fs",
                        ded_id, idempotency_key, elapsed,
                    )
                elif not retryable:
                    # Permanent failure — failed_terminal
                    reason = f"HTTP {status_code}" if status_code else "permanent_error"
                    if status_code == 402:
                        reason = "insufficient_funds"
                    conn.execute(
                        t.update()
                        .where(t.c.id == ded_id)
                        .values(status="failed_terminal", last_error=reason)
                    )
                    logger.warning(
                        "Deduction failed_terminal: id=%d key=%s reason=%s",
                        ded_id, idempotency_key, reason,
                    )
                else:
                    # Retryable failure
                    new_attempt = attempt_count + 1
                    if new_attempt >= MAX_ATTEMPTS:
                        conn.execute(
                            t.update()
                            .where(t.c.id == ded_id)
                            .values(
                                status="dead_letter",
                                attempt_count=new_attempt,
                                last_error=f"max_attempts ({MAX_ATTEMPTS})",
                            )
                        )
                        logger.error(
                            "Deduction dead_letter: id=%d key=%s attempts=%d",
                            ded_id, idempotency_key, new_attempt,
                        )
                    else:
                        backoff = _calculate_backoff(new_attempt)
                        next_at = datetime.utcnow() + timedelta(seconds=backoff)
                        error_msg = f"HTTP {status_code}" if status_code else "network_error"
                        conn.execute(
                            t.update()
                            .where(t.c.id == ded_id)
                            .values(
                                status="failed_retryable",
                                attempt_count=new_attempt,
                                next_retry_at=next_at,
                                last_error=error_msg,
                            )
                        )
                        logger.warning(
                            "Deduction failed_retryable: id=%d key=%s "
                            "attempt=%d backoff=%.1fs",
                            ded_id, idempotency_key, new_attempt, backoff,
                        )

        # Transition failed_retryable → pending (for next cycle)
        with engine.begin() as conn:
            conn.execute(
                t.update()
                .where(t.c.status == "failed_retryable")
                .where(t.c.next_retry_at <= datetime.utcnow())
                .where(t.c.attempt_count < MAX_ATTEMPTS)
                .values(status="pending")
            )

        return processed

    # ------------------------------------------------------------------
    # Lease monitor
    # ------------------------------------------------------------------

    def monitor_expired_leases(self) -> int:
        """
        Background monitor: reset stale processing rows back to pending.

        Any row with status='processing' AND leased_at + LEASE_TTL < now
        gets reset to pending with attempt_count incremented.

        Returns the number of rows reset.
        """
        engine = get_engine()
        t = deductions_table
        cutoff = datetime.utcnow() - timedelta(seconds=LEASE_TTL)

        with engine.begin() as conn:
            result = conn.execute(
                t.update()
                .where(t.c.status == "processing")
                .where(t.c.leased_at <= cutoff)
                .values(
                    status="pending",
                    worker_id=None,
                    leased_at=None,
                    attempt_count=t.c.attempt_count + 1,
                    last_error="lease_expired",
                )
            )
            reset_count = result.rowcount

        if reset_count > 0:
            logger.warning(
                "Lease monitor: reset %d expired processing rows to pending",
                reset_count,
            )

            # Check if any of these now exceed MAX_ATTEMPTS → dead_letter
            with engine.begin() as conn:
                conn.execute(
                    t.update()
                    .where(t.c.status == "pending")
                    .where(t.c.attempt_count >= MAX_ATTEMPTS)
                    .values(
                        status="dead_letter",
                        last_error="max_attempts after lease expiry",
                    )
                )

        return reset_count

    # ------------------------------------------------------------------
    # Metrics
    # ------------------------------------------------------------------

    def get_metrics(self) -> dict:
        """
        Return queue metrics: pending, processing, dead_letter counts,
        and average processing time.
        """
        engine = get_engine()
        t = deductions_table
        counts = {}

        with engine.connect() as conn:
            for status in ("pending", "processing", "completed",
                           "failed_terminal", "failed_retryable", "dead_letter"):
                row = conn.execute(
                    sa.select(sa.func.count())
                    .select_from(t)
                    .where(t.c.status == status)
                ).scalar()
                counts[status] = row

        return {
            "queue_depth_pending": counts["pending"],
            "in_flight_processing": counts["processing"],
            "dead_letter_count": counts["dead_letter"],
            "completed_count": counts["completed"],
            "failed_terminal_count": counts["failed_terminal"],
            "failed_retryable_count": counts["failed_retryable"],
        }

    def log_metrics(self) -> None:
        """Log queue metrics every METRICS_INTERVAL seconds."""
        metrics = self.get_metrics()
        logger.info(
            "Deduction queue metrics: pending=%d in_flight=%d "
            "dead_letter=%d completed=%d failed_terminal=%d",
            metrics["queue_depth_pending"],
            metrics["in_flight_processing"],
            metrics["dead_letter_count"],
            metrics["completed_count"],
            metrics["failed_terminal_count"],
        )

        # Alert thresholds
        if metrics["dead_letter_count"] > 0:
            logger.critical(
                "ALERT: %d deductions in dead_letter — requires manual review",
                metrics["dead_letter_count"],
            )
        if metrics["queue_depth_pending"] > 100:
            logger.critical(
                "ALERT: queue depth %d exceeds threshold (100)",
                metrics["queue_depth_pending"],
            )

    # ------------------------------------------------------------------
    # Legacy helpers (for metering_service compat)
    # ------------------------------------------------------------------

    def mark_completed(self, idempotency_key: str) -> None:
        engine = get_engine()
        t = deductions_table
        with engine.begin() as conn:
            conn.execute(
                t.update()
                .where(t.c.idempotency_key == idempotency_key)
                .where(t.c.status.in_(["pending", "processing"]))
                .values(status="completed")
            )

    def mark_failed_terminal(self, idempotency_key: str, error: str) -> None:
        engine = get_engine()
        t = deductions_table
        with engine.begin() as conn:
            conn.execute(
                t.update()
                .where(t.c.idempotency_key == idempotency_key)
                .where(t.c.status.in_(["pending", "processing"]))
                .values(status="failed_terminal", last_error=error)
            )

    # ------------------------------------------------------------------
    # Reconciliation query
    # ------------------------------------------------------------------

    def get_completed_last_24h(self) -> list[dict]:
        """Return all completed deductions in the last 24 hours."""
        engine = get_engine()
        t = deductions_table
        cutoff = datetime.utcnow() - timedelta(hours=24)

        with engine.connect() as conn:
            rows = conn.execute(
                sa.select(t.c.id, t.c.user_id, t.c.idempotency_key,
                          t.c.payload, t.c.created_at)
                .where(t.c.status == "completed")
                .where(t.c.created_at >= cutoff)
                .order_by(t.c.created_at.asc())
            ).fetchall()

        result = []
        for row in rows:
            payload = json.loads(row.payload)
            result.append({
                "id": row.id,
                "user_id": row.user_id,
                "idempotency_key": row.idempotency_key,
                "amount_cents": payload.get("amount_cents", 0),
                "created_at": row.created_at.isoformat() if row.created_at else None,
            })
        return result

    # ------------------------------------------------------------------
    # Network send
    # ------------------------------------------------------------------

    async def _attempt_send(
        self, payload: dict, idempotency_key: str
    ) -> tuple[bool, dict, bool, int]:
        """
        Attempt to send a deduction to ai-market-backend.

        Returns (success, response_data, retryable, status_code).

        Classification:
          - 200         → success (completed)
          - 402         → failed_terminal (insufficient_funds) — NOT success
          - 400/401/403/404 → failed_terminal (client error)
          - 5xx         → failed_retryable
          - timeout/network → failed_retryable
          - JSON parse on non-5xx → failed_terminal
          - JSON parse on 5xx → failed_retryable
        """
        url = f"{settings.ai_market_url}/api/v1/credits/deduct"
        headers: dict[str, str] = {
            "Content-Type": "application/json",
            "Idempotency-Key": idempotency_key,
        }
        if settings.internal_api_key:
            headers["X-Internal-API-Key"] = settings.internal_api_key

        try:
            async with httpx.AsyncClient(timeout=10.0) as client:
                response = await client.post(url, headers=headers, json=payload)

            status_code = response.status_code

            # Try to parse JSON
            try:
                data = response.json()
            except (ValueError, Exception):
                # JSON parse failure
                if 500 <= status_code < 600:
                    # 5xx + JSON parse → retryable
                    logger.error(
                        "JSON parse failure on 5xx from ai-market: status=%d",
                        status_code,
                    )
                    return False, {}, True, status_code
                else:
                    # Non-5xx + JSON parse → terminal
                    logger.error(
                        "JSON parse failure on non-5xx from ai-market: status=%d",
                        status_code,
                    )
                    return False, {}, False, status_code

            # 200 OK → success
            if status_code == 200:
                return True, data, False, status_code

            # 402 → failed_terminal (insufficient_funds)
            if status_code == 402:
                return False, data, False, status_code

            # 400/401/403/404 → failed_terminal (client error)
            if status_code in (400, 401, 403, 404):
                logger.error(
                    "Client error from ai-market deduct: status=%d body=%s",
                    status_code, response.text[:200],
                )
                return False, data, False, status_code

            # 5xx → retryable
            if 500 <= status_code < 600:
                logger.error(
                    "5xx error from ai-market deduct: status=%d body=%s",
                    status_code, response.text[:200],
                )
                return False, data, True, status_code

            # Other unexpected status → terminal
            logger.error(
                "Unexpected status from ai-market deduct: status=%d",
                status_code,
            )
            return False, data, False, status_code

        except httpx.TimeoutException:
            logger.error("Timeout sending deduction to ai-market")
            return False, {}, True, 0

        except Exception as exc:
            logger.error("Network error sending deduction: %s", exc)
            return False, {}, True, 0


# Module-level singleton
deduction_queue = DeductionQueue()
