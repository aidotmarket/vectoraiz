"""
Metering Service Tests
=======================

Tests for MeteringService: check_balance() pre-flight gate,
calculate_cost_cents() cost computation, and report_usage() post-flight
reporting via httpx mock.

Coverage:
  - Pre-flight balance gate (auth disabled, sufficient, zero, negative, exact, custom cost)
  - Token-to-cost conversion (zero, minimum charge, typical, large, custom markup, output-heavy)
  - Post-flight reporting (auth disabled, success, 402, network error, timeout, server error)
  - Input validation (negative tokens)
  - Zero-cost optimization (0 tokens → skip network call)
  - Property accessors for config values
  - Frozen dataclass immutability

PHASE: BQ-073 — allAI Usage Metering & Prepaid Credits (Sub-task 7)
CREATED: S94 (2026-02-06)
"""

import os
import math
import pytest
from unittest.mock import AsyncMock, patch, MagicMock

# Ensure auth is disabled for unit tests by default
os.environ.setdefault("VECTORAIZ_AUTH_ENABLED", "false")

from app.services.metering_service import (
    MeteringService,
    BalanceCheck,
    UsageReport,
    CLAUDE_INPUT_COST_PER_TOKEN_CENTS,
    CLAUDE_OUTPUT_COST_PER_TOKEN_CENTS,
)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture
def svc():
    """MeteringService with known config values."""
    service = MeteringService()
    # Override with test-predictable values
    service._markup_rate = 3.0
    service._min_cost_cents = 1
    service._estimated_query_cost = 3
    return service


@pytest.fixture
def svc_auth_enabled(svc):
    """MeteringService with auth enabled."""
    with patch("app.services.metering_service.settings") as mock_settings:
        mock_settings.auth_enabled = True
        mock_settings.copilot_markup_rate = 3.0
        mock_settings.copilot_min_cost_cents = 1
        mock_settings.copilot_estimated_query_cost_cents = 3
        mock_settings.ai_market_url = "https://ai-market-backend-production.up.railway.app"
        mock_settings.internal_api_key = "test-internal-key"
        yield svc


# ---------------------------------------------------------------------------
# check_balance() tests
# ---------------------------------------------------------------------------

class TestCheckBalance:
    """Pre-flight balance gate tests."""

    def test_allows_when_auth_disabled(self, svc):
        """When auth is disabled, always allow regardless of balance."""
        result = svc.check_balance(balance_cents=0)
        assert result.allowed is True
        assert result.reason == "auth_disabled"

    def test_allows_sufficient_balance(self, svc_auth_enabled):
        """Allow when balance exceeds estimated cost."""
        result = svc_auth_enabled.check_balance(balance_cents=100)
        assert result.allowed is True
        assert result.balance_cents == 100
        assert result.estimated_cost_cents == 3

    def test_blocks_zero_balance(self, svc_auth_enabled):
        """Block when balance is zero."""
        result = svc_auth_enabled.check_balance(balance_cents=0)
        assert result.allowed is False
        assert result.reason == "zero_balance"

    def test_blocks_negative_balance(self, svc_auth_enabled):
        """Block when balance is negative."""
        result = svc_auth_enabled.check_balance(balance_cents=-5)
        assert result.allowed is False
        assert result.reason == "zero_balance"

    def test_blocks_insufficient_balance(self, svc_auth_enabled):
        """Block when balance is below estimated cost."""
        result = svc_auth_enabled.check_balance(balance_cents=2)
        assert result.allowed is False
        assert result.reason == "insufficient_balance"

    def test_allows_exact_balance(self, svc_auth_enabled):
        """Allow when balance equals estimated cost exactly."""
        result = svc_auth_enabled.check_balance(balance_cents=3)
        assert result.allowed is True

    def test_custom_estimated_cost(self, svc_auth_enabled):
        """Allow custom estimated_cost_cents override."""
        result = svc_auth_enabled.check_balance(
            balance_cents=5, estimated_cost_cents=10
        )
        assert result.allowed is False
        assert result.estimated_cost_cents == 10
        assert result.reason == "insufficient_balance"

    def test_custom_estimated_cost_sufficient(self, svc_auth_enabled):
        """Custom estimated cost with sufficient balance."""
        result = svc_auth_enabled.check_balance(
            balance_cents=50, estimated_cost_cents=10
        )
        assert result.allowed is True
        assert result.estimated_cost_cents == 10


