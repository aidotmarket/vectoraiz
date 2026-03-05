# BQ-VZ-ARTIFACTS — allAI Artifacts (Generated Output Files)

**Version:** 1.1 (Gate 1 mandates resolved)  
**Priority:** P1  
**Origin:** S219 — allAI hallucinated creating a file for user; revealed both a prompt gap and a real feature need  
**Estimated hours:** 20-28  
**Council:** Required  
**Gate 1:** PASS_WITH_MANDATES (3/3 AG+MP+XAI, S219)  
**Gate 2:** PENDING

---

## Problem Statement

allAI can search, query, and analyze user data — but it can only show results in the chat window. When a user asks "extract all mentions of Russia from War and Peace and save them to a file," allAI has no tool to do this. Today it either hallucinates file creation (telling the user to check a non-existent tab) or refuses.

This is a critical capability gap in two contexts:

1. **VZ local mode:** A single user processing their own data wants tangible outputs — extracts, filtered datasets, reports, query results as downloadable files.

2. **Corporate webserver mode:** When allAI serves as the AI-powered browsing interface for company data, employees need to get outputs from their queries — not just see results scroll by in chat. An analyst asking "pull all Q4 transactions over $100k and export as CSV" needs a real file.

## Design Principle

**Artifacts are allAI's tangible outputs.** Datasets are what the user puts IN. Artifacts are what allAI produces OUT. They are first-class objects with their own storage, UI section, and lifecycle.

---

## Gate 1 Mandates (Resolved)

### BLOCKING — Resolved

| # | Mandate | Source | Resolution |
|---|---|---|---|
| M1 | Path traversal prevention — filename is display-only | AG, MP, XAI (unanimous) | Store as `{artifact_id}/content.{ext}`. User-provided filename stored in metadata only, never used in filesystem paths. Sanitize with `os.path.basename()`, regex whitelist `[a-zA-Z0-9._-]`, reject path separators/`..`/control chars/NUL, normalize Unicode. |
| M2 | AuthZ scoping from Phase 1 | MP, XAI | Every endpoint enforces `user_id`. Single-user mode uses a deterministic default identity (`"local"`), not null. Multi-user reads from OIDC claim. No "null means global" path. |
| M3 | Quota + disk exhaustion controls | MP, XAI | Enforce 50MB/file, 100 count/user, rate limit 5 creates/min server-side before write. Fail closed with clear HTTP 429/413 errors. |

### REQUIRED — Resolved

| # | Mandate | Source | Resolution |
|---|---|---|---|
| M4 | HTML XSS mitigation | AG, XAI | Download served with `Content-Disposition: attachment`. Frontend preview uses `<iframe sandbox="">` (no allow-scripts, no allow-same-origin). |
| M5 | Atomic writes | MP | Write content to temp file, rename on success. Measure `size_bytes` from actual written file. On partial failure (content written, metadata failed), cleanup orphan content file. |
| M6 | `create_artifact_from_query` in Phase 1 | MP, XAI | Included in Phase 1. Streams SQL results directly to file, bypassing LLM output token limits. |
| M7 | Content validation per format | XAI | UTF-8 only, reject embedded NUL bytes. HTML sanitized pre-write (strip `<script>`, external resources). CSV treated as data-only (no formula execution in preview). |
| M8 | Metadata storage: JSON sidecar with schema_version | AG wanted DuckDB; MP+XAI+Vulcan chose JSON | JSON sidecar for v1 with `schema_version: 1` field for future migration. 100 max artifacts/user makes directory scan viable. |

---

## Architecture

### Artifact Model

```
Artifact:
  id:             UUID
  schema_version: int             # always 1 for v1; enables future migration
  filename:       string          # display-only (e.g. "russia-references.txt")
  format:         enum            # txt, csv, json, md, html
  size_bytes:     int             # measured from actual written file, not input length
  content_hash:   string          # SHA-256 of content (dedup/integrity)
  created_at:     datetime
  source:         string          # "allai-copilot" | "allai-query" | "allai-agent" | "manual"
  source_ref:     string|null     # conversation ID or agent task ID
  description:    string|null     # allAI's summary of what this artifact contains
  dataset_refs:   list[UUID]      # which datasets were used to create this
  user_id:        string          # "local" in single-user; OIDC sub in multi-user (NEVER null)
  starred:        bool            # starred = persistent, unstarred = subject to cleanup
  expired:        bool            # soft-delete after TTL
```

### Storage

```
{data_dir}/
├── datasets/               # existing — user uploads
├── artifacts/               # NEW — allAI outputs
│   ├── {artifact_id}/
│   │   ├── metadata.json    # Artifact model as JSON (schema_version field)
│   │   └── content.{ext}   # actual file (ext from format enum, NOT user input)
│   └── ...
```

**Critical security invariant:** The user-provided `filename` is NEVER used in filesystem paths. Storage always uses `content.{ext}` where `ext` comes from the validated `format` enum. The `filename` field is only used for `Content-Disposition` headers on download.

