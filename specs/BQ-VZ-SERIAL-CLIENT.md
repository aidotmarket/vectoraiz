# BQ-VZ-SERIAL-CLIENT — vectorAIz Serial System Client Integration

**Version:** 1.0
**Gate 1:** APPROVED (3/3 — AG, MP, Vulcan; XAI unavailable)
**Session:** S178
**Estimated Hours:** 10-14h
**Repo:** vectoraiz-backend

---

## 1. Summary

Integrate vectorAIz backend with ai-market's serial authority (BQ-SERIAL). vectorAIz instances will activate serials on first boot, meter allAI usage against setup ($10) and data ($4) credit pools, gracefully wall users at the $4 data cap, refresh tokens on upgrade, and bridge to the BQ-073 credit ledger after user registration.

---

## 2. State Machine (M1)

The serial client has 5 states. All transitions are deterministic.

```
┌─────────────────┐
│  UNPROVISIONED   │ ← No serial on disk
│  (show setup UI) │
└────────┬────────┘
         │ bootstrap token provided (download page or start.sh)
         ▼
┌─────────────────┐
│  PROVISIONED     │ ← Has serial + bootstrap token, not yet activated
│  (attempt activate)│
└────────┬────────┘
         │ POST /activate succeeds → install token stored
         ▼
┌─────────────────┐
│     ACTIVE       │ ← Has install token, can meter
│  (normal ops)    │◄──────────────────────────┐
└────────┬────────┘                            │
         │                                     │
         ├── ai-market unreachable ──► DEGRADED_OFFLINE ──► (reconnect) ──┘
         │                            (policy-based allow/block)
         │
         │ POST /status returns billing_mode=ledger
         ▼
┌─────────────────┐
│    MIGRATED      │ ← Use BQ-073 credit ledger
│  (registered)    │
└─────────────────┘
```

### Transition rules:
- **UNPROVISIONED → PROVISIONED**: Bootstrap token written to `/data/serial.json` (by start.sh or setup UI)
- **PROVISIONED → ACTIVE**: `POST /serials/{serial}/activate` returns 200 with install token
- **ACTIVE → DEGRADED_OFFLINE**: 3 consecutive failed meter/status calls
- **DEGRADED_OFFLINE → ACTIVE**: Next successful server call
- **ACTIVE → MIGRATED**: `/status` response includes `migrated=true`
- **Any → UNPROVISIONED**: Token revoked server-side (401 on meter/status)

---

## 3. Components

### 3.1 SerialStore (`app/services/serial_store.py`)

Persistent local state in `/data/serial.json`. Docker volume mount survives restarts/upgrades.

```python
@dataclass
class SerialState:
    serial: str                          # VZ-xxxxxxxx-xxxxxxxx
    install_token: str | None            # vzit_... (None if not yet activated)
    bootstrap_token: str | None          # vzbt_... (cleared after activation)
    state: str                           # unprovisioned|provisioned|active|degraded|migrated
    last_app_version: str | None         # For upgrade detection
    last_status_cache: dict | None       # Cached credits for UI display
    last_status_at: str | None           # ISO timestamp
    consecutive_failures: int            # For degraded transition (threshold: 3)
```

**Atomic writes:** Write to `/data/serial.json.tmp`, fsync, rename. Prevents corruption on crash.

