"""
BQ-123A: Error code system.

VectorAIzError is the base exception for all structured errors.
Raise it with an error code from the registry, and the error middleware
will produce a structured JSON response.

Usage:
    from app.core.errors import VectorAIzError
    raise VectorAIzError("VAI-QDR-001", detail="connection refused to localhost:6333")
"""

from __future__ import annotations

import re

CODE_PATTERN = re.compile(r"^VAI-[A-Z]{2,6}-\d{3}$")


class VectorAIzError(Exception):
    """Structured application error tied to the error registry.

    Args:
        code: Registry error code, e.g. "VAI-QDR-001".
        detail: Internal-only detail message (never exposed to users).
        context: Arbitrary key-value context for structured logging.
    """

    def __init__(
        self,
        code: str,
        detail: str | None = None,
        context: dict | None = None,
    ) -> None:
        if not CODE_PATTERN.match(code):
            raise ValueError(f"Invalid error code format: {code!r}")
        self.code = code
        self.detail = detail
        self.context = context or {}
        super().__init__(f"{code}: {detail}" if detail else code)
