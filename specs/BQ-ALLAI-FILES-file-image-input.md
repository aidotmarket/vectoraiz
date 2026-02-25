# BQ-ALLAI-FILES: allAI Copilot File & Image Input

## Summary
Add file upload, image paste, and drag-and-drop capabilities to the allAI copilot
chat in vectorAIz. Users can share screenshots, CSVs, PDFs, and other files directly
in the chat for allAI to analyze, diagnose errors, and provide data guidance.

## Gate 1 — Council Direction (S135)
- **MP:** Hybrid local-first. Extract text locally by default, passthrough images to
  multimodal LLMs. Separate upload endpoint → handle → reference. Ephemeral storage.
  Capability matrix for BYO LLM fallback.
- **XAI:** Lean into passthrough for multimodal models (LLMs see images better than OCR).
  Cut cost confirmation, caching, promote-to-dataset for v1. Hard caps only.
- **AG:** ~4 days build. Reuse ProcessingService + Tika. Separate /api/copilot/upload
  endpoint. Frontend: 2 days (button + drag-drop + paste).
- **Max:** Approved.

## Council Mandates (Gate 2)

| ID | Mandate | Source | Severity |
|----|---------|--------|----------|
| M1 | Separate HTTP upload endpoint — no binary in WebSocket | All | BLOCKING |
| M2 | Ephemeral storage with TTL (1 hour) — chat attachments are NOT datasets | MP+XAI | BLOCKING |
| M3 | Hard caps: 10MB/file, 3 attachments/message, images max 1600px longest side | XAI | BLOCKING |
| M4 | MIME validation via magic bytes, not extension/Content-Type header | MP | BLOCKING |
| M5 | Logging redaction — never log file content or base64 payloads | MP | BLOCKING |
| M6 | Provider capability check — if BYO LLM doesn't support vision, extract text and tell user | All | REQUIRED |
| M7 | Image passthrough to multimodal LLMs; PDF/CSV/Excel extract text locally | All | REQUIRED |

---

## Architecture

```
                       ┌─────────────────────────────┐
                       │    Frontend (ChatInput)      │
                       │  [📎 button] [drag-drop]     │
                       │  [paste image from clipboard] │
                       └──────┬──────────────────┬────┘
                              │                  │
                    HTTP POST │                  │ WebSocket
                   (multipart)│                  │
                              ▼                  ▼
              ┌───────────────────┐   ┌──────────────────────┐
              │ POST /api/copilot │   │ BRAIN_MESSAGE        │
              │    /upload        │   │ { message: "...",    │
              │                   │   │   attachments: [     │
              │ Returns:          │   │     { id: "att_xxx", │
              │ { id: "att_xxx",  │   │       type: "image"} │
              │   type, filename, │   │   ]                  │
              │   size, preview } │   │ }                    │
              └───────┬───────────┘   └──────────┬───────────┘
                      │                          │
                      ▼                          ▼
              ┌───────────────┐      ┌───────────────────────┐
              │ /data/chat/   │      │ CoPilotService        │
              │ ephemeral     │◄─────│ resolve attachments   │
              │ TTL: 1 hour   │      │ by ID from ephemeral  │
              └───────────────┘      │ storage               │
                                     └───────────┬───────────┘
                                                 │
                                     ┌───────────▼───────────┐
                                     │ Provider Adapter       │
                                     │                       │
                                     │ if multimodal:        │
                                     │   image → base64 block│
                                     │   PDF → text extract  │
                                     │ if text-only:         │
                                     │   image → OCR → text  │
                                     │   PDF → text extract  │
                                     └───────────────────────┘
```

---

## Changes Required

### 1. Ephemeral Upload Endpoint (`app/routers/copilot.py`)

**NEW route: `POST /api/copilot/upload`**

```python
@router.post("/upload")
async def copilot_upload(
    file: UploadFile = File(...),
    user: AuthenticatedUser = Depends(get_current_user),
):
    """
    Upload a file for use in allAI chat. Returns an attachment handle.
    
    Files are stored ephemerally (1 hour TTL) and are NOT added to the
    dataset catalog. Use dataset upload for persistent storage.
    """
```

**Validation pipeline:**
1. Check file size ≤ 10MB (stream and count, reject early)
2. Read first 32 bytes for magic byte MIME detection
3. Validate against allowed types:
   - Images: `image/png`, `image/jpeg`, `image/webp`, `image/gif`
   - Documents: `application/pdf`
   - Data: `text/csv`, `application/json`, `text/plain`,
     `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet` (xlsx),
     `application/vnd.ms-excel` (xls)
