"""
MCP SSE Router — FastAPI routes for MCP SSE transport.

Mounts the FastMCP server's ASGI app under /mcp.
Auth via token in query param or header.

Only mounted if CONNECTIVITY_ENABLED (§5.1).

Phase: BQ-MCP-RAG — Universal LLM Connectivity
Created: S136
"""

import logging

from fastapi import FastAPI

logger = logging.getLogger(__name__)


def mount_mcp_sse(app: FastAPI) -> None:
    """Mount the MCP SSE transport on the FastAPI app.

    The MCP SDK's FastMCP.asgi property provides a Starlette ASGI app
    that handles /sse (SSE stream) and /messages (message handler).

    After mounting at /mcp, the effective routes are:
      GET  /mcp/sse       — SSE event stream
      POST /mcp/messages  — MCP message handler
    """
    from app.mcp_server import mcp_server, MCP_AVAILABLE

    if not MCP_AVAILABLE or mcp_server is None:
        logger.warning("MCP SDK not available — SSE transport not mounted")
        return

    try:
        app.mount("/mcp", mcp_server.asgi)
    except (AttributeError, Exception) as e:
        logger.warning("MCP SSE mount failed (SDK version mismatch?): %s", e)
    logger.info("MCP SSE transport mounted at /mcp/sse and /mcp/messages")
