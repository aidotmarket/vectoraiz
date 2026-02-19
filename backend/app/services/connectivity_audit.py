"""
Connectivity Audit Logger — Structured audit logging with redaction.

Redaction policy (§3.2, M25):
  - Never log token secrets
  - Truncate SQL to 500 chars
  - Log row_count but NOT row contents
  - Cap each audit entry to 4KB

Phase: BQ-MCP-RAG — Universal LLM Connectivity
Created: S136
"""

import json
import logging
from typing import Any, Dict, Optional

logger = logging.getLogger("connectivity.audit")

MAX_SQL_LOG_LENGTH = 500
MAX_ENTRY_BYTES = 4096


def _truncate(s: str, max_len: int) -> str:
    if len(s) <= max_len:
        return s
    return s[:max_len] + "...[truncated]"


def audit_log(
    tool_name: str,
    token_id: str,
    dataset_id: Optional[str],
    duration_ms: int,
    row_count: Optional[int],
    error_code: Optional[str],
    request_id: str,
    sql: Optional[str] = None,
    extra: Optional[Dict[str, Any]] = None,
) -> None:
    """Write a structured audit log entry for an external connectivity request.

    NEVER logs: token secrets, row contents, full SQL beyond 500 chars.
    """
    entry: Dict[str, Any] = {
        "audit": "connectivity",
        "tool": tool_name,
        "token_id": token_id,
        "request_id": request_id,
        "duration_ms": duration_ms,
    }

    if dataset_id:
        entry["dataset_id"] = dataset_id

    if row_count is not None:
        entry["row_count"] = row_count

    if error_code:
        entry["error_code"] = error_code

    if sql:
        entry["sql_preview"] = _truncate(sql, MAX_SQL_LOG_LENGTH)

    if extra:
        entry["extra"] = extra

    # Cap entry size at 4KB
    serialized = json.dumps(entry, default=str)
    if len(serialized.encode("utf-8")) > MAX_ENTRY_BYTES:
        # Drop extra and sql_preview to fit
        entry.pop("extra", None)
        entry.pop("sql_preview", None)
        entry["_truncated"] = True

    if error_code:
        logger.warning("ext_request", extra=entry)
    else:
        logger.info("ext_request", extra=entry)
