"""Local directory import service for vectorAIz."""
import os
import shutil
import uuid
import time
import logging
import asyncio
from pathlib import Path
from dataclasses import dataclass, field
from typing import Dict, List, Optional
from datetime import datetime, timezone

logger = logging.getLogger(__name__)

IMPORT_ROOT = Path('/imports').resolve()
UPLOAD_DIR = Path('/data/uploads')
COPY_CHUNK = 64 * 1024 * 1024  # 64MB

JUNK_FILES = {'.DS_Store', 'Thumbs.db', '.gitkeep', '.gitignore', 'desktop.ini'}


def _get_supported_extensions():
    """Import SUPPORTED_EXTENSIONS from datasets router (single source of truth)."""
    from app.routers.datasets import SUPPORTED_EXTENSIONS
    return SUPPORTED_EXTENSIONS


def validate_import_path(path_str: str) -> Path:
    """Validate path is within /imports/ and not a symlink."""
    p = Path(path_str)
    resolved = p.resolve()
    if not str(resolved).startswith(str(IMPORT_ROOT)):
        raise ValueError(f"Path outside import directory: {path_str}")
    # Walk the *original* (unresolved) path to detect symlinks before resolve
    # hides them. Normalize first to collapse redundant separators.
    normalized = Path(os.path.normpath(path_str))
    try:
        rel = normalized.relative_to(IMPORT_ROOT)
    except ValueError:
        # If the original string is already resolved (e.g. /private/... on macOS),
        # fall back to the resolved form.
        rel = resolved.relative_to(IMPORT_ROOT)
    check = IMPORT_ROOT
    for part in rel.parts:
        check = check / part
        if check.exists() and check.is_symlink():
            raise ValueError(f"Symlinks not allowed: {check}")
    return resolved


@dataclass
class ImportFileEntry:
    relative_path: str
    source_path: str
    size_bytes: int
    status: str = "pending"  # pending, copying, processing, ready, error
    dataset_id: Optional[str] = None
    bytes_copied: int = 0
    error: Optional[str] = None


@dataclass
class ImportJob:
    job_id: str
    status: str = "running"  # running, complete, cancelled, error
    files: List[ImportFileEntry] = field(default_factory=list)
    total_bytes: int = 0
    bytes_copied: int = 0
    created_at: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    cancelled: bool = False


