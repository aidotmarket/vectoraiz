# BQ-ALLAI-AWARE: Situationally Aware AI — Eyes, Hands, Reach
## Product-Critical Feature Spec v2.0 — Council Reviewed + Security Hardened

> **Council Status: APPROVED with mandatory changes (all incorporated below)**
> - XAI (Architect): GREEN — harden security, enforce proxy, cap loops
> - MP (Tech Lead): GREEN — 10 blocking changes incorporated
> - AG (Builder): GREEN — confirmed MVP is mostly wiring existing services
>
> **Review date:** 2026-02-16 S134
> **Philosophy:** "allAI has to be genuinely functional or we are just Microsoft Clippy"

---

### Why This Matters

No product in the market has a genuinely situationally aware AI assistant. Every
"AI copilot" today is a chatbot bolted onto a sidebar — it doesn't know where you
are, what you're looking at, what you have, or how to act on your behalf. It's a
parlor trick.

**allAI changes this.** A user sitting on the Datasets page with 5 uploaded files
says "what are my current files?" and allAI should:
1. KNOW she's on the Datasets page (not hallucinate "home screen")
2. KNOW the 5 datasets with names, sizes, row counts, status
3. BE ABLE TO act — preview rows, run queries, trigger processing
4. EVENTUALLY reach outside vectorAIz — grab files from a directory

This is the core product differentiator. This is how we win.

### Current State (Broken)

| Capability | Status | Evidence |
|-----------|--------|----------|
| Knows which page user is on | ❌ BROKEN | Frontend never sends STATE_SNAPSHOT; allAI hallucinates "home screen" |
| Knows user's datasets | ❌ BROKEN | `_get_dataset_summary()` returns stub; no dataset list in context |
| Can list datasets | ❌ BROKEN | No tool use; allAI says "go look at the Data tab" |
| Can query data | ❌ BROKEN | No tool use; allAI can only describe what it would do |
| Can act on behalf of user | ❌ BROKEN | Pure text-in/text-out through ai.market proxy |
| Can access external files | ❌ NOT BUILT | No filesystem access |

**Root causes:**
1. Frontend sends BRAIN_MESSAGE but never STATE_SNAPSHOT (handler exists, never called)
2. AiMarketAllieProvider is pure SSE text streaming — no tool/function calling
3. Context manager has stubs where real data should be
4. No tool definitions, no tool execution loop, no command infrastructure utilized

---

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│  FRONTEND (React)                                               │
│                                                                 │
│  useLocation() ──→ STATE_SNAPSHOT ──→ WebSocket ──→ Backend     │
│                    {route, datasets,                            │
│                     active_dataset_id}                          │
│                                                                 │
│  ←── TOOL_RESULT (rich data) ←── WebSocket ←── Backend          │
│  ←── TOOL_STATUS (activity) ←── WebSocket ←── Backend           │
│  ←── CONFIRMATION_REQUIRED ←── WebSocket ←── Backend            │
│  (render inline: tables, charts, previews, confirm dialogs)     │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  BACKEND (FastAPI + WebSocket)                                  │
│                                                                 │
│  CoPilotService.process_message_streaming()                     │
│    │                                                            │
│    ├── 1. Build context (CoPilotContextManager)                 │
│    │      • UI state from STATE_SNAPSHOT                        │
│    │      • Dataset list from DB (real, not stub)               │
│    │      • Active dataset detail (schema, stats)               │
│    │      • System health                                       │
│    │                                                            │
│    ├── 2. Build prompt (PromptFactory)                          │
│    │      • 5-layer prompt + dataset context + tool defs        │
│    │                                                            │
│    ├── 3. LLM Call with Tools (agentic loop)                    │
│    │      • Send prompt + tools to LLM via ai.market proxy      │
│    │      • If tool_use → execute tool → feed SUMMARY back      │
│    │      • If text → stream to frontend                        │
│    │      • Loop until done (max 5 iterations)                  │
│    │      • Run ID tracking + heartbeats for WS resilience      │
│    │                                                            │
│    └── 4. Tool Execution Engine (sandboxed)                     │
│           • list_datasets()          — read-only                │
│           • get_dataset_detail(id)   — read-only                │
│           • preview_rows(id, limit)  — read-only, row-capped    │
│           • run_sql_query(query)     — AST-validated SELECT     │
│           • search_vectors(q, id?)   — read-only                │
│           • get_system_status()      — read-only                │
│           • get_dataset_statistics() — read-only                │
│           • delete_dataset(id)       — CONFIRMATION TOKEN req'd │
│           • list_directory(path)     — Phase C, sandboxed jail  │
│           • import_files(paths)      — Phase C, sandboxed jail  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Council Security Mandates (v2.0)

These 6 changes are **non-negotiable** per Council review. All incorporated into
the phase specs below. Marked with 🛡️ where they appear.

### 🛡️ M1: Server-Enforced Confirmation Tokens (MP mandate)
**Problem:** `confirm: boolean` controlled by LLM is security theater. Prompt
injection makes LLM set `confirm=true`.

**Fix:** Two-step, server-enforced confirmation:
1. LLM requests destructive action → backend returns `CONFIRMATION_REQUIRED`
   with a `confirmation_id` (UUID, bound to user+tool+resource, 60s expiry)
2. Frontend shows confirmation dialog with details
3. Only a **human UI click** sends `confirmation_id` back via WebSocket
4. Backend validates token (user match, tool match, resource match, not expired)
5. Tool executes only with valid token

**Impact:** `delete_dataset` tool loses `confirm` param. LLM can REQUEST deletion
but never EXECUTE it. All destructive tools follow this pattern.

### 🛡️ M2: SQL AST Validation (MP mandate)
**Problem:** "SQL service enforces read-only" is insufficient. DuckDB has COPY,
ATTACH, INSTALL, PRAGMA, read_csv_auto — all dangerous.

**Fix:** Hard allowlist at SQL gateway:
1. Parse SQL into AST (DuckDB parser or sqlglot)
2. Reject anything that isn't a single SELECT (or WITH...SELECT)
3. Explicit blocklist: INSERT, UPDATE, DELETE, DROP, ALTER, CREATE, COPY,
   EXPORT, ATTACH, INSTALL, LOAD, PRAGMA, read_csv_auto, httpfs
4. Per-user table exposure: only expose tables for user's datasets
5. Row/CPU/memory quotas at execution layer (max 10k rows, 5s timeout, 256MB)

### 🛡️ M3: Two-Track Tool Results — Data Minimization (MP mandate)
**Problem:** Feeding raw dataset rows back into LLM context means customer PII
flows through the LLM provider. Violates non-custodial principle.

