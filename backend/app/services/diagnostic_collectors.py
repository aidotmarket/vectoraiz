"""
BQ-123B: Modular diagnostic collectors for the diagnostic bundle.

Each collector follows the BaseCollector pattern:
- name: identifier for the collector
- timeout: max seconds (default 10)
- collect(): returns a dict of diagnostic data

All collectors are fault-tolerant — a single failure returns partial
results with an error note rather than raising.
"""
from __future__ import annotations

import asyncio
import hashlib
import logging
import platform
import re
import socket
import sys
import time
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

import psutil

from app.config import Settings, settings
from app.core.structured_logging import APP_VERSION, get_uptime_s

logger = logging.getLogger(__name__)


@dataclass
class CollectorResult:
    """Result from a single collector run."""
    name: str
    data: dict
    collected_at: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    duration_ms: float = 0.0
    error: str | None = None


class BaseCollector(ABC):
    name: str = "base"
    timeout: float = 10.0

    @abstractmethod
    async def collect(self) -> dict:
        """Collect diagnostic data. Must complete within timeout."""
        ...

    async def safe_collect(self) -> CollectorResult:
        """Run collect() with timeout and error handling."""
        start = time.perf_counter()
        try:
            data = await asyncio.wait_for(self.collect(), timeout=self.timeout)
            duration_ms = round((time.perf_counter() - start) * 1000, 1)
            return CollectorResult(
                name=self.name,
                data=data,
                duration_ms=duration_ms,
            )
        except asyncio.TimeoutError:
            duration_ms = round((time.perf_counter() - start) * 1000, 1)
            return CollectorResult(
                name=self.name,
                data={},
                duration_ms=duration_ms,
                error=f"Collector timed out after {self.timeout}s",
            )
        except Exception as e:
            duration_ms = round((time.perf_counter() - start) * 1000, 1)
            logger.warning("collector_failed", extra={"collector": self.name, "error": str(e)})
            return CollectorResult(
                name=self.name,
                data={},
                duration_ms=duration_ms,
                error=f"{type(e).__name__}: {e}",
            )


class HealthCollector(BaseCollector):
    """Calls the deep health check internally."""
    name = "health"

    async def collect(self) -> dict:
        from app.routers.health import deep_health_check
        from app.services.duckdb_service import get_duckdb_service

        duckdb = get_duckdb_service()
        return await deep_health_check(duckdb)


class ConfigCollector(BaseCollector):
    """Dumps sanitized config with secrets redacted."""
    name = "config"

    async def collect(self) -> dict:
        from app.core.redaction import redact_config

        # Dump all settings as a dict, then redact
        raw = {}
        for field_name in Settings.model_fields:
            val = getattr(settings, field_name, None)
            if isinstance(val, (str, int, float, bool, type(None))):
                raw[field_name] = val
            elif isinstance(val, list):
                raw[field_name] = val
            else:
                raw[field_name] = str(val)

        return redact_config(raw)


class LogCollector(BaseCollector):
    """Collects recent structured log entries from the in-memory ring buffer."""
    name = "logs"

    async def collect(self) -> dict:
        from app.core.log_buffer import log_ring_buffer
        from app.core.redaction import redact_log_entry

        entries = log_ring_buffer.get_entries(limit=1000)
        redacted = [redact_log_entry(e) for e in entries]
        return {"count": len(redacted), "entries": redacted}