**Security:**
- `chmod 600` on the file
- Bootstrap token DELETED from disk after successful activation
- Local state is UX cache only — server is authority
- No encryption (customer hardware; won't stop determined tampering, adds support burden)

### 3.2 SerialClient (`app/services/serial_client.py`)

HTTP client wrapping ai-market serial endpoints.

```python
class SerialClient:
    base_url: str  # from VECTORAIZ_AIMARKET_URL config
    timeout: float = 10.0

    async def activate(serial, bootstrap_token, instance_id, hostname, version) -> ActivateResult
    async def meter(serial, install_token, category, cost_usd, request_id, ...) -> MeterResult
    async def status(serial, install_token) -> StatusResult
    async def refresh(serial, install_token, instance_id) -> RefreshResult
```

**Retry policy:** 2 retries with exponential backoff (1s, 3s). Timeout 10s per call.
**Circuit breaker:** After 3 consecutive failures, set state=DEGRADED_OFFLINE. Probe every 60s.

### 3.3 ActivationManager (`app/services/activation_manager.py`)

Runs during FastAPI lifespan (startup).

**Startup sequence:**
1. Load `/data/serial.json` → determine state
2. If UNPROVISIONED: log warning, allow local-only operations, show "needs serial" in UI
3. If PROVISIONED (has bootstrap, no install token): call `activate()`, store install token, delete bootstrap token, transition to ACTIVE
4. If ACTIVE:
   a. Compare `last_app_version` vs current `APP_VERSION` → if different, call `refresh()`, store new token
   b. Background task: call `status()`, update cache
5. If MIGRATED: no serial operations needed, bridge to BQ-073

**Failure handling:**
- Activation fails (network): stay PROVISIONED, allow setup operations only, retry in background every 30s
- Refresh fails (network): keep existing install token (optimistic), retry in background
- Refresh returns 401 (token invalid): drop install token, fall back to PROVISIONED, show re-activation banner

### 3.4 MeteringMiddleware (`app/services/serial_metering.py`)

**Strategy pattern** bridging serial metering and BQ-073 credit ledger:

```python
class MeteringStrategy(Protocol):
    async def check_and_meter(category: str, estimated_cost: Decimal, request_id: str) -> MeterDecision

class SerialMeteringStrategy(MeteringStrategy):
    """Pre-registration: meters against serial credit pools via ai-market"""

class LedgerMeteringStrategy(MeteringStrategy):
    """Post-registration: meters against BQ-073 credit ledger"""
```

**FastAPI dependency** for metered endpoints:

```python
async def require_metering(
    category: str,  # "setup" or "data" — set per-endpoint
    serial_store: SerialStore = Depends(get_serial_store),
) -> MeterDecision:
    strategy = get_active_strategy(serial_store)
    return await strategy.check_and_meter(...)
```

---

## 4. Metering Category Taxonomy (M2)

Every allAI-consuming endpoint is tagged. Classification is by route identity, NOT LLM inference.

### SETUP (draws from $10 pool)

| Router | Endpoint | Rationale |
|--------|----------|-----------|
| connectivity_mgmt | POST /setup | Source connection setup |
| connectivity_mgmt | POST /test | Test connection |
| connectivity_mgmt | POST /enable | Enable connected mode |
| datasets | POST /upload (first 3 per serial) | Initial data onboarding |
| datasets | POST /batch (first batch) | Initial batch onboarding |
| datasets | POST /{id}/confirm | Confirm dataset |
| datasets | POST /{id}/pipeline | Initial processing |
| datasets | POST /{id}/process-full | Initial full pipeline |
| datasets | POST /{id}/index (first 3) | Initial indexing |
| datasets | POST /{id}/attestation | Quality check |
| datasets | POST /{id}/compliance | Compliance check |
| datasets | POST /{id}/listing-metadata | Marketplace prep |
| datasets | POST /{id}/publish | Publish to ai.market |
| pii | POST /scan/{id} | PII detection |
| pii | POST /scrub/{id} | PII scrubbing |
| copilot | /ws/copilot BRAIN_MESSAGE (onboarding context) | Setup-related copilot questions |
| copilot | POST /brain (onboarding context) | Setup-related brain queries |

### DATA (draws from $4 pool)

| Router | Endpoint | Rationale |
|--------|----------|-----------|
| allai | POST /generate | RAG query |
| allai | POST /generate/stream | Streaming RAG query |
| copilot | /ws/copilot BRAIN_MESSAGE (data context) | Data questions via copilot |
| copilot | POST /brain (data context) | Data brain queries |
| sql | POST /query | SQL query execution |
| datasets | POST /upload (after first 3) | Ongoing data ingestion |
| datasets | POST /batch (after first) | Ongoing batch uploads |
| datasets | POST /{id}/index (after first 3) | Re-indexing |

### NOT METERED (no allAI cost)

All GET endpoints (read-only), health checks, version, docs, feedback, auth, local-only operations.

### Copilot context detection

Copilot messages are dual-category. Classification uses a **simple heuristic on the active UI context**, NOT LLM analysis:

```python
def classify_copilot_category(session_metadata: dict) -> str:
    """Classify based on what the user is doing when they ask the copilot."""
    active_view = session_metadata.get("active_view", "unknown")
    if active_view in ("onboarding", "setup", "connectivity", "metadata_builder", "publish"):
        return "setup"
    return "data"  # Default to data (conservative — costs user from data pool)
```

The frontend sends `active_view` in the WebSocket session metadata. If missing, defaults to `data`.

---

## 5. Offline Behavior Matrix (M5)

| State | Category | ai-market reachable | Behavior |
|-------|----------|-------------------|----------|
| ACTIVE | setup | Yes | Meter normally |
| ACTIVE | setup | No | **ALLOW** — log locally, sync later. Show "offline billing" banner. |
| ACTIVE | data | Yes | Meter normally |
| ACTIVE | data | No (transient, <3 fails) | **ALLOW** with warning banner |
| ACTIVE | data | No (3+ consecutive fails) | **BLOCK** — "Unable to verify credits. Check connection to ai.market." |
| DEGRADED | setup | No | **ALLOW** — same as above |
| DEGRADED | data | No | **BLOCK** |
| DEGRADED | any | Yes (probe succeeds) | Transition to ACTIVE, resume normal metering |
| PROVISIONED | setup | No | **ALLOW** — activation retry in background |
| PROVISIONED | data | No | **BLOCK** — "Activation required" |
| UNPROVISIONED | any | any | **BLOCK** all metered operations — "Enter serial to continue" |
| MIGRATED | any | any | Delegate to BQ-073 LedgerMeteringStrategy |

### Offline usage queue

When setup operations are allowed offline, log usage to `/data/pending_usage.jsonl` (append-only). On reconnect, flush to `/serials/{serial}/meter` with idempotent `request_id`s. Cap offline queue at 50 entries ($5 max estimated) to prevent unbounded free usage.

---

## 6. Idempotency (M3)

Every meter call includes a `request_id` (the server already supports this).

Format: `vz:{serial_short}:{endpoint_hash}:{timestamp_ms}`

Example: `vz:a1b2c3d4:brain_msg:1708800000123`

- Serial short = first 8 chars after `VZ-`
- Endpoint hash = deterministic from route + method
- Timestamp = milliseconds (generated once, not recomputed on retry)

On retry (network timeout), same `request_id` is sent. Server deduplicates.

---

## 7. Server Response Contract (M4)

### Meter response (from BQ-SERIAL spec)
```json
{
    "allowed": true,           // or false
    "category": "data",
    "cost_usd": "0.0300",
    "remaining_usd": "3.9700",
    "reason": null,            // or "insufficient_data_credits", "insufficient_setup_credits"
    "payment_enabled": false,
    "migrated": false
}
```

### Insufficient funds behavior
- `allowed=false, reason=insufficient_data_credits, payment_enabled=false`:
  → Show $4 wall modal: "You've used your free data credits. Add a payment method to continue using data features. Setup features are still available."
- `allowed=false, reason=insufficient_data_credits, payment_enabled=true`:
  → This shouldn't happen (payment tops up credits). If it does, show "Credits exhausted — purchase more at ai.market."
- `allowed=false, reason=insufficient_setup_credits`:
  → Show: "Setup credits exhausted. Register at ai.market to continue." (Rare — $10 is generous.)

### Migration detection
- `migrated=true` on any meter or status response:
  → Transition to MIGRATED state
  → Switch to LedgerMeteringStrategy
  → Status response includes `gateway_user_id` for BQ-073 auth

---

## 8. The $4 Wall UX

### Backend error response
When data metering is denied:
```json
{
    "error": "data_credits_exhausted",
    "message": "You've used your $4.00 free data credits.",
    "setup_remaining_usd": "7.50",
    "data_remaining_usd": "0.00",
    "payment_enabled": false,
    "register_url": "https://ai.market/register?serial=VZ-..."
}
```

### WebSocket (copilot)
New message type `CREDIT_WALL`:
```json
{
    "type": "CREDIT_WALL",
    "category": "data",
    "message": "You've used your free data credits. Add a payment method to continue.",
    "setup_remaining_usd": "7.50",
    "register_url": "https://ai.market/register?serial=VZ-..."
}
```

### Frontend contract (for VZ frontend build)
- On `CREDIT_WALL` or `data_credits_exhausted` error:
  - Show non-dismissible modal with friendly message
  - Include: remaining setup credits, CTA to register/pay, link to ai.market
  - Setup features remain fully accessible
  - Data features show lock icon with tooltip

---

## 9. Token Refresh on Upgrade

**Detection:** Compare `APP_VERSION` env var (set in Docker image) against `last_app_version` in serial.json.

**On version mismatch:**
1. Call `POST /serials/{serial}/refresh` with current install token + instance_id
2. Store new install token, update last_app_version
3. If refresh fails (network): keep old token, retry in background
4. If refresh returns 401: token was revoked/expired. Fall to PROVISIONED state, require re-activation.

---

## 10. Bridge to BQ-073 (Post-Migration)

When state transitions to MIGRATED:
1. Stop using SerialMeteringStrategy
2. Activate LedgerMeteringStrategy which wraps existing `metering_service.py`
3. The existing `check_balance()` / `report_usage()` flow continues unchanged
4. Serial store records `migrated=true`, stops all serial-specific operations
5. ActivationManager skips serial startup checks when migrated

This is a one-way transition. Once migrated, the serial is consumed.

---

## 11. Files to Create/Modify

### New files (vectoraiz-backend):
- `app/services/serial_store.py` — SerialState dataclass, atomic JSON persistence
- `app/services/serial_client.py` — HTTP client for ai-market serial endpoints
- `app/services/activation_manager.py` — Startup lifecycle, refresh, background probes
- `app/services/serial_metering.py` — MeteringStrategy protocol, Serial + Ledger strategies, FastAPI dependency
- `app/models/metering_category.py` — Category enum + route→category mapping

### Modified files:
- `app/main.py` — Add ActivationManager to lifespan
- `app/routers/copilot.py` — Add metering check before BRAIN_MESSAGE handling, CREDIT_WALL message type
- `app/routers/allai.py` — Add metering dependency to /generate, /generate/stream
- `app/routers/datasets.py` — Add metering dependency to upload, pipeline, index endpoints
- `app/routers/sql.py` — Add metering dependency to POST /query
- `app/routers/pii.py` — Add metering dependency to scan, scrub
- `app/config.py` — Add VECTORAIZ_AIMARKET_URL, APP_VERSION configs

### Tests:
- `tests/test_serial_store.py` — Persistence, atomic writes, state transitions
- `tests/test_serial_client.py` — HTTP mocking, retry, circuit breaker
- `tests/test_serial_metering.py` — Category classification, strategy routing, offline behavior
- `tests/test_activation_manager.py` — Boot sequences for each state

---

## 12. Acceptance Criteria

1. Fresh VZ instance with bootstrap token activates on first boot and stores install token
2. Copilot data query deducts from $4 data pool; setup query deducts from $10 setup pool
3. When data pool exhausted: CREDIT_WALL shown, setup features still work
4. Docker upgrade (version change) triggers token refresh transparently
5. When ai-market unreachable: setup allowed (offline queue), data blocked after 3 failures
6. Offline usage queue syncs on reconnect with idempotent request_ids
7. Migration flag transitions to BQ-073 metering seamlessly
8. `GET /api/v1/system/billing-status` returns mode, remaining credits, payment status
9. All meter calls include request_id for idempotency
10. Bootstrap token deleted from disk after successful activation