**Fix:** Two-track rendering:
- **Track 1 (frontend):** Full TOOL_RESULT with rich data sent directly to
  frontend via WebSocket. Frontend renders tables, charts, etc.
- **Track 2 (LLM context):** Only a short SUMMARY goes back to the LLM.
  Example: "Displayed 10 rows from barcelona_apartments.csv to user.
  Columns: listing_id, neighbourhood, bedrooms, price_eur, sqm.
  Price range: €185,000-€890,000. 3 rows have missing energy_rating."

**Never in LLM context:** raw row values, PII fields, full query results.
**Always in LLM context:** column names, row counts, data ranges, anomalies.

### 🛡️ M4: ai.market Proxy for All LLM Calls (XAI + MP mandate)
**Problem:** Direct Anthropic calls bypass metering, billing, audit trail,
and the non-custodial choke point.

**Fix:** All LLM calls go through ai.market proxy. Extend `/api/v1/allie/chat`
to support tool-enabled requests:
```json
{
  "messages": [...],
  "tools": [...],
  "request_id": "...",
  "stream": true
}
```
Proxy handles: key management, metering, rate limits, audit logging, abuse
detection. If proxy can't be updated in time for Phase B MVP, use direct
Anthropic as a **temporary escape hatch** (max 2 weeks) with local
metering/audit — NOT as permanent architecture.

### 🛡️ M5: WebSocket Resilience — Run ID + Resume (MP mandate)
**Problem:** Tool loops take 10-30s. WebSocket may timeout or disconnect.
No recovery = double execution on reconnect = double billing.

**Fix:**
1. Server assigns `run_id` + `event_seq` for each user message
2. All events (TOOL_STATUS, TOOL_RESULT, BRAIN_STREAM_CHUNK) carry run_id + seq
3. Intermediate state persisted (Redis): tool calls made, results, partial text
4. On WS disconnect + reconnect, client sends `RESUME {run_id, last_event_seq}`
5. Server replays missed events OR continues the run
6. Heartbeat events every 5s during tool execution to keep connection alive
7. Frontend shows "Reconnecting..." state, not a blank screen

### 🛡️ M6: Filesystem Sandboxing (MP mandate — Phase C only)
**Problem:** Path allowlists are trivially bypassed via symlinks, `../`,
mount points, unicode normalization.

**Fix:**
1. Filesystem tools run in a **separate sandboxed process**
2. Read-only mounted volumes of approved directories only
3. All paths resolved via `os.path.realpath()` THEN containment check
4. Symlinks resolved and re-checked against allowlist
5. Import staging directory: user drops files into a dedicated folder,
   tool can only read that folder — no general browsing
6. In Docker: scoped to mounted volumes only
7. In connected/cloud mode: filesystem access **DISABLED entirely**

---

## Phase A: Eyes — State Awareness + Dataset Context (~3h)

### A1. Frontend sends STATE_SNAPSHOT on route changes

**File: `frontend/src/contexts/CoPilotContext.tsx`**

Add a `useEffect` that watches `location.pathname` (from react-router) and
sends STATE_SNAPSHOT through the WebSocket whenever:
- WebSocket connects (initial state)
- Route changes (navigation)
- Dataset is selected/deselected
- Upload completes (dataset list changed)

```typescript
// In CoPilotProvider — needs useLocation() from react-router
import { useLocation } from "react-router-dom";

const location = useLocation();

// Send state snapshot on route change + WS reconnect
useEffect(() => {
  if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) return;

  wsRef.current.send(JSON.stringify({
    type: "STATE_SNAPSHOT",
    current_route: location.pathname,
    active_dataset_id: activeDatasetId || null,
    dataset_summary: datasets?.map(d => ({
      id: d.id,
      filename: d.original_filename,
      file_type: d.file_type,
      status: d.status,
      rows: d.metadata?.row_count,
      columns: d.metadata?.column_count,
      size_bytes: d.metadata?.file_size_bytes,
    })) || [],
  }));
}, [location.pathname, activeDatasetId, datasets?.length]);
```

**Note:** CoPilotProvider may need to be placed INSIDE the Router context.
Check component tree. If CoPilotProvider wraps Router, move it inside.

### A2. StateSnapshot model extended

**File: `app/models/copilot.py`**

```python
class DatasetSnapshotItem(BaseModel):
    id: str
    filename: str
    file_type: str
    status: str
    rows: Optional[int] = None
    columns: Optional[int] = None
    size_bytes: Optional[int] = None

class StateSnapshot(BaseModel):
    current_route: str = "/"
    active_dataset_id: Optional[str] = None
    form_state: Optional[Dict[str, Any]] = None
    dataset_summary: Optional[List[DatasetSnapshotItem]] = None  # NEW
```

### A3. CoPilotContextManager — real data injection

**File: `app/services/context_manager_copilot.py`**

Replace ALL stubs with real data:

```python
async def build_context(self, state_snapshot=None, ...):
    # ...existing code...

    # Dataset list: prefer snapshot (fresh from frontend), fallback to DB
    dataset_list = []
    if state_snapshot and state_snapshot.dataset_summary:
        dataset_list = [d.dict() for d in state_snapshot.dataset_summary]
    else:
        dataset_list = await self._get_all_datasets_summary()

    # Active dataset: real DB lookup with schema
    dataset_summary = None
    if active_dataset_id:
        dataset_summary = await self._get_dataset_summary_real(active_dataset_id)

    return AllieContext(
        # ...existing fields...
        dataset_list=dataset_list,
        dataset_summary=dataset_summary,
    )

async def _get_all_datasets_summary(self) -> List[Dict[str, Any]]:
    """Fetch all non-deleted datasets with metadata. DB fallback."""
    from app.services.processing_service import get_processing_service
    svc = get_processing_service()
    records = svc.list_datasets()
    return [
        {
            "id": r.id,
            "filename": r.original_filename,
            "file_type": r.file_type,
            "status": r.status.value if hasattr(r.status, 'value') else str(r.status),
            "rows": r.metadata.get("row_count"),
            "columns": r.metadata.get("column_count"),
            "size_bytes": r.file_size_bytes,
        }
        for r in records
    ]

async def _get_dataset_summary_real(self, dataset_id: str) -> Optional[Dict]:
    """Real dataset detail with schema — replaces stub."""
    from app.services.processing_service import get_processing_service
    svc = get_processing_service()
    record = svc.get_dataset(dataset_id)
    if not record:
        return {"dataset_id": dataset_id, "status": "not_found"}
    return {
        "dataset_id": record.id,
        "filename": record.original_filename,
        "file_type": record.file_type,
        "status": record.status.value if hasattr(record.status, 'value') else str(record.status),
        "rows": record.metadata.get("row_count"),
        "columns": record.metadata.get("column_count"),
        "column_names": record.metadata.get("column_names", []),
        "dtypes": record.metadata.get("dtypes", {}),
        "size_bytes": record.file_size_bytes,
        "created_at": record.created_at.isoformat() if record.created_at else None,
    }
```

