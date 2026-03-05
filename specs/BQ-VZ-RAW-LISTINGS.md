# BQ-VZ-RAW-LISTINGS — Raw File Listings on ai.market via vectorAIz

**Version:** 1.0
**Gate 1:** APPROVED_WITH_MANDATES (3/3 — Vulcan, MP, XAI)
**Session:** S209
**Estimated Hours:** 12h
**Repos:** vectoraiz-monorepo (backend + frontend), ai-market-backend

---

## 1. Summary

Allow sellers to list raw data files on ai.market without full vectorization/ingestion. Seller drops a file into VZ → metadata described (manual or allAI-assisted) → listing published to ai.market → buyer purchases → VZ serves the raw file directly via a signed, time-limited download URL. Non-custodial: file never leaves seller's VZ.

Two listing types coexist: **AI-queryable** (existing — processed, embedded, queryable via allAI/MCP) and **File download** (new — raw file, metadata-only discovery, direct download).

---

## 2. Architecture

```
 Seller's VZ                          ai.market                         Buyer
 ──────────                          ─────────                         ─────
                                                                        
 1. Drop file ──────► Raw file stored                                   
 2. Describe metadata ──────────────► POST /listings (type=raw)         
    (manual or allAI)                 Metadata indexed                  
                                      ◄──── Listing visible ──────────► 3. Browse/search
                                                                        4. Purchase
                                      5. Order created ──────────────►  
                                         order_id + entitlement_token   
                                                                        
 6. ◄───── GET /download?token=...  ◄────────────────────────────────── 7. Download request
    Validate entitlement                                                
    Verify file hash                                                    
    Serve file (signed URL, 1h TTL)                                     
```

### Key decisions:
- **Non-custodial** — ai.market stores metadata only. File lives on seller's VZ.
- **Entitlement = signed token** — ai.market issues; VZ validates. No VZ→ai.market callback needed at download time (M1).
- **Content hash pinned** — SHA256 at listing time prevents silent replacement (M2).
- **allAI auto-describe** — mandatory for raw listings to ensure minimum metadata quality (M5).

---

## 3. Database Schema

### 3.1 VZ Backend: `raw_files` table

```sql
CREATE TABLE raw_files (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    filename VARCHAR(512) NOT NULL,
    file_path VARCHAR(1024) NOT NULL,        -- Absolute path in /data/raw/
    file_size_bytes BIGINT NOT NULL,
    content_hash CHAR(64) NOT NULL,          -- SHA256 (M2)
    mime_type VARCHAR(128),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### 3.2 VZ Backend: `raw_listings` table

```sql
CREATE TABLE raw_listings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    raw_file_id UUID NOT NULL REFERENCES raw_files(id),
    marketplace_listing_id UUID,              -- ai.market listing UUID (set after publish)
    title VARCHAR(256) NOT NULL,
    description TEXT NOT NULL,
    tags JSONB DEFAULT '[]',                  -- ["csv", "financial", "timeseries"]
    auto_metadata JSONB,                      -- allAI-generated (M5)
    price_cents INTEGER,                      -- NULL = use ai.market default
    status VARCHAR(32) NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'listed', 'delisted')),  -- M3 lifecycle
    published_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_raw_listings_status ON raw_listings (status);
```

### 3.3 ai.market Backend: Extend `listings` table

```sql
ALTER TABLE listings ADD COLUMN listing_type VARCHAR(16)
    NOT NULL DEFAULT 'queryable'
    CHECK (listing_type IN ('queryable', 'raw'));

ALTER TABLE listings ADD COLUMN raw_metadata JSONB;
    -- {file_size_bytes, mime_type, content_hash, preview_snippet, tags}

ALTER TABLE listings ADD COLUMN file_hash CHAR(64);  -- M2 integrity
```

---

## 4. Components

### 4.1 VZ: Raw File Service (`app/services/raw_file_service.py`)

```python
class RawFileService:
    async def register_file(file_path: str) -> RawFile:
        """Hash file (SHA256), extract mime type, store metadata."""

    async def get_file(file_id: UUID) -> RawFile | None

    async def serve_file(file_id: UUID, entitlement_token: str) -> FileResponse:
        """Validate entitlement token, verify hash, return file stream."""

    async def generate_metadata(file_id: UUID) -> dict:
        """Use allAI to auto-describe file (M5): title, summary, tags, preview snippet."""
```

### 4.2 VZ: Raw Listings Router (`app/routers/raw_listings.py`)

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/api/raw/files` | POST | Admin | Upload/register raw file |
| `/api/raw/files/{id}` | GET | Admin | Get file metadata |
| `/api/raw/files/{id}/metadata` | POST | Admin | Generate allAI metadata (M5) |
| `/api/raw/listings` | GET | Admin | List raw listings |
| `/api/raw/listings` | POST | Admin | Create listing from raw file |
| `/api/raw/listings/{id}` | PUT | Admin | Update listing metadata |
| `/api/raw/listings/{id}/publish` | POST | Admin | Publish to ai.market |
| `/api/raw/listings/{id}/delist` | POST | Admin | Delist from ai.market |
| `/api/raw/download/{id}` | GET | Entitlement token | Serve raw file to buyer |

### 4.3 ai.market: Extended Listing Endpoints

| Endpoint | Change |
|----------|--------|
| `POST /api/v1/listings` | Accept `listing_type: "raw"` + `raw_metadata` |
| `GET /api/v1/listings` | Filter by `listing_type` |
| `GET /api/v1/listings/{id}` | Return `listing_type` + `raw_metadata` |
| `POST /api/v1/orders/{id}/download-token` | NEW: Issue entitlement token for raw download |

