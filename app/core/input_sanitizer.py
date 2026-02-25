"""
Input Sanitizer — OWASP-Grade LLM Input Sanitization
=====================================================

Defenses (XAI Council mandate):
1. Prompt injection detection (instruction override patterns)
2. Unicode normalization (prevent homoglyph attacks)
3. Control character stripping
4. Length enforcement (configurable max message length)
5. Secret detection (API keys, passwords — warn, don't echo)

On injection detection: message is still processed (Allie deflects
per personality spec), but the attempt is logged for security audit.

PHASE: BQ-128 Phase 2 — Personality + Context Engine (Task 2.4)
CREATED: 2026-02-14
SPEC: ALLAI-PERSONALITY-SPEC-v2.1 Section 6, Council Condition #3
"""

import base64
import logging
import os
import re
import unicodedata
from dataclasses import dataclass, field
from typing import List, Optional

logger = logging.getLogger(__name__)

# Configurable max message length (default 4000 chars)
MAX_MESSAGE_LENGTH = int(os.environ.get("ALLAI_MAX_MESSAGE_LENGTH", "4000"))


@dataclass
class SanitizeResult:
    """Result of input sanitization."""
    clean_text: str
    warnings: List[str] = field(default_factory=list)
    blocked: bool = False
    injection_detected: bool = False
    injection_pattern: Optional[str] = None


# ---------------------------------------------------------------------------
# Injection patterns
# ---------------------------------------------------------------------------

_INJECTION_PATTERNS: List[tuple[str, re.Pattern]] = [
    (
        "instruction_override",
        re.compile(
            r"ignore\s+(all\s+)?(previous|prior|above|earlier)\s+(instructions|prompts|rules|context)",
            re.IGNORECASE,
        ),
    ),
    (
        "identity_override",
        re.compile(
            r"you\s+are\s+now\b",
            re.IGNORECASE,
        ),
    ),
    (
        "system_prompt_leak",
        re.compile(
            r"(system\s+prompt|new\s+instructions|override\s+instructions)\s*:",
            re.IGNORECASE,
        ),
    ),
    (
        "role_injection",
        re.compile(
            r"\b(assistant|system)\s*:\s*",
            re.IGNORECASE,
        ),
    ),
    (
        "instruction_injection",
        re.compile(
            r"(forget|disregard|override|bypass)\s+(everything|all|your|the)(\s+\w+)*\s+(above|instructions|rules|guidelines|prompt)",
            re.IGNORECASE,
        ),
    ),
    (
        "jailbreak_attempt",
        re.compile(
            r"(do\s+anything\s+now|DAN\s+mode|jailbreak|developer\s+mode\s+enabled)",
            re.IGNORECASE,
        ),
    ),
    (
        "markdown_injection",
        re.compile(
            r"!\[.*?\]\(javascript:|<script|<img\s+[^>]*onerror|<iframe|<object|<embed",
            re.IGNORECASE,
        ),
    ),
]

# ---------------------------------------------------------------------------
# Secret patterns
# ---------------------------------------------------------------------------

_SECRET_PATTERNS: List[tuple[str, re.Pattern]] = [
    ("aws_key", re.compile(r"AKIA[0-9A-Z]{16}")),
    ("api_key_generic", re.compile(r"(sk-|sk_live_|sk_test_)[a-zA-Z0-9]{20,}")),
    ("bearer_token", re.compile(r"Bearer\s+[a-zA-Z0-9\-._~+/]+=*", re.IGNORECASE)),
    ("password_assignment", re.compile(r"(password|passwd|pwd)\s*[:=]\s*\S+", re.IGNORECASE)),
    ("connection_string", re.compile(r"(postgres|mysql|mongodb|redis)://\S+", re.IGNORECASE)),
    ("private_key", re.compile(r"-----BEGIN\s+(RSA\s+)?PRIVATE\s+KEY-----")),
    ("github_token", re.compile(r"(ghp_|gho_|ghu_|ghs_|ghr_)[a-zA-Z0-9]{36,}")),
]

# Control characters to strip (keep newline, tab, carriage return)
_CONTROL_CHAR_RE = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]")


