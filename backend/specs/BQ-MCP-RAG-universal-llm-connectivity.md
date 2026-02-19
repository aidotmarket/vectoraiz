# BQ-MCP-RAG: Universal LLM Connectivity for vectorAIz

**Status:** GATE 2 — Rev 2 (Council feedback incorporated)
**Author:** Vulcan (S136)
**Gate 1:** Council unanimous PROCEED (AG, XAI, MP)
**Gate 2 R1:** AG APPROVE, MP REQUEST CHANGES (6 issues), XAI REQUEST CHANGES (3 valid)
**Gate 2 R2:** This revision — all valid feedback incorporated
**Architecture Decision:** Embedded in FastAPI (Max-approved S136)
**Estimated Build:** ~10 days across 3 phases

---

## 1. Problem Statement

vectorAIz customers process their data locally and build a RAG database (Qdrant vectors + DuckDB structured data). Today, querying this data requires using the built-in Co-Pilot chat. Customers cannot use their preferred AI — Claude Desktop, ChatGPT, Gemini, DeepSeek, Cursor, or any other LLM — against their own local data.

This is the #1 friction point for adoption. Customers don't want to learn a new chat interface. They want to keep using the AI they already know and love, pointed at their own data.

## 2. Solution Overview

Three-tier connectivity that lets ANY LLM query vectorAIz data:

| Tier | Mechanism | Covers | Priority |
|------|-----------|--------|----------|
| **1. MCP Server** | Model Context Protocol (stdio + SSE) | Claude Desktop, ChatGPT Desktop, Gemini, Cursor, VS Code, Windsurf | P0 |
| **2. REST API** | OpenAPI-documented JSON endpoints | ChatGPT Custom GPTs, programmatic access, tool-calling LLMs | P0 |
| **3. System Prompt Generator** | Copy-paste instructions with API schema | DeepSeek, Qwen, Baichuan, browser LLMs, self-hosted models, anything | P1 |

### 2.1 Platform Compatibility Matrix (M18)

| Platform | Tier | Transport | Status | Notes |
|----------|------|-----------|--------|-------|
| Claude Desktop | 1 (MCP) | stdio | **Tested** | Native MCP support since mid-2025 |
| ChatGPT Desktop | 1 (MCP) | stdio | **Tested** | MCP support via Apps |
| Cursor | 1 (MCP) | stdio | **Tested** | Native MCP in settings |
| VS Code (Copilot) | 1 (MCP) | stdio | **Best-effort** | Extension-dependent |
| Windsurf | 1 (MCP) | stdio | **Best-effort** | MCP support announced |
| Gemini (Google) | 1 (MCP) | stdio/SSE | **Best-effort** | Support rolling out |
| ChatGPT Custom GPTs | 2 (REST) | HTTP | **Tested** | Via OpenAPI Actions |
| OpenAI API (direct) | 2 (REST) | HTTP | **Tested** | Function calling |
| DeepSeek | 3 (Prompt) | Copy-paste | **Best-effort** | System prompt + curl |
| Qwen | 3 (Prompt) | Copy-paste | **Best-effort** | System prompt + curl |
| Any browser LLM | 3 (Prompt) | Copy-paste | **Universal** | Works with anything |

**Tested** = validated with real client. **Best-effort** = config generated but not end-to-end tested. **Universal** = inherently works with any LLM accepting a system prompt.

**Critical UX requirement:** allAI (the embedded Co-Pilot) is the integration concierge. It detects what platform the customer wants to connect, generates instance-specific configuration, walks them through setup step-by-step, tests the connection, and troubleshoots failures. The customer should never need to read documentation.

## 3. Architecture

### 3.1 Embedded Design (Max-approved)

The MCP server and RAG REST API run inside the existing FastAPI process. A shared **Query Orchestrator** enforces auth, rate limits, sandboxing, and audit logging for all external access paths.

```
┌─── Docker Container (vectoraiz) ──────────────────────────────┐
│                                                                │
│   FastAPI Process                                              │
│   ├── Existing routes (/api/*)                                 │
│   │                                                            │
│   ├── NEW: MCP SSE endpoint (/mcp/sse)                         │
│   ├── NEW: MCP message handler (/mcp/messages)                 │
│   ├── NEW: RAG REST API (/api/v1/ext/*)                        │
│   │                                                            │
│   ├── NEW: QueryOrchestrator (shared gateway)                  │
│   │     ├── Auth (connectivity tokens)                         │
│   │     ├── Rate limiter (per-token + per-IP + global)         │
│   │     ├── SQL Sandbox (existing, enhanced)                   │
│   │     ├── Audit logger (with redaction)                      │
│   │     ├── Metrics collector (M17)                            │
│   │     └── Response formatter                                 │
│   │                                                            │
│   ├── EXISTING: QdrantService (vector search)                  │
│   ├── EXISTING: DuckDBService (SQL queries)                    │
│   ├── EXISTING: SQLSandbox (AST validation)                    │
│   └── EXISTING: allAI tools (copilot)                          │
│                                                                │
│   Port 8100 (all traffic)                                      │
└────────────────────────────────────────────────────────────────┘

External LLM clients connect via:
  1. MCP stdio:  docker exec -i vectoraiz python -m app.mcp_server
  2. MCP SSE:    http://localhost:8100/mcp/sse
  3. REST API:   http://localhost:8100/api/v1/ext/{tool}
```