class ImportService:
    """Manages local file import jobs."""

    def __init__(self):
        self._jobs: Dict[str, ImportJob] = {}
        self._current_job_id: Optional[str] = None

    @property
    def current_job(self) -> Optional[ImportJob]:
        if self._current_job_id:
            return self._jobs.get(self._current_job_id)
        return None

    def get_job(self, job_id: str) -> Optional[ImportJob]:
        return self._jobs.get(job_id)

    def browse(self, path_str: str, limit: int = 500, offset: int = 0) -> dict:
        """List directory contents."""
        supported = _get_supported_extensions()
        resolved = validate_import_path(path_str)
        if not resolved.is_dir():
            raise ValueError(f"Not a directory: {path_str}")

        entries = []
        for entry in sorted(resolved.iterdir(), key=lambda e: (not e.is_dir(), e.name.lower())):
            if entry.name.startswith('.') or entry.name in JUNK_FILES:
                continue
            if entry.is_symlink():
                continue
            if entry.is_dir():
                entries.append({"name": entry.name, "type": "directory"})
            elif entry.is_file() and entry.suffix.lower() in supported:
                entries.append({
                    "name": entry.name,
                    "type": "file",
                    "size_bytes": entry.stat().st_size,
                    "extension": entry.suffix.lower(),
                })

        total = len(entries)
        page = entries[offset:offset + limit]
        return {"path": path_str, "entries": page, "total": total, "limit": limit, "offset": offset}

    def scan(self, path_str: str, recursive: bool = True, max_depth: int = 5) -> dict:
        """Scan directory for importable files. Bounded: max 10K files, 30s timeout, max depth."""
        supported = _get_supported_extensions()
        resolved = validate_import_path(path_str)
        if not resolved.is_dir():
            raise ValueError(f"Not a directory: {path_str}")

        max_depth = min(max_depth, 10)  # hard cap
        files = []
        skipped = {"symlinks": 0, "unsupported": 0, "hidden": 0}
        truncated = False
        start = time.monotonic()

        def _scan(directory: Path, depth: int, base: Path):
            nonlocal truncated
            if depth > max_depth or truncated:
                return
            if time.monotonic() - start > 30:
                truncated = True
                return
            try:
                for entry in sorted(directory.iterdir(), key=lambda e: e.name.lower()):
                    if len(files) >= 10000:
                        truncated = True
                        return
                    if entry.name.startswith('.') or entry.name in JUNK_FILES:
                        skipped["hidden"] += 1
                        continue
                    if entry.is_symlink():
                        skipped["symlinks"] += 1
                        continue
                    if entry.is_dir() and recursive:
                        _scan(entry, depth + 1, base)
                    elif entry.is_file():
                        if entry.suffix.lower() in supported:
                            files.append({
                                "relative_path": str(entry.relative_to(base)),
                                "size_bytes": entry.stat().st_size,
                                "extension": entry.suffix.lower(),
                            })
                        else:
                            skipped["unsupported"] += 1
            except PermissionError:
                pass

        _scan(resolved, 1, resolved)
        total_bytes = sum(f["size_bytes"] for f in files)
        return {
            "files": files,
            "total_files": len(files),
            "total_bytes": total_bytes,
            "skipped": skipped,
            "truncated": truncated,
        }

    def start_import(self, path_str: str, file_paths: List[str]) -> ImportJob:
        """Start an import job. Max 1 concurrent."""
        if self._current_job_id and self.current_job and self.current_job.status == "running":
            raise ValueError("An import is already running")

        resolved_base = validate_import_path(path_str)

        # Build file entries and validate each
        entries = []
        total_bytes = 0
        for fp in file_paths:
            source = resolved_base / fp
            validate_import_path(str(source))
            if not source.is_file():
                raise ValueError(f"Not a file: {fp}")
            size = source.stat().st_size
            entries.append(ImportFileEntry(
                relative_path=fp,
                source_path=str(source),
                size_bytes=size,
            ))
            total_bytes += size

        # Disk preflight: check available space
        disk = shutil.disk_usage('/data')
        required = int(total_bytes * 1.1)  # 110% safety margin
        if disk.free < required:
            free_gb = round(disk.free / (1024**3), 1)
            need_gb = round(required / (1024**3), 1)
            raise ValueError(f"Insufficient disk space: {free_gb}GB free, need {need_gb}GB")

        job = ImportJob(
            job_id=f"imp_{uuid.uuid4().hex[:12]}",
            files=entries,
            total_bytes=total_bytes,
        )
        self._jobs[job.job_id] = job
        self._current_job_id = job.job_id
        return job

    async def run_import(self, job: ImportJob):
        """Execute the import — copy files and trigger processing."""
        from app.services.processing_service import get_processing_service
        from app.routers.datasets import process_dataset_task

        processing = get_processing_service()
        UPLOAD_DIR.mkdir(parents=True, exist_ok=True)

        for entry in job.files:
            if job.cancelled:
                job.status = "cancelled"
                return

            entry.status = "copying"
            src = Path(entry.source_path)
            filename = src.name

            # Create dataset record
            file_type = src.suffix.lstrip('.')
            record = processing.create_dataset(
                original_filename=filename,
                file_type=file_type,
            )
            entry.dataset_id = record.id
            dest = Path(record.upload_path)
            dest.parent.mkdir(parents=True, exist_ok=True)

            try:
                # Chunked copy with progress
                copied = 0
                with open(str(src), 'rb') as fin, open(str(dest), 'wb') as fout:
                    while True:
                        chunk = fin.read(COPY_CHUNK)
                        if not chunk:
                            break
                        fout.write(chunk)
                        copied += len(chunk)
                        entry.bytes_copied = copied
                        job.bytes_copied += len(chunk)
                        # Yield control so other requests can proceed
                        await asyncio.sleep(0)

                entry.status = "processing"

                # Trigger processing (same as upload endpoint)
                asyncio.create_task(process_dataset_task(record.id))

            except Exception as e:
                logger.error("Import copy failed for %s: %s", entry.relative_path, e)
                entry.status = "error"
                entry.error = str(e)

        # Mark complete
        if not job.cancelled:
            all_ok = all(e.status in ("processing", "ready") for e in job.files)
            job.status = "complete" if all_ok else "error"

    def cancel_job(self, job_id: str) -> bool:
        job = self._jobs.get(job_id)
        if job and job.status == "running":
            job.cancelled = True
            return True
        return False


# Singleton
_import_service: Optional[ImportService] = None


def get_import_service() -> ImportService:
    global _import_service
    if _import_service is None:
        _import_service = ImportService()
    return _import_service