class SystemCollector(BaseCollector):
    """Python version, OS info, psutil stats (memory, disk, CPU)."""
    name = "system"

    async def collect(self) -> dict:
        mem = psutil.virtual_memory()
        disk = psutil.disk_usage("/")
        cpu_freq = psutil.cpu_freq()

        return {
            "python_version": sys.version,
            "platform": platform.platform(),
            "architecture": platform.machine(),
            "host_id": hashlib.sha256(socket.gethostname().encode()).hexdigest()[:12],
            "cpu_count": psutil.cpu_count(),
            "cpu_freq_mhz": round(cpu_freq.current, 1) if cpu_freq else None,
            "memory_total_mb": round(mem.total / (1024 * 1024), 1),
            "memory_available_mb": round(mem.available / (1024 * 1024), 1),
            "memory_percent": mem.percent,
            "disk_total_gb": round(disk.total / (1024 ** 3), 1),
            "disk_free_gb": round(disk.free / (1024 ** 3), 1),
            "disk_percent": disk.percent,
            "uptime_s": round(get_uptime_s(), 1),
            "vectoraiz_version": APP_VERSION,
        }


class QdrantCollector(BaseCollector):
    """Qdrant collection stats, point counts, index status."""
    name = "qdrant"

    async def collect(self) -> dict:
        from qdrant_client import QdrantClient

        client = QdrantClient(
            host=settings.qdrant_host,
            port=settings.qdrant_port,
            timeout=5.0,
        )
        collections = client.get_collections()

        col_stats = []
        for col in collections.collections:
            info = client.get_collection(col.name)
            col_stats.append({
                "name": col.name,
                "vectors_count": info.vectors_count,
                "points_count": info.points_count,
                "status": info.status.value if info.status else "unknown",
                "config": {
                    "vector_size": info.config.params.vectors.size
                    if hasattr(info.config.params.vectors, "size")
                    else None,
                },
            })

        return {
            "collection_count": len(col_stats),
            "collections": col_stats,
        }


class DatabaseCollector(BaseCollector):
    """Alembic version + table row counts."""
    name = "database"

    async def collect(self) -> dict:
        from app.core.database import get_engine, DATABASE_URL
        from sqlalchemy import text, inspect

        engine = get_engine()
        result: dict[str, Any] = {
            "backend": "postgresql" if "postgresql" in DATABASE_URL else "sqlite",
        }

        # Get Alembic version
        try:
            with engine.connect() as conn:
                row = conn.execute(text("SELECT version_num FROM alembic_version")).fetchone()
                result["alembic_version"] = row[0] if row else None
        except Exception:
            result["alembic_version"] = None
            result["alembic_error"] = "alembic_version table not found"

        # Table row counts
        try:
            inspector = inspect(engine)
            table_names = inspector.get_table_names()
            table_stats = {}
            with engine.connect() as conn:
                for table in table_names:
                    if table == "alembic_version":
                        continue
                    try:
                        count = conn.execute(text(f'SELECT COUNT(*) FROM "{table}"')).scalar()
                        table_stats[table] = count
                    except Exception:
                        table_stats[table] = "error"
            result["tables"] = table_stats
        except Exception as e:
            result["tables_error"] = str(e)

        return result


class ErrorCollector(BaseCollector):
    """Error registry + recent error events from the ring buffer."""
    name = "errors"

    async def collect(self) -> dict:
        from app.core.errors.registry import error_registry
        from app.core.log_buffer import log_ring_buffer
        from app.core.redaction import redact_log_entry

        # Full registry dump
        registry_dump = []
        for code in error_registry.all_codes():
            entry = error_registry.get(code)
            if entry:
                registry_dump.append({
                    "code": entry.code,
                    "domain": entry.domain,
                    "title": entry.title,
                    "severity": entry.severity,
                    "retryable": entry.retryable,
                    "http_status": entry.http_status,
                    "safe_message": entry.safe_message,
                })

        # Recent error-level log entries (last 100) — redacted to strip secrets/PII
        all_entries = log_ring_buffer.get_entries(limit=1000)
        error_entries = [
            redact_log_entry(e) for e in all_entries
            if e.get("level", "").lower() in ("error", "critical")
        ][-100:]

        return {
            "registry": {
                "schema_version": error_registry.schema_version,
                "total_codes": len(error_registry),
                "codes": registry_dump,
            },
            "recent_errors": {
                "count": len(error_entries),
                "entries": error_entries,
            },
        }


