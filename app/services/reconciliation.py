"""
Reconciliation Worker — Daily Deduction/Balance Cross-Reference
================================================================

PURPOSE:
    Queries completed deductions in the last 24 hours from the local
    SQLite queue, cross-references with ai-market-backend balance per user,
    and flags discrepancies greater than 1 cent.

    Logs structured JSON for each discrepancy found.

SCHEDULE:
    Intended to be called once daily (e.g. via cron, APScheduler, or
    background task on startup).

BQ-113: Exactly-Once Billing (P0) — AC 15
CREATED: S120 (2026-02-12)
"""

from __future__ import annotations

import json
import logging
from collections import defaultdict
from typing import Any

import httpx

from app.config import settings
from .deduction_queue import deduction_queue

logger = logging.getLogger(__name__)

DISCREPANCY_THRESHOLD_CENTS = 1


async def run_daily_reconciliation() -> dict[str, Any]:
    """
    Daily reconciliation: compare local completed deductions against
    ai-market-backend user balances.

    Returns a summary dict with counts and any discrepancies found.
    """
    logger.info("Starting daily reconciliation run")

    # 1. Aggregate completed deductions by user_id
    completed = deduction_queue.get_completed_last_24h()
    user_totals: dict[str, int] = defaultdict(int)
    for row in completed:
        user_totals[row["user_id"]] += row["amount_cents"]

    if not user_totals:
        logger.info("Reconciliation: no completed deductions in last 24h")
        return {
            "status": "ok",
            "users_checked": 0,
            "discrepancies": [],
        }

    logger.info(
        "Reconciliation: %d completed deductions across %d users",
        len(completed),
        len(user_totals),
    )

    # 2. Cross-reference with ai-market-backend balances
    discrepancies: list[dict[str, Any]] = []

    for user_id, local_total_cents in user_totals.items():
        remote_balance = await _fetch_remote_balance(user_id)
        if remote_balance is None:
            discrepancies.append({
                "user_id": user_id,
                "local_deducted_cents": local_total_cents,
                "remote_balance_cents": None,
                "reason": "remote_balance_unavailable",
            })
            logger.warning(
                json.dumps({
                    "event": "reconciliation_discrepancy",
                    "user_id": user_id,
                    "local_deducted_cents": local_total_cents,
                    "remote_balance_cents": None,
                    "reason": "remote_balance_unavailable",
                })
            )
            continue

        # We can't know the user's starting balance, so we check if
        # the remote system acknowledges the same total deductions.
        # Fetch remote deduction total for same period if available.
        remote_deducted = await _fetch_remote_deductions_total(user_id)
        if remote_deducted is None:
            # If we can't get remote deduction total, just log balance info
            logger.info(
                json.dumps({
                    "event": "reconciliation_check",
                    "user_id": user_id,
                    "local_deducted_cents": local_total_cents,
                    "remote_balance_cents": remote_balance,
                })
            )
            continue

        diff = abs(local_total_cents - remote_deducted)
        if diff > DISCREPANCY_THRESHOLD_CENTS:
            discrepancy = {
                "user_id": user_id,
                "local_deducted_cents": local_total_cents,
                "remote_deducted_cents": remote_deducted,
                "diff_cents": diff,
                "reason": "amount_mismatch",
            }
            discrepancies.append(discrepancy)
            logger.warning(
                json.dumps({
                    "event": "reconciliation_discrepancy",
                    **discrepancy,
                })
            )

    summary = {
        "status": "completed",
        "users_checked": len(user_totals),
        "total_deductions": len(completed),
        "discrepancy_count": len(discrepancies),
        "discrepancies": discrepancies,
    }

    if discrepancies:
        logger.critical(
            "Reconciliation found %d discrepancies — manual review required",
            len(discrepancies),
        )
    else:
        logger.info("Reconciliation complete: no discrepancies found")

    logger.info(json.dumps({"event": "reconciliation_summary", **summary}))
    return summary


async def _fetch_remote_balance(user_id: str) -> int | None:
    """Fetch a user's current balance from ai-market-backend."""
    url = f"{settings.ai_market_url}/api/v1/credits/balance/{user_id}"
    headers: dict[str, str] = {"Content-Type": "application/json"}
    if settings.internal_api_key:
        headers["X-Internal-API-Key"] = settings.internal_api_key

    try:
        async with httpx.AsyncClient(timeout=10.0) as client:
            response = await client.get(url, headers=headers)
        if response.status_code == 200:
            data = response.json()
            return data.get("balance_cents", 0)
        logger.warning(
            "Failed to fetch balance for user=%s: status=%d",
            user_id, response.status_code,
        )
        return None
    except Exception as exc:
        logger.error(
            "Error fetching balance for user=%s: %s", user_id, exc
        )
        return None


async def _fetch_remote_deductions_total(user_id: str) -> int | None:
    """
    Fetch the total deductions for a user in the last 24 hours from
    ai-market-backend.  Returns None if the endpoint is unavailable.
    """
    url = f"{settings.ai_market_url}/api/v1/credits/deductions/{user_id}"
    headers: dict[str, str] = {"Content-Type": "application/json"}
    if settings.internal_api_key:
        headers["X-Internal-API-Key"] = settings.internal_api_key

    try:
        async with httpx.AsyncClient(timeout=10.0) as client:
            response = await client.get(
                url, headers=headers, params={"period": "24h"}
            )
        if response.status_code == 200:
            data = response.json()
            return data.get("total_deducted_cents", None)
        return None
    except Exception:
        return None
