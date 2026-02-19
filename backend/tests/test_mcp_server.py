"""
Tests for MCP server tool calls, error responses, auth.

BQ-MCP-RAG Phase 1.
"""

import json

import pytest
from unittest.mock import patch, AsyncMock, MagicMock

from app.mcp_server import (
    _format_error,
    vectoraiz_list_datasets,
    vectoraiz_get_schema,
    vectoraiz_search,
    vectoraiz_sql,
)
from app.models.connectivity import (
    ConnectivityToken,
    DatasetListResponse,
    SchemaResponse,
    SearchResponse,
    SQLResponse,
    SQLLimits,
)
from app.services.query_orchestrator import ConnectivityError


@pytest.fixture
def mock_token():
    return ConnectivityToken(
        id="test1234",
        label="Test",
        scopes=["ext:search", "ext:sql", "ext:schema", "ext:datasets"],
        secret_last4="abcd",
        created_at="2026-01-01T00:00:00",
    )


@pytest.fixture
def mock_orchestrator(mock_token):
    """Patch the global orchestrator in mcp_server."""
    orch = MagicMock()
    orch.validate_token.return_value = mock_token
    return orch


# ---------------------------------------------------------------------------
# Error formatting
# ---------------------------------------------------------------------------

class TestErrorFormatting:
    def test_format_error_structure(self):
        result = _format_error("test_code", "test message", {"key": "val"})
        parsed = json.loads(result)
        assert parsed["error"]["code"] == "test_code"
        assert parsed["error"]["message"] == "test message"
        assert parsed["error"]["details"]["key"] == "val"

    def test_format_error_no_details(self):
        result = _format_error("code", "msg")
        parsed = json.loads(result)
        assert parsed["error"]["details"] == {}


# ---------------------------------------------------------------------------
# Tool: list_datasets
# ---------------------------------------------------------------------------

class TestListDatasets:
    @pytest.mark.asyncio
    async def test_list_datasets_success(self, mock_orchestrator, mock_token):
        import app.mcp_server as mcp_mod
        mcp_mod._token_raw = "vzmcp_test1234_abcdef0123456789abcdef0123456789"

        mock_orchestrator.list_datasets = AsyncMock(
            return_value=DatasetListResponse(datasets=[], count=0)
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orchestrator):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                result = await vectoraiz_list_datasets()
                data = json.loads(result)
                assert data["count"] == 0
                assert data["datasets"] == []

    @pytest.mark.asyncio
    async def test_list_datasets_auth_error(self):
        import app.mcp_server as mcp_mod
        mcp_mod._token_raw = "invalid"

        mock_orch = MagicMock()
        mock_orch.validate_token.side_effect = ConnectivityError("auth_invalid", "Bad token")

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orch):
            with pytest.raises(ValueError) as exc_info:
                await vectoraiz_list_datasets()
            error_data = json.loads(str(exc_info.value))
            assert error_data["error"]["code"] == "auth_invalid"


# ---------------------------------------------------------------------------
# Tool: get_schema
# ---------------------------------------------------------------------------

class TestGetSchema:
    @pytest.mark.asyncio
    async def test_get_schema_success(self, mock_orchestrator, mock_token):
        import app.mcp_server as mcp_mod

        mock_orchestrator.get_schema = AsyncMock(
            return_value=SchemaResponse(
                dataset_id="abc123",
                table_name="dataset_abc123",
                row_count=100,
                columns=[],
            )
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orchestrator):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                result = await vectoraiz_get_schema("abc123")
                data = json.loads(result)
                assert data["dataset_id"] == "abc123"
                assert data["row_count"] == 100

    @pytest.mark.asyncio
    async def test_get_schema_not_found(self, mock_token):
        import app.mcp_server as mcp_mod

        mock_orch = MagicMock()
        mock_orch.get_schema = AsyncMock(
            side_effect=ConnectivityError("dataset_not_found", "Not found")
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orch):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                with pytest.raises(ValueError) as exc_info:
                    await vectoraiz_get_schema("nonexistent")
                error_data = json.loads(str(exc_info.value))
                assert error_data["error"]["code"] == "dataset_not_found"


# ---------------------------------------------------------------------------
# Tool: search
# ---------------------------------------------------------------------------

class TestSearch:
    @pytest.mark.asyncio
    async def test_search_success(self, mock_orchestrator, mock_token):
        import app.mcp_server as mcp_mod

        mock_orchestrator.search_vectors = AsyncMock(
            return_value=SearchResponse(
                matches=[], count=0, truncated=False, request_id="ext-test"
            )
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orchestrator):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                result = await vectoraiz_search("test query", top_k=5)
                data = json.loads(result)
                assert data["count"] == 0

    @pytest.mark.asyncio
    async def test_search_clamps_top_k(self, mock_orchestrator, mock_token):
        import app.mcp_server as mcp_mod

        mock_orchestrator.search_vectors = AsyncMock(
            return_value=SearchResponse(
                matches=[], count=0, truncated=False, request_id="ext-test"
            )
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orchestrator):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                # top_k > 20 should be clamped
                await vectoraiz_search("query", top_k=100)
                call_args = mock_orchestrator.search_vectors.call_args
                req = call_args[0][1]  # second positional arg
                assert req.top_k == 20