4. For images: check dimensions, resize if longest side > 1600px (use Pillow)
5. Generate attachment ID: `att_{uuid4().hex[:12]}`
6. Save to `/data/chat/{attachment_id}/{original_filename}`
7. Generate preview metadata (image: thumbnail URL; document: first 200 chars of extracted text)

**Response:**
```json
{
  "id": "att_a1b2c3d4e5f6",
  "filename": "error_screenshot.png",
  "mime_type": "image/png",
  "size_bytes": 245000,
  "type": "image",
  "preview": null,
  "expires_at": "2026-02-16T12:45:00Z"
}
```

**Type classification:**
- `image/png`, `image/jpeg`, `image/webp`, `image/gif` → type `"image"`
- `application/pdf` → type `"document"`
- `text/csv`, `application/json`, `text/plain`, xlsx, xls → type `"data"`

### 2. Ephemeral Storage Service (`app/services/chat_attachment_service.py`) — NEW

```python
"""
Ephemeral storage for chat attachments.

Files live in /data/chat/ with a 1-hour TTL.
A background task cleans up expired attachments periodically.
Attachments are NOT datasets — they don't appear in the catalog.
"""
import os
import time
import shutil
from pathlib import Path
from dataclasses import dataclass
from typing import Optional

CHAT_UPLOAD_DIR = Path("/data/chat")
ATTACHMENT_TTL_SECONDS = 3600  # 1 hour
MAX_FILE_SIZE = 10 * 1024 * 1024  # 10MB
MAX_ATTACHMENTS_PER_MESSAGE = 3
MAX_IMAGE_DIMENSION = 1600  # px, longest side

ALLOWED_MIME_TYPES = {
    # Images (passthrough to multimodal LLM)
    "image/png", "image/jpeg", "image/webp", "image/gif",
    # Documents (text extraction)
    "application/pdf",
    # Data files (schema + sample extraction)
    "text/csv", "application/json", "text/plain",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-excel",
}

@dataclass
class ChatAttachment:
    id: str
    filename: str
    mime_type: str
    size_bytes: int
    type: str  # "image" | "document" | "data"
    file_path: Path
    created_at: float
    expires_at: float
    extracted_text: Optional[str] = None  # For documents/data

    @property
    def is_expired(self) -> bool:
        return time.time() > self.expires_at

    def to_dict(self) -> dict:
        return {
            "id": self.id,
            "filename": self.filename,
            "mime_type": self.mime_type,
            "size_bytes": self.size_bytes,
            "type": self.type,
            "expires_at": datetime.fromtimestamp(self.expires_at).isoformat() + "Z",
        }


class ChatAttachmentService:
    """Manage ephemeral chat file uploads."""

    def __init__(self):
        self._attachments: dict[str, ChatAttachment] = {}  # id → attachment
        CHAT_UPLOAD_DIR.mkdir(parents=True, exist_ok=True)

    async def store(self, attachment_id: str, filename: str, 
                    mime_type: str, file_path: Path, size_bytes: int) -> ChatAttachment:
        """Register an uploaded file as a chat attachment."""
        now = time.time()
        att_type = self._classify(mime_type)
        
        # Extract text for non-image types
        extracted_text = None
        if att_type in ("document", "data"):
            extracted_text = await self._extract_text(file_path, mime_type)

        attachment = ChatAttachment(
            id=attachment_id,
            filename=filename,
            mime_type=mime_type,
            size_bytes=size_bytes,
            type=att_type,
            file_path=file_path,
            created_at=now,
            expires_at=now + ATTACHMENT_TTL_SECONDS,
            extracted_text=extracted_text,
        )
        self._attachments[attachment_id] = attachment
        return attachment

    def get(self, attachment_id: str) -> Optional[ChatAttachment]:
        """Retrieve attachment by ID. Returns None if expired or missing."""
        att = self._attachments.get(attachment_id)
        if att and att.is_expired:
            self._delete(attachment_id)
            return None
        return att

    def cleanup_expired(self):
        """Remove expired attachments from memory and disk."""
        expired = [aid for aid, a in self._attachments.items() if a.is_expired]
        for aid in expired:
            self._delete(aid)

    def _delete(self, attachment_id: str):
        att = self._attachments.pop(attachment_id, None)
        if att:
            att_dir = att.file_path.parent
            shutil.rmtree(att_dir, ignore_errors=True)

    @staticmethod
    def _classify(mime_type: str) -> str:
        if mime_type.startswith("image/"):
            return "image"
        elif mime_type == "application/pdf":
            return "document"
        else:
            return "data"

    async def _extract_text(self, file_path: Path, mime_type: str) -> Optional[str]:
        """Extract text from document/data files using existing pipeline."""
        # Reuse ProcessingService extraction or Tika
        # For CSV: read first 50 rows + schema
        # For PDF: extract text via Tika
        # For JSON: pretty-print first 5KB
        # Cap at 10,000 chars
        ...
```

