"""
SQL Query API endpoints for power users.

BQ-110: All sync SQLService calls wrapped via run_sync().
"""

from fastapi import APIRouter, HTTPException, Depends, Query
from typing import Optional
from pydantic import BaseModel

from app.core.async_utils import run_sync
from app.services.sql_service import (
    get_sql_service,
    SQLService,
    SQLValidationError,
    QueryTimeoutError,
    DEFAULT_ROW_LIMIT,
    MAX_ROW_LIMIT,
)
from app.auth.api_key_auth import get_current_user, AuthenticatedUser
from app.services.serial_metering import metered, MeterDecision


router = APIRouter()


class SQLQueryRequest(BaseModel):
    """SQL query request body."""
    query: str
    dataset_id: Optional[str] = None
    limit: int = DEFAULT_ROW_LIMIT
    offset: int = 0


@router.get("/tables")
async def list_tables(
    sql_service: SQLService = Depends(get_sql_service),
):
    """
    List all available dataset tables for querying.

    Returns table names, dataset IDs, and column information.
    """
    tables = await run_sync(sql_service.get_available_tables)
    return {
        "tables": tables,
        "count": len(tables),
        "usage_hint": "Use table names in your SELECT queries, e.g., SELECT * FROM dataset_abc123",
    }


@router.get("/tables/{dataset_id}")
async def get_table_schema(
    dataset_id: str,
    sql_service: SQLService = Depends(get_sql_service),
):
    """
    Get schema information for a specific dataset table.

    Returns column names, types, and row count.
    """
    try:
        schema = await run_sync(sql_service.get_table_schema, dataset_id)
        return schema
    except ValueError as e:
        raise HTTPException(status_code=404, detail=str(e))


@router.post("/query")
async def execute_query_post(
    request: SQLQueryRequest,
    sql_service: SQLService = Depends(get_sql_service),
    user: AuthenticatedUser = Depends(get_current_user),
    _meter: MeterDecision = Depends(metered("data")),
):
    """
    Execute a SQL SELECT query (POST method).

    Use this for complex queries. Only SELECT statements are allowed.
    Queries are validated for safety before execution.
    Requires X-API-Key header.
    """
    try:
        result = await run_sync(
            sql_service.execute_query,
            request.query, request.dataset_id, request.limit, request.offset,
        )
        return result
    except SQLValidationError as e:
        raise HTTPException(status_code=400, detail=f"Invalid query: {str(e)}")
    except (QueryTimeoutError, TimeoutError) as e:
        raise HTTPException(status_code=408, detail=str(e))
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    except ConnectionError:
        raise HTTPException(status_code=503, detail="SQL service unavailable")
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Query failed: {str(e)}")


@router.get("/query")
async def execute_query_get(
    q: str = Query(..., description="SQL SELECT query"),
    dataset_id: Optional[str] = Query(None, description="Target dataset (optional)"),
    limit: int = Query(DEFAULT_ROW_LIMIT, ge=1, le=MAX_ROW_LIMIT, description="Max rows"),
    offset: int = Query(0, ge=0, description="Row offset for pagination"),
    sql_service: SQLService = Depends(get_sql_service),
    user: AuthenticatedUser = Depends(get_current_user),
):
    """
    Execute a SQL SELECT query (GET method).

    For simple queries. Use POST for complex queries with special characters.
    Requires X-API-Key header.
    """
    try:
        result = await run_sync(
            sql_service.execute_query,
            q, dataset_id, limit, offset,
        )
        return result
    except SQLValidationError as e:
        raise HTTPException(status_code=400, detail=f"Invalid query: {str(e)}")
    except (QueryTimeoutError, TimeoutError) as e:
        raise HTTPException(status_code=408, detail=str(e))
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    except ConnectionError:
        raise HTTPException(status_code=503, detail="SQL service unavailable")
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Query failed: {str(e)}")


@router.post("/validate")
async def validate_query(
    request: SQLQueryRequest,
    sql_service: SQLService = Depends(get_sql_service),
):
    """
    Validate a SQL query without executing it.

    Returns whether the query would be allowed.
    """
    is_valid, error = sql_service.validate_query(request.query)
    return {
        "query": request.query,
        "valid": is_valid,
        "error": error,
    }


@router.get("/help")
async def sql_help():
    """
    Get help information for using the SQL API.
    """
    return {
        "overview": "Execute SQL SELECT queries against your processed datasets.",
        "allowed_operations": ["SELECT", "WITH (CTEs)"],
        "blocked_operations": [
            "CREATE", "DROP", "DELETE", "INSERT", "UPDATE", "ALTER",
            "COPY", "EXPORT", "IMPORT", "ATTACH", "Direct file access"
        ],
        "limits": {
            "default_row_limit": DEFAULT_ROW_LIMIT,
            "max_row_limit": MAX_ROW_LIMIT,
        },
        "examples": [
            {
                "description": "Select all rows from a dataset",
                "query": "SELECT * FROM dataset_abc123"
            },
            {
                "description": "Filter and aggregate",
                "query": "SELECT industry, COUNT(*) as count, AVG(revenue) as avg_revenue FROM dataset_abc123 GROUP BY industry"
            },
            {
                "description": "Join multiple datasets",
                "query": "SELECT a.name, b.value FROM dataset_abc123 a JOIN dataset_xyz789 b ON a.id = b.id"
            },
        ],
        "endpoints": {
            "GET /api/sql/tables": "List available tables",
            "GET /api/sql/tables/{id}": "Get table schema",
            "POST /api/sql/query": "Execute query (recommended)",
            "GET /api/sql/query?q=...": "Execute simple query",
            "POST /api/sql/validate": "Validate query without running",
        }
    }