### 3.2 Query Orchestrator (Central Gateway)

ALL external queries — whether from MCP or REST — flow through the QueryOrchestrator. This is the single enforcement point for security, limits, and auditing.

```python
# app/services/query_orchestrator.py

class QueryOrchestrator:
    """
    Central gateway for all external LLM connectivity.
    Both MCP tools and REST endpoints call into this.
    """

    # --- Token management ---
    def validate_token(self, raw_token: str) -> ConnectivityToken
    # Parse vzmcp_ format, HMAC verify (constant-time), check revoked, check scope.
    # Reject revoked tokens BEFORE any expensive work.

    # --- Tool methods ---
    async def list_datasets(self, token: ConnectivityToken) -> DatasetListResponse
    async def get_schema(self, token: ConnectivityToken, dataset_id: str) -> SchemaResponse
    async def search_vectors(self, token: ConnectivityToken, req: VectorSearchRequest) -> SearchResponse
    async def execute_sql(self, token: ConnectivityToken, req: SQLQueryRequest) -> SQLResponse

    # --- Health ---
    async def health_check(self) -> HealthResponse

    # --- Internal enforcement ---
    def _enforce_auth(self, token: ConnectivityToken, scope: str) -> None
    def _enforce_rate_limit(self, token: ConnectivityToken, client_ip: str) -> None
    def _audit_log(self, token: ConnectivityToken, tool: str, request: dict, response_summary: dict, duration_ms: int) -> None
    # Audit redaction policy: never log token secrets, truncate SQL to 500 chars,
    # log row_count but NOT row contents, cap audit entry to 4KB.
    def _record_metrics(self, tool: str, duration_ms: int, success: bool) -> None

    # --- Error formatting ---
    @staticmethod
    def format_error(code: str, message: str, details: dict = None) -> dict
```

### 3.3 Connectivity Metrics (M17)

In-memory counters exposed via `connectivity_status` copilot tool and `/api/v1/ext/health`:

| Metric | Type | Description |
|--------|------|-------------|
| `ext_requests_total` | Counter | Total external requests (by tool) |
| `ext_requests_errors` | Counter | Failed requests (by error code) |
| `ext_latency_ms` | Histogram | Request latency (by tool) |
| `ext_active_connections` | Gauge | Current SSE connections |
| `ext_auth_failures` | Counter | Auth failures (by IP) |

These are in-memory Python counters (not Prometheus) — sufficient for local single-instance. Exposed to allAI for diagnostics and to the health endpoint for monitoring.

## 4. Security Model

### 4.1 Council Non-Negotiables (Gate 1)

These requirements are mandatory. No exceptions.

| # | Requirement | Rationale |
|---|-------------|-----------|
| **S1** | Loopback-only binding by default (`127.0.0.1`) | Prevent network exposure |
| **S2** | Auth tokens required even on localhost | Malware/browser extensions can hit localhost |
| **S3** | Read-only access only (no mutations) | External LLMs must not modify data |
| **S4** | SQL AST validation before execution | LLMs will attempt dangerous SQL |
| **S5** | Dataset/table allowlist enforcement | Only explicitly published datasets are queryable |
| **S6** | Query timeouts + memory limits | Prevent resource exhaustion |
| **S7** | Full audit logging with correlation IDs | Every external query must be traceable |
| **S8** | Rate limiting per token + per IP + global | Prevent runaway LLM clients and token guessing |

### 4.2 Connectivity Tokens (M19 — enhanced)

New token type for external LLM access. Separate from the existing `vz_` API keys used by the web UI.

**Format:**
```
vzmcp_<token_id>_<secret>

- token_id: 8-char alphanumeric [a-zA-Z0-9], indexed for O(1) lookup
- secret: 32-char hex [a-f0-9], generated via secrets.token_hex(16)
- Separator: underscore (_) — exactly 2 underscores in valid token
- Total length: 5 (prefix) + 8 (id) + 1 (sep) + 32 (secret) = 46 chars

Parsing rules:
1. Must start with "vzmcp_"
2. Split on "_" — must yield exactly 3 parts
3. token_id must be 8 chars, alphanumeric only
4. secret must be 32 chars, hex only
5. Reject immediately if format invalid (no DB lookup)
```

**HMAC Storage (M19):**
- Server-side pepper key: reuse existing `VECTORAIZ_APIKEY_HMAC_SECRET` (same as vz_ keys)
- Stored hash: `HMAC-SHA256(pepper_key, secret_bytes)`
- Verification: `hmac.compare_digest()` — MANDATORY constant-time comparison
- Never store or log the raw secret

**Token lifecycle:**
- Created via allAI copilot tool or Settings > Connectivity page
- Labeled by customer (e.g., "Claude Desktop on MacBook", "Cursor workspace")
- Individually revocable — revoked tokens rejected BEFORE any query processing
- Optional `expires_at` — NULL by default (no expiration), customer can set
- Max 10 active (non-revoked) tokens per instance (configurable)
- Rotation flow: create new → update client config → revoke old