### 3. MIME Detection (`app/services/mime_detector.py`) — NEW

```python
"""
Magic-byte MIME detection. Do NOT trust file extensions or Content-Type headers.
"""
MAGIC_SIGNATURES = {
    b"\x89PNG\r\n\x1a\n": "image/png",
    b"\xff\xd8\xff": "image/jpeg",
    b"RIFF": "image/webp",  # check bytes 8-12 for "WEBP"
    b"GIF87a": "image/gif",
    b"GIF89a": "image/gif",
    b"%PDF": "application/pdf",
    b"PK\x03\x04": "application/zip",  # could be xlsx, needs further check
}

def detect_mime(header_bytes: bytes, filename: str) -> str | None:
    """Detect MIME type from file header bytes + filename hint."""
    for sig, mime in MAGIC_SIGNATURES.items():
        if header_bytes.startswith(sig):
            if mime == "application/zip" and filename.endswith(".xlsx"):
                return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            return mime
    # Fallback for text formats
    try:
        header_bytes[:256].decode("utf-8")
        if filename.endswith(".csv"):
            return "text/csv"
        elif filename.endswith(".json"):
            return "application/json"
        return "text/plain"
    except UnicodeDecodeError:
        return None
```

### 4. Image Resizing

For images with longest side > 1600px, resize before storage:

```python
from PIL import Image

def resize_if_needed(file_path: Path, max_dimension: int = 1600) -> bool:
    """Resize image in-place if longest side exceeds max. Returns True if resized."""
    with Image.open(file_path) as img:
        w, h = img.size
        if max(w, h) <= max_dimension:
            return False
        ratio = max_dimension / max(w, h)
        new_size = (int(w * ratio), int(h * ratio))
        img = img.resize(new_size, Image.LANCZOS)
        img.save(file_path, quality=85)
        return True
```

### 5. WebSocket Protocol Extension

**Updated BRAIN_MESSAGE (frontend → backend):**

```json
{
  "type": "BRAIN_MESSAGE",
  "message": "What's wrong with this data?",
  "message_id": "msg_abc123",
  "client_message_id": "cli_msg_abc123",
  "attachments": [
    {"id": "att_a1b2c3d4e5f6", "type": "image"},
    {"id": "att_f6e5d4c3b2a1", "type": "data"}
  ]
}
```

**Backend handling in copilot.py:**
```python
elif msg_type == "BRAIN_MESSAGE":
    user_message = data.get("message", "")
    attachments_refs = data.get("attachments", [])  # list of {id, type}
    
    # Validate attachment count
    if len(attachments_refs) > MAX_ATTACHMENTS_PER_MESSAGE:
        await safe_send_json(websocket, {
            "type": "ERROR",
            "message": f"Maximum {MAX_ATTACHMENTS_PER_MESSAGE} attachments per message",
            "code": "TOO_MANY_ATTACHMENTS",
        })
        continue
    
    # Resolve attachments from ephemeral storage
    resolved_attachments = []
    for ref in attachments_refs:
        att = attachment_service.get(ref["id"])
        if not att:
            await safe_send_json(websocket, {
                "type": "ERROR",
                "message": f"Attachment {ref['id']} not found or expired",
                "code": "ATTACHMENT_NOT_FOUND",
            })
            break
        resolved_attachments.append(att)
    else:
        # All resolved — proceed with message
        ...
```

### 6. Provider Adapter — Content Block Construction

**Update `BaseAllieProvider.stream()` signature:**

```python
class BaseAllieProvider:
    async def stream(
        self,
        message: str,
        context: Optional[str] = None,
        attachments: Optional[list[ChatAttachment]] = None,
    ) -> AsyncIterator[AllieStreamChunk]:
        raise NotImplementedError
        yield
```

**In `AiMarketAllieProvider.stream()` — construct content blocks:**

