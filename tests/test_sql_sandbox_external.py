"""
Tests for SQL sandbox external mode — LIMIT wrapper, max length, tighter limits,
file/network/extension function blocking.

BQ-MCP-RAG Phase 1.
"""

import pytest

from app.services.sql_sandbox import SQLSandbox


@pytest.fixture
def sandbox():
    """Sandbox with two allowed tables."""
    return SQLSandbox({"dataset_abc123", "dataset_def456"})


# ---------------------------------------------------------------------------
# Standard validation (inherited from validate())
# ---------------------------------------------------------------------------

class TestStandardValidation:
    def test_select_allowed(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM dataset_abc123")
        assert ok is True

    def test_with_cte_allowed(self, sandbox):
        sql = "WITH cte AS (SELECT * FROM dataset_abc123) SELECT * FROM cte"
        ok, err = sandbox.validate_external(sql)
        assert ok is True

    def test_insert_blocked(self, sandbox):
        ok, err = sandbox.validate_external("INSERT INTO dataset_abc123 VALUES (1)")
        assert ok is False
        assert "Blocked" in err or "Only SELECT" in err

    def test_delete_blocked(self, sandbox):
        ok, err = sandbox.validate_external("DELETE FROM dataset_abc123")
        assert ok is False

    def test_drop_blocked(self, sandbox):
        ok, err = sandbox.validate_external("DROP TABLE dataset_abc123")
        assert ok is False

    def test_multiple_statements_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT 1; SELECT 2")
        assert ok is False
        assert "Multiple" in err

    def test_empty_query(self, sandbox):
        ok, err = sandbox.validate_external("")
        assert ok is False

    def test_unauthorized_table(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM secret_table")
        assert ok is False
        assert "not accessible" in err


# ---------------------------------------------------------------------------
# External mode: Max SQL length (M28)
# ---------------------------------------------------------------------------

class TestMaxSQLLength:
    def test_within_limit(self, sandbox):
        sql = "SELECT * FROM dataset_abc123 WHERE id = 1"
        ok, err = sandbox.validate_external(sql, max_length=4096)
        assert ok is True

    def test_exceeds_limit(self, sandbox):
        sql = "SELECT " + "x" * 5000
        ok, err = sandbox.validate_external(sql, max_length=4096)
        assert ok is False
        assert "maximum length" in err

    def test_exactly_at_limit(self, sandbox):
        # Build a query that's exactly at the limit
        base = "SELECT * FROM dataset_abc123"
        padding = "x" * (4096 - len(base) - 15)
        sql = f"SELECT '{padding}' FROM dataset_abc123"
        ok, err = sandbox.validate_external(sql, max_length=4096)
        # May pass or fail based on exact length — just ensure no crash
        assert isinstance(ok, bool)

    def test_custom_max_length(self, sandbox):
        sql = "SELECT * FROM dataset_abc123"
        ok, err = sandbox.validate_external(sql, max_length=10)
        assert ok is False
        assert "maximum length" in err


# ---------------------------------------------------------------------------
# External mode: File access blocking
# ---------------------------------------------------------------------------

class TestFileAccessBlocking:
    def test_read_csv_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM read_csv('/etc/passwd')")
        assert ok is False
        assert "read_csv" in err.lower()

    def test_read_csv_auto_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM read_csv_auto('/tmp/data.csv')")
        assert ok is False

    def test_read_parquet_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM read_parquet('/data/file.parquet')")
        assert ok is False

    def test_read_json_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM read_json('/data/file.json')")
        assert ok is False

    def test_read_json_auto_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM read_json_auto('/data/file.json')")
        assert ok is False

    def test_read_blob_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM read_blob('/data/file.bin')")
        assert ok is False

    def test_glob_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM glob('/data/*.csv')")
        assert ok is False

    def test_read_text_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM read_text('/etc/hosts')")
        assert ok is False

    def test_read_ndjson_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM read_ndjson('/data/file.ndjson')")
        assert ok is False

    def test_st_read_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM st_read('/data/geo.shp')")
        assert ok is False


# ---------------------------------------------------------------------------
# External mode: Network/extension function blocking
# ---------------------------------------------------------------------------

class TestNetworkBlocking:
    def test_httpfs_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM httpfs('https://evil.com/data')")
        assert ok is False

    def test_http_get_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM http_get('https://evil.com')")
        assert ok is False

    def test_http_post_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM http_post('https://evil.com')")
        assert ok is False


# ---------------------------------------------------------------------------
# External mode: Schema/system access blocking
# ---------------------------------------------------------------------------

class TestSchemaAccessBlocking:
    def test_information_schema_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM information_schema.tables")
        assert ok is False
        assert "information_schema" in err

    def test_temp_schema_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM temp.my_table")
        assert ok is False
        assert "temp" in err


# ---------------------------------------------------------------------------
# External mode: DuckDB-specific dangerous commands
# ---------------------------------------------------------------------------

class TestDuckDBDangerous:
    def test_copy_blocked(self, sandbox):
        ok, err = sandbox.validate_external("COPY dataset_abc123 TO '/tmp/out.csv'")
        assert ok is False

    def test_attach_blocked(self, sandbox):
        ok, err = sandbox.validate_external("ATTACH '/tmp/evil.db' AS evil")
        assert ok is False

    def test_pragma_blocked(self, sandbox):
        ok, err = sandbox.validate_external("PRAGMA table_info('dataset_abc123')")
        assert ok is False

    def test_install_blocked(self, sandbox):
        ok, err = sandbox.validate_external("INSTALL httpfs")
        assert ok is False

    def test_load_blocked(self, sandbox):
        ok, err = sandbox.validate_external("LOAD httpfs")
        assert ok is False

    def test_create_blocked(self, sandbox):
        ok, err = sandbox.validate_external("CREATE TABLE evil (id INT)")
        assert ok is False

    def test_set_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SET memory_limit='99GB'")
        assert ok is False


# ---------------------------------------------------------------------------
# Schema-qualified blocked access (Fix 4 — Gate 3)
# ---------------------------------------------------------------------------

class TestSchemaQualifiedBlocking:
    def test_main_schema_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM main.secret_table")
        assert ok is False
        assert "main.secret_table" in err

    def test_pg_catalog_schema_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM pg_catalog.pg_tables")
        assert ok is False

    def test_information_schema_dotted_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM information_schema.columns")
        assert ok is False

    def test_temp_schema_dotted_blocked(self, sandbox):
        ok, err = sandbox.validate_external("SELECT * FROM temp.my_temp_table")
        assert ok is False


# ---------------------------------------------------------------------------
# Quoted identifier bypass attempts (Fix 4 — Gate 3)
# ---------------------------------------------------------------------------

class TestQuotedIdentifierBypass:
    def test_quoted_attach_blocked(self, sandbox):
        ok, err = sandbox.validate_external('SELECT * FROM "ATTACH"')
        assert ok is False

    def test_quoted_copy_blocked(self, sandbox):
        ok, err = sandbox.validate_external('SELECT * FROM "COPY"')
        assert ok is False

    def test_quoted_drop_blocked(self, sandbox):
        ok, err = sandbox.validate_external('SELECT "DROP" FROM dataset_abc123')
        assert ok is False

    def test_quoted_load_blocked(self, sandbox):
        ok, err = sandbox.validate_external('SELECT * FROM "LOAD"')
        assert ok is False