**Scopes (immutable in v1):**
`ext:search`, `ext:sql`, `ext:schema`, `ext:datasets`
Scopes are set at creation time and cannot be updated. To change scopes, revoke and recreate. Scope update may be added post-v1.

### 4.3 DuckDB Sandbox (Enhanced)

Existing `sql_sandbox.py` already handles most of this (Council mandate from BQ-ALLAI-B0). Enhancements for external access:

1. **Stricter table allowlist:** External queries can ONLY access tables for datasets the customer has explicitly marked `externally_queryable = TRUE`. Default for NEW datasets: `FALSE` (M20 — least privilege). Existing datasets on migration: `FALSE` (customer must explicitly publish).
2. **Tighter resource limits for external connections:**
   - `query_timeout`: 10 seconds (vs 30s for internal)
   - `memory_limit`: 256MB (vs configurable for internal)
   - `max_rows`: 500 (vs 10,000 for internal)
   - `max_threads`: 2 (vs configurable for internal)
   - Passed to `DuckDBService.create_ephemeral_connection()` as parameters (AG note: parameterize the method)
3. **Ephemeral connections:** Each external query gets a fresh `DuckDBService.create_ephemeral_connection()` (already exists).
4. **Enforced LIMIT:** QueryOrchestrator appends/enforces `LIMIT {max_rows}` on all external SQL, even if user SQL already has a LIMIT. Wraps as: `SELECT * FROM ({user_sql}) AS __ext_q LIMIT {max_rows}`.
5. **Max SQL length:** 4096 characters. Reject longer queries.

### 4.4 Rate Limiting (M21 — multi-layer)

Three layers of rate limiting:

| Layer | Scope | Default | Purpose |
|-------|-------|---------|---------|
| **Per-token** | Requests/min per token | 30/min | Prevent single chatty LLM |
| **Per-IP** | Auth failures/min per IP | 5/min | Prevent token brute-force |
| **Global** | Total ext requests/min | 120/min | Protect overall instance |
| **Per-tool** | sql requests/min per token | 10/min | SQL is expensive |
| **Concurrency** | Max in-flight per token | 3 | Prevent parallel exhaustion |

Implementation: In-memory token bucket / sliding window. Resets on restart (acceptable for local).

Auth failure rate limiting: after 5 failed auth attempts from an IP in 1 minute, block that IP for 5 minutes. Log blocked IPs.

### 4.5 Bind Address Configuration

```python
# app/config.py additions
CONNECTIVITY_ENABLED: bool = False          # Off by default — customer must opt in
CONNECTIVITY_BIND_HOST: str = "127.0.0.1"  # Loopback only by default
CONNECTIVITY_ALLOW_LAN: bool = False        # Explicit opt-in for 0.0.0.0 (v1: out of scope)
CONNECTIVITY_MAX_TOKENS: int = 10
CONNECTIVITY_RATE_LIMIT_RPM: int = 30       # Per-token requests/min
CONNECTIVITY_RATE_LIMIT_SQL_RPM: int = 10   # Per-token SQL requests/min
CONNECTIVITY_RATE_LIMIT_GLOBAL_RPM: int = 120
CONNECTIVITY_RATE_LIMIT_AUTH_FAIL: int = 5  # Auth failures/min per IP before block
CONNECTIVITY_MAX_CONCURRENT: int = 3        # Per-token concurrency cap
CONNECTIVITY_SQL_TIMEOUT_S: int = 10
CONNECTIVITY_SQL_MAX_ROWS: int = 500
CONNECTIVITY_SQL_MEMORY_MB: int = 256
CONNECTIVITY_SQL_MAX_LENGTH: int = 4096
```

## 5. MCP Server (Tier 1)

### 5.1 Transport Modes

**Stdio (Primary — Claude Desktop, Cursor, etc.):**
```bash
# Customer runs this from host to connect to containerized vectorAIz:
docker exec -i vectoraiz python -m app.mcp_server --token vzmcp_a1b2c3d4_...
```

The `app/mcp_server.py` module runs as a standalone MCP server over stdin/stdout, importing the QueryOrchestrator directly (no HTTP hop — same process imports).

**SSE (Secondary — remote or npx bridge):**
```
GET  /mcp/sse           → SSE event stream
POST /mcp/messages      → Send MCP messages
```

Mounted on the existing FastAPI app. Uses the `mcp` Python SDK's FastAPI integration.

### 5.2 MCP Tools (4 core tools)

#### `vectoraiz_list_datasets`

List all externally-queryable datasets with metadata.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {},
  "required": []
}
```

**Output Schema:**
```json
{
  "datasets": [
    {
      "id": "string (UUID)",
      "name": "string",
      "description": "string | null",
      "type": "string (csv | json | parquet | xlsx)",
      "row_count": "integer",
      "column_count": "integer",
      "created_at": "string (ISO 8601)",
      "has_vectors": "boolean"
    }
  ],
  "count": "integer"
}
```

#### `vectoraiz_get_schema`

Get column definitions for a specific dataset.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "dataset_id": { "type": "string", "description": "Dataset ID from list_datasets" }
  },
  "required": ["dataset_id"]
}
```

