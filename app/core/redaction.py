"""
BQ-123B: Redaction module for diagnostic bundles.

Key-based redaction for config values and value-based redaction for log entries.
Follows spec B.2 redaction rules (non-negotiable).
"""
from __future__ import annotations

import re
from typing import Any

# ── Key-based redaction (case-insensitive substring match) ───────────
_SENSITIVE_KEY_SUBSTRINGS = frozenset({
    "password", "passwd", "secret", "token", "apikey", "api_key",
    "authorization", "bearer", "cookie", "session", "private",
    "ssh", "cert", "key", "salt", "credential",
})

# ── Value-based patterns ────────────────────────────────────────────
_JWT_PATTERN = re.compile(
    r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+"
)
_EMAIL_PATTERN = re.compile(
    r"[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+"
)
_URL_QUERY_PATTERN = re.compile(
    r"(https?://[^\s?]+)\?[^\s]*"
)


def _is_sensitive_key(key: str) -> bool:
    """Check if a key name indicates a sensitive value."""
    lower = key.lower()
    return any(s in lower for s in _SENSITIVE_KEY_SUBSTRINGS)


def _redact_sensitive_value(value: str) -> str:
    """Partially redact a sensitive value: first 4 + **** + last 4 chars."""
    if len(value) <= 8:
        return "[REDACTED]"
    return value[:4] + "****" + value[-4:]


def redact_value(key: str, value: Any) -> Any:
    """Redact a single value based on its key name.

    Returns the value unchanged if the key is not sensitive,
    or a redacted form if it is.
    """
    if not isinstance(value, str):
        return value
    if _is_sensitive_key(key):
        return _redact_sensitive_value(value)
    return value


def redact_config(config: dict) -> dict:
    """Recursively redact sensitive values in a configuration dict.

    Uses key-based substring matching per spec B.2.
    """
    return _redact_dict(config)


def _redact_dict(obj: Any, parent_key: str = "") -> Any:
    """Recursively walk a nested structure and redact sensitive keys."""
    if isinstance(obj, dict):
        return {k: _redact_dict(v, k) for k, v in obj.items()}
    if isinstance(obj, list):
        return [_redact_dict(item, parent_key) for item in obj]
    if isinstance(obj, str) and _is_sensitive_key(parent_key):
        return _redact_sensitive_value(obj)
    return obj


def redact_log_entry(entry: dict) -> dict:
    """Apply both key-based and value-based redaction to a log entry.

    - Key-based: redact values for keys matching sensitive substrings
    - Value-based: redact JWTs, emails, URL query strings in any string value
    """
    result = {}
    for k, v in entry.items():
        if isinstance(v, dict):
            result[k] = redact_log_entry(v)
        elif isinstance(v, str):
            val = v
            # Key-based first
            if _is_sensitive_key(k):
                val = _redact_sensitive_value(val)
            else:
                # Value-based redaction on non-sensitive keys
                val = _redact_string_values(val)
            result[k] = val
        elif isinstance(v, list):
            result[k] = [
                redact_log_entry(item) if isinstance(item, dict)
                else _redact_string_values(item) if isinstance(item, str)
                else item
                for item in v
            ]
        else:
            result[k] = v
    return result


def _redact_string_values(value: str) -> str:
    """Apply value-based redaction patterns to a string."""
    # JWT tokens
    value = _JWT_PATTERN.sub("[REDACTED_JWT]", value)
    # Email addresses
    value = _EMAIL_PATTERN.sub("[REDACTED_EMAIL]", value)
    # URL query strings (keep scheme/host/path, drop query)
    value = _URL_QUERY_PATTERN.sub(r"\1?[QUERY_REDACTED]", value)
    return value
