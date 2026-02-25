"""
Tests for SerialMeteringStrategy — meter allow/deny, offline behavior, category routing.

BQ-VZ-SERIAL-CLIENT
"""

import os
import pytest
from decimal import Decimal
from unittest.mock import AsyncMock, MagicMock, patch

from app.services.serial_metering import (
    ActivationRequiredException,
    CreditExhaustedException,
    LedgerMeteringStrategy,
    MeterDecision,
    SerialMeteringStrategy,
    UnprovisionedException,
    _make_request_id,
    classify_copilot_category,
)
from app.services.serial_store import (
    ACTIVE,
    DEGRADED,
    MIGRATED,
    PROVISIONED,
    UNPROVISIONED,
    SerialState,
    SerialStore,
)
from app.services.serial_client import MeterResult
from app.services.offline_queue import OfflineQueue


@pytest.fixture
def mock_store(tmp_path):
    path = str(tmp_path / "serial.json")
    store = SerialStore(path=path)
    return store


@pytest.fixture
def mock_client():
    return AsyncMock()


@pytest.fixture
def mock_queue(tmp_path):
    path = str(tmp_path / "pending_usage.jsonl")
    return OfflineQueue(path=path)


class TestUnprovisionedState:
    @pytest.mark.asyncio
    async def test_blocks_all_operations(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = UNPROVISIONED
        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)

        with pytest.raises(UnprovisionedException):
            await strategy.check_and_meter("setup", Decimal("0.01"), "req_1")

        with pytest.raises(UnprovisionedException):
            await strategy.check_and_meter("data", Decimal("0.03"), "req_2")


class TestProvisionedState:
    @pytest.mark.asyncio
    async def test_allows_setup_offline(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = PROVISIONED
        mock_store.state.serial = "VZ-test"
        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)

        decision = await strategy.check_and_meter("setup", Decimal("0.01"), "req_1")
        assert decision.allowed is True
        assert decision.offline is True
        assert mock_queue.count() == 1

    @pytest.mark.asyncio
    async def test_blocks_data(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = PROVISIONED
        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)

        with pytest.raises(ActivationRequiredException):
            await strategy.check_and_meter("data", Decimal("0.03"), "req_1")


class TestActiveState:
    @pytest.mark.asyncio
    async def test_meter_allowed(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = ACTIVE
        mock_store.state.serial = "VZ-test"
        mock_store.state.install_token = "vzit_test"

        mock_client.meter = AsyncMock(return_value=MeterResult(
            allowed=True, category="data", cost_usd="0.03",
            remaining_usd="3.97", status_code=200,
        ))

        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)
        decision = await strategy.check_and_meter("data", Decimal("0.03"), "req_1")

        assert decision.allowed is True
        assert decision.category == "data"

    @pytest.mark.asyncio
    async def test_meter_denied_raises_credit_exhausted(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = ACTIVE
        mock_store.state.serial = "VZ-test1234"
        mock_store.state.install_token = "vzit_test"

        mock_client.meter = AsyncMock(return_value=MeterResult(
            allowed=False, category="data", cost_usd="0.03",
            remaining_usd="0.00", reason="insufficient_data_credits",
            status_code=402,
        ))

        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)

        with pytest.raises(CreditExhaustedException) as exc_info:
            await strategy.check_and_meter("data", Decimal("0.03"), "req_1")

        assert exc_info.value.category == "data"
        assert "insufficient_data_credits" in exc_info.value.reason

    @pytest.mark.asyncio
    async def test_meter_migration_detected(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = ACTIVE
        mock_store.state.serial = "VZ-test"
        mock_store.state.install_token = "vzit_test"
        mock_store.state.last_status_cache = {}
        mock_store.save()

        mock_client.meter = AsyncMock(return_value=MeterResult(
            allowed=True, category="data", migrated=True, status_code=200,
        ))

        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)
        decision = await strategy.check_and_meter("data", Decimal("0.03"), "req_1")

        assert decision.allowed is True
        assert mock_store.state.state == MIGRATED

    @pytest.mark.asyncio
    async def test_401_transitions_to_unprovisioned(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = ACTIVE
        mock_store.state.serial = "VZ-test"
        mock_store.state.install_token = "vzit_test"

        mock_client.meter = AsyncMock(return_value=MeterResult(
            allowed=False, status_code=401, error="token revoked",
        ))

        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)

        with pytest.raises(ActivationRequiredException):
            await strategy.check_and_meter("data", Decimal("0.03"), "req_1")

        assert mock_store.state.state == UNPROVISIONED