**Output Schema:**
```json
{
  "dataset_id": "string",
  "table_name": "string (dataset_{id})",
  "row_count": "integer",
  "columns": [
    {
      "name": "string",
      "type": "string (DuckDB type)",
      "nullable": "boolean",
      "description": "string | null",
      "sample_values": ["string", "string", "string"]
    }
  ]
}
```

Note: One dataset = one DuckDB table (`dataset_{id}`). No separate `table` parameter needed.

#### `vectoraiz_search`

Semantic vector search across indexed documents/chunks.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Natural language search query",
      "maxLength": 1000
    },
    "dataset_id": {
      "type": "string",
      "description": "Optional: limit to specific dataset"
    },
    "top_k": {
      "type": "integer",
      "minimum": 1,
      "maximum": 20,
      "default": 5,
      "description": "Number of results (default 5)"
    }
  },
  "required": ["query"]
}
```

**Output Schema:**
```json
{
  "matches": [
    {
      "id": "string (Qdrant point ID)",
      "score": "number (0.0-1.0)",
      "text": "string (chunk content, max 2000 chars)",
      "metadata": {
        "source_file": "string",
        "dataset_id": "string",
        "dataset_name": "string",
        "page": "integer | null",
        "chunk_index": "integer"
      }
    }
  ],
  "count": "integer",
  "truncated": "boolean",
  "request_id": "string"
}
```

#### `vectoraiz_sql`

Execute a read-only SQL query against structured data.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "sql": {
      "type": "string",
      "description": "SQL SELECT query. Tables are named dataset_{id}. Only SELECT allowed.",
      "maxLength": 4096
    },
    "dataset_id": {
      "type": "string",
      "description": "Optional: scope query to a specific dataset's table"
    }
  },
  "required": ["sql"]
}
```

**Output Schema:**
```json
{
  "columns": ["string"],
  "rows": [["any"]],
  "row_count": "integer",
  "truncated": "boolean",
  "execution_ms": "integer",
  "limits_applied": {
    "max_rows": "integer",
    "max_runtime_ms": "integer",
    "max_memory_mb": "integer"
  },
  "request_id": "string"
}
```

**Constraints:** Single statement only. No parameters/prepared statements in v1. Multiple statements (`;` separator) rejected.

### 5.3 Tool Naming Convention

All tools prefixed with `vectoraiz_` to avoid collisions with other MCP servers the customer may have installed. Tool descriptions are detailed enough that the LLM knows when and how to use each one without additional prompting.

### 5.4 Error Responses

All tools return structured errors in MCP's `isError: true` format:

```json
{
  "error": {
    "code": "forbidden_sql",
    "message": "Only SELECT queries against dataset tables are permitted.",
    "details": { "blocked_statement": "DROP" }
  },
  "request_id": "ext-abc123"
}
```

**Error codes (M22):**

| Code | Meaning | MCP | REST HTTP |
|------|---------|-----|-----------|
| `auth_invalid` | Malformed or unknown token | isError | 401 |
| `auth_revoked` | Token has been revoked | isError | 401 |
| `auth_expired` | Token past expires_at | isError | 401 |
| `scope_denied` | Token lacks required scope | isError | 403 |
| `rate_limited` | Rate limit exceeded | isError | 429 |
| `ip_blocked` | IP blocked due to auth failures | isError | 429 |
| `forbidden_sql` | SQL blocked by sandbox | isError | 400 |
| `sql_too_long` | SQL exceeds max length | isError | 400 |
| `dataset_not_found` | Dataset ID unknown or not externally queryable | isError | 404 |
| `query_timeout` | Query exceeded time limit | isError | 408 |
| `service_unavailable` | Qdrant/DuckDB unreachable | isError | 503 |
| `internal_error` | Unexpected error | isError | 500 |

Note: `truncated` is NOT an error — it's a boolean field in successful responses indicating results were capped at `max_rows`.

## 6. REST API (Tier 2)

### 6.1 Endpoints

All under `/api/v1/ext/` prefix. Auth via `Authorization: Bearer vzmcp_...` header.

| Method | Path | Maps to | HTTP Success |
|--------|------|---------|-------------|
| GET | `/api/v1/ext/datasets` | `QueryOrchestrator.list_datasets()` | 200 |
| GET | `/api/v1/ext/datasets/{id}/schema` | `QueryOrchestrator.get_schema()` | 200 |
| POST | `/api/v1/ext/search` | `QueryOrchestrator.search_vectors()` | 200 |
| POST | `/api/v1/ext/sql` | `QueryOrchestrator.execute_sql()` | 200 |
| GET | `/api/v1/ext/health` | Connectivity health check (no auth) | 200 |

**Health endpoint:** Returns `{"status": "ok", "connectivity_enabled": true, "version": "1.0"}`. No dataset info, no token info, no instance details. Minimal to prevent information leakage.

### 6.2 REST Error Envelope

```json
{
  "error": {
    "code": "forbidden_sql",
    "message": "Only SELECT queries against dataset tables are permitted.",
    "details": {}
  },
  "request_id": "ext-abc123"
}
```

