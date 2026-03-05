# BQ-VZ-NOTIFICATIONS: Notification System + Upload Resilience + allAI Diagnostic Access

**Created:** S214 (2026-03-04)
**Council:** Round 1 complete — AG APPROVE, MP APPROVE, XAI APPROVE w/ REVISIONS
**Estimated:** ~14-15h total across 4 phases
**Priority:** P1 — UX regression + allAI hallucination fix

---

## Problem Statement

1. File uploads fail silently — modal closes with no notification
2. Batch uploads are not fault-tolerant — one failure kills the batch
3. allAI hallucinated diagnostic bundle capability (no tool exists)
4. No persistent notification system for background/async events
5. allAI has no way to generate or transmit diagnostic bundles

## Architecture Decisions (Council Consensus)

- **Notification storage:** SQLite via SQLAlchemy. Single table. Survives Docker restarts, readable by allAI tools. Not DuckDB (OLAP, write contention). Not localStorage (not accessible by backend/allAI).
- **Frontend UX:** Bell icon (top-right) + shadcn Sheet (right sidebar). Unread badge. Polling every 15-30s (no SSE/WS needed for single-user).
- **Upload resilience:** Per-file try/except in backend batch loop. Return per-file results. Log failures + successes to notification system. Frontend shows summary.
- **Diagnostic transmission:** Local-first. allAI generates bundle → creates `action_required` notification → user clicks "Approve & Send" → HTTPS POST to fixed ai.market endpoint. Never automatic.
- **PII scrubbing:** Mandatory in diagnostic bundles. No raw user data, no env secrets, no chat history. System health + config shapes + app logs only.

## Deliverables & Phasing

### Phase 1: D3 + D4 — Stop the Bleeding (~2h)

**D3: allAI Anti-Hallucination**
- File: `app/services/prompt_factory.py`
- Add to Layer 1 Safety: "NEVER offer, suggest, or claim capabilities that are not explicitly defined in your tool list. If a user asks for something you don't have a tool for, say clearly that you cannot do it currently."
- Remove "Diagnostic bundle generation" from Layer 2 in-scope list (it will be re-added once D4 ships)
- Add regression test: system prompt must contain the anti-hallucination constraint

**D4: allAI Diagnostic Tool**
- File: `app/services/allai_tools.py` — add `generate_diagnostic_bundle` tool definition
- File: `app/services/allai_tool_executor.py` — wire tool to call `DiagnosticService.generate_bundle()` internally (not via HTTP)
- Tool returns: summary of bundle contents, file count, size, and a download URL (`/api/diagnostics/bundle`)
- allAI can then tell user: "I've generated a diagnostic bundle — you can download it [here]"

### Phase 2: D1 — Persistent Notification System (~5h)

**Backend:**
- New model: `app/models/notification.py` — `id`, `type` (enum: info/success/warning/error/action_required), `category` (upload/processing/system/diagnostic), `title`, `message`, `metadata_json`, `read` (bool), `batch_id` (optional grouping), `source` (system/allai/upload), `created_at`
- New service: `app/services/notification_service.py` — create, list (paginated, filterable), mark_read, mark_all_read, delete, get_unread_count
- New router: `app/routers/notifications.py` — `GET /api/notifications`, `GET /api/notifications/unread-count`, `PATCH /api/notifications/{id}/read`, `POST /api/notifications/read-all`, `DELETE /api/notifications/{id}`
- New allAI tools: `get_notifications` and `create_notification` in allai_tools.py
- SQLAlchemy + Alembic migration
- Auto-prune: notifications older than 30 days

**Frontend:**
- New: `frontend/src/components/notifications/NotificationBell.tsx` — bell icon with unread badge, polls `/api/notifications/unread-count` every 30s
- New: `frontend/src/components/notifications/NotificationSheet.tsx` — shadcn Sheet (right sidebar), lists notifications grouped by date, mark read on click
- Wire into top navigation bar

### Phase 3: D2 — Upload Resilience (~4h)

**Backend:**
- Refactor upload endpoint: per-file try/except in batch loop
- Generate `batch_id` for grouping
- On file failure: log to notification system (error), skip file, continue
- On file success: log to notification system (success) 
- On batch complete: create summary notification (X succeeded, Y failed, reasons)
- Return structured response with per-file status

**Frontend:**
- Upload modal stays open on partial failure
- Show per-file status (checkmark/X) as batch progresses
- Final summary: "5 of 7 files uploaded. 2 failed — see Notifications for details"
- Link to notification sheet for details

### Phase 4: D5 — Diagnostic Transmission (~3-4h)

**Backend:**
- New endpoint: `POST /api/diagnostics/transmit` — takes bundle, POSTs to fixed `api.ai.market/api/v1/support/upload-diagnostic` endpoint
- PII scrub pass before transmission (verify existing scrubbing in diagnostic_service.py)
- Rate limit: 1 transmission per hour
- Size cap: 50MB
- HTTPS only, fixed allowlisted host

**allAI flow:**
- allAI tool: `prepare_support_bundle` — generates bundle, creates `action_required` notification
- Notification shows: bundle contents summary, size, redaction note
- User clicks "Approve & Send" button in notification sheet
- Frontend calls `/api/diagnostics/transmit`
- Confirmation notification on success

**Privacy:**
- Explicit consent every time (no remembered consent)
- Preview of what's included before sending
- All transmission logged locally
- Never automatic, never without user action

---

## Security Mandates
- M1: PII/secret scrubbing mandatory in all diagnostic bundles
- M2: No automatic data transmission — always user-initiated with explicit consent
- M3: Fixed allowlisted transmission endpoint (no user-provided URLs)
- M4: Rate limiting on diagnostic generation (1/min) and transmission (1/hr)
- M5: Bundle size cap (50MB)

## Files Changed (estimated)
- `app/services/prompt_factory.py` (D3)
- `app/services/allai_tools.py` (D4, D1, D5)
- `app/services/allai_tool_executor.py` (D4, D1, D5)
- `app/models/notification.py` (D1 — new)
- `app/services/notification_service.py` (D1 — new)
- `app/routers/notifications.py` (D1 — new)
- `app/routers/copilot.py` (D2 — upload refactor)
- `app/routers/diagnostics.py` (D5 — transmit endpoint)
- `frontend/src/components/notifications/*` (D1 — new)
- `frontend/src/lib/api.ts` (D1, D2)