class TestDegradedState:
    @pytest.mark.asyncio
    async def test_allows_setup_offline(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = DEGRADED
        mock_store.state.serial = "VZ-test"
        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)

        decision = await strategy.check_and_meter("setup", Decimal("0.01"), "req_1")
        assert decision.allowed is True
        assert decision.offline is True

    @pytest.mark.asyncio
    async def test_blocks_data(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = DEGRADED
        mock_store.state.serial = "VZ-test"
        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)

        with pytest.raises(CreditExhaustedException):
            await strategy.check_and_meter("data", Decimal("0.03"), "req_1")


class TestNetworkFailureOfflinePolicy:
    @pytest.mark.asyncio
    async def test_setup_allowed_on_failure(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = ACTIVE
        mock_store.state.serial = "VZ-test"
        mock_store.state.install_token = "vzit_test"

        mock_client.meter = AsyncMock(return_value=MeterResult(
            allowed=False, status_code=0, error="network error",
        ))

        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)
        decision = await strategy.check_and_meter("setup", Decimal("0.01"), "req_1")

        assert decision.allowed is True
        assert decision.offline is True

    @pytest.mark.asyncio
    async def test_data_allowed_transient_failure(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = ACTIVE
        mock_store.state.serial = "VZ-test"
        mock_store.state.install_token = "vzit_test"
        mock_store.state.consecutive_failures = 0
        mock_store.save()

        mock_client.meter = AsyncMock(return_value=MeterResult(
            allowed=False, status_code=0, error="timeout",
        ))

        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)
        decision = await strategy.check_and_meter("data", Decimal("0.03"), "req_1")

        assert decision.allowed is True  # < 3 failures

    @pytest.mark.asyncio
    async def test_data_blocked_after_threshold(self, mock_store, mock_client, mock_queue):
        mock_store.state.state = ACTIVE
        mock_store.state.serial = "VZ-test"
        mock_store.state.install_token = "vzit_test"
        mock_store.state.consecutive_failures = 2  # Will become 3 after record_failure
        mock_store.save()

        mock_client.meter = AsyncMock(return_value=MeterResult(
            allowed=False, status_code=0, error="timeout",
        ))

        strategy = SerialMeteringStrategy(mock_store, mock_client, mock_queue)

        with pytest.raises(CreditExhaustedException):
            await strategy.check_and_meter("data", Decimal("0.03"), "req_1")


class TestLedgerMeteringStrategy:
    @pytest.mark.asyncio
    async def test_always_allows(self):
        strategy = LedgerMeteringStrategy()
        decision = await strategy.check_and_meter("data", Decimal("0.03"), "req_1")
        assert decision.allowed is True


class TestRequestIdGeneration:
    def test_format(self):
        rid = _make_request_id("VZ-abcd1234-efgh5678", "POST:/api/allai/generate")
        parts = rid.split(":")
        assert parts[0] == "vz"
        assert parts[1] == "abcd1234"
        assert len(parts) == 4

    def test_different_endpoints_different_hashes(self):
        rid1 = _make_request_id("VZ-test", "POST:/generate")
        rid2 = _make_request_id("VZ-test", "POST:/query")
        # Endpoint hash differs
        assert rid1.split(":")[2] != rid2.split(":")[2]


class TestCopilotCategoryClassification:
    def test_setup_views(self):
        for view in ("onboarding", "setup", "connectivity", "metadata_builder", "publish"):
            assert classify_copilot_category(view) == "setup"

    def test_data_views(self):
        for view in ("dashboard", "data", "query", "explore", "unknown"):
            assert classify_copilot_category(view) == "data"

    def test_none_defaults_to_data(self):
        assert classify_copilot_category(None) == "data"

    def test_empty_string_defaults_to_data(self):
        assert classify_copilot_category("") == "data"
