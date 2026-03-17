"""
Tests for auto-reload service — BQ-VZ-AUTO-RELOAD
"""

import json
import time
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from app.services.auto_reload_service import (
    check_auto_reload,
    DEBOUNCE_SECONDS,
)


@pytest.fixture
def auto_reload_config(tmp_path):
    """Patch AUTO_RELOAD_PATH and PENDING_PATH to use tmp_path."""
    config_path = str(tmp_path / "auto_reload.json")
    pending_path = str(tmp_path / "auto_reload_pending.json")
    with patch("app.services.auto_reload_service.AUTO_RELOAD_PATH", config_path), \
         patch("app.services.auto_reload_service.PENDING_PATH", pending_path):
        yield config_path, pending_path


def _write_config(path, enabled=True, threshold=5.0):
    with open(path, "w") as f:
        json.dump({"enabled": enabled, "threshold_usd": threshold, "reload_amount_usd": 25.0}, f)


def _write_pending(path, age_seconds=0):
    with open(path, "w") as f:
        json.dump({"checkout_url": "https://checkout.stripe.com/old", "created_at": time.time() - age_seconds}, f)


def _mock_serial_store(active=True):
    store = MagicMock()
    state = MagicMock()
    state.state = "active" if active else "unprovisioned"
    state.serial = "SER-123" if active else ""
    state.install_token = "tok-abc" if active else None
    store.state = state
    return store


@pytest.mark.asyncio
async def test_check_auto_reload_disabled(auto_reload_config):
    """Config disabled → returns None, no checkout call."""
    config_path, _ = auto_reload_config
    _write_config(config_path, enabled=False)

    result = await check_auto_reload()
    assert result is None


@pytest.mark.asyncio
async def test_check_auto_reload_no_config(auto_reload_config):
    """No config file → defaults to disabled → returns None."""
    result = await check_auto_reload()
    assert result is None


@pytest.mark.asyncio
async def test_check_auto_reload_above_threshold(auto_reload_config):
    """Balance above threshold → returns None."""
    config_path, _ = auto_reload_config
    _write_config(config_path, enabled=True, threshold=5.0)

    mock_store = _mock_serial_store(active=True)
    mock_client = AsyncMock()
    mock_client.credits_usage.return_value = {"success": True, "balance_usd": 10.0}

    with patch("app.services.auto_reload_service.get_serial_store", return_value=mock_store), \
         patch("app.services.auto_reload_service.SerialClient", return_value=mock_client):
        result = await check_auto_reload()

    assert result is None
    mock_client.credits_checkout.assert_not_called()


@pytest.mark.asyncio
async def test_check_auto_reload_triggers(auto_reload_config):
    """Balance below threshold → creates checkout, writes pending file."""
    config_path, pending_path = auto_reload_config
    _write_config(config_path, enabled=True, threshold=5.0)

    mock_store = _mock_serial_store(active=True)
    mock_client = AsyncMock()
    mock_client.credits_usage.return_value = {"success": True, "balance_usd": 2.50}
    mock_client.credits_checkout.return_value = {"success": True, "checkout_url": "https://checkout.stripe.com/new"}

    with patch("app.services.auto_reload_service.get_serial_store", return_value=mock_store), \
         patch("app.services.auto_reload_service.SerialClient", return_value=mock_client):
        result = await check_auto_reload()

    assert result == "https://checkout.stripe.com/new"
    mock_client.credits_checkout.assert_called_once_with("SER-123", "tok-abc")

    # Verify pending file was written
    with open(pending_path) as f:
        pending = json.load(f)
    assert pending["checkout_url"] == "https://checkout.stripe.com/new"
    assert pending["created_at"] > 0


@pytest.mark.asyncio
async def test_check_auto_reload_debounce(auto_reload_config):
    """Pending file < 1hr old → skips."""
    config_path, pending_path = auto_reload_config
    _write_config(config_path, enabled=True, threshold=5.0)
    _write_pending(pending_path, age_seconds=600)  # 10 minutes ago

    result = await check_auto_reload()
    assert result is None


@pytest.mark.asyncio
async def test_check_auto_reload_debounce_expired(auto_reload_config):
    """Pending file > 1hr old → proceeds."""
    config_path, pending_path = auto_reload_config
    _write_config(config_path, enabled=True, threshold=5.0)
    _write_pending(pending_path, age_seconds=DEBOUNCE_SECONDS + 60)  # Over 1 hour ago

    mock_store = _mock_serial_store(active=True)
    mock_client = AsyncMock()
    mock_client.credits_usage.return_value = {"success": True, "balance_usd": 1.0}
    mock_client.credits_checkout.return_value = {"success": True, "checkout_url": "https://checkout.stripe.com/fresh"}

    with patch("app.services.auto_reload_service.get_serial_store", return_value=mock_store), \
         patch("app.services.auto_reload_service.SerialClient", return_value=mock_client):
        result = await check_auto_reload()

    assert result == "https://checkout.stripe.com/fresh"


@pytest.mark.asyncio
async def test_check_auto_reload_serial_not_active(auto_reload_config):
    """Serial not active → returns None silently."""
    config_path, _ = auto_reload_config
    _write_config(config_path, enabled=True, threshold=5.0)

    mock_store = _mock_serial_store(active=False)

    with patch("app.services.auto_reload_service.get_serial_store", return_value=mock_store):
        result = await check_auto_reload()

    assert result is None


@pytest.mark.asyncio
async def test_deduction_queue_calls_auto_reload():
    """Verify check_auto_reload is called after successful deduction."""
    with patch("app.services.deduction_queue.check_auto_reload", new_callable=AsyncMock) as mock_reload:
        from app.services.deduction_queue import DeductionQueue

        queue = DeductionQueue()

        # Mock internal methods
        mock_items = [{"id": 1, "payload": json.dumps({"user_id": "u1", "idempotency_key": "k1"}), "attempt_count": 0, "idempotency_key": "k1"}]

        with patch.object(queue, "_claim_batch", return_value=mock_items), \
             patch.object(queue, "_attempt_send", new_callable=AsyncMock, return_value=(True, {}, False, 200)), \
             patch("app.services.deduction_queue.get_engine") as mock_engine:

            # Set up mock engine context manager
            mock_conn = MagicMock()
            mock_ctx = MagicMock()
            mock_ctx.__enter__ = MagicMock(return_value=mock_conn)
            mock_ctx.__exit__ = MagicMock(return_value=False)
            mock_engine.return_value.begin.return_value = mock_ctx

            await queue.process_all_pending()

        mock_reload.assert_called_once()