class IssueCollector(BaseCollector):
    """Current open issues from issue tracker."""
    name = "issues"

    async def collect(self) -> dict:
        from app.core.issue_tracker import issue_tracker

        issues = issue_tracker.get_active_issues()
        return {
            "active_count": len(issues),
            "issues": issues,
        }


class ProcessCollector(BaseCollector):
    """Background tasks: active asyncio tasks info."""
    name = "processes"

    async def collect(self) -> dict:
        import asyncio

        tasks = asyncio.all_tasks()
        task_info = []
        for task in tasks:
            task_info.append({
                "name": task.get_name(),
                "done": task.done(),
                "cancelled": task.cancelled(),
            })

        return {
            "asyncio_task_count": len(task_info),
            "tasks": task_info,
        }


def _sanitize_label(label: str) -> str:
    """Strip control characters and cap length for safe diagnostic output."""
    if not label:
        return ""
    # Remove control characters (keep printable ASCII + common unicode)
    sanitized = re.sub(r'[\x00-\x1f\x7f-\x9f]', '', label)
    return sanitized[:255]


class ConnectivityCollector(BaseCollector):
    """BQ-MCP-RAG Phase 3: Connectivity status, tokens (no secrets), metrics, rate limiter."""
    name = "connectivity"

    async def collect(self) -> dict:
        from app.config import settings
        from app.services.connectivity_metrics import get_connectivity_metrics
        from app.services.connectivity_token_service import list_tokens

        result: dict[str, Any] = {
            "enabled": settings.connectivity_enabled,
            "bind_host": settings.connectivity_bind_host,
        }

        # Tokens — labels and usage only, never secrets
        try:
            tokens = list_tokens()
            active_count = 0
            token_summaries = []
            for t in tokens:
                is_active = not getattr(t, "is_revoked", False)
                if is_active:
                    active_count += 1
                token_summaries.append({
                    "id": t.id,
                    "label": _sanitize_label(t.label),
                    "is_active": is_active,
                    "last_used_at": t.last_used_at.isoformat() if t.last_used_at else None,
                    "request_count": t.request_count,
                })
            result["token_count"] = len(tokens)
            result["active_token_count"] = active_count
            result["tokens"] = token_summaries
        except Exception as e:
            result["tokens_error"] = str(e)

        # Metrics snapshot
        try:
            result["metrics"] = get_connectivity_metrics().get_snapshot()
        except Exception as e:
            result["metrics_error"] = str(e)

        # Rate limiter state — blocked IPs count
        try:
            from app.services.query_orchestrator import get_query_orchestrator
            orch = get_query_orchestrator()
            blocked = orch.rate_limiter.get_blocked_ips() if hasattr(orch.rate_limiter, "get_blocked_ips") else []
            result["blocked_ips_count"] = len(blocked) if isinstance(blocked, list) else 0
        except Exception:
            result["blocked_ips_count"] = 0

        # Recent audit log entries (from structured log buffer, redacted)
        try:
            from app.core.log_buffer import log_ring_buffer
            from app.core.redaction import redact_log_entry
            all_entries = log_ring_buffer.get_entries(limit=500)
            audit_entries = [
                redact_log_entry(e) for e in all_entries
                if e.get("audit") == "connectivity" or "connectivity" in e.get("name", "")
            ][-20:]
            result["recent_audit_entries"] = audit_entries
        except Exception:
            result["recent_audit_entries"] = []

        return result


# ── Default collector set ───────────────────────────────────────────

def get_default_collectors() -> list[BaseCollector]:
    """Return the standard set of collectors for a diagnostic bundle."""
    return [
        HealthCollector(),
        ConfigCollector(),
        LogCollector(),
        SystemCollector(),
        QdrantCollector(),
        DatabaseCollector(),
        ErrorCollector(),
        IssueCollector(),
        ProcessCollector(),
        ConnectivityCollector(),
    ]