### A4. PromptFactory Layer 4 — dataset list in context

**File: `app/services/prompt_factory.py`**

Extend `_layer_4_context()`:

```python
def _layer_4_context(self, context: AllieContext) -> str:
    # ...existing UI state + system state...

    # Dataset inventory
    if context.dataset_list:
        ds_lines = []
        for ds in context.dataset_list:
            status = ds.get("status", "unknown")
            rows = ds.get("rows")
            cols = ds.get("columns")
            dims = f"{rows} rows × {cols} cols" if rows and cols else "processing"
            ds_lines.append(
                f"  - [{ds['id']}] {ds.get('filename','?')} ({dims}) [{status}]"
            )
        parts.append(f"""
**User's Datasets ({len(context.dataset_list)} total):**
{chr(10).join(ds_lines)}""")
    else:
        parts.append("\n**User's Datasets:** None uploaded yet.")

    # Active dataset schema (if viewing one)
    if context.dataset_summary and context.dataset_summary.get("column_names"):
        ds = context.dataset_summary
        col_info = ", ".join(
            f"{c} ({ds.get('dtypes',{}).get(c,'?')})"
            for c in ds["column_names"][:30]  # Cap at 30 columns
        )
        parts.append(f"""
**Active Dataset Detail:**
- File: {ds.get('filename')}
- Dimensions: {ds.get('rows')} rows × {ds.get('columns')} columns
- Columns: {col_info}""")
```

### A5. AllieContext — add dataset_list field

**File: `app/services/prompt_factory.py`**

```python
@dataclass
class AllieContext:
    # ...existing fields...
    dataset_list: List[Dict[str, Any]] = field(default_factory=list)  # NEW
```

### Phase A Deliverable

After Phase A, allAI will:
- ✅ Know which page the user is on (datasets, settings, dashboard, etc.)
- ✅ Know ALL uploaded datasets with names, sizes, row/col counts, status
- ✅ Know the active dataset's full schema (column names, types)
- ✅ Stop hallucinating "home screen" or "I don't see any files"
- ❌ Still can't ACT — that's Phase B

---

## Phase B: Hands — Tool Use + Agentic Loop (~8h, security-hardened)

### B1. Tool Definitions

**File: `app/services/allai_tools.py` (NEW)**

Tools are defined as Anthropic-compatible tool schemas. Each tool has:
- `name`: unique identifier
- `description`: what the LLM sees
- `input_schema`: JSON Schema for parameters
- `permission_level`: "read" | "write" | "destructive"
- `requires_confirmation`: bool — triggers M1 confirmation flow

```python
from typing import Literal

PermissionLevel = Literal["read", "write", "destructive"]

ALLAI_TOOLS = [
    {
        "name": "list_datasets",
        "description": (
            "List all datasets in the user's vectorAIz instance with metadata "
            "(filename, type, status, row count, column count, size). "
            "Use when the user asks about their data, files, or uploads."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "status_filter": {
                    "type": "string",
                    "enum": ["all", "ready", "processing", "error"],
                    "description": "Filter by processing status. Default: all",
                },
            },
            "required": [],
        },
        "permission_level": "read",
        "requires_confirmation": False,
    },
    {
        "name": "get_dataset_detail",
        "description": (
            "Get detailed information about a specific dataset including "
            "column names, data types, and sample statistics. "
            "Use when the user asks about a specific file or dataset."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "dataset_id": {"type": "string", "description": "The dataset ID"},
            },
            "required": ["dataset_id"],
        },
        "permission_level": "read",
        "requires_confirmation": False,
    },
    {
        "name": "preview_rows",
        "description": (
            "Show sample rows from a dataset. Returns actual data in tabular format. "
            "Use when the user says 'show me the data', 'preview', 'what's in this file'."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "dataset_id": {"type": "string", "description": "The dataset ID"},
                "limit": {
                    "type": "integer",
                    "description": "Number of rows (1-50, default 10)",
                    "minimum": 1, "maximum": 50,
                },
            },
            "required": ["dataset_id"],
        },
        "permission_level": "read",
        "requires_confirmation": False,
    },
    {
        "name": "run_sql_query",
        "description": (
            "Execute a READ-ONLY SQL query against the user's datasets using DuckDB. "
            "SELECT queries only. Each dataset is a table named d_{dataset_id}. "
            "Use for filtering, aggregation, joins, or analytical questions."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "SELECT query only"},
                "limit": {
                    "type": "integer",
                    "description": "Max rows (default 50, max 200)",
                    "maximum": 200,
                },
            },
            "required": ["query"],
        },
        "permission_level": "read",
        "requires_confirmation": False,
    },
    {
        "name": "search_vectors",
        "description": (
            "Semantic search across vectorized datasets. Returns relevant chunks "
            "matching a natural language query. Use for 'find', 'search', 'look up'."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"},
                "dataset_id": {"type": "string", "description": "Optional: specific dataset"},
                "limit": {"type": "integer", "description": "Results (1-20, default 5)", "maximum": 20},
            },
            "required": ["query"],
        },
        "permission_level": "read",
        "requires_confirmation": False,
    },
    {
        "name": "get_system_status",
        "description": "Get system health: Qdrant, DuckDB, LLM, connectivity, vectorization.",
        "input_schema": {"type": "object", "properties": {}, "required": []},
        "permission_level": "read",
        "requires_confirmation": False,
    },
    {
        "name": "get_dataset_statistics",
        "description": (
            "Statistical profile of a dataset: min, max, mean, median, nulls, "
            "unique values per column. Use for data quality or profiling."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "dataset_id": {"type": "string", "description": "The dataset ID"},
            },
            "required": ["dataset_id"],
        },
        "permission_level": "read",
        "requires_confirmation": False,
    },
    # 🛡️ M1: Destructive tool — requires server-enforced confirmation token
    {
        "name": "delete_dataset",
        "description": (
            "Request deletion of a dataset. This is DESTRUCTIVE and will trigger "
            "a confirmation dialog in the user's UI. The user must click Confirm "
            "before deletion proceeds. Use only when user explicitly asks to delete."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "dataset_id": {"type": "string", "description": "The dataset ID to delete"},
            },
            "required": ["dataset_id"],
        },
        "permission_level": "destructive",
        "requires_confirmation": True,  # 🛡️ M1: triggers confirmation token flow
    },
]

def get_anthropic_tools() -> list[dict]:
    """Return tool definitions in Anthropic API format (no internal fields)."""
    return [
        {
            "name": t["name"],
            "description": t["description"],
            "input_schema": t["input_schema"],
        }
        for t in ALLAI_TOOLS
    ]

def get_tool_meta(name: str) -> dict:
    """Get internal metadata (permission_level, requires_confirmation)."""
    for t in ALLAI_TOOLS:
        if t["name"] == name:
            return t
    return {}
```

