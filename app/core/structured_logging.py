"""
BQ-123A: Structured logging with structlog.

Configures structlog to output JSON lines with rotation.
Backward-compatible with stdlib logging — existing logger.info() calls
continue to work and get enriched with structlog processors.
"""
from __future__ import annotations

import logging
import logging.handlers
import os
import sys
import time
from contextvars import ContextVar

import structlog

# ── Context vars for correlation ──────────────────────────────────────
request_id_var: ContextVar[str | None] = ContextVar("request_id", default=None)
correlation_id_var: ContextVar[str | None] = ContextVar("correlation_id", default=None)
session_id_var: ContextVar[str | None] = ContextVar("session_id", default=None)

APP_VERSION = "1.8.0"
SERVICE_NAME = "vectoraiz-backend"

_startup_time: float = time.time()


def get_uptime_s() -> float:
    return time.time() - _startup_time


def _inject_context(logger_name: str, method_name: str, event_dict: dict) -> dict:
    """Structlog processor: inject correlation context from contextvars."""
    event_dict["service"] = SERVICE_NAME
    event_dict["version"] = APP_VERSION

    rid = request_id_var.get(None)
    if rid:
        event_dict["request_id"] = rid

    cid = correlation_id_var.get(None)
    if cid:
        event_dict["correlation_id"] = cid

    sid = session_id_var.get(None)
    if sid:
        event_dict["session_id"] = sid

    return event_dict


def _rename_event_key(logger_name: str, method_name: str, event_dict: dict) -> dict:
    """Ensure 'event' key exists (structlog uses it by default)."""
    return event_dict


def _add_log_level_upper(logger_name: str, method_name: str, event_dict: dict) -> dict:
    """Normalize log level to lowercase for consistency."""
    level = event_dict.get("level")
    if level:
        event_dict["level"] = level.lower()
    return event_dict


def setup_logging(
    log_dir: str = "logs",
    log_file: str = "vectoraiz.jsonl",
    max_bytes: int = 10 * 1024 * 1024,  # 10MB
    backup_count: int = 5,
    log_level: int = logging.INFO,
) -> None:
    """Initialize structlog + stdlib logging with JSON output and rotation.

    This must be called once at startup (before any logging calls).
    After this, both structlog.get_logger() and logging.getLogger() produce
    JSON-formatted output with correlation context.
    """
    # Ensure log directory exists
    os.makedirs(log_dir, exist_ok=True)
    log_path = os.path.join(log_dir, log_file)

    # ── Shared processors (used by both structlog and stdlib bridge) ──
    shared_processors: list = [
        structlog.contextvars.merge_contextvars,
        structlog.stdlib.add_log_level,
        _add_log_level_upper,
        structlog.stdlib.add_logger_name,
        _inject_context,
        structlog.processors.TimeStamper(fmt="iso", utc=True, key="ts"),
        structlog.processors.StackInfoRenderer(),
        structlog.processors.UnicodeDecoder(),
    ]

    # ── Configure structlog ──────────────────────────────────────────
    structlog.configure(
        processors=[
            *shared_processors,
            structlog.stdlib.ProcessorFormatter.wrap_for_formatter,
        ],
        logger_factory=structlog.stdlib.LoggerFactory(),
        wrapper_class=structlog.stdlib.BoundLogger,
        cache_logger_on_first_use=True,
    )

    # ── Formatter that renders JSON ──────────────────────────────────
    # BQ-123B: Capture entries into the in-memory ring buffer before rendering
    from app.core.log_buffer import structlog_buffer_processor

    formatter = structlog.stdlib.ProcessorFormatter(
        processors=[
            structlog.stdlib.ProcessorFormatter.remove_processors_meta,
            structlog_buffer_processor,
            structlog.processors.JSONRenderer(),
        ],
    )

    # ── Handlers ─────────────────────────────────────────────────────
    # File handler with rotation
    try:
        file_handler = logging.handlers.RotatingFileHandler(
            log_path,
            maxBytes=max_bytes,
            backupCount=backup_count,
            encoding="utf-8",
        )
        file_handler.setFormatter(formatter)
    except OSError:
        # Resource exhaustion guard: fall back to stderr, never crash
        file_handler = None

    # Console handler (stderr)
    console_handler = logging.StreamHandler(sys.stderr)
    console_handler.setFormatter(formatter)

    # ── Configure root logger ────────────────────────────────────────
    root = logging.getLogger()
    root.handlers.clear()
    root.setLevel(log_level)
    root.addHandler(console_handler)
    if file_handler:
        root.addHandler(file_handler)

    # Quiet noisy third-party loggers
    for noisy in ("httpcore", "httpx", "urllib3", "asyncio", "watchfiles"):
        logging.getLogger(noisy).setLevel(logging.WARNING)