HTTP status codes mapped per §5.4 error code table.

### 6.3 OpenAPI Spec

Auto-generated by FastAPI with explicit examples, descriptions, and error schemas. This spec can be directly imported into:
- ChatGPT Custom GPT Actions
- Any OpenAPI-compatible tool platform

### 6.4 CORS

Disabled by default for `/api/v1/ext/` routes. If `CONNECTIVITY_ALLOW_LAN=true`, add configurable allowed origins.

## 7. System Prompt Generator (Tier 3)

For LLMs that don't support MCP or custom tools (browser-based LLMs, Chinese models, self-hosted models), allAI generates a tailored system prompt.

### 7.1 Generated Prompt Template

```
You have access to a local data API. When the user asks questions about their data,
use these endpoints:

API Base: http://localhost:8100/api/v1/ext
Auth Header: Authorization: Bearer {token}

Available Datasets:
{auto-generated list with names, descriptions, row counts}

Endpoints:
1. GET /datasets — List all available datasets
2. GET /datasets/{id}/schema — Get column definitions
3. POST /search — Semantic search: {"query": "...", "top_k": 5}
4. POST /sql — SQL query: {"sql": "SELECT ... FROM dataset_{id}", "dataset_id": "..."}

When answering data questions:
- First call /datasets to see what's available
- Use /search for natural language questions about document content
- Use /sql for structured data questions (counts, aggregations, filters)
- Always cite which dataset your answer comes from
```

### 7.2 Generation Logic

The prompt is dynamically generated based on the customer's actual datasets, schemas, and connectivity config. allAI generates it via a copilot tool and presents it as a copyable block.

## 8. allAI Copilot Integration — The Connectivity Concierge

This is the core UX differentiator. allAI becomes the customer's guide for connecting any AI to their data.

### 8.1 New Copilot Tools (7 tools — M23 adds connectivity_disable)

Added to `ALLAI_TOOLS` in `app/services/allai_tools.py` (appended, not replacing existing) and implemented in `app/services/allai_tool_executor.py`.

#### `connectivity_status`

**Description:** "Check the current status of external LLM connectivity — whether it's enabled, which tokens exist, which ports are active, and recent connection activity."

**Input:** `{}`
**Output:** enabled (bool), bind_host, tokens (list with id, label, scopes, last_used, created_at, `secret_last4` — NEVER full secret), metrics (request counts, error counts, latency), recent_queries (last 5 with timestamps, tool name, duration — NO row data).

#### `connectivity_enable`

**Description:** "Enable external LLM connectivity so the customer can connect Claude Desktop, ChatGPT, Cursor, or other AI tools to their data. This starts the MCP server and REST API."

**Input:** `{}`
**Output:** Confirmation with bind address, port, and next step (create a token).

#### `connectivity_disable` (M23)

**Description:** "Disable external LLM connectivity. All active SSE connections will be closed and all external requests will be rejected. Tokens are preserved but inactive."

**Input:** `{}`
**Output:** Confirmation that connectivity is disabled.

#### `connectivity_create_token`

**Description:** "Create a new connectivity token for an external AI tool. Each tool should have its own labeled token for easy management and revocation."

**Input:**
```json
{
  "label": { "type": "string", "description": "Human label, e.g. 'Claude Desktop on MacBook'" },
  "scopes": {
    "type": "array",
    "items": { "type": "string", "enum": ["ext:search", "ext:sql", "ext:schema", "ext:datasets"] },
    "description": "Permissions. Default: all scopes."
  }
}
```
**Output:** Token value (shown ONCE — allAI must warn user to save it), token_id, secret_last4, label, scopes, instructions to save it.

**allAI behavior:** After creating a token, allAI must explicitly warn: "Save this token now — it cannot be retrieved later. If you lose it, you'll need to create a new one."

#### `connectivity_revoke_token`

**Description:** "Revoke an existing connectivity token. The external tool using this token will immediately lose access."

**Input:** `{ "token_id": "string" }`
**Output:** Confirmation with label of revoked token.

#### `connectivity_generate_setup`

**Description:** "Generate step-by-step setup instructions for connecting a specific AI platform to vectorAIz data. Includes the exact configuration to copy-paste."

**Input:**
```json
{
  "platform": {
    "type": "string",
    "enum": ["claude_desktop", "chatgpt_desktop", "cursor", "gemini", "vscode", "openai_custom_gpt", "generic_rest", "generic_llm"],
    "description": "Target platform"
  },
  "token_id": {
    "type": "string",
    "description": "Optional: use existing token. If omitted, creates a new one."
  }
}
```
**Output:** Platform-specific step-by-step instructions with:
- Exact config JSON/YAML to paste
- Where to put it (file path, settings page, etc.)
- Token pre-filled in the config
- Validation checkpoint at each step (M24)
- How to verify it works
- Common troubleshooting tips

**Platform-specific outputs:**