In multi-user/corporate mode, artifacts are scoped by `user_id` from auth context (SSO/OIDC claim). Every list/get/download/delete enforces `artifact.user_id == caller.user_id`.

### Size & Lifecycle Limits

| Constraint | Value | Rationale |
|---|---|---|
| Max artifact size | 50 MB | Generous for text/CSV exports; prevents abuse |
| Max artifacts per user | 100 | Prevents disk bloat in corporate mode |
| Rate limit | 5 creates/min/user | Prevents churn attacks |
| Auto-cleanup TTL | 7 days (unstarred) | Ephemeral by default |
| Starred artifacts | No TTL | User explicitly chose to keep |
| Max filename length | 255 chars | Filesystem safe |
| Allowed formats | txt, csv, json, md, html | No binary formats in v1 |
| Filename charset | `[a-zA-Z0-9._-]` only | Reject path separators, `..`, control chars, NUL |

---

## Backend

### New Service: `app/services/artifacts_service.py`

```python
class ArtifactsService:
    async def create_artifact(filename, content, format, description, dataset_refs, user_id) -> Artifact
    async def create_artifact_from_query(filename, query, format, description, user_id) -> Artifact
    async def list_artifacts(user_id, include_expired=False, offset=0, limit=50) -> list[Artifact]
    async def get_artifact(artifact_id, user_id) -> Artifact
    async def download_artifact(artifact_id, user_id) -> FileResponse
    async def delete_artifact(artifact_id, user_id) -> bool
    async def star_artifact(artifact_id, user_id, starred: bool) -> Artifact
    async def cleanup_expired() -> int  # called by scheduled task
```

**Write flow (atomic):**
1. Validate filename (sanitize, check charset/length)
2. Validate content (UTF-8, no NUL, format-specific validation)
3. Check quotas (count, rate limit)
4. Generate UUID, create temp dir
5. Write content to temp file
6. Compute SHA-256 hash, measure actual size
7. Write metadata.json to temp dir
8. Atomic rename temp dir → `artifacts/{artifact_id}/`
9. Return Artifact

**Query-to-file flow (`create_artifact_from_query`):**
1. Validate SQL (SELECT only, same as existing `run_sql_query` tool)
2. Execute via DuckDB, stream results
3. Write directly to temp file (CSV format with header row)
4. Same atomic finalization as above

Storage is filesystem-based (consistent with datasets). Metadata in JSON sidecar with `schema_version: 1`.

### New Router: `app/routers/artifacts.py`

```
GET    /api/artifacts                — list artifacts (pagination: offset+limit, sort: created_at desc, format filter)
GET    /api/artifacts/{id}           — get artifact metadata
GET    /api/artifacts/{id}/download  — download file (Content-Disposition: attachment; filename="<display_name>")
DELETE /api/artifacts/{id}           — delete artifact
PATCH  /api/artifacts/{id}/star      — toggle star
```

All endpoints enforce `user_id` scoping. Download sets correct MIME type from `format` enum (not user-supplied extension).

### New allAI Tools

**Tool 1: `create_artifact`** — for small/curated LLM-generated output:

```python
{
    "name": "create_artifact",
    "description": (
        "Create an output file (artifact) from analysis results, data extracts, "
        "query results, or generated reports. The file is saved to the user's "
        "Artifacts section where they can view, download, or share it. "
        "Use when the user asks to: export data, create a file, save results, "
        "generate a report, extract and save, or write output. "
        "For large data exports (more than ~100 rows), prefer create_artifact_from_query."
    ),
    "input_schema": {
        "type": "object",
        "properties": {
            "filename": {
                "type": "string",
                "description": "Display filename with extension (e.g. 'russia-references.txt', 'q4-summary.csv')"
            },
            "content": {
                "type": "string",
                "description": "The file content to write. For CSV: include header row. For JSON: valid JSON string. For HTML: complete HTML."
            },
            "format": {
                "type": "string",
                "enum": ["txt", "csv", "json", "md", "html"],
                "description": "File format. Must match filename extension."
            },
            "description": {
                "type": "string",
                "description": "Brief description of what this artifact contains and how it was generated."
            },
            "dataset_refs": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Dataset IDs that were used to generate this artifact (for provenance tracking)."
            }
        },
        "required": ["filename", "content", "format", "description"]
    }
}
```

**Tool 2: `create_artifact_from_query`** — for large SQL-based exports:

```python
{
    "name": "create_artifact_from_query",
    "description": (
        "Create a CSV artifact by running a SQL query and saving the results directly to a file. "
        "Use this for large data exports that would exceed chat display limits — e.g. "
        "'export all transactions from Q4', 'save all rows where price > 500'. "
        "Results are streamed directly to file, bypassing output size limits. "
        "Only SELECT queries are allowed. Tables are named dataset_{dataset_id}."
    ),
    "input_schema": {
        "type": "object",
        "properties": {
            "filename": {
                "type": "string",
                "description": "Display filename (e.g. 'q4-transactions.csv')"
            },
            "query": {
                "type": "string",
                "description": "SQL SELECT query. Tables: dataset_{dataset_id}"
            },
            "description": {
                "type": "string",
                "description": "Brief description of what this export contains."
            }
        },
        "required": ["filename", "query", "description"]
    }
}
```

