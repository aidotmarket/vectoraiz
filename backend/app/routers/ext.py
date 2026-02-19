"""
External REST API Router — /api/v1/ext/* endpoints for LLM connectivity.

Auth via Authorization: Bearer vzmcp_... header.
Error envelope: {"error": {"code": "...", "message": "...", "details": {}}, "request_id": "..."}

Only mounted if CONNECTIVITY_ENABLED (§6).

Phase: BQ-MCP-RAG — Universal LLM Connectivity
Created: S136
"""

import logging
import uuid
from typing import Optional

from fastapi import APIRouter, Header, Request
from fastapi.responses import JSONResponse

from app.models.connectivity import (
    ConnectivityErrorResponse,
    DatasetListResponse,
    ERROR_HTTP_STATUS,
    HealthResponse,
    SchemaResponse,
    SearchResponse,
    SQLQueryRequest,
    SQLResponse,
    VectorSearchRequest,
)
from app.services.connectivity_metrics import get_connectivity_metrics
from app.services.query_orchestrator import ConnectivityError, get_query_orchestrator

logger = logging.getLogger(__name__)

router = APIRouter(
    prefix="/api/v1/ext",
    tags=["External Connectivity"],
)


def _extract_token(authorization: Optional[str]) -> str:
    """Extract token from Authorization: Bearer <token> header."""
    if not authorization:
        raise ConnectivityError("auth_invalid", "Authorization header is missing")
    parts = authorization.split(" ", 1)
    if len(parts) != 2 or parts[0].lower() != "bearer":
        raise ConnectivityError("auth_invalid", "Authorization header must be 'Bearer <token>'")
    return parts[1].strip()


def _get_client_ip(request: Request) -> str:
    """Extract client IP from request."""
    # Check X-Forwarded-For for proxied requests
    forwarded = request.headers.get("x-forwarded-for")
    if forwarded:
        return forwarded.split(",")[0].strip()
    return request.client.host if request.client else "127.0.0.1"


def _make_request_id() -> str:
    return f"ext-{uuid.uuid4().hex[:12]}"


def _error_response(e: ConnectivityError, request_id: Optional[str] = None) -> JSONResponse:
    """Build structured error JSONResponse."""
    status = ERROR_HTTP_STATUS.get(e.code, 500)
    body = {
        "error": {
            "code": e.code,
            "message": e.message,
            "details": e.details,
        },
        "request_id": request_id or _make_request_id(),
    }
    return JSONResponse(status_code=status, content=body)


# ------------------------------------------------------------------
# Health (no auth)
# ------------------------------------------------------------------

@router.get(
    "/health",
    response_model=HealthResponse,
    summary="Connectivity health check",
    description="Returns connectivity status including version and enabled flag. No authentication required.",
    responses={
        503: {"model": ConnectivityErrorResponse, "description": "Connectivity is disabled"},
    },
)
async def ext_health():
    orch = get_query_orchestrator()
    result = await orch.health_check()
    return result


# ------------------------------------------------------------------
# List datasets
# ------------------------------------------------------------------

@router.get(
    "/datasets",
    response_model=DatasetListResponse,
    summary="List externally-queryable datasets",
    description="Returns all datasets accessible to the authenticated token, including metadata like row count, column count, and vector status.",
    responses={
        401: {"model": ConnectivityErrorResponse, "description": "Invalid, revoked, or expired token"},
        403: {"model": ConnectivityErrorResponse, "description": "Token lacks ext:datasets scope"},
        429: {"model": ConnectivityErrorResponse, "description": "Rate limited or IP blocked"},
    },
)
async def ext_list_datasets(
    request: Request,
    authorization: Optional[str] = Header(None),
):
    request_id = _make_request_id()
    try:
        client_ip = _get_client_ip(request)
        orch = get_query_orchestrator()
        blocked = orch.rate_limiter.check_ip_blocked(client_ip)
        if blocked:
            return _error_response(ConnectivityError("ip_blocked", "Too many auth failures"), request_id)
        raw_token = _extract_token(authorization)
        token = orch.validate_token(raw_token)
        return await orch.list_datasets(token, client_ip=client_ip)
    except ConnectivityError as e:
        _record_auth_failure_if_needed(e, request)
        return _error_response(e, request_id)


# ------------------------------------------------------------------
# Get schema
# ------------------------------------------------------------------