**Claude Desktop:**
```json
{
  "steps": [
    { "step": 1, "instruction": "Open Claude Desktop Settings > Developer > MCP Servers", "validation": "Can you see the Developer tab?" },
    { "step": 2, "instruction": "Click 'Add MCP Server' and paste this configuration:", "config": "..." },
    { "step": 3, "instruction": "Restart Claude Desktop", "validation": "Has Claude Desktop restarted?" },
    { "step": 4, "instruction": "You should see 'vectoraiz' in the MCP tools list", "validation": "Do you see vectoraiz in the tools?" },
    { "step": 5, "instruction": "Try asking: 'What datasets do I have in vectorAIz?'", "validation": "Did you get a response listing your datasets?" }
  ],
  "config": {
    "mcpServers": {
      "vectoraiz": {
        "command": "docker",
        "args": ["exec", "-i", "vectoraiz", "python", "-m", "app.mcp_server", "--token", "vzmcp_a1b2c3d4_..."]
      }
    }
  },
  "config_file_path": "~/Library/Application Support/Claude/claude_desktop_config.json (macOS) or %APPDATA%/Claude/claude_desktop_config.json (Windows)",
  "troubleshooting": [
    "If tools don't appear: make sure the Docker container 'vectoraiz' is running (docker ps)",
    "If auth fails: verify the token hasn't been revoked in vectorAIz Settings > Connectivity",
    "If 'command not found': ensure Docker is installed and in your PATH"
  ]
}
```

**Generic LLM (system prompt):**
```json
{
  "steps": [
    { "step": 1, "instruction": "Copy the system prompt below" },
    { "step": 2, "instruction": "Paste it into your LLM's system prompt or instructions field" },
    { "step": 3, "instruction": "Start asking questions about your data" }
  ],
  "system_prompt": "You have access to a local data API at http://localhost:8100/api/v1/ext ..."
}
```

#### `connectivity_test`

**Description:** "Test the connectivity setup by running a self-diagnostic. Checks if the MCP server is responding, tokens are valid, and data is accessible."

**Input:** `{ "token_id": "string" }`
**Output:** Diagnostic report — token_valid (bool), scopes (list), datasets_accessible (count), sample_query_ok (bool — runs list_datasets, no row data returned), latency_ms, errors (list), recommendations (list).

### 8.2 allAI Intent Detection

allAI must recognize connectivity-related intents from natural language. Add to the system prompt / tool descriptions:

**Trigger phrases:**
- "connect Claude to my data", "use ChatGPT with my files"
- "how do I query this from Cursor", "MCP setup"
- "I want to use my own AI", "connect external AI"
- "API key for external access", "REST API"
- "use DeepSeek/Qwen/Gemini with my data"

**Behavior (with validation checkpoints — M24):** When allAI detects these intents, it should:
1. Check `connectivity_status` — is it already enabled?
2. If not enabled: explain what connectivity does, then offer to enable it
3. **Checkpoint:** Confirm user wants to proceed
4. Ask which platform (or detect from context)
5. Create a token (or use existing)
6. **Checkpoint:** Confirm user saved the token
7. Call `connectivity_generate_setup` with the platform
8. Present step-by-step instructions with validation at each step
9. **Checkpoint:** After config is applied, offer to run `connectivity_test`
10. Report test results and troubleshoot if needed

### 8.3 allAI System Prompt Addition

Add to the allAI system prompt (in `prompt_registry.py` or `prompt_factory.py`):

```
## External Connectivity Guide

You can help users connect their preferred AI tools (Claude Desktop, ChatGPT,
Cursor, Gemini, etc.) to query their vectorAIz data. This is a key feature —
customers should be able to use ANY AI they want with their data.

When a user asks about connecting external AI tools:
1. Use connectivity_status to check current state
2. If not enabled, explain the feature and offer to enable it
3. Detect which platform they want to connect
4. Create a labeled token for that platform — WARN them to save it
5. Generate platform-specific setup instructions
6. Walk them through each step, asking for confirmation at key points
7. Offer to test the connection when done

Be specific and practical. Give exact commands and config blocks.
The user should never need to read external documentation.

Supported platforms: Claude Desktop, ChatGPT Desktop, Cursor, VS Code,
Gemini, OpenAI Custom GPTs, and any LLM via REST API or system prompt.

IMPORTANT: When showing tokens in config blocks, remind the user this is
a secret they should not share publicly. Each connected tool should have
its own token for easy revocation if compromised.
```

## 9. New Files

### Backend (vectoraiz-backend)

| File | Purpose |
|------|---------|
| `app/services/query_orchestrator.py` | Central gateway — auth, limits, sandbox, audit, metrics |
| `app/services/connectivity_token_service.py` | Token CRUD, HMAC storage (uses VECTORAIZ_APIKEY_HMAC_SECRET), scope validation |
| `app/services/connectivity_audit.py` | Audit logging with redaction policy |
| `app/services/connectivity_metrics.py` | In-memory counters for ext request metrics |
| `app/services/connectivity_rate_limiter.py` | Multi-layer rate limiting (token + IP + global + per-tool + concurrency) |
| `app/mcp_server.py` | Standalone MCP server (stdio mode) using `mcp` SDK |
| `app/routers/mcp.py` | FastAPI routes for MCP SSE transport |
| `app/routers/ext.py` | REST API routes (`/api/v1/ext/*`) |
| `app/models/connectivity.py` | Pydantic models: tokens, requests, responses, errors |
| `alembic/versions/xxx_add_connectivity.py` | Alembic migration for connectivity_tokens + externally_queryable |
| `tests/test_query_orchestrator.py` | Orchestrator unit tests |
| `tests/test_connectivity_tokens.py` | Token lifecycle + HMAC + constant-time tests |
| `tests/test_mcp_server.py` | MCP protocol tests |
| `tests/test_ext_api.py` | REST API integration tests + HTTP status codes |
| `tests/test_sql_sandbox_external.py` | Enhanced sandbox: file/network/extension vectors |
| `tests/test_connectivity_copilot.py` | allAI copilot tool tests |
| `tests/test_rate_limiter.py` | Multi-layer rate limiting tests |

