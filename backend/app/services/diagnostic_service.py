"""
BQ-123B: Diagnostic bundle service.

Orchestrates collectors and packages results into a ZIP archive
(in-memory, never written to disk). Total generation capped at 30s.
"""
from __future__ import annotations

import asyncio
import hashlib
import io
import json
import logging
import socket
import zipfile
from datetime import datetime, timezone

from app.core.structured_logging import APP_VERSION
from app.services.diagnostic_collectors import (
    BaseCollector,
    CollectorResult,
    get_default_collectors,
)

logger = logging.getLogger(__name__)

BUNDLE_SCHEMA_VERSION = 1
BUNDLE_TOTAL_TIMEOUT = 30.0  # seconds


class DiagnosticService:
    """Collects system diagnostics into a downloadable ZIP bundle."""

    def __init__(self, collectors: list[BaseCollector] | None = None):
        self._collectors = collectors or get_default_collectors()

    async def generate_bundle(self) -> io.BytesIO:
        """Run all collectors and package into an in-memory ZIP.

        Returns a BytesIO containing the ZIP archive, seeked to 0.
        """
        # Run all collectors concurrently, with an overall timeout
        results = await asyncio.wait_for(
            self._run_collectors(),
            timeout=BUNDLE_TOTAL_TIMEOUT,
        )

        return self._package_zip(results)

    async def _run_collectors(self) -> list[CollectorResult]:
        """Run all collectors concurrently via safe_collect()."""
        coros = [c.safe_collect() for c in self._collectors]
        return await asyncio.gather(*coros)

    def _package_zip(self, results: list[CollectorResult]) -> io.BytesIO:
        """Build the ZIP archive from collector results."""
        buf = io.BytesIO()

        with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
            # Metadata file
            metadata = {
                "bundle_version": BUNDLE_SCHEMA_VERSION,
                "generated_at": datetime.now(timezone.utc).isoformat(),
                "vectoraiz_version": APP_VERSION,
                "host_id": hashlib.sha256(socket.gethostname().encode()).hexdigest()[:12],
            }
            zf.writestr("metadata.json", _to_json(metadata))

            # Collector results
            for r in results:
                path = _collector_path(r)
                content = r.data
                if r.error:
                    content = {**content, "_collector_error": r.error}
                content["_collector_duration_ms"] = r.duration_ms
                content["_collected_at"] = r.collected_at

                if r.name == "logs":
                    # Logs go as NDJSON for streaming parsers
                    entries = content.pop("entries", [])
                    zf.writestr(
                        "logs/recent.jsonl",
                        "\n".join(json.dumps(e, default=str) for e in entries) + "\n"
                        if entries else "",
                    )
                    # Also write a summary
                    zf.writestr("logs/summary.json", _to_json(content))
                else:
                    zf.writestr(path, _to_json(content))

            # Summary of all collector statuses
            collector_summary = {
                r.name: {
                    "duration_ms": r.duration_ms,
                    "error": r.error,
                    "collected_at": r.collected_at,
                }
                for r in results
            }
            zf.writestr("collector_summary.json", _to_json(collector_summary))

        buf.seek(0)
        return buf


def _collector_path(result: CollectorResult) -> str:
    """Map collector name → ZIP file path."""
    mapping = {
        "health": "health/health_snapshot.json",
        "config": "config/redacted_config.json",
        "system": "system/runtime.json",
        "qdrant": "qdrant/collections.json",
        "database": "db/schema_version.json",
        "errors": "errors/registry.json",
        "issues": "issues.json",
        "processes": "system/processes.json",
    }
    return mapping.get(result.name, f"{result.name}.json")


def _to_json(obj: dict) -> str:
    return json.dumps(obj, indent=2, default=str)