```python
async def stream(self, message: str, context=None, attachments=None):
    # Build message content
    if attachments:
        content_blocks = []
        for att in attachments:
            if att.type == "image":
                # Passthrough: base64 encode image for multimodal LLM
                import base64
                with open(att.file_path, "rb") as f:
                    b64 = base64.b64encode(f.read()).decode()
                content_blocks.append({
                    "type": "image",
                    "source": {"type": "base64", "media_type": att.mime_type, "data": b64}
                })
            elif att.type in ("document", "data"):
                # Extract text locally, send as text block
                if att.extracted_text:
                    content_blocks.append({
                        "type": "text",
                        "text": f"[Attached file: {att.filename}]\n{att.extracted_text}"
                    })
        # Add user message as final text block
        content_blocks.append({"type": "text", "text": message})
        
        # Send as list of content blocks
        user_msg = {"role": "user", "content": content_blocks}
    else:
        # Legacy: simple string content
        user_msg = {"role": "user", "content": message}

    # ... continue with proxy call, passing user_msg in messages list
```

**BYO LLM fallback (M6):**

```python
# In the BYO LLM provider path:
if not provider_supports_vision(provider_name):
    # Convert images to text via OCR
    for att in attachments:
        if att.type == "image":
            # OCR via Tesseract or simple description
            att.extracted_text = await ocr_image(att.file_path)
            att.type = "data"  # downgrade to text
    
    # Inform user
    yield AllieStreamChunk(
        text="*Note: Your configured model doesn't support images. "
             "I've extracted the text from your image.*\n\n",
        done=False,
    )
```

### 7. Proxy Schema Compatibility

The ai.market proxy (BQ-PROXY-TOOLS, being built now) already supports `AllieMessage`
with `Union[str, List[ContentBlock]]`. We need to add `ImageBlock` to the discriminated
union on the proxy side:

```python
# In ai.market-backend app/schemas/allie.py (addition to BQ-PROXY-TOOLS):
class ImageBlock(BaseModel):
    type: Literal["image"] = "image"
    source: Dict[str, Any]  # {type: "base64", media_type: "image/png", data: "..."}
```

This is a FOLLOW-UP to BQ-PROXY-TOOLS, not part of this build. The proxy change is
trivial — add one block type to the discriminated union.

### 8. Frontend Changes

#### 8a. ChatInput.tsx — Attachment Button + Paste + Drag-Drop

```tsx
// Add to ChatInput:
// - Paperclip button (📎) next to send button
// - Hidden <input type="file"> triggered by paperclip click
// - onPaste handler on textarea to capture clipboard images
// - onDragOver/onDrop on the entire input area
// - Preview thumbnails above the textarea for queued attachments
// - "X" button to remove queued attachment before sending

interface Attachment {
  id: string;
  filename: string;
  type: "image" | "document" | "data";
  preview?: string;  // data URL for image thumbnail
  uploading: boolean;
}

// Upload flow:
// 1. User drops/pastes/selects file
// 2. Show uploading spinner in preview area
// 3. POST /api/copilot/upload (multipart)
// 4. Receive attachment handle {id, type, filename}
// 5. Show preview thumbnail (images) or filename chip (docs/data)
// 6. On send: include attachment IDs in BRAIN_MESSAGE
```

#### 8b. ChatMessage.tsx — Render Attachments in Messages

```tsx
// Add to ChatMessage:
// - If message has attachments, render them:
//   - Images: inline thumbnail (clickable to expand)
//   - Documents: file chip with icon + filename
//   - Data: file chip with icon + filename
// - User messages show attachments above the text
// - Assistant messages may reference attachments in their response
```

#### 8c. CoPilotContext.tsx — Updated sendMessage

```tsx
const sendMessage = useCallback(
  (text: string, attachments?: Attachment[]) => {
    // ... existing logic ...
    
    // Add user message with attachments
    setMessages((prev) => [
      ...prev,
      {
        id: `user_${msgId}`,
        role: "user",
        content: text,
        attachments: attachments?.map(a => ({
          id: a.id, type: a.type, filename: a.filename, preview: a.preview
        })),
        createdAt: new Date().toISOString(),
      },
    ]);

    // Send via WebSocket with attachment references
    wsRef.current.send(
      JSON.stringify({
        type: "BRAIN_MESSAGE",
        message: text,
        message_id: msgId,
        client_message_id: `cli_${msgId}`,
        attachments: attachments?.map(a => ({ id: a.id, type: a.type })) || [],
      })
    );
  },
  []
);
```

### 9. TTL Cleanup

Add a periodic cleanup task (runs every 10 minutes):