### 4.4 Entitlement Protocol (M1)

```
Purchase flow:
1. Buyer purchases listing on ai.market → Order created
2. Buyer requests download: POST /api/v1/orders/{order_id}/download-token
3. ai.market validates: order is paid, listing_type=raw, buyer=requester
4. ai.market issues signed token:
   {
     "order_id": "...",
     "listing_id": "...",
     "file_hash": "abc123...",         // Expected hash
     "buyer_id": "...",
     "issued_at": "2026-03-03T...",
     "expires_at": "2026-03-03T+1h",   // 1h TTL
     "nonce": "random-uuid",           // Replay protection (M1)
     "sig": "HMAC-SHA256(...)"         // Signed with shared secret
   }
5. Buyer sends token to seller's VZ: GET /api/raw/download/{file_id}?token=...
6. VZ validates: signature, expiry, nonce (one-time use), file_hash matches
7. VZ streams file to buyer
```

**Shared secret:** Derived from the existing VZ install token. No new secret distribution needed.

**Nonce store:** Redis SET with TTL=1h. Prevents replay. If Redis unavailable, fall back to in-memory set.

### 4.5 Content Integrity (M2)

```python
async def verify_and_serve(file_id: UUID, expected_hash: str) -> FileResponse:
    raw_file = await get_file(file_id)
    if raw_file.content_hash != expected_hash:
        raise HTTPException(409, "File has changed since listing. Contact seller.")
    return FileResponse(raw_file.file_path, media_type=raw_file.mime_type)
```

Hash computed once at registration time using streaming SHA256 (handles large files):
```python
import hashlib
async def compute_hash(file_path: str) -> str:
    h = hashlib.sha256()
    with open(file_path, "rb") as f:
        while chunk := f.read(8192):
            h.update(chunk)
    return h.hexdigest()
```

### 4.6 allAI Metadata Generation (M5)

When seller clicks "Auto-describe" on a raw file:

1. Extract file sample: first 4KB (text) or schema (CSV/JSON)
2. Send to allAI copilot: "Describe this dataset for a marketplace listing. Generate: title, one-paragraph description, up to 5 tags, and a preview snippet."
3. Store result in `raw_listings.auto_metadata`
4. Seller can edit before publishing

---

## 5. Offline Handling (M4)

When buyer attempts download and seller VZ is unreachable:

1. ai.market download-token endpoint succeeds (token issued regardless of VZ status)
2. Buyer's request to VZ times out
3. **Buyer experience:** ai.market order page shows "Seller's data server is currently offline. Download will be available when the seller comes back online. Your purchase is valid — try again later."
4. **Refund policy:** If VZ remains offline for 72h post-purchase, buyer can request automatic refund via ai.market dispute flow (existing).
5. **No SLA enforcement in MVP** — seller uptime is seller's responsibility.

---

## 6. Frontend Changes (VZ)

### 6.1 New "Marketplace" Tab → "Raw Listings" Section

Add a sub-tab under the existing ai.market section in VZ:
- **File browser:** Drop zone + file list showing registered raw files
- **Metadata editor:** Title, description, tags (pre-filled by allAI), price
- **Publish button:** Sends metadata to ai.market, updates status to "listed"
- **Status badges:** draft / listed / delisted

### 6.2 ai.market Frontend

- Listing detail page shows `listing_type` badge: "AI-Queryable" or "File Download"
- Raw listings show: title, description, tags, file size, mime type, preview snippet
- Download button (post-purchase): requests entitlement token → redirects to VZ download URL

---

## 7. Phasing

### Phase 1: VZ Backend (5h)
- `raw_files` + `raw_listings` tables + Alembic migration
- RawFileService (register, hash, serve, metadata gen)
- Raw listings router (CRUD, publish, delist, download)
- Entitlement token validation
- Tests: 25+

### Phase 2: ai.market Backend (4h)
- Extend listings table: `listing_type`, `raw_metadata`, `file_hash`
- Listing CRUD accepts raw type
- Download token issuance endpoint
- Trust Channel: publish/delist raw listings from VZ
- Tests: 15+

### Phase 3: Frontend (3h)
- VZ: Raw listings UI (file drop, metadata editor, publish)
- ai.market: Listing type badges, download flow
- Tests: 5+

---

## 8. Test Plan

| Category | Tests | Description |
|----------|-------|-------------|
| Raw file registration | 5 | Upload, hash, mime detection, large file |
| Listing CRUD | 8 | Create, update, publish, delist, lifecycle |
| Entitlement protocol | 8 | Token issue, validate, expired, replayed, wrong hash |
| File serving | 5 | Stream file, hash mismatch, missing file |
| allAI metadata | 3 | Auto-describe, edit, empty file |
| ai.market integration | 8 | Raw listing type, search, download token |
| Frontend | 5 | Drop zone, metadata form, publish flow |
| **Total** | **42+** | |

---

## 9. Dependencies

- No new backend deps (hashlib, JWT already available)
- Shared secret for token signing: derive from existing install token via HKDF
- allAI copilot must be functional (connected mode only for auto-describe)

---

## 10. Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Seller replaces file post-listing | SHA256 pin; VZ blocks serve if hash mismatch (M2) |
| VZ offline post-purchase | 72h refund window; status visibility for buyer (M4) |
| Low-quality raw listings | allAI auto-describe mandatory; admin review stretch goal |
| Token replay attack | Nonce in Redis with TTL; one-time use enforcement (M1) |
| Large file download timeout | Streaming FileResponse; no size limit in MVP |
| Seller exposes VZ to internet | Docs: recommend reverse proxy + TLS; not enforced in MVP |
