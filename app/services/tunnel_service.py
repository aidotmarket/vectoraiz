"""
Tunnel Service — Cloudflare Quick Tunnel Management

Manages a cloudflared quick tunnel subprocess that exposes the local
vectorAIz instance via a free *.trycloudflare.com public URL.

No Cloudflare account required. URLs are random and temporary (change on restart).
Only started when explicitly requested by the user (not at container startup).

PHASE: BQ-TUNNEL — Public URL Access (Option B)
CREATED: 2026-02-19
"""

import asyncio
import logging
import re
import shutil
from typing import Optional

logger = logging.getLogger(__name__)


class TunnelService:
    """Manages a cloudflared quick tunnel for public URL access."""

    _instance: Optional["TunnelService"] = None
    _process: Optional[asyncio.subprocess.Process] = None
    _public_url: Optional[str] = None
    _running: bool = False

    @classmethod
    def get_instance(cls) -> "TunnelService":
        if cls._instance is None:
            cls._instance = cls()
        return cls._instance

    @property
    def public_url(self) -> Optional[str]:
        return self._public_url

    @property
    def is_running(self) -> bool:
        return self._running and self._process is not None

    @staticmethod
    def is_available() -> bool:
        """Check if cloudflared binary is installed."""
        return shutil.which("cloudflared") is not None

    async def start(self) -> str:
        """Start a cloudflared quick tunnel. Returns the public URL."""
        if self._running and self._public_url:
            return self._public_url

        if not self.is_available():
            raise RuntimeError(
                "cloudflared is not installed. "
                "This feature is available in the Docker deployment. "
                "To install manually: https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/"
            )

        # Start cloudflared quick tunnel
        self._process = await asyncio.create_subprocess_exec(
            "cloudflared", "tunnel", "--url", "http://localhost:80",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )

        # Parse the URL from cloudflared stderr output
        url = await self._wait_for_url(timeout=30)

        if url:
            self._public_url = url
            self._running = True
            logger.info("Tunnel started: %s", url)

            # Monitor process in background so we detect unexpected exits
            asyncio.create_task(self._monitor_process())

            return url
        else:
            await self.stop()
            raise RuntimeError("Failed to start tunnel — could not obtain public URL within 30s")

    async def _wait_for_url(self, timeout: int = 30) -> Optional[str]:
        """Read cloudflared stderr until we find the public URL."""
        pattern = re.compile(r"https://[a-zA-Z0-9-]+\.trycloudflare\.com")

        try:
            end_time = asyncio.get_event_loop().time() + timeout
            while asyncio.get_event_loop().time() < end_time:
                if self._process is None or self._process.stderr is None:
                    break
                try:
                    line = await asyncio.wait_for(
                        self._process.stderr.readline(), timeout=5
                    )
                    if not line:
                        break
                    text = line.decode("utf-8", errors="replace")
                    match = pattern.search(text)
                    if match:
                        return match.group(0)
                except asyncio.TimeoutError:
                    continue
        except Exception as e:
            logger.error("Error waiting for tunnel URL: %s", e)

        return None

    async def _monitor_process(self) -> None:
        """Monitor the cloudflared process and update state on exit."""
        if self._process is None:
            return
        try:
            await self._process.wait()
        except Exception:
            pass
        finally:
            if self._running:
                logger.warning("Tunnel process exited unexpectedly")
                self._running = False
                self._public_url = None
                self._process = None

    async def stop(self) -> None:
        """Stop the tunnel."""
        if self._process:
            self._process.terminate()
            try:
                await asyncio.wait_for(self._process.wait(), timeout=5)
            except asyncio.TimeoutError:
                self._process.kill()
            self._process = None
        self._public_url = None
        self._running = False
        logger.info("Tunnel stopped")

    def get_status(self) -> dict:
        return {
            "running": self.is_running,
            "public_url": self._public_url,
            "cloudflared_installed": self.is_available(),
        }