### Prompt Factory Update

Remove the "no phantom file creation" rule (Layer 1 rule 9) added as stopgap in S219. Replace with tool-aware guidance in Layer 2:

```
When the user asks to create, export, or save output:
- Small/curated output (text, summaries, <100 rows): use create_artifact
- Large data exports (full tables, filtered datasets, >100 rows): use create_artifact_from_query
Tell the user: "I've created [filename] — you'll find it in your Artifacts section."
```

---

## Frontend

### Sidebar Addition

New nav item in `topNavItems` (between "SQL Query" and "Databases"):

```tsx
{ path: "/artifacts", label: "Artifacts", icon: FileOutput }
```

Icon: `FileOutput` from lucide-react (or `Package` / `FileBox` if not available).

### Artifacts Page: `/artifacts`

**Layout:** Table of artifacts, paginated (default 50), sorted by created_at desc, with columns:
- Filename (clickable → detail/preview)
- Format (badge: TXT, CSV, JSON, MD, HTML)
- Size
- Created (relative time: "2 hours ago")
- Source (e.g. "allAI Copilot", "SQL Export")
- Star toggle
- Actions: Download, Delete

**Empty state:** "No artifacts yet. Ask allAI to create an export, extract, or report from your data."

**Preview (inline on click):**
- TXT/MD: rendered text (MD sanitized to prevent XSS)
- CSV: table view (reuse existing table component; no formula execution)
- JSON: syntax-highlighted JSON
- HTML: rendered in `<iframe sandbox="">` (no allow-scripts, no allow-same-origin)

### Chat Integration

When allAI creates an artifact via either tool, the chat shows an inline card:

```
┌──────────────────────────────────┐
│ 📄 russia-references.txt         │
│ 20 passages • 4.2 KB • TXT      │
│ [Download]  [View in Artifacts]  │
└──────────────────────────────────┘
```

New message type in the chat renderer: `artifact_card`, rendered when tool result comes back from `create_artifact` or `create_artifact_from_query`. Tool must return `artifact_id` so the card can link to the right artifact.

---

## Corporate Webserver Mode Considerations

When VZ runs as a corporate data server with allAI as the browsing interface:

1. **User scoping is mandatory from Phase 1.** Every artifact has `user_id` (never null). Single-user = `"local"`. Multi-user = OIDC `sub` claim.
2. **Admin visibility (Phase 2).** Admins can see all artifacts for audit/compliance.
3. **Shared artifacts (Phase 2).** Allow users to share an artifact link with colleagues.
4. **Quota management (Phase 2).** Per-user artifact count/size limits configurable in settings.
5. **Cleanup job.** Scheduled task runs daily, removes unstarred artifacts older than TTL. Respects per-user settings in corporate mode.

---

## Phases

### Phase 1 — Core (this BQ, 20-28h)
- `ArtifactsService` with filesystem storage + atomic writes
- `create_artifact` allAI tool + tool executor handler
- `create_artifact_from_query` allAI tool (SQL → CSV direct to file)
- API endpoints (list with pagination, get, download, delete, star)
- Path traversal prevention (filename display-only, `content.{ext}` storage)
- Content validation (UTF-8, no NUL, HTML sanitization)
- Quota enforcement (50MB/file, 100/user, 5/min rate limit)
- User scoping (`"local"` default, OIDC-ready)
- Frontend Artifacts page (table + preview with sandbox)
- Chat artifact card rendering
- Cleanup scheduled task
- Update prompt factory (remove stopgap, add tool guidance)

### Phase 2 — Corporate Enhancements (future BQ)
- Admin audit view for all artifacts
- Shared artifact links
- Configurable quotas per user/group
- Artifact search (by content, description, dataset)
- S3/object store shim for large corporate deployments

### Phase 3 — Rich Artifacts (future BQ)
- Multi-file artifacts (ZIP bundles)
- Chart/visualization artifacts (allAI generates chart as HTML/SVG)
- Template-based reports (allAI fills template with data)
- Artifact versioning (re-run same extract, get updated version)
- Dataset lineage graphs (artifact → source datasets → provenance)

---

## Dependencies

- **Existing:** DuckDB (for query-to-artifact), filesystem storage, allAI agentic loop
- **New:** None. Pure feature addition on existing infrastructure.
- **Blocks:** Nothing. Independent feature.
- **Blocked by:** Nothing. Can build immediately after Gate 2.

---

## Success Criteria

1. User asks allAI to "create a file with X" → allAI calls `create_artifact` → file appears in Artifacts tab → user can download it
2. User asks "export all rows where X" → allAI calls `create_artifact_from_query` → large CSV saved directly → user downloads
3. No more hallucinated file creation — allAI uses real tools or explains limitation
4. Artifacts page shows all generated files with preview and download
5. Cleanup job removes expired artifacts without user intervention
6. Path traversal, XSS, and disk exhaustion attacks are blocked
7. In corporate mode: artifacts are user-scoped and isolated