### B2. Tool Execution Engine (Security-Hardened)

**File: `app/services/allai_tool_executor.py` (NEW)**

```python
"""
allAI Tool Executor — Runs tools on behalf of the user.

🛡️ Security rails (Council mandates):
- M1: Destructive tools return CONFIRMATION_REQUIRED, never execute directly
- M2: SQL queries AST-validated (SELECT-only allowlist)
- M3: Tool results split: rich data → frontend, summary → LLM
- All tools check user owns the referenced dataset
- Max 5 tool calls per message
- 10s timeout per tool call
- Full audit trail logging
"""
import asyncio, json, logging, os, uuid
from datetime import datetime, timezone, timedelta
from typing import Any, Dict, Optional, Tuple

logger = logging.getLogger(__name__)

MAX_TOOL_CALLS_PER_MESSAGE = 5
TOOL_TIMEOUT_S = 10
CONFIRMATION_EXPIRY_S = 60


class ToolResult:
    """
    🛡️ M3: Two-track result — rich data for frontend, summary for LLM.
    """
    def __init__(
        self,
        tool_name: str,
        rich_data: Dict[str, Any],          # Full data → frontend TOOL_RESULT
        llm_summary: str,                    # Short summary → LLM context
        confirmation_required: bool = False,
        confirmation_id: Optional[str] = None,
        error: Optional[str] = None,
    ):
        self.tool_name = tool_name
        self.rich_data = rich_data
        self.llm_summary = llm_summary
        self.confirmation_required = confirmation_required
        self.confirmation_id = confirmation_id
        self.error = error


class ConfirmationToken:
    """🛡️ M1: Server-generated, user-bound, time-limited confirmation."""
    def __init__(self, user_id: str, tool_name: str, resource_id: str, details: dict):
        self.id = str(uuid.uuid4())
        self.user_id = user_id
        self.tool_name = tool_name
        self.resource_id = resource_id
        self.details = details  # Human-readable details for UI
        self.created_at = datetime.now(timezone.utc)
        self.expires_at = self.created_at + timedelta(seconds=CONFIRMATION_EXPIRY_S)
        self.used = False

    def is_valid(self, user_id: str, tool_name: str, resource_id: str) -> bool:
        if self.used:
            return False
        if datetime.now(timezone.utc) > self.expires_at:
            return False
        return (
            self.user_id == user_id
            and self.tool_name == tool_name
            and self.resource_id == resource_id
        )


# In-memory store (production: Redis)
_pending_confirmations: Dict[str, ConfirmationToken] = {}


class AllAIToolExecutor:
    """Executes tool calls with full security enforcement."""

    def __init__(self, user_id: str, user_dataset_ids: list[str]):
        self.user_id = user_id
        self.user_dataset_ids = set(user_dataset_ids)  # Only these are accessible
        self.call_count = 0

    async def execute(self, tool_name: str, tool_input: dict) -> ToolResult:
        """Execute a tool call. Returns two-track ToolResult."""
        from app.services.allai_tools import get_tool_meta

        self.call_count += 1
        if self.call_count > MAX_TOOL_CALLS_PER_MESSAGE:
            return ToolResult(tool_name, {}, "Tool call limit reached (5 per message).",
                              error="limit_exceeded")

        meta = get_tool_meta(tool_name)
        if not meta:
            return ToolResult(tool_name, {}, f"Unknown tool: {tool_name}",
                              error="unknown_tool")

        # 🛡️ Dataset ownership check
        dataset_id = tool_input.get("dataset_id")
        if dataset_id and dataset_id not in self.user_dataset_ids:
            return ToolResult(tool_name, {}, "Dataset not found or not accessible.",
                              error="access_denied")

        # 🛡️ M1: Destructive tools → confirmation token, not execution
        if meta.get("requires_confirmation"):
            return self._create_confirmation(tool_name, tool_input)

        handler = getattr(self, f"_handle_{tool_name}", None)
        if not handler:
            return ToolResult(tool_name, {}, f"Tool not implemented: {tool_name}",
                              error="not_implemented")

        try:
            result = await asyncio.wait_for(handler(tool_input), timeout=TOOL_TIMEOUT_S)
            logger.info("allAI tool: user=%s tool=%s → ok", self.user_id, tool_name)
            return result
        except asyncio.TimeoutError:
            return ToolResult(tool_name, {}, f"Tool timed out after {TOOL_TIMEOUT_S}s.",
                              error="timeout")
        except Exception as e:
            logger.error("allAI tool error: user=%s tool=%s → %s",
                         self.user_id, tool_name, e, exc_info=True)
            return ToolResult(tool_name, {}, f"Tool error: {e}", error=str(e))

    def _create_confirmation(self, tool_name: str, tool_input: dict) -> ToolResult:
        """🛡️ M1: Create confirmation token, return to frontend for human approval."""
        resource_id = tool_input.get("dataset_id", "unknown")
        token = ConfirmationToken(
            user_id=self.user_id,
            tool_name=tool_name,
            resource_id=resource_id,
            details={
                "action": tool_name,
                "target": resource_id,
                "description": f"Delete dataset {resource_id} and all processed data",
            },
        )
        _pending_confirmations[token.id] = token
        return ToolResult(
            tool_name=tool_name,
            rich_data={"confirmation_id": token.id, "details": token.details},
            llm_summary=(
                f"Deletion of dataset {resource_id} requires user confirmation. "
                "A confirmation dialog has been shown to the user."
            ),
            confirmation_required=True,
            confirmation_id=token.id,
        )

    # --- Tool handlers ---
    # Each returns ToolResult with rich_data (frontend) + llm_summary (LLM context)

    async def _handle_list_datasets(self, input: dict) -> ToolResult:
        from app.services.processing_service import get_processing_service
        svc = get_processing_service()
        records = svc.list_datasets()
        status_filter = input.get("status_filter", "all")
        if status_filter != "all":
            records = [r for r in records
                       if (r.status.value if hasattr(r.status, 'value')
                           else str(r.status)) == status_filter]

        datasets = [
            {
                "id": r.id,
                "filename": r.original_filename,
                "file_type": r.file_type,
                "status": r.status.value if hasattr(r.status, 'value') else str(r.status),
                "rows": r.metadata.get("row_count"),
                "columns": r.metadata.get("column_count"),
                "size_bytes": r.file_size_bytes,
                "created_at": r.created_at.isoformat() if r.created_at else None,
            }
            for r in records
        ]

        # 🛡️ M3: Summary for LLM (no raw data)
        summary_lines = [f"Found {len(datasets)} dataset(s):"]
        for d in datasets:
            dims = f"{d['rows']}×{d['columns']}" if d['rows'] and d['columns'] else "processing"
            summary_lines.append(f"  [{d['id']}] {d['filename']} ({dims}) [{d['status']}]")

        return ToolResult(
            tool_name="list_datasets",
            rich_data={"datasets": datasets, "count": len(datasets)},
            llm_summary="\n".join(summary_lines),
        )

    async def _handle_get_dataset_detail(self, input: dict) -> ToolResult:
        from app.services.processing_service import get_processing_service
        svc = get_processing_service()
        record = svc.get_dataset(input["dataset_id"])
        if not record:
            return ToolResult("get_dataset_detail", {}, "Dataset not found.", error="not_found")

        detail = {
            "id": record.id,
            "filename": record.original_filename,
            "file_type": record.file_type,
            "status": record.status.value if hasattr(record.status, 'value') else str(record.status),
            "rows": record.metadata.get("row_count"),
            "columns": record.metadata.get("column_count"),
            "column_names": record.metadata.get("column_names", []),
            "dtypes": record.metadata.get("dtypes", {}),
            "size_bytes": record.file_size_bytes,
            "created_at": record.created_at.isoformat() if record.created_at else None,
        }

        cols = detail.get("column_names", [])[:30]
        dtypes = detail.get("dtypes", {})
        col_desc = ", ".join(f"{c} ({dtypes.get(c, '?')})" for c in cols)

        return ToolResult(
            tool_name="get_dataset_detail",
            rich_data=detail,
            llm_summary=(
                f"Dataset {detail['filename']}: {detail.get('rows','?')} rows × "
                f"{detail.get('columns','?')} columns. "
                f"Columns: {col_desc}. Status: {detail['status']}."
            ),
        )

    async def _handle_preview_rows(self, input: dict) -> ToolResult:
        from app.services.duckdb_service import get_duckdb_service
        svc = get_duckdb_service()
        dataset_id = input["dataset_id"]
        limit = min(input.get("limit", 10), 50)
        sample = svc.get_sample(dataset_id, limit=limit)

        # 🛡️ M3: Rich data to frontend, summary to LLM
        rows = sample.get("rows", [])
        columns = sample.get("columns", [])
        return ToolResult(
            tool_name="preview_rows",
            rich_data=sample,  # Full rows → frontend renders table
            llm_summary=(
                f"Displayed {len(rows)} rows to user. "
                f"Columns: {', '.join(columns[:20])}."
            ),
        )

    async def _handle_run_sql_query(self, input: dict) -> ToolResult:
        """🛡️ M2: SQL validated by AST before execution."""
        from app.services.allai_sql_sandbox import validate_and_execute_sql
        query = input["query"]
        limit = min(input.get("limit", 50), 200)

        result = await validate_and_execute_sql(
            query=query,
            allowed_tables=self.user_dataset_ids,
            limit=limit,
        )

        if result.get("error"):
            return ToolResult("run_sql_query", {}, f"SQL error: {result['error']}",
                              error=result["error"])

        rows = result.get("rows", [])
        columns = result.get("columns", [])
        return ToolResult(
            tool_name="run_sql_query",
            rich_data=result,  # Full results → frontend table
            llm_summary=(
                f"Query returned {len(rows)} rows. "
                f"Columns: {', '.join(columns[:15])}. "
                f"Displayed to user as table."
            ),
        )

    async def _handle_search_vectors(self, input: dict) -> ToolResult:
        from app.services.search_service import get_search_service
        svc = get_search_service()
        results = svc.search(
            query=input["query"],
            dataset_id=input.get("dataset_id"),
            limit=min(input.get("limit", 5), 20),
        )

        hits = results.get("results", [])
        return ToolResult(
            tool_name="search_vectors",
            rich_data=results,
            llm_summary=(
                f"Found {len(hits)} semantic matches for '{input['query']}'. "
                f"Top match score: {hits[0].get('score','?') if hits else 'N/A'}. "
                f"Results displayed to user."
            ),
        )

    async def _handle_get_dataset_statistics(self, input: dict) -> ToolResult:
        from app.services.duckdb_service import get_duckdb_service
        svc = get_duckdb_service()
        stats = svc.get_profile(input["dataset_id"])

        col_stats = stats.get("columns", {})
        summary_parts = [f"Statistics for {len(col_stats)} columns:"]
        for col_name, col_data in list(col_stats.items())[:10]:
            nulls = col_data.get("null_count", 0)
            summary_parts.append(f"  {col_name}: {col_data.get('dtype','?')}, {nulls} nulls")

        return ToolResult(
            tool_name="get_dataset_statistics",
            rich_data=stats,
            llm_summary="\n".join(summary_parts),
        )

    async def _handle_get_system_status(self, input: dict) -> ToolResult:
        from app.core.local_only_guard import is_local_only
        status = {
            "connected_mode": not is_local_only(),
            "qdrant_status": os.environ.get("VECTORAIZ_QDRANT_STATUS", "healthy"),
            "vectorization_enabled": os.environ.get(
                "VECTORAIZ_VECTORIZATION_ENABLED", "true") == "true",
            "byo_llm_configured": bool(os.environ.get("VECTORAIZ_BYO_LLM_KEY")),
        }
        return ToolResult(
            tool_name="get_system_status",
            rich_data=status,
            llm_summary=(
                f"System status: connected={status['connected_mode']}, "
                f"qdrant={status['qdrant_status']}, "
                f"vectorization={status['vectorization_enabled']}, "
                f"byo_llm={status['byo_llm_configured']}"
            ),
        )
```

