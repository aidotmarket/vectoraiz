"""
MCP Server — Standalone stdio server for external LLM connectivity.

Invocation:
    docker exec -i vectoraiz python -m app.mcp_server --token vzmcp_...

Uses FastMCP from the `mcp` SDK. 4 tools delegate to QueryOrchestrator.

Phase: BQ-MCP-RAG — Universal LLM Connectivity
Created: S136
"""

import argparse
import asyncio
import json
import logging
import sys
from typing import Any, Dict, Optional

try:
    from mcp.server.fastmcp import FastMCP
    MCP_AVAILABLE = True
except ImportError:
    MCP_AVAILABLE = False

from app.models.connectivity import VectorSearchRequest, SQLQueryRequest
from app.services.connectivity_token_service import ConnectivityTokenError
from app.services.query_orchestrator import ConnectivityError, QueryOrchestrator

logger = logging.getLogger(__name__)

# Global state set during startup
_token_raw: str = ""
_orchestrator: Optional[QueryOrchestrator] = None

# Create MCP server only if SDK available
mcp_server = None
if MCP_AVAILABLE:
    mcp_server = FastMCP(name="vectoraiz", json_response=True)


def _noop_decorator(*args, **kwargs):
    """No-op decorator when MCP is not available."""
    def wrapper(fn):
        return fn
    if args and callable(args[0]):
        return args[0]
    return wrapper


def _tool():
    """Return the mcp_server.tool() decorator or a no-op if MCP unavailable."""
    if mcp_server is not None:
        return mcp_server.tool()
    return _noop_decorator


def _format_error(code: str, message: str, details: Optional[Dict[str, Any]] = None) -> str:
    """Format a structured error as JSON string for MCP isError responses."""
    return json.dumps({
        "error": {
            "code": code,
            "message": message,
            "details": details or {},
        },
    })


def _get_orchestrator() -> QueryOrchestrator:
    global _orchestrator
    if _orchestrator is None:
        _orchestrator = QueryOrchestrator()
    return _orchestrator


def _validate_token():
    """Validate the global token. Raises on failure."""
    orch = _get_orchestrator()
    return orch.validate_token(_token_raw)


@_tool()
async def vectoraiz_list_datasets() -> str:
    """List all externally-queryable datasets in vectorAIz with metadata including name, type, row count, and whether vectors are available."""
    try:
        token = _validate_token()
        orch = _get_orchestrator()
        result = await orch.list_datasets(token)
        return result.model_dump_json()
    except ConnectivityError as e:
        raise ValueError(_format_error(e.code, e.message, e.details))
    except Exception as e:
        logger.exception("Unexpected error in vectoraiz_list_datasets")
        raise ValueError(_format_error("internal_error", "An internal error occurred. Check vectorAIz logs for details."))


@_tool()
async def vectoraiz_get_schema(dataset_id: str) -> str:
    """Get column definitions for a specific dataset. Returns column names, types, nullable status, and sample values. Use dataset IDs from vectoraiz_list_datasets."""
    try:
        token = _validate_token()
        orch = _get_orchestrator()
        result = await orch.get_schema(token, dataset_id)
        return result.model_dump_json()
    except ConnectivityError as e:
        raise ValueError(_format_error(e.code, e.message, e.details))
    except Exception as e:
        logger.exception("Unexpected error in vectoraiz_get_schema")
        raise ValueError(_format_error("internal_error", "An internal error occurred. Check vectorAIz logs for details."))


@_tool()
async def vectoraiz_search(query: str, dataset_id: str = "", top_k: int = 5) -> str:
    """Semantic vector search across indexed documents and data chunks. Use natural language queries. Optionally limit to a specific dataset."""
    try:
        token = _validate_token()
        orch = _get_orchestrator()
        req = VectorSearchRequest(
            query=query,
            dataset_id=dataset_id if dataset_id else None,
            top_k=max(1, min(top_k, 20)),
        )
        result = await orch.search_vectors(token, req)
        return result.model_dump_json()
    except ConnectivityError as e:
        raise ValueError(_format_error(e.code, e.message, e.details))
    except Exception as e:
        logger.exception("Unexpected error in vectoraiz_search")
        raise ValueError(_format_error("internal_error", "An internal error occurred. Check vectorAIz logs for details."))


@_tool()
async def vectoraiz_sql(sql: str, dataset_id: str = "") -> str:
    """Execute a read-only SQL SELECT query against structured data. Tables are named dataset_{id}. Only SELECT queries are allowed. Use vectoraiz_get_schema to discover column names first."""
    try:
        token = _validate_token()
        orch = _get_orchestrator()
        req = SQLQueryRequest(
            sql=sql,
            dataset_id=dataset_id if dataset_id else None,
        )
        result = await orch.execute_sql(token, req)
        return result.model_dump_json()
    except ConnectivityError as e:
        raise ValueError(_format_error(e.code, e.message, e.details))
    except Exception as e:
        logger.exception("Unexpected error in vectoraiz_sql")
        raise ValueError(_format_error("internal_error", "An internal error occurred. Check vectorAIz logs for details."))


def main():
    global _token_raw

    if not MCP_AVAILABLE:
        print("Error: MCP SDK not installed. Run: pip install 'mcp>=1.8.0,<1.9'", file=sys.stderr)
        sys.exit(1)

    parser = argparse.ArgumentParser(description="vectorAIz MCP Server")
    parser.add_argument("--token", required=True, help="Connectivity token (vzmcp_...)")
    args = parser.parse_args()

    _token_raw = args.token

    # Validate token before starting
    try:
        _validate_token()
    except (ConnectivityError, ConnectivityTokenError) as e:
        print(f"Token validation failed: {e}", file=sys.stderr)
        sys.exit(1)

    mcp_server.run(transport="stdio")


if __name__ == "__main__":
    main()
