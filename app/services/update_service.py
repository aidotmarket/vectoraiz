"""BQ-VZ-AUTO-UPDATE: Software Update Service

- Checks GHCR for the latest published image tag
- Compares against current APP_VERSION
- Hands an exact release to the independently supervised original-user owner
- Caches GHCR results for 6 hours
"""
from __future__ import annotations

import asyncio
import logging
import re
import time

import httpx

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# In-memory cache (module-level singleton)
# ---------------------------------------------------------------------------
_cache: dict = {
    "latest_version": None,
    "latest_published_at": None,
    "checked_at": 0.0,  # monotonic timestamp
}

CACHE_TTL_S = 6 * 60 * 60  # 6 hours

GHCR_IMAGE = "aidotmarket/vectoraiz"
GHCR_REGISTRY = "ghcr.io"

# Semver-like tag pattern: optional v prefix, digits.digits.digits, optional -rc.N
_SEMVER_RE = re.compile(r"^v?(\d+\.\d+\.\d+)(?:-rc\.(\d+))?$")


def _get_current_version() -> str:
    """Import API_VERSION lazily to avoid circular imports."""
    from app.main import API_VERSION
    return API_VERSION


def _parse_semver(tag: str) -> tuple[int, ...] | None:
    """Parse a version tag into a sortable tuple.

    Stable versions sort higher than RC of the same base:
      1.20.34        → (1, 20, 34, 1, 0)   (stable flag=1)
      1.20.34-rc.3   → (1, 20, 34, 0, 3)   (rc flag=0, rc_num=3)
    """
    m = _SEMVER_RE.match(tag)
    if not m:
        return None
    base = tuple(int(p) for p in m.group(1).split("."))
    rc = m.group(2)
    if rc is not None:
        return base + (0, int(rc))  # RC: sorts below stable
    return base + (1, 0)  # Stable: sorts above any RC


# ---------------------------------------------------------------------------
# GHCR version check
# ---------------------------------------------------------------------------
async def _fetch_ghcr_token() -> str:
    """Get an anonymous bearer token for pulling from GHCR."""
    url = f"https://{GHCR_REGISTRY}/token?scope=repository:{GHCR_IMAGE}:pull"
    async with httpx.AsyncClient(timeout=10) as client:
        resp = await client.get(url)
        resp.raise_for_status()
        return resp.json()["token"]


async def _fetch_latest_tag() -> str | None:
    """Fetch tags from GHCR and return the highest semver tag."""
    token = await _fetch_ghcr_token()
    url = f"https://{GHCR_REGISTRY}/v2/{GHCR_IMAGE}/tags/list"
    async with httpx.AsyncClient(timeout=10) as client:
        resp = await client.get(url, headers={"Authorization": f"Bearer {token}"})
        resp.raise_for_status()
        data = resp.json()

    tags = data.get("tags", [])
    semver_tags: list[tuple[tuple[int, ...], str]] = []
    for tag in tags:
        parsed = _parse_semver(tag)
        if parsed:
            semver_tags.append((parsed, tag))

    if not semver_tags:
        return None

    # Prefer the highest stable tag; fall back to highest RC if no stable exists
    stable = [(t, raw) for t, raw in semver_tags if t[-2] == 1]  # stable flag == 1
    if stable:
        stable.sort(reverse=True)
        return stable[0][1]
    semver_tags.sort(reverse=True)
    return semver_tags[0][1]


async def check_for_updates(force: bool = False) -> dict:
    """
    Return version info. Uses cache unless *force* is True or cache is stale.
    """
    current = _get_current_version()
    now = time.monotonic()

    if not force and _cache["latest_version"] and (now - _cache["checked_at"]) < CACHE_TTL_S:
        latest = _cache["latest_version"]
        return _build_response(current, latest, _cache["latest_published_at"], _cache.get("error"))

    # Fetch from GHCR
    try:
        latest = await _fetch_latest_tag()
        _cache["latest_version"] = latest
        _cache["latest_published_at"] = None  # GHCR tags/list doesn't include dates
        _cache["checked_at"] = now
        _cache.pop("error", None)
        logger.info("GHCR version check: current=%s latest=%s", current, latest)
    except Exception as e:
        logger.warning("GHCR version check failed: %s", e)
        latest = _cache.get("latest_version")
        # Cached tags remain context, but must not hide a failed refresh on later reads.
        _cache["error"] = str(e) or "Registry version check failed."

    return _build_response(current, latest, _cache.get("latest_published_at"), _cache.get("error"))


def _build_response(
    current: str, latest: str | None, published_at: str | None, error: str | None = None
) -> dict:
    update_available = False
    if not latest:
        error = error or "No recognized latest version tag was found."
    else:
        cur = _parse_semver(current)
        lat = _parse_semver(latest)
        if cur and lat:
            update_available = lat > cur
        else:
            error = error or "Cannot compare current and latest versions: unrecognized version format."

    response = {
        "current": current,
        "latest": latest,
        "update_available": update_available,
        "latest_published_at": published_at,
        "changelog_url": "https://github.com/aidotmarket/vectoraiz/releases",
        "can_auto_update": _docker_available(),
    }
    if error:
        response["error"] = error
    return response


# Host owner capability is independent of the app's Docker socket.
from app.services import update_owner_client as _owner


def _docker_available() -> bool:
    # Retained name for older callers; socket existence conveys no readiness.
    return _owner.available()


async def trigger_update(target: str, idempotency_key: str, principal: str) -> dict:
    return await asyncio.to_thread(_owner.submit, target, idempotency_key, principal)


async def owner_status(identifier: str = "current") -> dict:
    return await asyncio.to_thread(_owner.status, identifier)


# ---------------------------------------------------------------------------
# Background check (called from lifespan)
# ---------------------------------------------------------------------------
async def background_update_check_loop():
    """Check for updates on startup, then every 6 hours."""
    # Pairing must keep working even when a registry check fails or stalls.
    async def mailbox_loop():
        while True:
            await asyncio.to_thread(_owner.heartbeat)
            await asyncio.sleep(1)

    mailbox_task = asyncio.create_task(mailbox_loop())

    try:
        while True:
            try:
                info = await check_for_updates()
                if info.get("update_available"):
                    logger.info(
                        "Update available: current=%s latest=%s",
                        info["current"], info.get("latest"),
                    )
            except Exception as e:
                logger.warning("Background update check failed: %s", e)

            await asyncio.sleep(CACHE_TTL_S)
    finally:
        mailbox_task.cancel()