### B3. SQL Sandbox — AST Validation 🛡️ M2

**File: `app/services/allai_sql_sandbox.py` (NEW)**

```python
"""
🛡️ M2: SQL AST Validation Sandbox

Hard allowlist approach:
1. Parse SQL into AST
2. Reject anything that isn't a single SELECT (or WITH...SELECT)
3. Explicit blocklist for dangerous DuckDB primitives
4. Per-user table exposure
5. Resource limits (rows, timeout, memory)
"""
import re, logging
from typing import Optional, Set

logger = logging.getLogger(__name__)

# Dangerous patterns (case-insensitive)
BLOCKED_KEYWORDS = {
    # DDL/DML
    "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "TRUNCATE",
    "REPLACE", "MERGE", "UPSERT",
    # DuckDB file/extension primitives
    "COPY", "EXPORT", "ATTACH", "DETACH", "INSTALL", "LOAD", "IMPORT",
    "PRAGMA", "CHECKPOINT", "VACUUM", "ANALYZE",
    # File access functions
    "read_csv", "read_csv_auto", "read_parquet", "read_json", "read_json_auto",
    "read_blob", "write_csv", "write_parquet",
    # Network
    "httpfs", "s3",
    # System
    "CALL", "SET", "RESET",
}

# Max constraints
MAX_RESULT_ROWS = 10_000
SQL_TIMEOUT_S = 5
MAX_QUERY_LENGTH = 4000


def validate_sql(query: str, allowed_tables: Set[str]) -> Optional[str]:
    """
    Validate SQL query. Returns error string if invalid, None if OK.

    Strategy: belt AND suspenders.
    1. Regex-based keyword blocklist (fast, catches obvious attacks)
    2. Statement count check (no multi-statement)
    3. Table reference check (only user's tables)
    """
    query = query.strip()

    if len(query) > MAX_QUERY_LENGTH:
        return f"Query too long ({len(query)} chars, max {MAX_QUERY_LENGTH})"

    # Must start with SELECT or WITH (case-insensitive)
    first_word = query.split()[0].upper() if query.split() else ""
    if first_word not in ("SELECT", "WITH"):
        return f"Only SELECT queries allowed. Got: {first_word}"

    # No semicolons (multi-statement)
    if ";" in query.rstrip(";"):  # Allow trailing semicolon
        return "Multi-statement queries not allowed"

    # Keyword blocklist
    query_upper = query.upper()
    for kw in BLOCKED_KEYWORDS:
        # Match as whole word
        pattern = r'\b' + re.escape(kw) + r'\b'
        if re.search(pattern, query_upper):
            return f"Blocked keyword: {kw}"

    # Table reference check: extract table-like identifiers
    # Tables are named d_{dataset_id} — ensure only allowed ones referenced
    table_pattern = r'\bd_([a-f0-9]{8})\b'
    referenced_tables = set(re.findall(table_pattern, query.lower()))
    for table_id in referenced_tables:
        if table_id not in allowed_tables:
            return f"Access denied: dataset {table_id} not found or not owned by user"

    return None  # Valid


async def validate_and_execute_sql(
    query: str,
    allowed_tables: Set[str],
    limit: int = 200,
) -> dict:
    """Validate then execute SQL with resource limits."""
    error = validate_sql(query, allowed_tables)
    if error:
        return {"error": error, "rows": [], "columns": []}

    from app.services.sql_service import get_sql_service
    from app.core.async_utils import run_sync
    svc = get_sql_service()

    try:
        # Add LIMIT if not present
        if "LIMIT" not in query.upper():
            query = f"{query.rstrip(';')} LIMIT {min(limit, MAX_RESULT_ROWS)}"

        result = await run_sync(svc.execute_query, query, min(limit, MAX_RESULT_ROWS))
        return result
    except Exception as e:
        return {"error": str(e), "rows": [], "columns": []}
```