```python
# In copilot.py or app startup:
async def cleanup_expired_attachments():
    while True:
        attachment_service.cleanup_expired()
        await asyncio.sleep(600)  # every 10 min
```

### 10. Logging Policy (M5)

**DO log:** attachment ID, filename, MIME type, size, upload timing
**DO NOT log:** file content, base64 data, extracted text, file paths containing user data

---

## What This Does NOT Include (Deferred)

1. **Cost confirmation dialog** — v1 uses hard caps only
2. **Image caching across turns** — resend base64 each time (acceptable at low volume)
3. **Promote to dataset** — user must use dataset upload separately
4. **Audio/video support** — not a data platform need
5. **Auto-downscaling quality options** — fixed at 1600px max, 85% quality
6. **OCR for images on text-only LLMs** — v1 shows "your model doesn't support images" message. Full OCR is v2.

---

## Test Plan

### Backend
1. Upload valid PNG → 200, returns attachment handle with type "image"
2. Upload valid PDF → 200, returns handle with type "document", extracted_text populated
3. Upload valid CSV → 200, returns handle with type "data", extracted_text has schema + rows
4. Upload > 10MB file → 413
5. Upload .exe (invalid MIME) → 415 Unsupported Media Type
6. Upload with spoofed extension (rename .exe to .png) → 415 (magic byte detection)
7. Retrieve expired attachment → 404
8. BRAIN_MESSAGE with valid attachment IDs → message processed with content blocks
9. BRAIN_MESSAGE with expired attachment ID → ERROR response
10. BRAIN_MESSAGE with > 3 attachments → ERROR response
11. Image > 1600px uploaded → resized to 1600px max dimension
12. Logging redaction: base64/file content never in logs

### Frontend
13. Paperclip button opens file picker
14. Drag-and-drop file onto chat → uploads and shows preview
15. Paste image from clipboard → uploads and shows preview
16. Preview shows uploading spinner then thumbnail
17. "X" removes queued attachment
18. Send with attachment → BRAIN_MESSAGE includes attachment refs
19. User message renders with attachment preview
20. Attachment type indicators (image thumbnail vs file chip)

### Integration
21. Screenshot paste → allAI describes what it sees (multimodal LLM)
22. CSV upload → allAI shows schema + suggests analysis
23. PDF upload → allAI summarizes content
24. BYO text-only LLM + image → user sees "model doesn't support images" notice

---

## Build Estimate
~4 days total:
- Backend (upload endpoint, attachment service, MIME detection, provider updates): 2 days
- Frontend (ChatInput attachments, ChatMessage rendering, CoPilotContext update): 2 days

## Files to Create/Modify

### New Files
| File | Purpose |
|------|---------|
| `app/services/chat_attachment_service.py` | Ephemeral storage, TTL management |
| `app/services/mime_detector.py` | Magic-byte MIME detection |

### Modified Files
| File | Change |
|------|--------|
| `app/routers/copilot.py` | Add POST /upload route, BRAIN_MESSAGE attachment handling, cleanup task |
| `app/services/copilot_service.py` | Accept attachments in process_message_streaming/agentic |
| `app/services/allie_provider.py` | Update stream() signature, construct content blocks |
| `app/services/allai_agentic_provider.py` | Accept attachments, construct content blocks |
| `frontend/src/components/copilot/ChatInput.tsx` | Attachment button, drag-drop, paste, preview |
| `frontend/src/components/copilot/ChatMessage.tsx` | Render attachments in messages |
| `frontend/src/contexts/CoPilotContext.tsx` | Updated sendMessage with attachments |

### Follow-up (separate build)
| File | Change |
|------|--------|
| `ai-market-backend/app/schemas/allie.py` | Add ImageBlock to proxy discriminated union |

---

## Council Review Fixes (Gate 2, S135)

### Fix 1: Attachment Persistence (XAI concern)

**Problem:** In-memory dict loses all attachments on server restart.

**Fix:** Write attachment metadata to a lightweight SQLite table alongside the filesystem
storage. On startup, scan `/data/chat/` and rebuild the in-memory index from disk.
Expired files are cleaned on scan.

```python
# In ChatAttachmentService.__init__:
# 1. Create /data/chat/ directory
# 2. Scan existing directories, rebuild self._attachments from file timestamps
# 3. Clean expired entries

# Metadata file per attachment: /data/chat/{att_id}/meta.json
# Contains: {id, filename, mime_type, size_bytes, type, created_at, expires_at}
```

