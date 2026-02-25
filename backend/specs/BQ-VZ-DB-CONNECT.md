# BQ-VZ-DB-CONNECT — Database Connectivity Phase 1

**Version:** 1.0
**Author:** Vulcan (S180)
**Status:** Ready for Gate 1
**Estimated Hours:** 20
**Priority:** P1
**Track:** D (vectorAIz Integration)
**Triggered by:** Beta tester Daniel Weselius (oalta.io)

---

## 1. Problem Statement

vectorAIz only ingests files today — CSV, JSON, Parquet, XLSX, PDF, DOCX, etc. But most enterprise data lives in databases. A customer with 50 tables in Postgres or MySQL has no way to get that data into vectorAIz without manually exporting CSVs.

Daniel Weselius (oalta.io) needs this for a demo. It's the #1 beta blocker for enterprise adoption.

The Council consultation in S151 produced a 3-phase roadmap:
- **Phase 1 (this BQ):** Postgres + MySQL via SQLAlchemy → DuckDB extract. 20h.
- **Phase 2 (BQ-VZ-DB-EXPAND):** SQL Server + additional DBs as separate Docker images. 15h.
- **Phase 3 (BQ-VZ-DB-ADVANCED):** CDC/incremental sync + NoSQL via dlt. 25h. Deferred until demand proven.

## 2. Architecture

### 2.1 Design Principles

1. **Read-only always.** The connection to the customer's database is strictly read-only. Enforced at 3 levels: SQLAlchemy `execution_options(postgresql_readonly=True)`, SQL statement validation (SELECT only), and credential-level (recommend `pg_read_all_data` role).
2. **Extract, don't bridge.** We pull data out of the external DB into local Parquet files, then process them through the existing pipeline (DuckDB → chunking → embedding → Qdrant). We do NOT query the external DB at runtime for RAG.
3. **Encrypted credentials.** Connection strings are encrypted at rest using the same Fernet key infrastructure as LLM API keys (BQ-125).
4. **One dataset per table/query.** Each extracted table becomes a dataset in vectorAIz, going through the standard pipeline.
5. **Local only.** The external DB connection is made from the customer's vectorAIz instance on their LAN. No data leaves their network (unless they later publish to ai.market).

### 2.2 Data Flow

```
Customer DB (Postgres/MySQL)
    │
    ▼ SQLAlchemy (read-only connection)
    │
    ▼ Schema introspection → table list + column types + row counts
    │
    ▼ User selects tables (or writes custom SQL)
    │
    ▼ Extract: SELECT * FROM table → PyArrow batches → Parquet file
    │
    ▼ Parquet file saved to {data_directory}/{dataset_id}.parquet (raw, matching upload convention)
    │
    ▼ Standard pipeline: DatasetRecord created → PipelineService.run_full_pipeline()
    │     (chunking → embedding → Qdrant indexing)
    │
    ▼ Dataset appears in vectorAIz like any uploaded file
```

### 2.3 Credential Storage

Reuse the encrypted key storage from BQ-125 (LLM settings):

```python
# Existing: app/services/llm_settings_service.py
# Uses Fernet symmetric encryption with VECTORAIZ_APIKEY_HMAC_SECRET

class DatabaseConnection(SQLModel, table=True):
    __tablename__ = "database_connections"

    id: str = Field(primary_key=True, max_length=36)  # UUID
    name: str = Field(max_length=255)                  # Human label: "Production Analytics DB"
    db_type: str = Field(max_length=32)                # "postgresql" | "mysql"
    host: str = Field(max_length=512)                  # Encrypted
    port: int
    database: str = Field(max_length=255)              # Encrypted
    username: str = Field(max_length=255)               # Encrypted
    password_encrypted: str = Field(sa_column=Column(Text))  # Fernet-encrypted
    ssl_mode: str = Field(default="prefer", max_length=32)   # disable, prefer, require
    extra_options: Optional[str] = Field(default=None, sa_column=Column(Text))  # JSON: schema, charset, etc.
    
    # Metadata
    last_connected_at: Optional[datetime] = Field(default=None, nullable=True)
    last_sync_at: Optional[datetime] = Field(default=None, nullable=True)
    table_count: Optional[int] = Field(default=None, nullable=True)
    status: str = Field(default="configured", max_length=32)  # configured, connected, error
    error_message: Optional[str] = Field(default=None, sa_column=Column(Text, nullable=True))
    
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    updated_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
```

### 2.4 Connection Manager