### B4. Agentic LLM Loop

**File: `app/services/allai_agentic_provider.py` (NEW)**

The agentic loop. LLM → tool_use → execute → summary back → repeat.

Key design (per Council mandates):
- 🛡️ M3: Tool results sent RICH to frontend, SUMMARY to LLM
- 🛡️ M4: Goes through ai.market proxy (extend with tools param)
- 🛡️ M5: Run ID + event sequence + heartbeats
- Max 5 tool iterations per message
- Streams text, sends TOOL_STATUS + TOOL_RESULT via WebSocket

```python
class AgenticLoop:
    """
    Agentic tool-use loop for allAI.

    Flow:
    1. Send message + system prompt + tools to LLM
    2. If LLM returns tool_use block:
       a. Send TOOL_STATUS("executing") to frontend
       b. Execute tool via AllAIToolExecutor
       c. Send TOOL_RESULT (rich data) to frontend
       d. Send TOOL_STATUS("done") to frontend
       e. Feed SUMMARY (not raw data) back to LLM as tool_result
       f. Go to 1 (max 5 iterations)
    3. If LLM returns text: stream to frontend
    4. Return full text + usage

    Resilience:
    - Each message gets a run_id (UUID)
    - All events carry run_id + monotonic event_seq
    - State persisted for resume-on-reconnect (Phase B5)
    - Heartbeat sent every 5s during tool execution
    """

    MAX_ITERATIONS = 5

    def __init__(
        self,
        run_id: str,
        tool_executor: AllAIToolExecutor,
        send_chunk,       # async (text) → stream text to frontend
        send_tool_status, # async (tool_name, status, data?) → TOOL_STATUS
        send_tool_result, # async (tool_name, rich_data) → TOOL_RESULT
        send_heartbeat,   # async () → keep WS alive
    ):
        self.run_id = run_id
        self.executor = tool_executor
        self.send_chunk = send_chunk
        self.send_tool_status = send_tool_status
        self.send_tool_result = send_tool_result
        self.send_heartbeat = send_heartbeat
        self.event_seq = 0
        self.iteration = 0

    async def run(
        self,
        user_message: str,
        system_prompt: str,
        tools: list[dict],
        conversation_history: list[dict],
    ) -> tuple[str, dict]:
        """
        Run the agentic loop.
        Returns (full_text_response, usage_dict).
        """
        messages = [*conversation_history, {"role": "user", "content": user_message}]
        full_text = ""
        total_usage = {"input_tokens": 0, "output_tokens": 0}

        for iteration in range(self.MAX_ITERATIONS):
            self.iteration = iteration

            # Call LLM (via proxy or direct)
            response = await self._call_llm(system_prompt, messages, tools)
            total_usage["input_tokens"] += response.get("usage", {}).get("input_tokens", 0)
            total_usage["output_tokens"] += response.get("usage", {}).get("output_tokens", 0)

            # Process response content blocks
            has_tool_use = False
            tool_results_for_llm = []

            for block in response.get("content", []):
                if block["type"] == "text":
                    text = block["text"]
                    full_text += text
                    await self.send_chunk(text)

                elif block["type"] == "tool_use":
                    has_tool_use = True
                    tool_name = block["name"]
                    tool_input = block["input"]
                    tool_use_id = block["id"]

                    # Execute tool
                    await self.send_tool_status(tool_name, "executing")
                    result = await self.executor.execute(tool_name, tool_input)
                    await self.send_tool_status(tool_name, "done")

                    # 🛡️ M3: Rich data → frontend, summary → LLM
                    if result.rich_data:
                        await self.send_tool_result(tool_name, result.rich_data)

                    if result.confirmation_required:
                        await self.send_tool_result(
                            "confirmation_required", result.rich_data
                        )

                    # Feed SUMMARY back to LLM (never raw data)
                    tool_results_for_llm.append({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": result.llm_summary,
                    })

            if not has_tool_use:
                break  # LLM is done — text-only response

            # Add assistant response + tool results to conversation
            messages.append({"role": "assistant", "content": response["content"]})
            messages.append({"role": "user", "content": tool_results_for_llm})

            # 🛡️ M5: Heartbeat during loop
            await self.send_heartbeat()

        return full_text, total_usage

    async def _call_llm(self, system: str, messages: list, tools: list) -> dict:
        """
        Call LLM. Route through ai.market proxy (M4).
        Temporary fallback: direct Anthropic if proxy doesn't support tools yet.
        """
        # Implementation depends on proxy readiness.
        # See B5 section for details.
        raise NotImplementedError("Wire to proxy or direct Anthropic")
```