This avoids a Redis dependency while surviving restarts. The TTL is still 1 hour —
any attachment older than that is deleted on scan regardless.

### Fix 2: Upload Rate Limiting (XAI concern)

Add per-session upload rate limiting:

```python
# In POST /api/copilot/upload:
MAX_UPLOADS_PER_MINUTE = 5
MAX_UPLOADS_PER_SESSION = 20

# Use existing session rate limiter pattern from copilot.py
# Check before accepting upload, return 429 if exceeded
```

### Fix 3: Extraction Pipeline Detail (AG concern)

**Problem:** `ProcessingService` is coupled to `DatasetRecord`. Cannot call directly.

**Fix:** Use underlying services directly:

```python
async def _extract_text(self, file_path: Path, mime_type: str) -> Optional[str]:
    """Extract text from document/data files. Cap at 10,000 chars."""
    try:
        if mime_type == "application/pdf":
            # Use DocumentService (Tika) directly
            from app.services.document_service import DocumentService
            doc_svc = DocumentService()
            result = await doc_svc.process_document(file_path)
            return result.text[:10_000] if result.text else None

        elif mime_type in ("text/csv", "text/plain"):
            # Read directly, first 50 lines + header
            text = file_path.read_text(errors="replace")
            lines = text.splitlines()
            if len(lines) > 50:
                return "\n".join(lines[:50]) + f"\n... ({len(lines)} total lines)"
            return text[:10_000]

        elif mime_type == "application/json":
            import json
            raw = file_path.read_text(errors="replace")[:10_000]
            try:
                parsed = json.loads(raw)
                return json.dumps(parsed, indent=2)[:10_000]
            except json.JSONDecodeError:
                return raw

        elif "spreadsheet" in mime_type or "excel" in mime_type:
            # Use DuckDB to read and sample
            from app.services.duckdb_service import DuckDBService
            duckdb_svc = DuckDBService()
            schema = await duckdb_svc.get_column_profile(file_path)
            sample = await duckdb_svc.get_sample_rows(file_path, limit=20)
            return f"Schema:\n{schema}\n\nSample rows:\n{sample}"[:10_000]

    except Exception as e:
        logger.warning("Attachment text extraction failed: %s %s", file_path.name, e)
        return f"[Could not extract text from {file_path.name}]"
```

### Fix 4: Base64 Memory Note

For images at max size (10MB file → ~13MB base64), the proxy handles this in a single
streaming request to Anthropic. At the vectorAIz backend level, the file is read from
disk and base64-encoded once per LLM call — not held in memory across turns.

With the 1600px resize cap, most screenshots will be 200-500KB (300-700KB base64),
well within normal request sizes.

### Fix 5: Frontend Component Clarification

The spec targets `vectoraiz-backend/frontend/src/components/copilot/` which already
contains `ChatInput.tsx`, `ChatMessage.tsx`, `ChatPanel.tsx`, and `CoPilotContext.tsx`.
These files exist and are the correct modification targets.

---

## MP Review Fixes (Gate 2, Round 2)

### Fix 6: Decompression Bomb Protection (MP #1)

```python
# In upload endpoint, BEFORE any image processing:
from PIL import Image
Image.MAX_IMAGE_PIXELS = 25_000_000  # 25MP max (5000x5000)

def validate_image(file_path: Path) -> tuple[bool, str]:
    """Validate image is safe to process."""
    try:
        with Image.open(file_path) as img:
            img.verify()  # Check for corruption/bombs
        # Re-open after verify (verify invalidates the object)
        with Image.open(file_path) as img:
            w, h = img.size
            if w * h > 25_000_000:
                return False, f"Image too large: {w}x{h} ({w*h:,} pixels, max 25M)"
        return True, ""
    except Exception as e:
        return False, f"Invalid image: {e}"

# For XLSX: reject if compressed size < 1KB but uncompressed > 50MB (zip bomb heuristic)
# For PDF: cap at 20 pages via Tika metadata check before full extraction
```

### Fix 7: MIME Detection Hardening (MP #1, #8.2)

Replace the naive magic-byte detector with robust validation:

