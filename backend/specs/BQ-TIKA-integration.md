# BQ-TIKA: Apache Tika Integration — 28+ File Format Support

## Overview
Add Apache Tika as a Docker sidecar to expand vectorAIz from 13 file formats to 28+.
Tika runs as a self-contained REST server — zero external API calls, works fully offline in standalone mode.

## Strategic Value
- Strengthens beta positioning: "28+ formats" vs "13 formats"
- Covers enterprise formats (RTF, email archives, OpenDocument, iWork, XML/RSS)
- Free, open-source, self-hosted — no cost per page
- Fallback chain: Unstructured (best for PDF/Word layout) → Tika (everything else)

## Architecture

```
Upload → SUPPORTED_EXTENSIONS check → magic bytes → routing:
  ├── CSV/JSON/Parquet/Excel → existing pandas/DuckDB pipeline (unchanged)
  ├── PDF/DOCX/PPTX → Unstructured (existing, best layout extraction)
  └── RTF/EML/MSG/ODT/ODS/ODP/EPUB/XML/... → Tika (new)
                                                  │
                                    PUT http://tika:9998/tika
                                    Returns: extracted plain text
```

## Council Review — R2 (Final)

### Round 1 incorporated (XAI + MP):
- Docker resource limits (memory: 1G)
- Fallback logging (logger.warning on fallback, logger.error on total failure)
- Graceful failure mode (metadata-only return on all-processor failure)
- Healthcheck changed from curl
- get_processor() keyword-only filepath parameter
- TikaDocumentProcessor output matches LocalDocumentProcessor format
- SUPPORTED_EXTENSIONS sync across both files
- RTF BOM stripping in magic bytes check

### Round 2 fixes (MP):
- .msg: added OLE2 magic bytes (b"\xd0\xcf\x11\xe0") — NOT text-based
- Per-block metadata: exact match to LocalDocumentProcessor (type=classname, text=str, metadata={page_number, filename optional})
- Healthcheck: VERIFIED apache/tika:3.1.0.0 has bash but NO wget/curl — use bash TCP check
- RTF BOM: proper sequence strip (BOM first, then whitespace) not lstrip
- RSS/XML routing clarified: they ARE documents for Tika, route through _process_document()

---

## Implementation

### 1. Docker Compose — Add Tika Sidecar

Add to `docker-compose.customer.yml`:

```yaml
  tika:
    image: apache/tika:3.1.0.0
    deploy:
      resources:
        limits:
          memory: 1G
    healthcheck:
      # VERIFIED: apache/tika:3.1.0.0 is Ubuntu 24.10 with bash, NO wget/curl
      test: ["CMD-SHELL", "bash -c 'cat < /dev/null > /dev/tcp/localhost/9998'"]
      interval: 10s
      timeout: 5s
      retries: 5
      start_period: 15s
    restart: unless-stopped
```

Add `depends_on` to vectoraiz service:
```yaml
      tika:
        condition: service_healthy
```

Add env var to vectoraiz service:
```yaml
      - TIKA_URL=http://tika:9998
```

### 2. Config

In `app/config.py`, add:
```python
tika_url: Optional[str] = Field(None, env="TIKA_URL")
```

### 3. New Processor: `TikaDocumentProcessor`

File: `app/services/document_service.py`

**CRITICAL**: Output format MUST exactly match `LocalDocumentProcessor.process()` output.

Reference — LocalDocumentProcessor per-block format:
```python
{
    "type": type(element).__name__,  # e.g. "NarrativeText", "Title", "ListItem"
    "text": str(element),
    "metadata": {
        "page_number": meta.page_number,  # int or absent
        "filename": meta.filename,        # str or absent
    }
}
```

TikaDocumentProcessor implementation:
```python
class TikaDocumentProcessor(DocumentProcessor):
    """Process documents via Apache Tika REST API."""
    
    TIKA_TYPES = {
        'rtf', 'odt', 'ods', 'odp', 'epub',
        'eml', 'msg', 'mbox',
        'xml', 'rss',
        'pages', 'numbers', 'key',
        'wps', 'wpd',
        'ics', 'vcf',
    }
    
    def __init__(self, tika_url: str = "http://tika:9998"):
        self.tika_url = tika_url
    
    def supported_types(self) -> List[str]:
        return list(self.TIKA_TYPES)
    
    def process(self, filepath: Path) -> Dict[str, Any]:
        """PUT file to Tika, get extracted text. Returns SAME format as LocalDocumentProcessor."""
        import httpx
        
        with open(filepath, 'rb') as f:
            response = httpx.put(
                f"{self.tika_url}/tika",
                content=f.read(),
                headers={
                    "Accept": "text/plain",
                    "Content-Disposition": f'attachment; filename="{filepath.name}"',
                },
                timeout=120,
            )
        
        if response.status_code != 200:
            raise Exception(f"Tika processing failed: HTTP {response.status_code}")
        
        text = response.text.strip()
        
        # Split into paragraphs as text blocks
        paragraphs = [p.strip() for p in text.split('\n\n') if p.strip()]
        
        # Match LocalDocumentProcessor per-block format EXACTLY
        text_content = [
            {
                "type": "NarrativeText",  # Tika doesn't classify block types
                "text": p,
                "metadata": {
                    "page_number": 0,     # Tika plain text mode doesn't track pages
                    "filename": filepath.name,
                }
            }
            for p in paragraphs
        ]
        
        return {
            "text_content": text_content,
            "tables": [],
            "metadata": {
                "filename": filepath.name,
                "file_type": filepath.suffix[1:],
                "element_count": len(text_content),
                "text_blocks": len(text_content),
                "table_count": 0,
                "processed_at": datetime.utcnow().isoformat(),
                "processor": "tika",
            }
        }
```