# ---------------------------------------------------------------------------
# Tool: sql
# ---------------------------------------------------------------------------

class TestSQL:
    @pytest.mark.asyncio
    async def test_sql_success(self, mock_orchestrator, mock_token):
        import app.mcp_server as mcp_mod

        mock_orchestrator.execute_sql = AsyncMock(
            return_value=SQLResponse(
                columns=["id", "name"],
                rows=[[1, "test"]],
                row_count=1,
                truncated=False,
                execution_ms=50,
                limits_applied=SQLLimits(max_rows=500, max_runtime_ms=10000, max_memory_mb=256),
                request_id="ext-test",
            )
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orchestrator):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                result = await vectoraiz_sql("SELECT * FROM dataset_abc123")
                data = json.loads(result)
                assert data["row_count"] == 1
                assert data["columns"] == ["id", "name"]

    @pytest.mark.asyncio
    async def test_sql_forbidden(self, mock_token):
        import app.mcp_server as mcp_mod

        mock_orch = MagicMock()
        mock_orch.execute_sql = AsyncMock(
            side_effect=ConnectivityError("forbidden_sql", "DROP not allowed")
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orch):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                with pytest.raises(ValueError) as exc_info:
                    await vectoraiz_sql("DROP TABLE users")
                error_data = json.loads(str(exc_info.value))
                assert error_data["error"]["code"] == "forbidden_sql"


# ---------------------------------------------------------------------------
# Error hardening — no raw exception strings (Fix 3 — Gate 3)
# ---------------------------------------------------------------------------

class TestErrorHardening:
    """Verify that unexpected exceptions don't leak internal details to clients."""

    @pytest.mark.asyncio
    async def test_list_datasets_hides_internal_error(self, mock_token):
        import app.mcp_server as mcp_mod

        mock_orch = MagicMock()
        mock_orch.list_datasets = AsyncMock(
            side_effect=RuntimeError("/internal/path/to/database.db: connection refused")
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orch):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                with pytest.raises(ValueError) as exc_info:
                    await vectoraiz_list_datasets()
                error_data = json.loads(str(exc_info.value))
                assert error_data["error"]["code"] == "internal_error"
                # Must NOT contain the raw exception message
                assert "/internal/path" not in error_data["error"]["message"]
                assert "connection refused" not in error_data["error"]["message"]
                assert "Check vectorAIz logs" in error_data["error"]["message"]

    @pytest.mark.asyncio
    async def test_get_schema_hides_internal_error(self, mock_token):
        import app.mcp_server as mcp_mod

        mock_orch = MagicMock()
        mock_orch.get_schema = AsyncMock(
            side_effect=FileNotFoundError("/secret/path/data.parquet not found")
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orch):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                with pytest.raises(ValueError) as exc_info:
                    await vectoraiz_get_schema("abc123")
                error_data = json.loads(str(exc_info.value))
                assert error_data["error"]["code"] == "internal_error"
                assert "/secret/path" not in error_data["error"]["message"]

    @pytest.mark.asyncio
    async def test_search_hides_internal_error(self, mock_token):
        import app.mcp_server as mcp_mod

        mock_orch = MagicMock()
        mock_orch.search_vectors = AsyncMock(
            side_effect=ConnectionError("qdrant://localhost:6333 unreachable")
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orch):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                with pytest.raises(ValueError) as exc_info:
                    await vectoraiz_search("test query")
                error_data = json.loads(str(exc_info.value))
                assert error_data["error"]["code"] == "internal_error"
                assert "qdrant" not in error_data["error"]["message"]

    @pytest.mark.asyncio
    async def test_sql_hides_internal_error(self, mock_token):
        import app.mcp_server as mcp_mod

        mock_orch = MagicMock()
        mock_orch.execute_sql = AsyncMock(
            side_effect=MemoryError("DuckDB out of memory at 0x7fff...")
        )

        with patch.object(mcp_mod, "_get_orchestrator", return_value=mock_orch):
            with patch.object(mcp_mod, "_validate_token", return_value=mock_token):
                with pytest.raises(ValueError) as exc_info:
                    await vectoraiz_sql("SELECT 1")
                error_data = json.loads(str(exc_info.value))
                assert error_data["error"]["code"] == "internal_error"
                assert "DuckDB" not in error_data["error"]["message"]
                assert "0x7fff" not in error_data["error"]["message"]