### Modified Files

| File | Changes |
|------|---------|
| `app/main.py` | Mount MCP SSE + ext router, add connectivity startup/shutdown |
| `app/config.py` | Add CONNECTIVITY_* settings (all from §4.5) |
| `app/services/allai_tools.py` | Append 7 connectivity copilot tools to existing ALLAI_TOOLS |
| `app/services/allai_tool_executor.py` | Implement 7 connectivity tool handlers |
| `app/services/sql_sandbox.py` | Add external-mode with tighter limits |
| `app/services/duckdb_service.py` | Parameterize `create_ephemeral_connection()` to accept memory_limit, threads |
| `app/models/dataset.py` | Add `externally_queryable` flag (default FALSE) |
| `app/services/prompt_factory.py` or `prompt_registry.py` | Add connectivity guide to allAI system prompt |
| `requirements.txt` | Add `mcp` SDK (pinned version) |

## 10. Database Changes

### New Table: `connectivity_tokens` (via Alembic migration)

```sql
CREATE TABLE connectivity_tokens (
    id TEXT PRIMARY KEY,              -- 8-char alphanumeric
    label TEXT NOT NULL,              -- Human label ("Claude Desktop on MacBook")
    hmac_hash TEXT NOT NULL,          -- HMAC-SHA256(pepper, secret)
    secret_last4 TEXT NOT NULL,       -- Last 4 chars of secret for UX identification
    scopes TEXT NOT NULL DEFAULT '["ext:search","ext:sql","ext:schema","ext:datasets"]',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP,             -- NULL = no expiration (default)
    last_used_at TIMESTAMP,
    request_count INTEGER NOT NULL DEFAULT 0,
    is_revoked BOOLEAN NOT NULL DEFAULT FALSE,
    revoked_at TIMESTAMP
);

CREATE INDEX idx_connectivity_tokens_revoked ON connectivity_tokens(is_revoked);
CREATE INDEX idx_connectivity_tokens_last_used ON connectivity_tokens(last_used_at);
```

### Modified Table: `datasets` (via same Alembic migration)

```sql
ALTER TABLE datasets ADD COLUMN externally_queryable BOOLEAN NOT NULL DEFAULT FALSE;
-- DEFAULT FALSE: least privilege. Customer must explicitly publish datasets for external access.
```

## 11. Build Phases

### Phase 1 — Core Infrastructure (~5 days)

**Deliverables:** QueryOrchestrator, connectivity tokens, rate limiter, 4 MCP tools, REST API, SQL sandbox enhancements.

**Build order:**
1. `connectivity.py` models + `connectivity_token_service.py` + Alembic migration
2. `connectivity_rate_limiter.py` (multi-layer)
3. `connectivity_audit.py` + `connectivity_metrics.py`
4. `query_orchestrator.py` with auth, rate limiting, audit logging, metrics
5. Enhance `sql_sandbox.py` for external mode (tighter limits, enforced LIMIT wrapper)
6. Parameterize `duckdb_service.py` `create_ephemeral_connection()`
7. `mcp_server.py` (stdio) with 4 tools calling QueryOrchestrator
8. `routers/mcp.py` (SSE transport) mounted on FastAPI
9. `routers/ext.py` (REST API) with OpenAPI docs + HTTP error mapping
10. Tests: orchestrator, tokens, MCP protocol, REST API, sandbox, rate limiter

**Tests target:** 70+ tests covering auth (incl. constant-time), sandbox (incl. file/network/extension vectors), rate limits (all layers), all tool paths, error codes + HTTP mapping, token parsing edge cases.

### Phase 2 — allAI Concierge (~3 days)

**Deliverables:** 7 copilot tools, platform detection, config generation, system prompt generator.

**Build order:**
1. Add 7 tool definitions to `allai_tools.py` (append to existing)
2. Implement handlers in `allai_tool_executor.py`
3. Platform-specific config templates (Claude, ChatGPT, Cursor, generic)
4. System prompt generator with dynamic dataset injection
5. Update allAI system prompt for connectivity guidance
6. Tests: all copilot tools, platform detection, config generation, token display safety

**Tests target:** 35+ tests covering all platforms, token creation flow (secret shown once), config generation, test diagnostics, validation checkpoints.

### Phase 3 — Polish & Cross-Platform (~2 days)

**Deliverables:** Connection Hub UI page, smoke tests, documentation.