# ---------------------------------------------------------------------------
# calculate_cost_cents() tests
# ---------------------------------------------------------------------------

class TestCalculateCostCents:
    """Token-to-cost conversion tests."""

    def test_zero_tokens(self, svc):
        """Zero tokens = zero cost."""
        assert svc.calculate_cost_cents(0, 0) == 0

    def test_minimum_charge_for_nonzero_usage(self, svc):
        """Even tiny usage incurs minimum charge."""
        # 1 input token = 0.0003 cents wholesale * 3.0 = 0.0009 cents → ceil = 1 cent
        # But min_cost_cents = 1, so floor is 1 cent
        cost = svc.calculate_cost_cents(1, 0)
        assert cost == 1

    def test_typical_copilot_query(self, svc):
        """Typical query: ~1000 input, ~500 output tokens."""
        # Wholesale: 1000 * 0.0003 + 500 * 0.0015 = 0.3 + 0.75 = 1.05 cents
        # With 3x markup: 1.05 * 3 = 3.15 → ceil = 4 cents
        cost = svc.calculate_cost_cents(1000, 500)
        expected_wholesale = (
            1000 * CLAUDE_INPUT_COST_PER_TOKEN_CENTS
            + 500 * CLAUDE_OUTPUT_COST_PER_TOKEN_CENTS
        )
        expected = math.ceil(expected_wholesale * 3.0)
        assert cost == expected
        assert cost == 4  # Verify the math

    def test_large_query(self, svc):
        """Large query: 10K input, 4K output."""
        # Wholesale: 10000 * 0.0003 + 4000 * 0.0015 = 3.0 + 6.0 = 9.0 cents
        # With 3x markup: 9.0 * 3 = 27.0 → ceil = 27 cents
        cost = svc.calculate_cost_cents(10000, 4000)
        assert cost == 27

    def test_custom_markup_rate(self, svc):
        """Custom markup rate override."""
        cost_3x = svc.calculate_cost_cents(1000, 500, markup_rate=3.0)
        cost_5x = svc.calculate_cost_cents(1000, 500, markup_rate=5.0)
        assert cost_5x > cost_3x

        # 5x: 1.05 * 5 = 5.25 → ceil = 6
        expected_wholesale = (
            1000 * CLAUDE_INPUT_COST_PER_TOKEN_CENTS
            + 500 * CLAUDE_OUTPUT_COST_PER_TOKEN_CENTS
        )
        assert cost_5x == math.ceil(expected_wholesale * 5.0)

    def test_output_heavy_query(self, svc):
        """Output tokens cost 5x more than input tokens."""
        cost_input = svc.calculate_cost_cents(1000, 0)
        cost_output = svc.calculate_cost_cents(0, 1000)
        # Output cost per token is 5x input (15 vs 3 per M)
        assert cost_output == cost_input * 5

    def test_markup_rate_1x(self, svc):
        """At 1x markup, cost equals wholesale (rounded up)."""
        cost = svc.calculate_cost_cents(10000, 5000, markup_rate=1.0)
        wholesale = (
            10000 * CLAUDE_INPUT_COST_PER_TOKEN_CENTS
            + 5000 * CLAUDE_OUTPUT_COST_PER_TOKEN_CENTS
        )
        assert cost == math.ceil(wholesale)

    def test_negative_input_tokens_raises(self, svc):
        """Negative input tokens raise ValueError."""
        with pytest.raises(ValueError, match="non-negative"):
            svc.calculate_cost_cents(-1, 0)

    def test_negative_output_tokens_raises(self, svc):
        """Negative output tokens raise ValueError."""
        with pytest.raises(ValueError, match="non-negative"):
            svc.calculate_cost_cents(0, -1)


