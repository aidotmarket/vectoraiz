"""
BQ-123A: Error registry — loads and validates registry.yaml.
"""

from __future__ import annotations

import logging
import os
from dataclasses import dataclass, field
from typing import Dict, List, Optional

import yaml

from app.core.errors import CODE_PATTERN

logger = logging.getLogger(__name__)

VALID_DOMAINS = {"API", "CFG", "DB", "QDR", "LLM", "ING", "EMB", "RAG", "COP", "SEC", "SYS", "UX"}
VALID_SEVERITIES = {"DEBUG", "INFO", "WARN", "ERROR", "CRITICAL"}
REQUIRED_FIELDS = {"code", "domain", "title", "severity", "retryable", "user_action_required", "http_status", "safe_message", "remediation"}


@dataclass(frozen=True)
class ErrorEntry:
    code: str
    domain: str
    title: str
    severity: str
    retryable: bool
    user_action_required: bool
    http_status: int
    safe_message: str
    remediation: List[str] = field(default_factory=list)
    detail_template: Optional[str] = None
    tags: List[str] = field(default_factory=list)
    deprecated: bool = False
    replaced_by: Optional[str] = None
    docs_url: Optional[str] = None


class RegistryValidationError(Exception):
    """Raised when registry.yaml has structural errors."""


class ErrorRegistry:
    """Loads, validates, and provides lookup for error codes."""

    def __init__(self) -> None:
        self._entries: Dict[str, ErrorEntry] = {}
        self.schema_version: int = 0

    def load(self, path: str | None = None) -> None:
        if path is None:
            path = os.path.join(os.path.dirname(__file__), "registry.yaml")

        with open(path, "r") as f:
            data = yaml.safe_load(f)

        self.schema_version = data.get("schema_version", 0)
        errors_list = data.get("errors", [])

        if not isinstance(errors_list, list):
            raise RegistryValidationError("'errors' must be a list")

        seen_codes: set[str] = set()
        entries: Dict[str, ErrorEntry] = {}

        for idx, raw in enumerate(errors_list):
            # Check required fields
            missing = REQUIRED_FIELDS - set(raw.keys())
            if missing:
                raise RegistryValidationError(
                    f"Entry {idx} ({raw.get('code', '?')}): missing fields {missing}"
                )

            code = raw["code"]

            # Validate code format
            if not CODE_PATTERN.match(code):
                raise RegistryValidationError(f"Invalid code format: {code!r}")

            # Validate domain matches code prefix
            domain = raw["domain"]
            code_domain = code.split("-")[1]
            if domain != code_domain:
                raise RegistryValidationError(
                    f"{code}: domain {domain!r} doesn't match code prefix {code_domain!r}"
                )

            if domain not in VALID_DOMAINS:
                raise RegistryValidationError(f"{code}: unknown domain {domain!r}")

            if raw["severity"] not in VALID_SEVERITIES:
                raise RegistryValidationError(f"{code}: unknown severity {raw['severity']!r}")

            if code in seen_codes:
                raise RegistryValidationError(f"Duplicate code: {code}")
            seen_codes.add(code)

            entries[code] = ErrorEntry(
                code=code,
                domain=domain,
                title=raw["title"],
                severity=raw["severity"],
                retryable=bool(raw["retryable"]),
                user_action_required=bool(raw["user_action_required"]),
                http_status=int(raw["http_status"]),
                safe_message=raw["safe_message"],
                remediation=raw.get("remediation", []),
                detail_template=raw.get("detail_template"),
                tags=raw.get("tags", []),
                deprecated=raw.get("deprecated", False),
                replaced_by=raw.get("replaced_by"),
                docs_url=raw.get("docs_url"),
            )

        self._entries = entries
        logger.info("error_registry_loaded", extra={"count": len(entries), "schema_version": self.schema_version})

    def get(self, code: str) -> ErrorEntry | None:
        return self._entries.get(code)

    def lookup(self, code: str) -> ErrorEntry:
        """Lookup by code, raising KeyError if not found."""
        entry = self._entries.get(code)
        if entry is None:
            raise KeyError(f"Unknown error code: {code!r}")
        return entry

    def all_codes(self) -> list[str]:
        return list(self._entries.keys())

    def codes_for_domain(self, domain: str) -> list[str]:
        return [c for c, e in self._entries.items() if e.domain == domain]

    def __len__(self) -> int:
        return len(self._entries)


# Module-level singleton — loaded once at startup
error_registry = ErrorRegistry()