**Build order:**
1. Frontend: Settings > Connectivity page (enable/disable, token management, test button, setup guides)
2. Cross-platform smoke tests (Claude Desktop config format, ChatGPT config format)
3. OpenAPI spec review and examples
4. Error message quality pass
5. Diagnostic bundle integration (add connectivity logs + metrics to existing bundle)

## 12. Council Mandates

All mandates incorporated:

| # | Mandate | Source | Addressed In |
|---|---------|--------|-------------|
| **M1** | Loopback-only bind by default | AG, XAI, MP | §4.5 |
| **M2** | Auth tokens even on localhost | XAI, MP | §4.2 |
| **M3** | Read-only only (no mutations) | AG, XAI, MP | §4.1 S3 |
| **M4** | SQL AST validation | MP | §4.3 (existing + enhanced) |
| **M5** | Dataset allowlist | MP | §4.3 item 1, §10 |
| **M6** | Query timeouts + memory limits | AG, MP | §4.3 item 2, §4.5 |
| **M7** | Full audit logging with redaction | MP | §3.2, §4.1 S7 |
| **M8** | Rate limiting per token | AG, MP | §4.4 |
| **M9** | Shared QueryOrchestrator for MCP + REST | AG, MP | §3.2 |
| **M10** | Ephemeral DuckDB connections for external | MP | §4.3 item 3 |
| **M11** | Scoped bearer tokens with pairing flow | MP | §4.2 |
| **M12** | allAI connectivity management tools | AG, XAI, MP | §8 |
| **M13** | Token labeling + individual revocation | MP | §4.2, §8.1 |
| **M14** | Structured error responses | MP | §5.4 |
| **M15** | Tool names prefixed to avoid collisions | Vulcan | §5.3 |
| **M16** | Disabled by default — customer must opt in | XAI | §4.5 |
| **M17** | Connectivity metrics (request count, latency, errors) | XAI | §3.3 |
| **M18** | Platform compatibility matrix (tested vs best-effort) | XAI | §2.1 |
| **M19** | HMAC pepper key (reuse existing), constant-time compare, token parsing rules, optional expires_at, rotation flow | MP | §4.2 |
| **M20** | externally_queryable default FALSE (least privilege) | MP | §4.3, §10 |
| **M21** | Multi-layer rate limiting: per-token + per-IP + global + per-tool + concurrency | MP | §4.4 |
| **M22** | Full error code → HTTP status mapping, remove result_truncated from errors | MP | §5.4 |
| **M23** | connectivity_disable copilot tool | MP | §8.1 |
| **M24** | allAI validation checkpoints in setup flow | XAI | §8.1, §8.2 |
| **M25** | Audit log redaction policy (no secrets, no row data, size caps) | MP | §3.2 |
| **M26** | Token secret_last4 for UX identification | MP | §10 |
| **M27** | Enforced LIMIT wrapper on external SQL | MP | §4.3 item 4 |
| **M28** | Max SQL length (4096 chars) | MP | §4.3 item 5 |
| **M29** | Use Alembic for DB migration | AG | §10 |
| **M30** | Parameterize DuckDBService.create_ephemeral_connection() | AG | §9 modified files |

## 13. Dependencies

- `mcp` Python SDK (pinned version for stability)
- No other new dependencies — everything else uses existing services

## 14. Out of Scope (v1)

- Write access for external LLMs (INSERT/UPDATE/DELETE)
- OAuth / OIDC authentication
- Multi-user access control (single-tenant local deployment)
- `llms.txt` / `.well-known/ai-plugin.json` discovery files
- Usage metering / billing for external queries
- LAN/remote access (loopback only in v1)
- Sidecar process extraction
- Per-dataset scoped tokens (all-or-nothing in v1 via `externally_queryable` flag)
- Token scope updates (immutable in v1 — revoke and recreate)

## 15. Success Criteria

1. Customer can connect Claude Desktop to vectorAIz in under 3 minutes with allAI guidance
2. Customer can connect ANY LLM via system prompt in under 5 minutes
3. All external queries are sandboxed, rate-limited, and audit-logged
4. Zero data mutations possible through external connectivity
5. allAI can troubleshoot common setup failures without human documentation

## 16. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| LLM crafts malicious SQL | High | High | AST sandbox + allowlist + LIMIT wrapper + ephemeral connections + timeouts |
| MCP SDK instability | Medium | Medium | Pin version, fallback to REST-only if needed |
| Resource exhaustion from chatty LLM | Medium | Medium | Multi-layer rate limiting + query budgets + DuckDB thread/memory caps |
| Customer exposes to LAN accidentally | Low | High | Loopback-only default + CONNECTIVITY_ALLOW_LAN out of scope v1 |
| Token leaked in system prompt | Medium | Low | Tokens are local-only, revocable, labeled for tracing, per-client |
| Token brute-force | Low | Medium | Per-IP auth failure rate limiting + 5min block after 5 failures |

---

*Spec version: 2.0 — S136 Gate 2 Rev 2 (Council feedback incorporated)*
*30 mandates from AG, MP, XAI addressed.*
*Re-review required before build execution.*