### 4. Update DocumentService — Fallback Chain

Update `_init_processors()` to initialize Tika:
```python
tika_url = getattr(settings, 'tika_url', None) or os.environ.get('TIKA_URL')
if tika_url:
    try:
        self.tika_processor = TikaDocumentProcessor(tika_url)
        logger.info(f"Tika processor initialized: {tika_url}")
    except Exception as e:
        logger.warning(f"Tika processor unavailable: {e}")
        self.tika_processor = None
else:
    self.tika_processor = None
```

Update `get_processor()` — backward-compatible signature:
```python
def get_processor(
    self,
    prefer_premium: bool = False,
    *,                                    # keyword-only after this
    filepath: Optional[Path] = None,
) -> DocumentProcessor:
    file_type = filepath.suffix.lower()[1:] if filepath else None
    
    # Route Tika-specific formats to Tika
    if file_type and self.tika_processor and file_type in TikaDocumentProcessor.TIKA_TYPES:
        return self.tika_processor
    
    # Existing chain: premium → local
    if prefer_premium and self.premium_processor:
        return self.premium_processor
    if self.local_processor:
        return self.local_processor
    
    # Tika as final fallback
    if self.tika_processor:
        return self.tika_processor
    if self.premium_processor:
        return self.premium_processor
    
    raise RuntimeError("No document processor available")
```

Update `process_document()` to pass filepath + fallback + graceful failure:
```python
def process_document(self, filepath, prefer_premium=False):
    fp = Path(filepath)
    processor = self.get_processor(prefer_premium, filepath=fp)
    try:
        return processor.process(fp)
    except Exception as e:
        # Fallback: if primary processor fails, try Tika
        if self.tika_processor and not isinstance(processor, TikaDocumentProcessor):
            logger.warning(f"Primary processor failed for {fp.name}, falling back to Tika: {e}")
            try:
                return self.tika_processor.process(fp)
            except Exception as tika_err:
                logger.error(f"Tika fallback also failed for {fp.name}: {tika_err}")
        
        # Graceful failure: return metadata-only (no crash)
        logger.error(f"All processors failed for {fp.name}: {e}")
        return {
            "text_content": [],
            "tables": [],
            "metadata": {
                "filename": fp.name,
                "file_type": fp.suffix[1:] if fp.suffix else "unknown",
                "element_count": 0,
                "text_blocks": 0,
                "table_count": 0,
                "processor": "failed",
                "error": str(e),
            }
        }
```

### 5. Expand SUPPORTED_EXTENSIONS — BOTH files in sync

**`app/routers/datasets.py`** AND **`app/services/batch_service.py`** — MUST match:

```python
SUPPORTED_EXTENSIONS = {
    # Data formats (existing — pandas/DuckDB pipeline)
    '.csv', '.json', '.parquet', '.xlsx', '.xls',
    # Documents (existing — Unstructured)
    '.pdf', '.docx', '.doc', '.pptx', '.ppt',
    # Plain text (existing)
    '.txt', '.md', '.html',
    # Tika-powered document formats (NEW)
    '.rtf', '.odt', '.ods', '.odp', '.epub',
    '.eml', '.msg', '.mbox',
    '.xml', '.rss',
    '.pages', '.numbers', '.key',
    '.wps', '.wpd',
    '.ics', '.vcf',
}
```

### 6. Magic Bytes — new signatures + fixes

In `app/services/batch_service.py`, add to `_MAGIC_SIGNATURES`:
```python
# Tika format signatures
".rtf": [b"{\\rtf"],
".epub": [b"PK\x03\x04"],     # ZIP-based
".odt": [b"PK\x03\x04"],
".ods": [b"PK\x03\x04"],
".odp": [b"PK\x03\x04"],
".pages": [b"PK\x03\x04"],
".numbers": [b"PK\x03\x04"],
".key": [b"PK\x03\x04"],
".wps": [b"\xd0\xcf\x11\xe0"],  # OLE2
".msg": [b"\xd0\xcf\x11\xe0"],  # OLE2 (NOT text-based)
```

Skip magic check (no entry → returns True) for text-based only:
`.xml`, `.rss`, `.eml`, `.mbox`, `.ics`, `.vcf`, `.wpd`

