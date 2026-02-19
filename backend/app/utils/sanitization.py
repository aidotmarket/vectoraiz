"""Filename and path sanitization utilities for secure file handling."""

import re
from pathlib import Path

# Characters unsafe in filenames: quotes, slashes, backslashes, semicolons, null bytes
_UNSAFE_CHARS = re.compile(r"""['"\\\/;:\x00]""")

MAX_FILENAME_LENGTH = 200


def sanitize_filename(original: str) -> str:
    """Sanitize a user-supplied filename for safe filesystem storage.

    - Strips directory components (uses Path.name)
    - Replaces unsafe characters with underscores
    - Lowercases the extension
    - Enforces max 200 character length
    - Returns '_unnamed' for empty/whitespace-only inputs
    """
    if not original or not original.strip():
        return "_unnamed"

    # Strip directory components — only keep the final filename
    name = Path(original).name

    # After stripping path, could be empty (e.g. input was "/")
    if not name or not name.strip():
        return "_unnamed"

    # Replace unsafe characters with underscores
    name = _UNSAFE_CHARS.sub("_", name)

    # Remove any remaining ".." sequences (traversal via backslash on non-Windows)
    name = name.replace("..", "")

    # After all stripping, could be empty
    if not name or not name.strip() or name.strip("_. ") == "":
        return "_unnamed"

    # Lowercase the extension
    stem = Path(name).stem
    suffix = Path(name).suffix.lower()
    name = stem + suffix

    # Truncate to max length (preserve extension)
    if len(name) > MAX_FILENAME_LENGTH:
        max_stem = MAX_FILENAME_LENGTH - len(suffix)
        name = stem[:max_stem] + suffix

    return name


def sql_quote_literal(path: str) -> str:
    """Escape a string for safe interpolation into a DuckDB SQL single-quoted literal.

    Replaces every single quote with two single quotes (standard SQL escaping).
    """
    return path.replace("'", "''")