class InputSanitizer:
    """
    OWASP-grade sanitization for LLM inputs.

    Usage:
        sanitizer = InputSanitizer()
        result = sanitizer.sanitize(user_message)
        if result.blocked:
            # reject message
        if result.injection_detected:
            # log for audit, Allie will deflect in-character
    """

    def __init__(self, max_length: int = MAX_MESSAGE_LENGTH):
        self.max_length = max_length

    def sanitize(self, text: str, user_id: Optional[str] = None) -> SanitizeResult:
        """
        Full sanitization pipeline.

        Returns SanitizeResult with .clean_text, .warnings[], .blocked,
        .injection_detected, .injection_pattern.
        """
        warnings: List[str] = []

        if not text:
            return SanitizeResult(clean_text="", warnings=warnings)

        # 1. Unicode normalization (NFKC — compatibility decomposition then composition)
        clean = unicodedata.normalize("NFKC", text)

        # 2. Strip control characters (keep \n \t \r)
        clean = _CONTROL_CHAR_RE.sub("", clean)

        # 3. Length enforcement
        if len(clean) > self.max_length:
            clean = clean[: self.max_length]
            warnings.append(f"Message truncated to {self.max_length} characters")

        # 4. Prompt injection detection
        injection = self.detect_injection(clean)
        injection_detected = injection is not None

        if injection_detected:
            _audit_log_injection(user_id, injection, clean[:200])

        # 5. Secret detection
        secrets_found = self.detect_secrets(clean)
        if secrets_found:
            warnings.append(
                "Your message appears to contain sensitive data "
                "(API keys, passwords, or tokens). This will not be echoed back."
            )
            # Redact secrets from the clean text
            for secret_type, match_text in secrets_found:
                clean = clean.replace(match_text, f"[REDACTED_{secret_type.upper()}]")
            _audit_log_secrets(user_id, [s[0] for s in secrets_found])

        return SanitizeResult(
            clean_text=clean,
            warnings=warnings,
            blocked=False,
            injection_detected=injection_detected,
            injection_pattern=injection,
        )

    def detect_injection(self, text: str) -> Optional[str]:
        """
        Pattern-match known prompt injection techniques.
        Returns description of detected pattern or None.
        """
        for pattern_name, pattern in _INJECTION_PATTERNS:
            if pattern.search(text):
                return pattern_name

        # Base64 payload detection: look for long base64 strings that
        # decode to suspicious content
        if self._detect_base64_injection(text):
            return "base64_encoded_injection"

        return None

    def detect_secrets(self, text: str) -> List[tuple[str, str]]:
        """Detect ALL potential secrets in user input. Returns list of (type, matched_text)."""
        found: List[tuple[str, str]] = []
        for secret_type, pattern in _SECRET_PATTERNS:
            for match in pattern.finditer(text):
                found.append((secret_type, match.group(0)))
        return found

    def _detect_base64_injection(self, text: str) -> bool:
        """Check for base64-encoded payloads containing injection patterns."""
        # Look for base64 strings >= 20 chars
        b64_pattern = re.compile(r"[A-Za-z0-9+/]{20,}={0,2}")
        for match in b64_pattern.finditer(text):
            try:
                decoded = base64.b64decode(match.group(0)).decode("utf-8", errors="ignore")
                # Check if the decoded content contains injection patterns
                for _, pattern in _INJECTION_PATTERNS:
                    if pattern.search(decoded):
                        return True
            except Exception:
                continue
        return False


def _audit_log_injection(
    user_id: Optional[str], pattern_name: Optional[str], text_preview: str,
) -> None:
    """Log injection attempt for security audit."""
    logger.warning(
        "SECURITY_AUDIT: Prompt injection detected | "
        "user=%s pattern=%s preview=%s",
        user_id or "unknown",
        pattern_name,
        text_preview[:100],
    )


def _audit_log_secrets(user_id: Optional[str], secret_types: List[str]) -> None:
    """Log secret detection for security audit."""
    logger.warning(
        "SECURITY_AUDIT: Secret detected in user input | "
        "user=%s types=%s",
        user_id or "unknown",
        ",".join(secret_types),
    )


# Module-level singleton
input_sanitizer = InputSanitizer()
