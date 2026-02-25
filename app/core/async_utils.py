"""
Async utilities for wrapping synchronous service calls.

Provides run_sync() to offload blocking I/O to threads,
preventing event loop starvation under concurrent load.
"""

import asyncio
import logging
import time
from typing import TypeVar, Callable, Any

logger = logging.getLogger(__name__)

T = TypeVar("T")


async def run_sync(func: Callable[..., T], *args: Any, timeout: float = 30) -> T:
    """
    Run a synchronous function in a thread without blocking the event loop.

    Uses the loop's default executor (configured as ThreadPoolExecutor in main.py).
    Logs function name and duration at DEBUG level.

    Args:
        func: Synchronous callable to execute.
        *args: Positional arguments forwarded to func.
        timeout: Maximum seconds to wait (default 30).

    Returns:
        The return value of func(*args).

    Raises:
        TimeoutError: If execution exceeds the timeout.
        Exception: Any exception raised by func propagates unchanged.
    """
    name = getattr(func, "__qualname__", None) or getattr(func, "__name__", repr(func))
    start = time.perf_counter()
    try:
        result = await asyncio.wait_for(
            asyncio.to_thread(func, *args),
            timeout=timeout,
        )
        elapsed = (time.perf_counter() - start) * 1000
        logger.debug("run_sync %s completed in %.2fms", name, elapsed)
        return result
    except asyncio.TimeoutError:
        elapsed = (time.perf_counter() - start) * 1000
        raise TimeoutError(
            f"{name} timed out after {elapsed:.0f}ms (limit {timeout}s)"
        )
