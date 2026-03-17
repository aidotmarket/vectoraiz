"""
Auto-Reload Service — Trigger Stripe checkout when balance is low.
===================================================================

Called after each successful deduction in the deduction queue.
Reads auto_reload.json config, checks balance, creates a Stripe
checkout session if below threshold.  Debounces via a pending file.

BQ-VZ-AUTO-RELOAD
"""

import json
import logging
import os
import time
from typing import Optional

from app.config import settings
from app.services.serial_client import SerialClient
from app.services.serial_store import get_serial_store, ACTIVE, DEGRADED

logger = logging.getLogger(__name__)

AUTO_RELOAD_PATH = os.path.join(settings.data_directory, "auto_reload.json")
PENDING_PATH = os.path.join(settings.data_directory, "auto_reload_pending.json")
DEBOUNCE_SECONDS = 3600  # 1 hour

_DEFAULT_AUTO_RELOAD = {"enabled": False, "threshold_usd": 5.0, "reload_amount_usd": 25.0}


def _read_auto_reload() -> dict:
    """Read auto-reload config from disk."""
    try:
        with open(AUTO_RELOAD_PATH) as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return dict(_DEFAULT_AUTO_RELOAD)


def _read_pending() -> Optional[dict]:
    """Read pending auto-reload file if it exists."""
    try:
        with open(PENDING_PATH) as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def _write_pending(checkout_url: str) -> None:
    """Write pending auto-reload file with checkout URL and timestamp."""
    os.makedirs(os.path.dirname(PENDING_PATH), exist_ok=True)
    with open(PENDING_PATH, "w") as f:
        json.dump({"checkout_url": checkout_url, "created_at": time.time()}, f, indent=2)


async def check_auto_reload() -> Optional[str]:
    """
    Check if auto-reload should trigger.

    Returns the checkout URL if a session was created, None otherwise.
    Never raises — all errors are caught and logged.
    """
    try:
        config = _read_auto_reload()
        if not config.get("enabled"):
            return None

        # Debounce: skip if pending file exists and is < 1 hour old
        pending = _read_pending()
        if pending and pending.get("created_at"):
            age = time.time() - pending["created_at"]
            if age < DEBOUNCE_SECONDS:
                logger.debug("Auto-reload: skipping, pending checkout is %.0fs old", age)
                return None

        # Get serial credentials
        store = get_serial_store()
        state = store.state
        if state.state not in (ACTIVE, DEGRADED):
            return None
        if not state.serial or not state.install_token:
            return None

        serial = state.serial
        install_token = state.install_token

        # Check current balance
        client = SerialClient()
        usage = await client.credits_usage(serial, install_token)
        if not usage.get("success"):
            logger.warning("Auto-reload: failed to fetch balance: %s", usage.get("error"))
            return None

        balance_usd = float(usage.get("balance_usd", 0))
        threshold_usd = float(config.get("threshold_usd", 5.0))

        if balance_usd > threshold_usd:
            return None

        # Balance is low — create checkout session
        result = await client.credits_checkout(serial, install_token)
        if not result.get("success"):
            logger.warning("Auto-reload: failed to create checkout: %s", result.get("error"))
            return None

        checkout_url = result.get("checkout_url", "")
        if checkout_url:
            _write_pending(checkout_url)
            logger.info(
                "Auto-reload triggered: balance=$%.2f below threshold=$%.2f, checkout session created",
                balance_usd, threshold_usd,
            )

        return checkout_url or None

    except Exception:
        logger.exception("Auto-reload: unexpected error")
        return None