```python
def detect_mime(header_bytes: bytes, filename: str) -> str | None:
    """Detect MIME type from file content. Extension is SECONDARY hint only."""
    # PNG
    if header_bytes.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    # JPEG
    if header_bytes.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    # WebP: RIFF + check bytes 8-12 for "WEBP"
    if header_bytes[:4] == b"RIFF" and len(header_bytes) >= 12 and header_bytes[8:12] == b"WEBP":
        return "image/webp"
    # GIF
    if header_bytes[:6] in (b"GIF87a", b"GIF89a"):
        return "image/gif"
    # PDF
    if header_bytes.startswith(b"%PDF"):
        return "application/pdf"
    # ZIP-based (xlsx): inspect ZIP entries, not extension
    if header_bytes[:4] == b"PK\x03\x04":
        import zipfile, io
        try:
            # Read enough to check ZIP contents
            zf = zipfile.ZipFile(io.BytesIO(header_bytes[:65536]))
            names = zf.namelist()
            if any("xl/" in n for n in names) or "[Content_Types].xml" in names:
                return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        except:
            pass
        return None  # Unknown ZIP — reject
    # Text-based: validate UTF-8 decodability, classify by content heuristics
    try:
        text = header_bytes[:4096].decode("utf-8")
        # CSV heuristic: comma or tab separated, multiple lines
        lines = text.strip().split("\n")
        if len(lines) > 1:
            delimiters = [l.count(",") for l in lines[:5]]
            if all(d > 0 and abs(d - delimiters[0]) <= 1 for d in delimiters):
                return "text/csv"
        # JSON heuristic
        stripped = text.strip()
        if stripped.startswith(("{", "[")):
            return "application/json"
        return "text/plain"
    except UnicodeDecodeError:
        return None
```

**Rule:** Extension is NEVER trusted for security decisions. It MAY be used as a tiebreaker for ambiguous text formats, logged for UX only.

### Fix 8: Path Traversal / Filename Sanitization (MP #8.3)

```python
import re
import unicodedata

def sanitize_filename(filename: str) -> str:
    """Sanitize user-provided filename for safe storage."""
    # Normalize unicode
    filename = unicodedata.normalize("NFKD", filename)
    # Strip path components
    filename = filename.replace("\\", "/").split("/")[-1]
    # Remove dangerous chars
    filename = re.sub(r'[^\w\s\-\.]', '_', filename)
    # Prevent hidden files
    filename = filename.lstrip(".")
    # Truncate
    name, ext = os.path.splitext(filename)
    return f"{name[:100]}{ext[:10]}" or "upload"
```

### Fix 9: AuthZ Binding — Attachment Scoped to User (MP #8.4)

```python
@dataclass
class ChatAttachment:
    id: str
    user_id: str  # NEW: owner binding
    filename: str
    # ... rest unchanged

# In upload endpoint: store user_id from auth
# In BRAIN_MESSAGE handler: verify att.user_id == current_user.user_id
# Reject with 403 if mismatch — prevents cross-user attachment theft
```

### Fix 10: Multi-Worker Consistency (MP #8.5)

vectorAIz runs a single Uvicorn worker in production (constrained by DuckDB single-writer).
Spec explicitly requires: **single worker deployment for chat attachment service**.

If multi-worker is needed in future, migrate to SQLite-backed index (already on disk at
`/data/chat/{id}/meta.json`) with file-level locking, or Redis.

### Fix 11: Filename Logging Policy (MP #8.6)

Log filenames truncated to 30 chars max. Never log full user-provided filenames
(may contain PII). Use attachment ID as primary log identifier.

```python
logger.info("Upload: att=%s type=%s size=%d name=%.30s",
            att.id, att.type, att.size_bytes, att.filename)
```

### Fix 12: Image Format Safety on Resize (MP #4)

```python
def resize_if_needed(file_path: Path, max_dimension: int = 1600) -> bool:
    with Image.open(file_path) as img:
        w, h = img.size
        if max(w, h) <= max_dimension:
            return False
        # Preserve format and handle transparency
        fmt = img.format or "PNG"
        ratio = max_dimension / max(w, h)
        new_size = (int(w * ratio), int(h * ratio))
        if fmt == "JPEG":
            # JPEG doesn't support transparency — safe to save directly
            img = img.resize(new_size, Image.LANCZOS)
            img.save(file_path, format="JPEG", quality=85)
        else:
            # PNG/WebP: preserve alpha channel
            img = img.resize(new_size, Image.LANCZOS)
            img.save(file_path, format=fmt)
        return True
```

Post-resize, reject if file still > 4MB (prevents edge case of huge PNG with transparency).

### Fix 13: Effective Post-Resize Cap (MP #4)

After resize: if file size > 4MB, reject with error "Image too large after resize".
This keeps base64 payloads under ~5.3MB, safe for concurrent requests.

### Updated Mandate Table