```python
# app/services/db_connector.py

class DatabaseConnector:
    """Manages external database connections for data extraction."""
    
    SUPPORTED_TYPES = {"postgresql", "mysql"}
    
    # Connection pool per connection_id, lazy-initialized
    _engines: Dict[str, Engine] = {}
    
    def get_engine(self, connection: DatabaseConnection) -> Engine:
        """Create or return cached SQLAlchemy engine. Read-only enforced."""
        if connection.id not in self._engines:
            url = self._build_url(connection)
            engine = create_engine(
                url,
                pool_size=2,           # Small pool — extraction workload
                max_overflow=1,
                pool_timeout=10,
                pool_recycle=300,
                connect_args=self._connect_args(connection),
                execution_options={"postgresql_readonly": True},  # Postgres-specific
            )
            self._engines[connection.id] = engine
        return self._engines[connection.id]
    
    def test_connection(self, connection: DatabaseConnection) -> Dict:
        """Test connectivity. Returns {ok, latency_ms, server_version, error}."""
        
    def introspect_schema(self, connection: DatabaseConnection) -> List[TableInfo]:
        """Return all tables/views with columns, types, row count estimates."""
        inspector = inspect(engine)
        tables = []
        for table_name in inspector.get_table_names(schema=schema):
            columns = inspector.get_columns(table_name, schema=schema)
            pk = inspector.get_pk_constraint(table_name, schema=schema)
            # Row count estimate from pg_class.reltuples (fast) not COUNT(*) (slow)
            row_count = self._estimate_row_count(engine, schema, table_name)
            tables.append(TableInfo(
                name=table_name, schema=schema, columns=columns,
                primary_key=pk, estimated_rows=row_count
            ))
        return tables
    
    def extract_table(self, connection: DatabaseConnection, 
                      table_name: str, schema: str = None,
                      custom_sql: str = None,
                      row_limit: int = None) -> Path:
        """Extract table data to Parquet file. Returns path to parquet."""
        # Validate: only SELECT allowed if custom_sql
        if custom_sql:
            self._validate_readonly_sql(custom_sql)
        
        query = custom_sql or f'SELECT * FROM "{schema}"."{table_name}"'
        if row_limit:
            query += f" LIMIT {row_limit}"
        
        # Stream in batches via server-side cursor to handle large tables
        parquet_path = self._stream_to_parquet(engine, query, dataset_dir)
        return parquet_path
    
    def _stream_to_parquet(self, engine, query, output_dir) -> Path:
        """Execute query with server-side cursor, write Arrow batches to Parquet."""
        with engine.connect() as conn:
            # Use server-side cursor for memory efficiency
            result = conn.execution_options(stream_results=True).execute(text(query))
            
            batch_size = 10_000
            writer = None
            output_path = output_dir / "processed.parquet"
            
            while True:
                rows = result.fetchmany(batch_size)
                if not rows:
                    break
                # Convert to PyArrow RecordBatch
                batch = pa.RecordBatch.from_pylist(
                    [dict(row._mapping) for row in rows],
                    schema=arrow_schema
                )
                if writer is None:
                    writer = pq.ParquetWriter(output_path, batch.schema)
                writer.write_batch(batch)
            
            if writer:
                writer.close()
        
        return output_path
    
    @staticmethod
    def _validate_readonly_sql(sql: str):
        """Reject any non-SELECT statement."""
        # Reuse pattern from existing sql_service.py BLOCKED_PATTERNS
        normalized = sql.strip().upper()
        if not normalized.startswith("SELECT") and not normalized.startswith("WITH"):
            raise ValueError("Only SELECT queries are allowed")
        for pattern in BLOCKED_PATTERNS:
            if re.search(pattern, normalized):
                raise ValueError(f"Blocked SQL pattern detected")
```

### 2.5 DatasetRecord Integration

When extracting from a database, we create a DatasetRecord with a new source type:

```python
# Add to DatasetRecord or use metadata_json:
# source_type: "file" (default, existing) | "database"
# source_connection_id: UUID of DatabaseConnection
# source_table: "schema.table_name" or "custom_query"

# The dataset then flows through the standard pipeline identically.
# From PipelineService's perspective, it's just another processed.parquet.
```

### 2.6 API Endpoints

All under `/api/v1/db/` prefix. Auth: existing session auth (vectorAIz is single-user).