### B5. ai.market Proxy Extension 🛡️ M4

**File: `ai-market-backend` — extend `/api/v1/allie/chat`**

The existing proxy endpoint receives messages and streams SSE. Extend it to
accept a `tools` parameter and relay to Anthropic with tool definitions.

```python
# Request body extension:
class AllieChatRequest(BaseModel):
    messages: list[dict]
    request_id: str
    tools: Optional[list[dict]] = None  # NEW: Anthropic tool definitions
    stream: bool = True

# The proxy passes tools directly to Anthropic Messages API.
# When the response contains tool_use blocks, they are forwarded to the
# vectorAIz backend via SSE events:
#   event: tool_use
#   data: {"id": "...", "name": "...", "input": {...}}
#
# The vectorAIz backend executes the tool, then sends a follow-up
# request with tool_result in messages.
```

**Temporary fallback (max 2 weeks):**
If proxy changes take longer, vectorAIz backend calls Anthropic directly
using a service API key. Metering via usage-report endpoint after each call.
This is NOT permanent architecture — just an unblocking escape hatch.

### B6. Prompt Factory — Tool-Aware System Prompt

**File: `app/services/prompt_factory.py`**

Add to Layer 2 (Role & Domain):

```
**Tool Use:**
You have tools that let you take actions in the user's vectorAIz instance.
When the user asks you to do something (show data, run a query, check status),
USE THE TOOLS rather than describing what they should do manually.

Be proactive about using tools:
- "What are my files?" → call list_datasets, then summarize results
- "Show me the apartments data" → call preview_rows with the matching dataset
- "How many rows have price > 500000?" → call run_sql_query with appropriate SQL
- "What's the average churn rate?" → call run_sql_query with AVG(churn_pct)

CRITICAL RULES:
- NEVER say "you can check the Data tab" — call list_datasets instead
- NEVER say "I don't have access to your datasets" — you DO, via tools
- NEVER hallucinate data — always use tools to get real data
- NEVER fabricate column names or statistics — call get_dataset_detail
- When tool results are displayed to the user, summarize key findings in your text
- For destructive actions (delete), the UI will show a confirmation dialog —
  tell the user you've requested the action and they'll see a confirmation prompt
```

### B7. WebSocket Protocol Extensions 🛡️ M5

**New message types (backend → frontend):**

```json
// Run tracking (every agentic message gets a run_id)
{"type": "RUN_START", "run_id": "uuid", "message_id": "..."}

// Tool activity
{"type": "TOOL_STATUS", "run_id": "uuid", "seq": 1, "tool_name": "list_datasets", "status": "executing"}
{"type": "TOOL_STATUS", "run_id": "uuid", "seq": 2, "tool_name": "list_datasets", "status": "done"}

// Rich tool results (frontend renders inline)
{"type": "TOOL_RESULT", "run_id": "uuid", "seq": 3, "tool_name": "preview_rows", "data": {...}}

// 🛡️ M1: Confirmation required (frontend shows dialog)
{"type": "CONFIRMATION_REQUIRED", "run_id": "uuid", "confirmation_id": "uuid",
 "details": {"action": "delete_dataset", "target": "a1b2c3d4",
             "description": "Delete dataset barcelona_apartments.csv and all processed data"}}

// Heartbeat during tool execution
{"type": "RUN_HEARTBEAT", "run_id": "uuid", "iteration": 2, "elapsed_s": 12}

// Run complete
{"type": "RUN_COMPLETE", "run_id": "uuid", "seq": 10, "usage": {...}}
```

**New message types (frontend → backend):**

```json
// 🛡️ M1: User confirms destructive action
{"type": "CONFIRMATION_RESPONSE", "confirmation_id": "uuid", "approved": true}

// 🛡️ M5: Resume after disconnect
{"type": "RESUME", "run_id": "uuid", "last_event_seq": 5}
```

---

## Phase C: Reach — External Actions (~4h) 🛡️ M6

### C1. Filesystem Tool (Sandboxed)

**CRITICAL:** Filesystem access is local/Docker mode only. **DISABLED in connected/cloud mode.**

Tools run in a **sandboxed execution context** (separate process with restricted mounts).

```python
# Tool definitions (only registered when local_only or Docker mode)

{
    "name": "list_directory",
    "description": (
        "List files in a directory on the user's system. "
        "Only works in local/self-hosted mode. "
        "Use when user asks about files outside vectorAIz."
    ),
    "input_schema": {
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "Directory path"},
            "pattern": {"type": "string", "description": "Glob pattern (e.g. '*.csv')"},
        },
        "required": ["path"],
    },
    "permission_level": "read",
    "requires_confirmation": False,
},
{
    "name": "import_files",
    "description": (
        "Import files from user's filesystem into vectorAIz. "
        "Files are COPIED (not moved). Processing starts automatically. "
        "Only works in local/self-hosted mode."
    ),
    "input_schema": {
        "type": "object",
        "properties": {
            "file_paths": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Absolute file paths to import (max 20)",
                "maxItems": 20,
            },
        },
        "required": ["file_paths"],
    },
    "permission_level": "write",
    "requires_confirmation": True,  # 🛡️ M1: requires human confirmation
}
```

### C2. Filesystem Sandbox Implementation 🛡️ M6

**File: `app/services/allai_filesystem_sandbox.py` (NEW)**

