"""
Offline Queue — Append usage to /data/pending_usage.jsonl when offline.
========================================================================

When setup operations are allowed offline, log usage entries. On reconnect,
flush to /serials/{serial}/meter with idempotent request_ids. Cap 50 entries.

BQ-VZ-SERIAL-CLIENT
"""

from __future__ import annotations

import json
import logging
import os
from pathlib import Path
from typing import Optional

from app.config import settings

logger = logging.getLogger(__name__)

MAX_QUEUE_ENTRIES = 50


class OfflineQueue:
    """Append-only JSONL queue for offline usage metering."""

    def __init__(self, path: Optional[str] = None):
        self._path = Path(path or os.path.join(settings.serial_data_dir, "pending_usage.jsonl"))

    def append(self, entry: dict) -> bool:
        """Append a usage entry. Returns False if queue is full (50 cap)."""
        current_count = self.count()
        if current_count >= MAX_QUEUE_ENTRIES:
            logger.warning("Offline queue full (%d entries) — rejecting new entry", current_count)
            return False

        self._path.parent.mkdir(parents=True, exist_ok=True)
        with open(self._path, "a") as f:
            f.write(json.dumps(entry) + "\n")
        logger.info("Queued offline usage: request_id=%s", entry.get("request_id", "?"))
        return True

    def count(self) -> int:
        if not self._path.exists():
            return 0
        try:
            with open(self._path) as f:
                return sum(1 for line in f if line.strip())
        except OSError:
            return 0

    def read_all(self) -> list[dict]:
        """Read all entries from the queue."""
        if not self._path.exists():
            return []
        entries = []
        with open(self._path) as f:
            for line in f:
                line = line.strip()
                if line:
                    try:
                        entries.append(json.loads(line))
                    except json.JSONDecodeError:
                        logger.warning("Skipping malformed offline queue entry")
        return entries

    def clear(self) -> None:
        """Remove all entries (after successful flush)."""
        try:
            self._path.unlink(missing_ok=True)
            logger.info("Offline queue cleared")
        except OSError as e:
            logger.error("Failed to clear offline queue: %s", e)

    async def flush(self, serial_client, serial: str, install_token: str) -> int:
        """Flush all pending entries to the server. Returns count of successfully sent."""
        entries = self.read_all()
        if not entries:
            return 0

        from decimal import Decimal

        sent = 0
        for entry in entries:
            result = await serial_client.meter(
                serial=serial,
                install_token=install_token,
                category=entry.get("category", "setup"),
                cost_usd=Decimal(entry.get("cost_usd", "0.00")),
                request_id=entry["request_id"],
                description=entry.get("description", "offline-queued"),
            )
            if result.status_code in (200, 409):
                # 200 = metered, 409 = already metered (idempotent)
                sent += 1
            else:
                logger.warning(
                    "Failed to flush offline entry %s: status=%d",
                    entry.get("request_id"), result.status_code,
                )
                break  # Stop on first failure — retry later

        if sent == len(entries):
            self.clear()
            logger.info("Flushed all %d offline entries", sent)
        else:
            # Rewrite remaining entries
            remaining = entries[sent:]
            self.clear()
            for entry in remaining:
                self.append(entry)
            logger.info("Flushed %d/%d offline entries, %d remaining", sent, len(entries), len(remaining))

        return sent


# Module-level singleton
_queue: Optional[OfflineQueue] = None


def get_offline_queue() -> OfflineQueue:
    global _queue
    if _queue is None:
        _queue = OfflineQueue()
    return _queue