```
POST   /api/v1/db/connections              — Create connection (encrypted storage)
GET    /api/v1/db/connections              — List all connections
GET    /api/v1/db/connections/{id}         — Get connection details (password masked)
PUT    /api/v1/db/connections/{id}         — Update connection
DELETE /api/v1/db/connections/{id}         — Delete connection + cleanup engines
POST   /api/v1/db/connections/{id}/test    — Test connectivity (returns latency, version, ok/error)
GET    /api/v1/db/connections/{id}/schema  — Introspect: list tables, columns, row counts
POST   /api/v1/db/connections/{id}/extract — Extract table(s) → create dataset(s) + trigger pipeline

# Extract request body:
{
  "tables": [
    {"table": "users", "schema": "public"},
    {"table": "orders", "schema": "public", "row_limit": 100000}
  ],
  // OR custom SQL:
  "custom_sql": "SELECT u.name, COUNT(o.id) FROM users u JOIN orders o ON ... GROUP BY u.name",
  "dataset_name": "User Order Summary"  // Required for custom SQL
}
```

### 2.7 Frontend UI

New "Database" tab in the vectorAIz sidebar (alongside Files, Search, etc.):

**Connection Setup View:**
- Form: name, type (dropdown: PostgreSQL / MySQL), host, port, database, username, password
- SSL mode toggle
- "Test Connection" button → shows green check + latency or red error
- Save → encrypted storage

**Schema Browser View:**
- Tree: Connection → Schema → Tables
- Each table shows: name, column count, estimated rows, column details expandable
- Checkbox to select tables for extraction
- Optional: "Custom SQL" textarea with syntax highlighting
- "Extract Selected" button → triggers extraction + pipeline

**Extraction Progress View:**
- Reuse existing pipeline progress UI
- Shows: extracting → processing → embedding → ready
- Each table extraction as a separate dataset card

## 3. Dependencies

**New Python packages:**
- `sqlalchemy>=2.0` — Connection management, introspection, query execution
- `pymysql>=1.1` — MySQL driver (pure Python, no C deps needed in Docker)
- `cryptography` — Already present (Fernet for BQ-125)
- `psycopg2-binary` — Already present

**No new Docker services.** The external DB is the customer's, not ours.

## 4. Security

### 4.1 Read-Only Enforcement (3 layers)

1. **Connection level:** SQLAlchemy `execution_options(postgresql_readonly=True)`. MySQL: `SET SESSION TRANSACTION READ ONLY` on connect.
2. **SQL validation:** Block INSERT/UPDATE/DELETE/DROP/ALTER/TRUNCATE/COPY via regex + AST check (reuse `sql_service.py` patterns).
3. **Credential level:** Documentation recommends customers create a read-only DB user. Setup wizard shows exact `CREATE ROLE` / `GRANT` SQL for Postgres and MySQL.

### 4.2 Credential Security

- Fernet encryption at rest (same as LLM API keys)
- Password never returned in API responses (masked as `****`)
- Connection strings never logged
- Credentials deleted when connection is deleted

### 4.3 Network Security

- vectorAIz runs on the customer's LAN — connections are local/internal
- SSL mode configurable (default: prefer)
- No credentials leave the customer's network
- Connection timeout: 10s (prevents hanging on unreachable hosts)

## 5. Alembic Migration

One new migration:

```python
# alembic/versions/xxx_add_database_connections.py

def upgrade():
    op.create_table(
        "database_connections",
        sa.Column("id", sa.String(36), primary_key=True),
        sa.Column("name", sa.String(255), nullable=False),
        sa.Column("db_type", sa.String(32), nullable=False),
        sa.Column("host", sa.String(512), nullable=False),
        sa.Column("port", sa.Integer, nullable=False),
        sa.Column("database", sa.String(255), nullable=False),
        sa.Column("username", sa.String(255), nullable=False),
        sa.Column("password_encrypted", sa.Text, nullable=False),
        sa.Column("ssl_mode", sa.String(32), server_default="prefer"),
        sa.Column("extra_options", sa.Text, nullable=True),
        sa.Column("last_connected_at", sa.DateTime, nullable=True),
        sa.Column("last_sync_at", sa.DateTime, nullable=True),
        sa.Column("table_count", sa.Integer, nullable=True),
        sa.Column("status", sa.String(32), server_default="configured"),
        sa.Column("error_message", sa.Text, nullable=True),
        sa.Column("created_at", sa.DateTime, nullable=False),
        sa.Column("updated_at", sa.DateTime, nullable=False),
    )
```


## 5.5 Gate 1 Mandate Resolutions

**M1 — Pipeline Pathing (AG+MP):** Extraction writes the raw Parquet file to `{settings.data_directory}/{dataset_id}.parquet`, matching the convention of file uploads. PipelineService then creates `processed.parquet` in its standard location during the analyze/process step. The extraction does NOT write directly to `processed.parquet`.