# ---------------------------------------------------------------------------
# report_usage() tests
# ---------------------------------------------------------------------------

class TestReportUsage:
    """Post-flight usage reporting tests."""

    @pytest.mark.asyncio
    async def test_skips_when_auth_disabled(self, svc):
        """When auth is disabled, skip reporting and return success."""
        report = await svc.report_usage(
            user_id="user-123",
            service="copilot",
            model="claude-sonnet-4-20250514",
            input_tokens=1000,
            output_tokens=500,
        )
        assert report.success is True
        assert report.cost_cents == 0
        assert report.allowed is True

    @pytest.mark.asyncio
    async def test_successful_deduction(self, svc_auth_enabled):
        """Successful deduction returns new balance."""
        mock_response = MagicMock()
        mock_response.status_code = 200
        mock_response.json.return_value = {
            "success": True,
            "balance_cents": 96,
            "amount_deducted_cents": 4,
            "user_id": "user-123",
        }

        mock_client = AsyncMock()
        mock_client.__aenter__ = AsyncMock(return_value=mock_client)
        mock_client.__aexit__ = AsyncMock(return_value=False)
        mock_client.post = AsyncMock(return_value=mock_response)

        with patch("app.services.metering_service.httpx.AsyncClient", return_value=mock_client):
            report = await svc_auth_enabled.report_usage(
                user_id="user-123",
                service="copilot",
                model="claude-sonnet-4-20250514",
                input_tokens=1000,
                output_tokens=500,
            )

        assert report.success is True
        assert report.cost_cents == 4  # 1000in + 500out at 3x
        assert report.new_balance_cents == 96
        assert report.allowed is True

        # Verify the POST was called correctly
        call_args = mock_client.post.call_args
        assert "/api/v1/credits/deduct" in call_args[0][0]
        payload = call_args[1]["json"]
        assert payload["user_id"] == "user-123"
        assert payload["amount_cents"] == 4
        assert payload["service"] == "copilot"
        assert payload["tokens_in"] == 1000
        assert payload["tokens_out"] == 500
        assert payload["markup_rate"] == 3.0
        assert payload["idempotency_key"] is not None

        # Verify internal API key header
        headers = call_args[1]["headers"]
        assert headers["X-Internal-API-Key"] == "test-internal-key"

    @pytest.mark.asyncio
    async def test_insufficient_credits_402(self, svc_auth_enabled):
        """402 response means insufficient credits."""
        mock_response = MagicMock()
        mock_response.status_code = 402
        mock_response.json.return_value = {
            "detail": {
                "error": "insufficient_credits",
                "balance_cents": 2,
                "required_cents": 4,
            }
        }

        mock_client = AsyncMock()
        mock_client.__aenter__ = AsyncMock(return_value=mock_client)
        mock_client.__aexit__ = AsyncMock(return_value=False)
        mock_client.post = AsyncMock(return_value=mock_response)

        with patch("app.services.metering_service.httpx.AsyncClient", return_value=mock_client):
            report = await svc_auth_enabled.report_usage(
                user_id="user-123",
                service="copilot",
                model="claude-sonnet-4-20250514",
                input_tokens=1000,
                output_tokens=500,
            )

        assert report.success is False
        assert report.allowed is False
        assert report.new_balance_cents == 2

    @pytest.mark.asyncio
    async def test_network_error_fails_open(self, svc_auth_enabled):
        """Network errors fail open (allow next request)."""
        mock_client = AsyncMock()
        mock_client.__aenter__ = AsyncMock(return_value=mock_client)
        mock_client.__aexit__ = AsyncMock(return_value=False)
        mock_client.post = AsyncMock(side_effect=Exception("Connection refused"))

        with patch("app.services.metering_service.httpx.AsyncClient", return_value=mock_client):
            report = await svc_auth_enabled.report_usage(
                user_id="user-123",
                service="copilot",
                model="claude-sonnet-4-20250514",
                input_tokens=1000,
                output_tokens=500,
            )

        assert report.success is False
        assert report.allowed is True  # Fail open

    @pytest.mark.asyncio
    async def test_timeout_fails_open(self, svc_auth_enabled):
        """Timeout errors fail open (allow next request)."""
        import httpx

        mock_client = AsyncMock()
        mock_client.__aenter__ = AsyncMock(return_value=mock_client)
        mock_client.__aexit__ = AsyncMock(return_value=False)
        mock_client.post = AsyncMock(
            side_effect=httpx.TimeoutException("read timeout")
        )

        with patch("app.services.metering_service.httpx.AsyncClient", return_value=mock_client):
            report = await svc_auth_enabled.report_usage(
                user_id="user-123",
                service="copilot",
                model="claude-sonnet-4-20250514",
                input_tokens=1000,
                output_tokens=500,
            )

        assert report.success is False
        assert report.allowed is True  # Fail open

    @pytest.mark.asyncio
    async def test_server_error_fails_open(self, svc_auth_enabled):
        """500 errors fail open (allow next request)."""
        mock_response = MagicMock()
        mock_response.status_code = 500
        mock_response.text = "Internal Server Error"

        mock_client = AsyncMock()
        mock_client.__aenter__ = AsyncMock(return_value=mock_client)
        mock_client.__aexit__ = AsyncMock(return_value=False)
        mock_client.post = AsyncMock(return_value=mock_response)

        with patch("app.services.metering_service.httpx.AsyncClient", return_value=mock_client):
            report = await svc_auth_enabled.report_usage(
                user_id="user-123",
                service="copilot",
                model="claude-sonnet-4-20250514",
                input_tokens=1000,
                output_tokens=500,
            )

        assert report.success is False
        assert report.allowed is True  # Fail open

    @pytest.mark.asyncio
    async def test_zero_tokens_skips_network_call(self, svc_auth_enabled):
        """Zero tokens (0 cost) skips network call entirely."""
        # Should NOT make any HTTP call
        with patch("app.services.metering_service.httpx.AsyncClient") as mock_client_cls:
            report = await svc_auth_enabled.report_usage(
                user_id="user-123",
                service="copilot",
                model="claude-sonnet-4-20250514",
                input_tokens=0,
                output_tokens=0,
            )
        # httpx.AsyncClient should never be instantiated
        mock_client_cls.assert_not_called()
        assert report.success is True
        assert report.cost_cents == 0
        assert report.allowed is True


