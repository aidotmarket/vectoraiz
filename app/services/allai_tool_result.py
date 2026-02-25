"""
Two-Track Tool Result — Data Minimization for allAI

[COUNCIL MANDATE — MP #5: Raw data must NOT flow back into LLM context]

When a tool returns data (e.g., preview_rows returns 10 rows), the system
produces TWO outputs:
1. frontend_data: Full rich data → sent to frontend via TOOL_RESULT WebSocket message
2. llm_summary: Short text summary → fed back to LLM as tool_result (NO raw rows)

PHASE: BQ-ALLAI-B0 — Security Infrastructure
CREATED: 2026-02-16
"""

from dataclasses import dataclass, field
from typing import Any, Dict


@dataclass
class ToolResult:
    """Two-track tool result: rich data for frontend, summary for LLM."""

    # Sent to frontend for rendering (full data — tables, lists, charts)
    frontend_data: Dict[str, Any] = field(default_factory=dict)

    # Sent to LLM as tool_result (summary only — NO raw rows/PII)
    llm_summary: str = ""
