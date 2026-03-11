"""
Tests that channel_config is presentation-only (BQ-VZ-CHANNEL, Condition C2).

Verifies channel_config is NOT imported in auth, billing, or feature-gate code.
Only allowed in: channel_config.py, channel_prompts.py, prompt_factory.py,
health.py (public config endpoint), and test files.
"""

import os
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Files that are ALLOWED to import channel_config
ALLOWED_FILES = {
    "app/core/channel_config.py",
    "app/prompts/channel_prompts.py",
    "app/services/prompt_factory.py",
    "app/routers/health.py",
}

CHANNEL_IMPORT_PATTERN = re.compile(
    r"(from\s+app\.core\.channel_config\s+import|import\s+app\.core\.channel_config)"
)


def _find_python_files(directory: Path):
    """Yield .py files in a directory tree."""
    if not directory.exists():
        return
    for path in directory.rglob("*.py"):
        yield path


def test_no_channel_in_auth():
    """Grep: no import of channel_config in auth modules (C2)."""
    auth_dir = REPO_ROOT / "app" / "auth"
    for py_file in _find_python_files(auth_dir):
        content = py_file.read_text()
        assert not CHANNEL_IMPORT_PATTERN.search(content), (
            f"C2 violation: {py_file.relative_to(REPO_ROOT)} imports channel_config. "
            f"Channel must NEVER affect auth."
        )


def test_no_channel_in_billing():
    """Grep: no import of channel_config in billing modules (C2)."""
    for pattern in ["app/billing", "app/services/billing", "app/routers/billing"]:
        billing_dir = REPO_ROOT / pattern
        for py_file in _find_python_files(billing_dir):
            content = py_file.read_text()
            assert not CHANNEL_IMPORT_PATTERN.search(content), (
                f"C2 violation: {py_file.relative_to(REPO_ROOT)} imports channel_config. "
                f"Channel must NEVER affect billing."
            )


def test_channel_only_in_allowed_files():
    """channel_config only imported in explicitly allowed files (C2)."""
    app_dir = REPO_ROOT / "app"
    violations = []

    for py_file in _find_python_files(app_dir):
        rel_path = str(py_file.relative_to(REPO_ROOT)).replace(os.sep, "/")
        if rel_path in ALLOWED_FILES:
            continue
        content = py_file.read_text()
        if CHANNEL_IMPORT_PATTERN.search(content):
            violations.append(rel_path)

    assert not violations, (
        f"C2 violation: channel_config imported in non-allowed files: {violations}. "
        f"Allowed: {ALLOWED_FILES}"
    )
