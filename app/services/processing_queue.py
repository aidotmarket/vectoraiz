"""
Sequential file processing queue.

Ensures only ONE dataset processes at a time to prevent CPU starvation
that blocks the API event loop. All file processing requests are routed
through this queue instead of running as concurrent background tasks.

Before this queue, uploading 10 files spawned 10 concurrent processing
tasks — each running embedding computation via onnxruntime — which
consumed 1300%+ CPU and starved the event loop for 2-5 minutes.
"""

import asyncio
import logging
from dataclasses import dataclass, field
from typing import Optional

from datetime import datetime, timezone

logger = logging.getLogger(__name__)


@dataclass
class _QueueItem:
    dataset_id: str
    skip_indexing: bool = False
    index_only: bool = False
    submitted_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))


class ProcessingQueue:
    """Singleton queue that processes datasets one at a time."""

    def __init__(self):
        self._queue: asyncio.Queue[_QueueItem] = asyncio.Queue()
        self._current: Optional[_QueueItem] = None
        self._worker_task: Optional[asyncio.Task] = None

    async def submit(
        self,
        dataset_id: str,
        skip_indexing: bool = False,
        index_only: bool = False,
    ) -> int:
        """Enqueue a dataset for processing. Returns queue depth."""
        item = _QueueItem(
            dataset_id=dataset_id,
            skip_indexing=skip_indexing,
            index_only=index_only,
        )
        await self._queue.put(item)
        depth = self._queue.qsize()
        logger.info(
            "Queued %s (queue_depth=%d, skip_indexing=%s, index_only=%s)",
            dataset_id, depth, skip_indexing, index_only,
        )
        return depth

    @property
    def queue_depth(self) -> int:
        return self._queue.qsize()

    @property
    def current_dataset_id(self) -> Optional[str]:
        return self._current.dataset_id if self._current else None

    def get_position(self, dataset_id: str) -> Optional[int]:
        """Queue position: 0 = processing now, 1+ = waiting, None = not queued."""
        if self._current and self._current.dataset_id == dataset_id:
            return 0
        pos = 1
        for item in list(self._queue._queue):
            if item.dataset_id == dataset_id:
                return pos
            pos += 1
        return None

    async def worker_loop(self):
        """Process one dataset at a time, forever."""
        logger.info("Processing queue worker started (concurrency=1)")
        while True:
            item = await self._queue.get()
            self._current = item
            try:
                if item.index_only:
                    logger.info("Indexing dataset %s", item.dataset_id)
                    await self._run_index(item.dataset_id)
                else:
                    logger.info(
                        "Processing dataset %s (skip_indexing=%s)",
                        item.dataset_id, item.skip_indexing,
                    )
                    await self._run_process(item.dataset_id, item.skip_indexing)
                logger.info("Completed dataset %s", item.dataset_id)
            except Exception:
                logger.exception("Failed dataset %s", item.dataset_id)
            finally:
                self._current = None
                self._queue.task_done()
                await asyncio.sleep(0)  # yield to event loop

    # ------------------------------------------------------------------
    # Internal helpers (lazy imports to avoid circular dependencies)
    # ------------------------------------------------------------------

    async def _run_process(self, dataset_id: str, skip_indexing: bool):
        """Process a dataset (extract + optionally index)."""
        from app.services.processing_service import get_processing_service
        from app.models.dataset import DatasetStatus

        processing = get_processing_service()
        await processing.process_file(dataset_id, skip_indexing=skip_indexing)

        if not skip_indexing:
            return

        # Auto-index if batch was confirmed during extraction
        from app.core.database import get_session_context
        from app.models.dataset import DatasetRecord as DBDatasetRecord

        should_index = False
        with get_session_context() as session:
            db_row = session.get(DBDatasetRecord, dataset_id)
            if (
                db_row
                and db_row.confirmed_at
                and db_row.status == DatasetStatus.PREVIEW_READY.value
            ):
                logger.info(
                    "Auto-indexing dataset %s (batch confirmed during extraction)",
                    dataset_id,
                )
                db_row.status = DatasetStatus.INDEXING.value
                db_row.updated_at = datetime.now(timezone.utc)
                session.add(db_row)
                session.commit()
                should_index = True

        if should_index:
            await self._run_index(dataset_id)

    async def _run_index(self, dataset_id: str):
        """Run index phase for a dataset."""
        from app.services.processing_service import get_processing_service

        processing = get_processing_service()
        await processing.run_index_phase(dataset_id)

    def start(self) -> asyncio.Task:
        """Start the worker as a background task."""
        if self._worker_task is None or self._worker_task.done():
            self._worker_task = asyncio.create_task(self.worker_loop())
        return self._worker_task

    async def shutdown(self):
        """Cancel the worker task."""
        if self._worker_task:
            self._worker_task.cancel()
            try:
                await self._worker_task
            except asyncio.CancelledError:
                pass
            self._worker_task = None


_instance: Optional[ProcessingQueue] = None


def get_processing_queue() -> ProcessingQueue:
    """Get the singleton ProcessingQueue."""
    global _instance
    if _instance is None:
        _instance = ProcessingQueue()
    return _instance