@router.get(
    "/datasets/{dataset_id}/schema",
    response_model=SchemaResponse,
    summary="Get dataset column definitions",
    description="Returns column names, types, nullability, and sample values for a specific dataset. Useful for constructing SQL queries.",
    responses={
        401: {"model": ConnectivityErrorResponse, "description": "Invalid, revoked, or expired token"},
        403: {"model": ConnectivityErrorResponse, "description": "Token lacks ext:schema scope"},
        404: {"model": ConnectivityErrorResponse, "description": "Dataset not found"},
        429: {"model": ConnectivityErrorResponse, "description": "Rate limited or IP blocked"},
    },
)
async def ext_get_schema(
    dataset_id: str,
    request: Request,
    authorization: Optional[str] = Header(None),
):
    request_id = _make_request_id()
    try:
        client_ip = _get_client_ip(request)
        orch = get_query_orchestrator()
        blocked = orch.rate_limiter.check_ip_blocked(client_ip)
        if blocked:
            return _error_response(ConnectivityError("ip_blocked", "Too many auth failures"), request_id)
        raw_token = _extract_token(authorization)
        token = orch.validate_token(raw_token)
        return await orch.get_schema(token, dataset_id, client_ip=client_ip)
    except ConnectivityError as e:
        _record_auth_failure_if_needed(e, request)
        return _error_response(e, request_id)


# ------------------------------------------------------------------
# Search vectors
# ------------------------------------------------------------------

@router.post(
    "/search",
    response_model=SearchResponse,
    summary="Semantic vector search",
    description="Performs semantic vector similarity search across datasets. Accepts a natural language query and returns the most relevant matches ranked by score.",
    responses={
        400: {"model": ConnectivityErrorResponse, "description": "Invalid request body"},
        401: {"model": ConnectivityErrorResponse, "description": "Invalid, revoked, or expired token"},
        403: {"model": ConnectivityErrorResponse, "description": "Token lacks ext:search scope"},
        429: {"model": ConnectivityErrorResponse, "description": "Rate limited or IP blocked"},
        503: {"model": ConnectivityErrorResponse, "description": "Service unavailable"},
    },
)
async def ext_search(
    body: VectorSearchRequest,
    request: Request,
    authorization: Optional[str] = Header(None),
):
    request_id = _make_request_id()
    try:
        client_ip = _get_client_ip(request)
        orch = get_query_orchestrator()
        blocked = orch.rate_limiter.check_ip_blocked(client_ip)
        if blocked:
            return _error_response(ConnectivityError("ip_blocked", "Too many auth failures"), request_id)
        raw_token = _extract_token(authorization)
        token = orch.validate_token(raw_token)
        return await orch.search_vectors(token, body, client_ip=client_ip)
    except ConnectivityError as e:
        _record_auth_failure_if_needed(e, request)
        return _error_response(e, request_id)


# ------------------------------------------------------------------
# Execute SQL
# ------------------------------------------------------------------

@router.post(
    "/sql",
    response_model=SQLResponse,
    summary="Execute read-only SQL query",
    description="Execute a read-only SQL SELECT query against dataset tables. Tables are named dataset_<id>. Only SELECT is allowed; DDL and DML are rejected. Queries have server-enforced row limits and timeouts.",
    responses={
        400: {"model": ConnectivityErrorResponse, "description": "Forbidden SQL (non-SELECT) or query too long"},
        401: {"model": ConnectivityErrorResponse, "description": "Invalid, revoked, or expired token"},
        403: {"model": ConnectivityErrorResponse, "description": "Token lacks ext:sql scope"},
        408: {"model": ConnectivityErrorResponse, "description": "Query timed out"},
        429: {"model": ConnectivityErrorResponse, "description": "Rate limited or IP blocked"},
        503: {"model": ConnectivityErrorResponse, "description": "Service unavailable"},
    },
)
async def ext_sql(
    body: SQLQueryRequest,
    request: Request,
    authorization: Optional[str] = Header(None),
):
    request_id = _make_request_id()
    try:
        client_ip = _get_client_ip(request)
        orch = get_query_orchestrator()
        blocked = orch.rate_limiter.check_ip_blocked(client_ip)
        if blocked:
            return _error_response(ConnectivityError("ip_blocked", "Too many auth failures"), request_id)
        raw_token = _extract_token(authorization)
        token = orch.validate_token(raw_token)
        return await orch.execute_sql(token, body, client_ip=client_ip)
    except ConnectivityError as e:
        _record_auth_failure_if_needed(e, request)
        return _error_response(e, request_id)


# ------------------------------------------------------------------
# Helpers
# ------------------------------------------------------------------

def _record_auth_failure_if_needed(e: ConnectivityError, request: Request) -> None:
    """Record auth failure for IP rate limiting — only for auth_invalid (attack indicator).

    auth_revoked and auth_expired are legitimate token lifecycle events, not attacks.
    """
    if e.code == "auth_invalid":
        client_ip = _get_client_ip(request)
        metrics = get_connectivity_metrics()
        metrics.record_auth_failure(client_ip)
        orch = get_query_orchestrator()
        blocked = orch.rate_limiter.record_auth_failure(client_ip)
        if blocked:
            logger.warning("IP blocked after auth failures: %s", client_ip)
