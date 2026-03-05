"""
Output Redaction — Targeted Pattern-Based Redaction for LLM Context

Strips sensitive patterns from text before it enters LLM context:
- API keys by prefix (sk-*, xai-*, gsk_*, AIza*)
- Bearer tokens
- Key-value secrets (password=, secret=, etc.)
- Filesystem paths (/Users/*, /home/*)
- PEM private keys
- Database URLs with credentials

IMPORTANT: UUIDs are NEVER redacted (breaks agentic loop).

PHASE: BQ-VZ-CONTROL-PLANE Step 2 — Security Foundation
CREATED: 2026-03-05
"""

import re
from typing import List, Tuple

# UUID pattern — NEVER redact these
UUID_PATTERN = re.compile(
    r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}',
    re.IGNORECASE,
)

REDACTION_RULES: List[Tuple[re.Pattern, str]] = [
    # PEM private keys (must be before shorter patterns)
    (re.compile(
        r'-----BEGIN[A-Z ]*PRIVATE KEY-----[\s\S]*?-----END[A-Z ]*PRIVATE KEY-----'
    ), '[PRIVATE_KEY_REDACTED]'),

    # API keys by prefix
    (re.compile(r'sk-ant-[a-zA-Z0-9\-]{20,}'), '[API_KEY_REDACTED]'),
    (re.compile(r'sk-[a-zA-Z0-9]{20,}'), '[API_KEY_REDACTED]'),
    (re.compile(r'xai-[a-zA-Z0-9]{20,}'), '[API_KEY_REDACTED]'),
    (re.compile(r'gsk_[a-zA-Z0-9]{20,}'), '[API_KEY_REDACTED]'),
    (re.compile(r'AIza[a-zA-Z0-9_\-]{30,}'), '[API_KEY_REDACTED]'),

    # Bearer tokens
    (re.compile(r'Bearer\s+[^\s]{10,}'), 'Bearer [REDACTED]'),

    # Key-value context: password/secret/token/key = value
    (re.compile(
        r'(password|secret|token|api_key|apikey|auth)\s*[=:]\s*[^\s,;}\]\)"\']{6,}',
        re.IGNORECASE,
    ), r'\1=[REDACTED]'),

    # Database URLs with credentials
    (re.compile(
        r'(postgres|mysql|redis|mongodb)://[^\s]+:[^\s]+@'
    ), r'\1://[CREDENTIALS_REDACTED]@'),

    # Filesystem paths (macOS/Linux home dirs)
    (re.compile(r'/Users/[^\s"\']+'), '[PATH_REDACTED]'),
    (re.compile(r'/home/[^\s"\']+'), '[PATH_REDACTED]'),
]


def redact_for_llm(text: str) -> str:
    """Strip sensitive patterns from text before it enters LLM context.

    UUIDs are preserved by extracting them first, applying redaction rules,
    then restoring them.
    """
    if not text:
        return text

    # Extract UUIDs with their positions so we can restore them
    uuid_map = {}
    for match in UUID_PATTERN.finditer(text):
        placeholder = f"__UUID_{len(uuid_map)}__"
        uuid_map[placeholder] = match.group()

    # Replace UUIDs with placeholders
    protected = text
    for placeholder, uuid_val in uuid_map.items():
        protected = protected.replace(uuid_val, placeholder, 1)

    # Apply redaction rules
    for pattern, replacement in REDACTION_RULES:
        protected = pattern.sub(replacement, protected)

    # Restore UUIDs
    for placeholder, uuid_val in uuid_map.items():
        protected = protected.replace(placeholder, uuid_val)

    return protected