# ---------------------------------------------------------------------------
# Dataclass immutability tests
# ---------------------------------------------------------------------------

class TestDataclasses:
    """Verify frozen dataclass behavior."""

    def test_balance_check_is_frozen(self):
        """BalanceCheck fields cannot be mutated."""
        bc = BalanceCheck(allowed=True, balance_cents=100, estimated_cost_cents=3)
        with pytest.raises(AttributeError):
            bc.allowed = False  # type: ignore[misc]

    def test_usage_report_is_frozen(self):
        """UsageReport fields cannot be mutated."""
        ur = UsageReport(success=True, cost_cents=4, new_balance_cents=96, allowed=True)
        with pytest.raises(AttributeError):
            ur.success = False  # type: ignore[misc]


# ---------------------------------------------------------------------------
# Property accessor tests
# ---------------------------------------------------------------------------

class TestProperties:
    """Verify read-only config property accessors."""

    def test_markup_rate_property(self, svc):
        """markup_rate property returns configured value."""
        assert svc.markup_rate == 3.0

    def test_min_cost_cents_property(self, svc):
        """min_cost_cents property returns configured value."""
        assert svc.min_cost_cents == 1

    def test_estimated_query_cost_property(self, svc):
        """estimated_query_cost property returns configured value."""
        assert svc.estimated_query_cost == 3
