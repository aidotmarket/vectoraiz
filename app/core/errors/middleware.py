"""
BQ-123A: FastAPI exception handler for VectorAIzError.

Catches VectorAIzError, looks up the registry, and returns a structured
JSON error response. Unknown codes get a safe fallback.
"""

import logging

from fastapi import Request
from fastapi.responses import JSONResponse

from app.core.errors import VectorAIzError
from app.core.errors.registry import error_registry

logger = logging.getLogger(__name__)


async def vectoraiz_error_handler(request: Request, exc: VectorAIzError) -> JSONResponse:
    """Convert VectorAIzError into a structured JSON response."""
    entry = error_registry.get(exc.code)

    if entry is None:
        # Code not in registry — log a warning, return generic 500
        logger.error(
            "unregistered_error_code",
            extra={"error.code": exc.code, "error.message": exc.detail},
        )
        return JSONResponse(
            status_code=500,
            content={
                "error": {
                    "code": exc.code,
                    "title": "Internal error",
                    "message": "An unexpected error occurred.",
                    "retryable": False,
                    "user_action_required": False,
                    "remediation": [],
                }
            },
        )

    # Log the error with full context (internal)
    log_extra = {
        "error.code": exc.code,
        "error.kind": type(exc).__name__,
        "error.message_safe": entry.safe_message,
        "error.message": exc.detail,
        "error.retryable": entry.retryable,
        "error.user_action_required": entry.user_action_required,
        **{f"error.ctx.{k}": v for k, v in exc.context.items()},
    }

    log_fn = _severity_to_log_fn(entry.severity)
    log_fn(entry.title, extra=log_extra)

    return JSONResponse(
        status_code=entry.http_status,
        content={
            "error": {
                "code": entry.code,
                "title": entry.title,
                "message": entry.safe_message,
                "retryable": entry.retryable,
                "user_action_required": entry.user_action_required,
                "remediation": entry.remediation,
            }
        },
    )


def _severity_to_log_fn(severity: str):
    """Map registry severity to logger method."""
    return {
        "DEBUG": logger.debug,
        "INFO": logger.info,
        "WARN": logger.warning,
        "ERROR": logger.error,
        "CRITICAL": logger.critical,
    }.get(severity, logger.error)