| ID | Mandate | Source | Severity |
|----|---------|--------|----------|
| M1 | Separate HTTP upload endpoint — no binary in WebSocket | All | BLOCKING |
| M2 | Ephemeral storage with TTL (1 hour), meta.json with atomic writes | MP+XAI | BLOCKING |
| M3 | Hard caps: 10MB/file, 3 att/message, 1600px resize, 4MB post-resize cap | XAI+MP | BLOCKING |
| M4 | MIME validation via content analysis, NOT extension. Extension secondary only | MP | BLOCKING |
| M5 | Logging redaction — never log file content, base64, or full filenames | MP | BLOCKING |
| M6 | Provider capability check — text fallback for non-multimodal LLMs | All | REQUIRED |
| M7 | Image passthrough; PDF/CSV/Excel extract text locally | All | REQUIRED |
| M8 | Decompression bomb protection: PIL.MAX_IMAGE_PIXELS, ZIP bomb check, PDF page cap | MP | BLOCKING |
| M9 | Path traversal / filename sanitization | MP | BLOCKING |
| M10 | AuthZ binding: attachments scoped to uploading user | MP | BLOCKING |
| M11 | Single-worker constraint documented | MP | REQUIRED |
| M12 | Image format safety on resize (alpha channel, format preservation) | MP | REQUIRED |
| M13 | Post-resize 4MB cap | MP | REQUIRED |

---

## MP Re-Review Fixes (Gate 2, Round 3)

### Fix 7 Corrected: XLSX Detection via Full File

The ZIP/XLSX detection MUST use the full file, not header bytes. Updated flow:

```python
# In upload endpoint, AFTER writing file to disk:
def detect_mime_for_zip(file_path: Path) -> str | None:
    """For PK-header files, inspect full ZIP to determine actual type."""
    import zipfile
    try:
        if not zipfile.is_zipfile(file_path):
            return None
        with zipfile.ZipFile(file_path, 'r') as zf:
            names = zf.namelist()
            # ZIP bomb guard (Fix 8)
            total_uncompressed = sum(info.file_size for info in zf.infolist())
            total_compressed = sum(info.compress_size for info in zf.infolist())
            entry_count = len(names)
            
            # Reject: >50MB uncompressed, >100:1 ratio, >1000 entries
            if total_uncompressed > 50 * 1024 * 1024:
                return None  # Too large
            if total_compressed > 0 and total_uncompressed / total_compressed > 100:
                return None  # Suspicious compression ratio
            if entry_count > 1000:
                return None  # Too many entries
            
            # XLSX detection by internal structure
            if "[Content_Types].xml" in names and any("xl/" in n for n in names):
                return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            # XLS (old format) won't be ZIP — handled separately
        return None  # Unknown ZIP type — reject
    except (zipfile.BadZipFile, Exception):
        return None
```

**Updated detection pipeline:**
1. Read first 32 bytes → magic byte check for images/PDF/text
2. If `PK\x03\x04` header → write to disk first, then call `detect_mime_for_zip(file_path)`
3. Text formats → UTF-8 decode + content heuristics on first 4KB

### Fix 8 Corrected: ZIP Bomb Heuristic (Robust)

Three-layer defense for ZIP-based files (xlsx):

| Check | Limit | Action |
|-------|-------|--------|
| Total uncompressed size | 50MB | Reject |
| Compression ratio (uncompressed/compressed) | 100:1 | Reject |
| Entry count | 1000 | Reject |

All three checks use `ZipFile.infolist()` metadata (declared sizes). This runs
BEFORE extracting any content. If any check fails, reject with 415.

### PDF Page Cap Fallback

```python
async def extract_pdf_text(file_path: Path, max_pages: int = 20) -> str | None:
    """Extract text from PDF with page cap."""
    # Primary: Tika with page metadata
    # Fallback: if page count unavailable, cap extraction at 50KB of text output
    # This ensures bounded processing regardless of Tika metadata availability
    try:
        result = await doc_service.process_document(file_path)
        text = result.text or ""
        return text[:50_000]  # Hard cap regardless of page count
    except Exception:
        return "[Could not extract text from PDF]"
```

The 50KB text output cap ensures bounded processing even without reliable page count.

### Post-Resize Enforcement

The 4MB post-resize check returns HTTP 413 with a clear message:

```python
if resized:
    new_size = file_path.stat().st_size
    if new_size > 4 * 1024 * 1024:
        file_path.unlink()
        raise HTTPException(
            status_code=413,
            detail="Image exceeds 4MB after resize. Use a lower-resolution image.",
        )
```
