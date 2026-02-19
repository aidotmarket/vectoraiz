"""
Tests for connectivity rate limiter — all 5 layers.

BQ-MCP-RAG Phase 1: per-token, per-IP block after 5 failures,
global, per-tool SQL, concurrency.
"""

import time

import pytest

from app.services.connectivity_rate_limiter import ConnectivityRateLimiter


@pytest.fixture
def limiter():
    return ConnectivityRateLimiter(
        per_token_rpm=5,
        per_ip_auth_fail_limit=3,
        ip_block_duration_s=10,
        global_rpm=10,
        per_tool_sql_rpm=2,
        max_concurrent_per_token=2,
    )


# ---------------------------------------------------------------------------
# Per-token rate limit
# ---------------------------------------------------------------------------

class TestPerTokenRateLimit:
    def test_allows_within_limit(self, limiter):
        for _ in range(5):
            result = limiter.check_rate_limits("tok1", "list_datasets", "127.0.0.1")
            assert result is None
            limiter.record_request("tok1", "list_datasets")

    def test_blocks_above_limit(self, limiter):
        for _ in range(5):
            limiter.record_request("tok1", "list_datasets")

        result = limiter.check_rate_limits("tok1", "list_datasets", "127.0.0.1")
        assert result == "rate_limited"

    def test_different_tokens_independent(self, limiter):
        for _ in range(5):
            limiter.record_request("tok1", "list_datasets")

        # tok2 should still be allowed
        result = limiter.check_rate_limits("tok2", "list_datasets", "127.0.0.1")
        assert result is None


# ---------------------------------------------------------------------------
# Per-IP auth failure blocking
# ---------------------------------------------------------------------------

class TestPerIPBlock:
    def test_allows_below_threshold(self, limiter):
        limiter.record_auth_failure("192.168.1.1")
        limiter.record_auth_failure("192.168.1.1")
        result = limiter.check_ip_blocked("192.168.1.1")
        assert result is None

    def test_blocks_at_threshold(self, limiter):
        for _ in range(3):
            limiter.record_auth_failure("192.168.1.2")

        result = limiter.check_ip_blocked("192.168.1.2")
        assert result == "ip_blocked"

    def test_block_expires(self, limiter):
        # Use a very short block duration for testing
        limiter.ip_block_duration_s = 0.1
        for _ in range(3):
            limiter.record_auth_failure("192.168.1.3")

        assert limiter.check_ip_blocked("192.168.1.3") == "ip_blocked"

        time.sleep(0.15)
        assert limiter.check_ip_blocked("192.168.1.3") is None

    def test_record_auth_failure_returns_blocked(self, limiter):
        for i in range(2):
            result = limiter.record_auth_failure("192.168.1.4")
            assert result is None

        result = limiter.record_auth_failure("192.168.1.4")
        assert result == "ip_blocked"

    def test_different_ips_independent(self, limiter):
        for _ in range(3):
            limiter.record_auth_failure("192.168.1.5")

        result = limiter.check_ip_blocked("192.168.1.6")
        assert result is None

    def test_ip_block_remaining(self, limiter):
        limiter.ip_block_duration_s = 60
        for _ in range(3):
            limiter.record_auth_failure("192.168.1.7")

        remaining = limiter.get_ip_block_remaining("192.168.1.7")
        assert remaining > 50  # should be close to 60

    def test_ip_not_blocked_remaining(self, limiter):
        assert limiter.get_ip_block_remaining("1.2.3.4") == 0.0


# ---------------------------------------------------------------------------
# Global rate limit
# ---------------------------------------------------------------------------

class TestGlobalRateLimit:
    def test_blocks_above_global_limit(self, limiter):
        for i in range(10):
            limiter.record_request(f"tok{i}", "list_datasets")

        result = limiter.check_rate_limits("tokNew", "list_datasets", "127.0.0.1")
        assert result == "rate_limited"


# ---------------------------------------------------------------------------
# Per-tool SQL rate limit
# ---------------------------------------------------------------------------

class TestPerToolSQLRateLimit:
    def test_sql_within_limit(self, limiter):
        limiter.record_request("tok1", "execute_sql")
        result = limiter.check_rate_limits("tok1", "execute_sql", "127.0.0.1")
        assert result is None

    def test_sql_above_limit(self, limiter):
        for _ in range(2):
            limiter.record_request("tok1", "execute_sql")

        result = limiter.check_rate_limits("tok1", "execute_sql", "127.0.0.1")
        assert result == "rate_limited"

    def test_sql_limit_independent_of_non_sql(self, limiter):
        """Non-SQL requests don't count toward SQL limit."""
        for _ in range(4):
            limiter.record_request("tok1", "list_datasets")

        result = limiter.check_rate_limits("tok1", "execute_sql", "127.0.0.1")
        assert result is None

    def test_vectoraiz_sql_tool_name(self, limiter):
        """Tool name 'vectoraiz_sql' should also count toward SQL limit."""
        for _ in range(2):
            limiter.record_request("tok1", "vectoraiz_sql")

        result = limiter.check_rate_limits("tok1", "vectoraiz_sql", "127.0.0.1")
        assert result == "rate_limited"


# ---------------------------------------------------------------------------
# Concurrency
# ---------------------------------------------------------------------------

class TestConcurrency:
    def test_acquire_within_limit(self, limiter):
        assert limiter.acquire_concurrency("tok1") is True
        assert limiter.acquire_concurrency("tok1") is True

    def test_acquire_at_limit(self, limiter):
        limiter.acquire_concurrency("tok1")
        limiter.acquire_concurrency("tok1")
        assert limiter.acquire_concurrency("tok1") is False

    def test_release_allows_new(self, limiter):
        limiter.acquire_concurrency("tok1")
        limiter.acquire_concurrency("tok1")
        assert limiter.acquire_concurrency("tok1") is False

        limiter.release_concurrency("tok1")
        assert limiter.acquire_concurrency("tok1") is True

    def test_release_does_not_go_negative(self, limiter):
        limiter.release_concurrency("tok_nonexistent")
        # Should not crash

    def test_concurrency_check_in_rate_limits(self, limiter):
        limiter.acquire_concurrency("tok1")
        limiter.acquire_concurrency("tok1")

        result = limiter.check_rate_limits("tok1", "list_datasets", "127.0.0.1")
        assert result == "rate_limited"

    def test_different_tokens_independent(self, limiter):
        limiter.acquire_concurrency("tok1")
        limiter.acquire_concurrency("tok1")

        assert limiter.acquire_concurrency("tok2") is True
