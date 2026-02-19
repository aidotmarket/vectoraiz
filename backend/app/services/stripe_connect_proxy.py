"""
Stripe Connect Proxy Service
==============================

PURPOSE:
    Proxies Stripe Connect operations from vectorAIz to ai.market.
    Keeps ai.market API communication server-side, supporting both
    current Railway deployment and future customer-hosted instances.

ENDPOINTS PROXIED:
    POST /api/v1/connect/onboarding → Create/get Connect account + onboarding link
    GET  /api/v1/connect/status     → Get seller's Stripe Connect status
    POST /api/v1/connect/login-link → Get Stripe dashboard link (Standard accounts)

AUTH:
    Forwards the user's X-API-Key to ai.market for user identification.
    ai.market's get_current_user_flexible resolves the seller.

BQ-103 ST-1 (2026-02-11)
"""

import logging
from typing import Any, Dict, Optional

import httpx

from app.config import settings

logger = logging.getLogger(__name__)

# Timeout: 10s total, 5s connect (Stripe account creation can be slow)
_TIMEOUT = httpx.Timeout(10.0, connect=5.0)

# Shared client singleton
_client: Optional[httpx.AsyncClient] = None


def _get_client() -> httpx.AsyncClient:
    """Return shared httpx.AsyncClient, creating on first use."""
    global _client
    if _client is None or _client.is_closed:
        _client = httpx.AsyncClient(
            timeout=_TIMEOUT,
            limits=httpx.Limits(max_connections=10, max_keepalive_connections=5),
        )
    return _client


class StripeConnectProxyError(Exception):
    """Raised when ai.market returns a non-2xx response."""

    def __init__(self, status_code: int, detail: str):
        self.status_code = status_code
        self.detail = detail
        super().__init__(f"ai.market returned {status_code}: {detail}")


class StripeConnectProxy:
    """
    Proxies Stripe Connect operations to ai.market backend.

    All methods require the caller's API key (user-level) which is
    forwarded to ai.market for user identification. This avoids
    exposing ai.market URLs or credentials to the frontend.
    """

    def __init__(self):
        self.base_url = settings.ai_market_url.rstrip("/")

    def _headers(self, api_key: str) -> Dict[str, str]:
        """Build headers for ai.market requests."""
        return {
            "X-API-Key": api_key,
            "Content-Type": "application/json",
        }

    async def _request(
        self,
        method: str,
        path: str,
        api_key: str,
        **kwargs: Any,
    ) -> Dict[str, Any]:
        """
        Make a request to ai.market and return the JSON response.

        Raises StripeConnectProxyError on non-2xx responses.
        """
        url = f"{self.base_url}{path}"
        client = _get_client()

        try:
            response = await client.request(
                method,
                url,
                headers=self._headers(api_key),
                **kwargs,
            )

            if response.status_code >= 400:
                detail = response.text
                try:
                    detail = response.json().get("detail", detail)
                except Exception:
                    pass
                logger.error(
                    "ai.market %s %s returned %d: %s",
                    method, path, response.status_code, detail,
                )
                raise StripeConnectProxyError(response.status_code, detail)

            return response.json()

        except httpx.TimeoutException as exc:
            logger.error("Timeout calling ai.market %s %s: %s", method, path, exc)
            raise StripeConnectProxyError(504, "ai.market request timed out")

        except httpx.RequestError as exc:
            logger.error("Connection error to ai.market %s %s: %s", method, path, exc)
            raise StripeConnectProxyError(502, "Cannot reach ai.market")

    async def initiate_onboarding(self, api_key: str) -> Dict[str, Any]:
        """
        Create or retrieve a Stripe Connect account and return an onboarding link.

        Returns:
            {
                "onboarding_url": "https://connect.stripe.com/...",
                "account_id": "acct_...",
                "type": "hosted"
            }
        """
        logger.info("Initiating Stripe Connect onboarding via ai.market")
        return await self._request(
            "POST",
            "/api/v1/connect/onboarding",
            api_key,
        )

    async def get_status(self, api_key: str) -> Dict[str, Any]:
        """
        Get the seller's Stripe Connect status.

        Returns:
            {
                "status": "not_connected" | "pending" | "complete",
                "account_id": "acct_..." | null,
                "details_submitted": bool,
                "payouts_enabled": bool,
                "charges_enabled": bool,
                "requirements": {...} | null
            }
        """
        logger.info("Fetching Stripe Connect status via ai.market")
        return await self._request(
            "GET",
            "/api/v1/connect/status",
            api_key,
        )

    async def get_login_link(self, api_key: str) -> Dict[str, Any]:
        """
        Get Stripe dashboard link for connected sellers.

        Note: Standard Connect accounts access dashboard.stripe.com directly.
        This endpoint returns the account_id for reference.

        Returns:
            {
                "message": "Standard Connect accounts access Stripe directly at dashboard.stripe.com",
                "account_id": "acct_..."
            }
        """
        logger.info("Fetching Stripe dashboard link via ai.market")
        return await self._request(
            "POST",
            "/api/v1/connect/login-link",
            api_key,
        )


async def close_proxy_client():
    """Gracefully close the shared httpx client. Call during app shutdown."""
    global _client
    if _client and not _client.is_closed:
        await _client.aclose()
        _client = None


# Module-level singleton
stripe_connect_proxy = StripeConnectProxy()
