"""
BQ-123A: Resource exhaustion guards.

- Disk full detection: startup + every 60s (warn at 15%, block ingestion at 5%)
- Memory pressure monitoring
- Log rotation failure fallback to stderr

These guards run as a periodic asyncio task started from main.py.
"""

import asyncio
import logging
import sys

import psutil

from app.core.issue_tracker import issue_tracker

logger = logging.getLogger(__name__)

# Thresholds (percent free)
DISK_WARN_PCT = 15.0
DISK_CRITICAL_PCT = 5.0
MEMORY_WARN_PCT = 10.0
MEMORY_CRITICAL_PCT = 3.0

# Module-level flag — checked by ingestion endpoints
ingestion_blocked: bool = False


def check_disk() -> dict:
    """Check disk space and emit issues if thresholds crossed."""
    global ingestion_blocked
    try:
        usage = psutil.disk_usage("/")
        free_pct = round(100.0 - usage.percent, 1)

        if free_pct < DISK_CRITICAL_PCT:
            ingestion_blocked = True
            issue_tracker.record("VAI-SYS-001", "disk")
            logger.critical(
                "disk_space_critical",
                extra={"disk.free_pct": free_pct, "ingestion_blocked": True},
            )
            return {"status": "down", "free_pct": free_pct}
        elif free_pct < DISK_WARN_PCT:
            ingestion_blocked = False
            issue_tracker.record("VAI-SYS-001", "disk")
            logger.warning(
                "disk_space_low",
                extra={"disk.free_pct": free_pct},
            )
            return {"status": "degraded", "free_pct": free_pct}
        else:
            ingestion_blocked = False
            return {"status": "ok", "free_pct": free_pct}
    except Exception as e:
        logger.error("disk_check_failed", extra={"error": str(e)})
        return {"status": "unknown"}


def check_memory() -> dict:
    """Check memory and emit issues if thresholds crossed."""
    try:
        mem = psutil.virtual_memory()
        avail_pct = round(100.0 - mem.percent, 1)
        rss_mb = round(psutil.Process().memory_info().rss / (1024 * 1024), 1)

        if avail_pct < MEMORY_CRITICAL_PCT:
            issue_tracker.record("VAI-SYS-002", "memory")
            logger.critical(
                "memory_critical",
                extra={"mem.avail_pct": avail_pct, "mem.rss_mb": rss_mb},
            )
            return {"status": "down", "avail_pct": avail_pct, "rss_mb": rss_mb}
        elif avail_pct < MEMORY_WARN_PCT:
            issue_tracker.record("VAI-SYS-002", "memory")
            logger.warning(
                "memory_pressure",
                extra={"mem.avail_pct": avail_pct, "mem.rss_mb": rss_mb},
            )
            return {"status": "degraded", "avail_pct": avail_pct, "rss_mb": rss_mb}
        else:
            return {"status": "ok", "avail_pct": avail_pct, "rss_mb": rss_mb}
    except Exception as e:
        logger.error("memory_check_failed", extra={"error": str(e)})
        return {"status": "unknown"}


async def resource_monitor_loop(interval: int = 60) -> None:
    """Periodic resource check task — started from main.py lifespan."""
    # Run once at startup
    check_disk()
    check_memory()

    while True:
        await asyncio.sleep(interval)
        try:
            check_disk()
            check_memory()
        except Exception:
            # Resource guards must never crash the app
            logger.exception("resource_monitor_error")


def ensure_log_fallback() -> None:
    """Ensure stderr handler exists so logging never dies if files fail."""
    root = logging.getLogger()
    has_stderr = any(
        isinstance(h, logging.StreamHandler) and h.stream is sys.stderr
        for h in root.handlers
    )
    if not has_stderr:
        handler = logging.StreamHandler(sys.stderr)
        root.addHandler(handler)
        logger.warning("log_rotation_fallback", extra={"fallback": "stderr"})