**Fix RTF BOM handling** — update `_check_magic_bytes()`:
```python
def _check_magic_bytes(data: bytes, extension: str) -> bool:
    sigs = _MAGIC_SIGNATURES.get(extension)
    if sigs is None:
        return True  # No check for text formats
    
    check_data = data
    # RTF files may have UTF-8 BOM before the opening brace
    if extension == '.rtf':
        # Strip BOM sequence first (exact 3-byte sequence), then whitespace
        if check_data.startswith(b'\xef\xbb\xbf'):
            check_data = check_data[3:]
        check_data = check_data.lstrip(b' \t\r\n')
    
    return any(check_data.startswith(sig) for sig in sigs)
```

### 7. Update Processing Pipeline Routing

In `app/services/processing_service.py`, expand `DOCUMENT_TYPES`:

```python
DOCUMENT_TYPES = {
    # Existing (Unstructured)
    'pdf', 'docx', 'doc', 'pptx', 'ppt',
    # Tika-powered (NEW) — all route through _process_document()
    'rtf', 'odt', 'ods', 'odp', 'epub',
    'eml', 'msg', 'mbox',
    'xml', 'rss',
    'pages', 'numbers', 'key',
    'wps', 'wpd',
    'ics', 'vcf',
}
```

Note: xml/rss ARE routed as documents here. Tika extracts text content from XML/RSS feeds.
They do NOT go through the data pipeline (pandas/DuckDB) — they go through document pipeline → Parquet.

### 8. Update Frontend — DataTypesPage

In `frontend/src/pages/DataTypesPage.tsx`:
- Expand "Included Formats" grid: 28+ formats grouped by category
  - Data: CSV, JSON, Excel (.xlsx/.xls), Parquet (5)
  - Documents: PDF, Word (.docx/.doc), PowerPoint (.pptx/.ppt), RTF, ODT, ODS, ODP, Pages, Numbers, Keynote, WPS, WordPerfect (14)
  - Email: EML, MSG, MBOX (3)
  - Publishing: ePub (1)
  - Plain Text: TXT, Markdown, HTML (3)
  - Structured: XML, RSS, iCalendar, vCard (4)
- Premium section narrows to: OCR scanned PDFs, Salesforce exports, SharePoint, LLM-enhanced extraction
- Add "Powered by Apache Tika" attribution line

### 9. Update Frontend — FileUploadModal

Add all new extensions to the file input `accept` attribute or accepted types list.

---

## Files to Modify
1. `docker-compose.customer.yml` — add tika service + depends_on + TIKA_URL env
2. `app/config.py` — add tika_url setting
3. `app/services/document_service.py` — TikaDocumentProcessor class + fallback chain + graceful failure
4. `app/services/processing_service.py` — expand DOCUMENT_TYPES
5. `app/routers/datasets.py` — expand SUPPORTED_EXTENSIONS
6. `app/services/batch_service.py` — expand SUPPORTED_EXTENSIONS + magic bytes + RTF BOM fix
7. `frontend/src/pages/DataTypesPage.tsx` — update included/premium grids
8. `frontend/src/components/FileUploadModal.tsx` — add new extensions

## New Formats (15 new → 28 total)
| Format | Extension | Processor | Magic Check |
|--------|-----------|-----------|-------------|
| Rich Text | .rtf | Tika | b"{\\rtf" (BOM-tolerant) |
| OpenDocument Text | .odt | Tika | PK (ZIP) |
| OpenDocument Sheet | .ods | Tika | PK (ZIP) |
| OpenDocument Slides | .odp | Tika | PK (ZIP) |
| ePub | .epub | Tika | PK (ZIP) |
| Email Message | .eml | Tika | skip (text) |
| Outlook Email | .msg | Tika | OLE2 |
| Email Mailbox | .mbox | Tika | skip (text) |
| XML | .xml | Tika | skip (text) |
| RSS Feed | .rss | Tika | skip (text) |
| Apple Pages | .pages | Tika | PK (ZIP) |
| Apple Numbers | .numbers | Tika | PK (ZIP) |
| Apple Keynote | .key | Tika | PK (ZIP) |
| WPS Writer | .wps | Tika | OLE2 |
| WordPerfect | .wpd | Tika | skip (variable) |
| Calendar | .ics | Tika | skip (text) |
| vCard | .vcf | Tika | skip (text) |

## Effort Estimate
- Docker + config: 30 min
- TikaDocumentProcessor + fallback chain: 2 hours
- SUPPORTED_EXTENSIONS + magic bytes + routing: 1 hour
- Frontend (DataTypes + upload modal): 1.5 hours
- Testing: 1 hour
- **Total: ~6 hours**

## Acceptance Criteria
1. `docker compose -f docker-compose.customer.yml up -d` starts Tika alongside postgres/qdrant/vectoraiz
2. Tika healthcheck passes (bash TCP on 9998) and vectoraiz depends_on waits
3. Upload .rtf → processed to Parquet, searchable via SQL and semantic search
4. Upload .eml → extracted text stored as Parquet, queryable
5. Upload .odt → same pipeline as .docx
6. Upload .msg → magic bytes validated (OLE2), processed via Tika
7. Existing 13 formats unchanged (regression-free)
8. If Tika is unreachable, primary processor fallback works; if both fail, graceful metadata-only return
9. Data Types page shows 28+ "Included" formats with category grouping
10. Frontend build passes (`npm run build`)
11. Works in standalone mode (no external API calls)
