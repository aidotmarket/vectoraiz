"""
Tunnel Router — Public URL tunnel management endpoints.

POST /api/tunnel/start   → Start tunnel, return public URL
POST /api/tunnel/stop    → Stop tunnel
GET  /api/tunnel/status  → Check tunnel status

PHASE: BQ-TUNNEL — Public URL Access (Option B)
CREATED: 2026-02-19
"""

import logging

from fastapi import APIRouter

from app.services.tunnel_service import TunnelService

logger = logging.getLogger(__name__)

router = APIRouter()


@router.post("/start")
async def start_tunnel():
    """Start a Cloudflare quick tunnel for public URL access."""
    svc = TunnelService.get_instance()

    if svc.is_running and svc.public_url:
        return {
            "status": "already_running",
            "public_url": svc.public_url,
        }

    try:
        url = await svc.start()
        return {
            "status": "started",
            "public_url": url,
        }
    except RuntimeError as e:
        return {
            "status": "error",
            "error": str(e),
        }


@router.post("/stop")
async def stop_tunnel():
    """Stop the Cloudflare tunnel."""
    svc = TunnelService.get_instance()

    if not svc.is_running:
        return {"status": "not_running"}

    await svc.stop()
    return {"status": "stopped"}


@router.get("/status")
async def tunnel_status():
    """Get current tunnel status."""
    svc = TunnelService.get_instance()
    return svc.get_status()
