"""
Magic-byte MIME detection for chat attachments.

NEVER trust file extensions or Content-Type headers for security decisions.
Extensions may be used as a secondary hint for ambiguous text formats only.

BQ-ALLAI-FILES (S135)
"""

import zipfile
from pathlib import Path
from typing import Optional


def detect_mime_from_header(header_bytes: bytes) -> Optional[str]:
    """
    Detect MIME type from file header bytes (first 32+ bytes).

    Returns a MIME type string or None if unrecognized.
    For PK (ZIP) headers, returns "application/zip" — caller must use
    detect_mime_for_zip() on the full file for further classification.
    """
    # PNG
    if header_bytes.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    # JPEG
    if header_bytes.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    # WebP: RIFF header + "WEBP" at bytes 8-12
    if header_bytes[:4] == b"RIFF" and len(header_bytes) >= 12 and header_bytes[8:12] == b"WEBP":
        return "image/webp"
    # GIF
    if header_bytes[:6] in (b"GIF87a", b"GIF89a"):
        return "image/gif"
    # PDF
    if header_bytes.startswith(b"%PDF"):
        return "application/pdf"
    # ZIP-based (xlsx, etc.) — needs full-file inspection
    if header_bytes[:4] == b"PK\x03\x04":
        return "application/zip"

    # Text-based: validate UTF-8, classify by content heuristics
    try:
        text = header_bytes[:4096].decode("utf-8")
    except UnicodeDecodeError:
        return None

    # CSV heuristic: comma-separated, consistent across lines
    lines = text.strip().split("\n")
    if len(lines) > 1:
        delimiters = [line.count(",") for line in lines[:5]]
        if all(d > 0 and abs(d - delimiters[0]) <= 1 for d in delimiters):
            return "text/csv"
    # JSON heuristic
    stripped = text.strip()
    if stripped.startswith(("{", "[")):
        return "application/json"
    return "text/plain"


# ZIP bomb guard limits
_ZIP_MAX_UNCOMPRESSED = 50 * 1024 * 1024  # 50MB
_ZIP_MAX_RATIO = 100  # 100:1 compression ratio
_ZIP_MAX_ENTRIES = 1000


def detect_mime_for_zip(file_path: Path) -> Optional[str]:
    """
    For PK-header files written to disk, inspect the full ZIP to determine type.

    Returns MIME type or None (unknown/rejected ZIP).
    Includes ZIP bomb guards per spec Fix 8.
    """
    try:
        if not zipfile.is_zipfile(file_path):
            return None
        with zipfile.ZipFile(file_path, "r") as zf:
            info_list = zf.infolist()
            names = zf.namelist()

            total_uncompressed = sum(i.file_size for i in info_list)
            total_compressed = sum(i.compress_size for i in info_list)
            entry_count = len(names)

            # ZIP bomb guards
            if total_uncompressed > _ZIP_MAX_UNCOMPRESSED:
                return None
            if total_compressed > 0 and total_uncompressed / total_compressed > _ZIP_MAX_RATIO:
                return None
            if entry_count > _ZIP_MAX_ENTRIES:
                return None

            # XLSX detection by internal structure
            if "[Content_Types].xml" in names and any("xl/" in n for n in names):
                return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"

        return None  # Unknown ZIP — reject
    except (zipfile.BadZipFile, Exception):
        return None