**M2 — Type Mapping (AG):** Complex database types are explicitly handled during Arrow conversion:
- `JSONB`, `JSON`, `ARRAY`, `HSTORE` → cast to `TEXT` (JSON string representation)
- `UUID` → cast to `VARCHAR(36)`
- `BYTEA`, `BLOB` → skip column (log warning)
- `DECIMAL`/`NUMERIC` → cast to `FLOAT64` (with precision loss warning if scale > 15)
- `TIMESTAMPTZ` → normalize to UTC before Arrow conversion
- `ENUM` → cast to `VARCHAR`
- Unknown types → cast to `TEXT` with warning logged

**M3 — Disk Exhaustion (AG):** Configurable max row limit via `DB_EXTRACT_MAX_ROWS` env var (default: 5,000,000). Extraction aborts with clear error if limit exceeded. UI shows estimated extraction size (row_count × avg_row_bytes from pg_class) before user confirms.

**M4 — Read-Only Per DB (MP):** Per-database enforcement:
- **Postgres:** `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY` on connect + `execution_options(postgresql_readonly=True)` + SQL validation
- **MySQL:** `SET SESSION TRANSACTION READ ONLY` on connect + SQL validation
- Both: SELECT-only AST validation via `sqlglot` (already in requirements) as final defense layer

**M5 — Docker Networking (MP):** New section in spec + setup wizard guidance:
- Docker Desktop (Mac/Win): Use `host.docker.internal` as hostname to reach host-network databases
- Linux: Use `--network host` in docker-compose or `host.docker.internal` with `extra_hosts` mapping
- Same Docker network: Use service name directly (e.g., `postgres` if customer DB is in same compose)
- Setup wizard: auto-detect environment + suggest correct hostname pattern
- Connection test validates reachability before save

## 6. Build Plan

### Phase 1A: Backend Core (8-10h)
- `app/models/database_connection.py` — SQLModel + Alembic migration
- `app/services/db_connector.py` — Connection manager, introspection, extraction
- `app/services/db_credential_service.py` — Fernet encrypt/decrypt (reuse BQ-125 pattern)
- `app/routers/database.py` — All API endpoints
- Wire extraction output into existing `PipelineService.run_full_pipeline()`
- Add `source_type` / `source_connection_id` to DatasetRecord metadata
- Tests: connection CRUD, encryption round-trip, SQL validation, mock extraction

### Phase 1B: Frontend UI (6-8h)
- Database sidebar tab + navigation
- Connection setup form with test button
- Schema browser with table tree
- Table selection + extract trigger
- Pipeline progress (reuse existing components)
- Error states and connection status indicators

### Phase 1C: Integration Testing + Hardening (3-4h)
- End-to-end: create connection → introspect → extract → pipeline → query via RAG
- Test with real Postgres (vectorAIz's own DB as test target)
- Test with MySQL via Docker testcontainer (if CI available) or mock
- Edge cases: empty tables, huge tables (streaming), special characters, binary columns
- Connection cleanup on delete (engine disposal, dataset cleanup policy)

## 7. Files Changed

### New Files
- `app/models/database_connection.py`
- `app/services/db_connector.py`
- `app/services/db_credential_service.py`
- `app/routers/database.py`
- `alembic/versions/xxx_add_database_connections.py`
- `tests/test_db_connector.py`
- `tests/test_db_credential_service.py`
- `tests/test_database_router.py`

### Modified Files
- `app/main.py` — Register database router
- `app/models/dataset.py` — Add source metadata fields (or use metadata_json)
- `requirements.txt` — Add sqlalchemy, pymysql
- Frontend: sidebar, new pages/components

## 8. Scope Constraints

- **Phase 1 only:** Postgres + MySQL. No SQL Server, Oracle, MongoDB.
- **Full extract only.** No incremental sync, no CDC, no change detection. Re-extract replaces previous dataset.
- **No query federation.** We do NOT query the external DB at RAG time. Data is extracted to local Parquet first.
- **Single schema at a time.** User picks schema, browses tables, selects. No cross-schema joins.
- **No scheduled sync.** Manual extract only. Scheduled re-sync is Phase 2+.
- **Row limit recommended.** UI suggests LIMIT for tables >1M rows. No hard cap, but warns about extraction time/storage.
- **Custom SQL is advanced.** Hidden behind "Advanced" toggle. No query builder — raw SQL textarea.

## 9. Test Strategy

- **Unit:** SQL validation (20+ attack patterns), credential encryption round-trip, connection URL building, schema parsing
- **Integration:** Extract from vectorAIz's own Postgres (self-referential test), pipeline completion, dataset queryable via RAG after extract
- **Edge cases:** Empty tables, NULL-heavy columns, JSONB/array columns, timestamp timezone handling, very wide tables (100+ columns)
- **Security:** Injection attempts in custom SQL, connection string injection, password never in logs/responses