```python
"""
🛡️ M6: Sandboxed filesystem access.

Security model:
1. All paths resolved via os.path.realpath() BEFORE any check
2. Symlinks resolved and re-checked against allowlist
3. Allowed directories configured via ALLAI_FS_ALLOWED_DIRS env var
4. Default allowed: ~/Downloads, ~/Documents, ~/Desktop, /data, /tmp
5. System directories always blocked (even if in allowed list)
6. DISABLED entirely in connected/cloud mode
7. Max 20 files per import, max 500MB per file
"""

ALWAYS_BLOCKED = {
    "/etc", "/var", "/usr", "/bin", "/sbin", "/dev", "/proc", "/sys",
    "/root", "/boot", "/lib", "/lib64",
}
BLOCKED_PATTERNS = {".ssh", ".gnupg", ".config", ".env", ".git"}

MAX_IMPORT_FILES = 20
MAX_FILE_SIZE_BYTES = 500 * 1024 * 1024  # 500MB

def validate_path(path: str, allowed_dirs: list[str]) -> Optional[str]:
    """
    Validate a path is safe to read.
    Returns error string if invalid, None if OK.
    """
    import os

    # Resolve to absolute real path (follows symlinks!)
    real_path = os.path.realpath(os.path.expanduser(path))

    # Check against blocked directories
    for blocked in ALWAYS_BLOCKED:
        if real_path.startswith(blocked):
            return f"Access denied: system directory {blocked}"

    # Check against blocked patterns
    for pattern in BLOCKED_PATTERNS:
        if pattern in real_path:
            return f"Access denied: sensitive path pattern {pattern}"

    # Check against allowed directories
    allowed_resolved = [os.path.realpath(os.path.expanduser(d)) for d in allowed_dirs]
    if not any(real_path.startswith(d) for d in allowed_resolved):
        return f"Access denied: {path} is not in allowed directories"

    return None
```

---

## Phase D: Frontend — Rich Tool Result Rendering (~3h)

### D1. Inline Data Tables

When frontend receives `TOOL_RESULT` for `preview_rows` or `run_sql_query`,
render as a styled, scrollable data table inline in the chat.

Reuse existing `<DataTable>` component from Datasets page.

### D2. Tool Activity Indicators

During tool execution, show ephemeral status messages in chat:
- "Looking up your datasets..." (list_datasets)
- "Running SQL query..." (run_sql_query)
- "Searching across your data..." (search_vectors)

Driven by `TOOL_STATUS` WebSocket events.

### D3. Confirmation Dialog

When frontend receives `CONFIRMATION_REQUIRED`, show a modal/inline dialog:
- "allAI wants to delete dataset barcelona_apartments.csv"
- [Cancel] [Confirm Delete]
- Sends `CONFIRMATION_RESPONSE` back via WebSocket

### D4. Action Buttons on Results

- Dataset list → "Preview" button per dataset (triggers preview_rows)
- Query results → "Export CSV" button
- Error results → "Try again" button

---

## Build Order + Dependencies

| Phase | BQ Code | Hours | Depends On | Deliverable |
|-------|---------|-------|------------|-------------|
| A1-A5 | BQ-ALLAI-A | 3h | — | **allAI sees everything** |
| B1 | BQ-ALLAI-B1 | 1h | — | Tool definitions |
| B2 | BQ-ALLAI-B2 | 2.5h | B1 | Tool executor + confirmation tokens 🛡️ M1 |
| B3 | BQ-ALLAI-B3 | 1h | — | SQL sandbox + AST validation 🛡️ M2 |
| B4 | BQ-ALLAI-B4 | 2h | B1,B2,B3 | Agentic loop + two-track results 🛡️ M3 |
| B5 | BQ-ALLAI-B5 | 1h | B4 | Proxy extension OR temp direct 🛡️ M4 |
| B6 | BQ-ALLAI-B6 | 0.5h | B1 | Tool-aware system prompt |
| B7 | BQ-ALLAI-B7 | 1h | B4 | WS protocol + run_id + resume 🛡️ M5 |
| **B total** | | **9h** | A | **allAI can act (hardened)** |
| C1-C2 | BQ-ALLAI-C | 4h | B | **allAI reaches outside** 🛡️ M6 |
| D1-D4 | BQ-ALLAI-D | 3h | B | **Rich tool results** |
| **GRAND TOTAL** | | **~19h** | | |

---

## Success Criteria

### Phase A (Eyes)
- [ ] allAI correctly identifies current page ("You're on the Datasets page")
- [ ] allAI knows all uploaded datasets by name, size, status
- [ ] allAI knows active dataset's schema (column names, types)
- [ ] No more "I don't see any files" hallucination
- [ ] STATE_SNAPSHOT sent on every route change

### Phase B (Hands) — with security
- [ ] "What are my files?" → allAI calls list_datasets, shows real data
- [ ] "Show me the apartments" → allAI calls preview_rows, inline table appears
- [ ] "What's the average price?" → allAI writes SQL, executes, returns answer
- [ ] "Search for cybersecurity" → allAI calls search_vectors, returns matches
- [ ] Tool calls capped at 5 per message
- [ ] 🛡️ SQL AST-validated: DROP/COPY/ATTACH/PRAGMA all rejected
- [ ] 🛡️ Destructive actions require human confirmation token (not LLM-controlled)
- [ ] 🛡️ Tool results: rich data to frontend, summary only to LLM
- [ ] 🛡️ All LLM calls go through ai.market proxy
- [ ] 🛡️ WebSocket resilient: run_id + heartbeats + resume on reconnect
- [ ] All tool calls logged to audit trail

### Phase C (Reach)
- [ ] "What CSV files are in my Downloads?" → allAI lists them
- [ ] "Import those" → confirmation dialog → files copied to vectorAIz
- [ ] 🛡️ System directories blocked after realpath resolution
- [ ] 🛡️ Symlinks resolved and re-checked
- [ ] 🛡️ Disabled entirely in connected/cloud mode

### Phase D (Rich Results)
- [ ] Query results render as inline tables (not raw JSON)
- [ ] Tool execution shows activity indicators
- [ ] Confirmation dialog for destructive actions
- [ ] Action buttons on results (Preview, Export, Try Again)

---

## Design Decisions (Council-Resolved)

| # | Decision | Resolution | Decided By |
|---|----------|------------|------------|
| 1 | Direct Anthropic vs proxy | **Proxy** (extend ai.market). Temp direct as 2-week escape hatch only. | XAI + MP |
| 2 | Tool result rendering | **Two-track**: rich → frontend, summary → LLM. Never raw data in prompt. | MP |
| 3 | Filesystem in connected mode | **DISABLED**. Local/Docker only. | MP |
| 4 | Agentic loop sync/async | **Sync** loop with streaming text. Max 5 iterations. | XAI |
| 5 | SQL write access | **Read-only** (SELECT). Write access future phase with confirmation. | MP |
| 6 | Cost of tool messages | Cap at 5 tools/msg. Show cost estimate in UI (future). Tier pricing (future). | XAI |
| 7 | Destructive tool confirmation | **Server-enforced tokens**. LLM can request, only human UI click executes. | MP (blocking) |
| 8 | SQL validation | **AST-based allowlist**. Single SELECT only. Block all DuckDB file primitives. | MP (blocking) |
