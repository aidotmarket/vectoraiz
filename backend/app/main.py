from concurrent.futures import ThreadPoolExecutor
from contextlib import asynccontextmanager
from fastapi import FastAPI, Request, Depends
from fastapi.middleware.cors import CORSMiddleware
from fastapi.openapi.utils import get_openapi
from fastapi.responses import JSONResponse
import logging
import asyncio
import os

from app.config import settings

# BQ-127: Stock routers — always imported regardless of mode
from app.routers import health, datasets, search, sql, vectors, pii, docs, llm_admin, diagnostics
from app.routers import auth as auth_router_module
from app.auth.api_key_auth import get_current_user
from app.core.database import init_db, close_db
from app.core.structured_logging import setup_logging
from app.core.errors import VectorAIzError
from app.core.errors.registry import error_registry
from app.core.errors.middleware import vectoraiz_error_handler
from app.core.log_middleware import CorrelationMiddleware
from app.core.issue_tracker import issue_tracker
from app.core.resource_guards import resource_monitor_loop, ensure_log_fallback
from app.services.deduction_queue import deduction_queue

# BQ-127 (C5): Premium modules are NOT imported at module level.
# DeviceCrypto, register_with_marketplace, stripe_connect_proxy,
# allai, billing, integrations, webhooks are lazy-imported in connected mode only.

# BQ-123A: Initialize structured logging before any logger calls
setup_logging()

logger = logging.getLogger(__name__)

# API metadata
API_TITLE = "vectorAIz API"
API_VERSION = "1.2.0"  # BQ-127: Air-Gap Architecture

# BQ-127 (§7): Mode-aware API descriptions
API_DESCRIPTION_STANDALONE = """
## vectorAIz - Data Processing & Semantic Search

Upload, process, vectorize, and search your data using your own LLM.
Runs entirely on your infrastructure with no internet required.

### Quick Start
1. Access the web interface at http://your-hostname
2. Complete the setup wizard (create admin account)
3. Upload data files
4. Configure your LLM provider (Settings > LLM)
5. Search and query your data

### Authentication

All data endpoints require authentication via API key.
Include in requests: `X-API-Key: vz_your_key_here`

### Premium Features
Set VECTORAIZ_MODE=connected to enable ai.market integration:
- allAI intelligent data assistant
- Premium data connectors
- Marketplace listing & discovery
"""

API_DESCRIPTION_CONNECTED = """
## vectorAIz - Data Processing & Semantic Search (Connected)

Full-featured mode with ai.market integration for premium features,
billing, and marketplace access.

### Authentication

All data endpoints require authentication via API key.
- Local keys: `X-API-Key: vz_your_key_here`
- Marketplace keys: `X-API-Key: aim_your_key_here`

### Additional Features
- allAI intelligent assistant for data exploration
- Premium data connectors
- List your data on ai.market for discovery and sale
- Usage-based billing via ai.market
"""

API_DESCRIPTION = (
    API_DESCRIPTION_CONNECTED if settings.mode == "connected"
    else API_DESCRIPTION_STANDALONE
)

# Tag metadata for organizing endpoints
TAGS_METADATA = [
    {
        "name": "health",
        "description": "Health check and readiness endpoints for monitoring. No authentication required.",
    },
    {
        "name": "datasets",
        "description": "Dataset upload, processing, and management. Supports CSV, JSON, Parquet, PDF, Word, Excel, and PowerPoint files. **Requires API Key.**",
    },
    {
        "name": "search",
        "description": "Semantic search using natural language queries. Powered by sentence-transformers embeddings and Qdrant vector database. This is a read-only, public endpoint.",
    },
    {
        "name": "allai",
        "description": "RAG (Retrieval-Augmented Generation) powered Q&A. Ask questions and get AI-generated answers grounded in your indexed datasets. **Requires API Key for generation.**",
    },
    {
        "name": "sql",
        "description": "SQL query interface for power users. Execute SELECT queries directly against your processed datasets. **Requires API Key.**",
    },
    {
        "name": "vectors",
        "description": "Vector database management. Create, inspect, and delete Qdrant collections. **Requires API Key.**",
    },
    {
        "name": "pii",
        "description": "PII (Personally Identifiable Information) detection using Microsoft Presidio. Scan datasets for sensitive data. **Requires API Key.**",
    },
    {
        "name": "documentation",
        "description": "API documentation, usage guides, and exportable collections. No authentication required.",
    },
    {
        "name": "External Connectivity",
        "description": "External LLM connectivity endpoints (MCP + REST). Authenticated via Bearer token (vzmcp_...). Allows external AI tools to search, query, and explore your datasets.",
    },
    {
        "name": "Connectivity Management",
        "description": "Internal management endpoints for the Settings > Connectivity page. Authenticated via session/API key.",
    },
]


async def queue_processor_loop():
    while True:
        processed = await deduction_queue.process_all_pending()
        logger.debug(f"Queue processor: processed {processed} items")
        await asyncio.sleep(30)  # every 30 seconds


@asynccontextmanager
async def lifespan(app: FastAPI):
    """
    Application lifespan manager.
    Handles startup and shutdown events.
    """
    # Startup
    logger.info(
        "Starting vectorAIz API v%s in %s mode...",
        API_VERSION, settings.mode.upper(),
    )

    # BQ-123A: Load error registry + issue tracker
    error_registry.load()
    issue_tracker.reload()
    ensure_log_fallback()

    # BQ-116: Co-Pilot requires single-worker mode
    web_concurrency = int(os.environ.get("WEB_CONCURRENCY", "1"))
    uvicorn_workers = int(os.environ.get("UVICORN_WORKERS", "1"))
    if web_concurrency > 1 or uvicorn_workers > 1:
        logger.critical(
            f"Co-Pilot requires single-worker mode but "
            f"WEB_CONCURRENCY={web_concurrency}, UVICORN_WORKERS={uvicorn_workers}"
        )
        raise RuntimeError("Co-Pilot requires single-worker mode")

    # BQ-116: File lock to prevent multiple processes
    import fcntl
    _lock_path = "/var/tmp/vectoraiz_copilot.lock"
    _lock_file = open(_lock_path, "w")
    try:
        fcntl.flock(_lock_file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        logger.info("Acquired single-worker lock: %s", _lock_path)
    except OSError:
        _lock_file.close()
        logger.critical("Another vectoraiz process is already running (lock: %s)", _lock_path)
        raise RuntimeError("Co-Pilot requires single-worker mode")

    # BQ-110: Configure thread pool for run_sync() / asyncio.to_thread()
    executor = ThreadPoolExecutor(max_workers=32)
    loop = asyncio.get_running_loop()
    loop.set_default_executor(executor)
    logger.info("ThreadPoolExecutor configured (max_workers=32)")

    init_db()  # Initialize SQLite databases + run Alembic migrations
    logger.info("Database initialized")

    # BQ-111: Auto-migrate datasets.json → SQL on first startup
    try:
        from app.scripts.migrate_json import migrate_datasets_json
        from app.core.database import get_engine
        migrate_datasets_json(get_engine())
    except Exception as e:
        logger.error("datasets.json migration error (will retry next startup): %s", e)

    # BQ-127: Device registration and marketplace connect only in connected mode
    if settings.mode == "connected":
        # BQ-102: Initialize device cryptographic identity
        from app.core.crypto import DeviceCrypto
        from app.services.registration_service import register_with_marketplace

        if settings.keystore_passphrase:
            try:
                crypto = DeviceCrypto(
                    keystore_path=settings.keystore_path,
                    passphrase=settings.keystore_passphrase,
                )
                crypto.get_or_create_keypairs()
                logger.info("Device keypairs initialized (Ed25519 + X25519)")

                # BQ-102 ST-3: Register with ai.market (non-blocking background task)
                async def _register_background():
                    try:
                        await register_with_marketplace(crypto)
                    except Exception as e:
                        logger.warning(f"Background registration failed: {e}")

                asyncio.create_task(_register_background())
            except Exception as e:
                logger.error(f"Failed to initialize device keypairs: {e}")
        else:
            logger.warning(
                "VECTORAIZ_KEYSTORE_PASSPHRASE not set — device keypair generation skipped. "
                "Set this env var to enable Trust Channel device registration."
            )
    else:
        logger.info("Standalone mode — skipping device registration and marketplace connect.")

    # BQ-110: Start queue processor with cancellation support
    queue_task = asyncio.create_task(queue_processor_loop())

    # BQ-123A: Start resource monitor (disk/memory checks every 60s)
    resource_task = asyncio.create_task(resource_monitor_loop())

    # BQ-ALLAI-FILES: Start chat attachment cleanup (every 10 min)
    async def _attachment_cleanup_loop():
        from app.services.chat_attachment_service import chat_attachment_service
        while True:
            await asyncio.sleep(600)
            chat_attachment_service.cleanup_expired()

    attachment_cleanup_task = asyncio.create_task(_attachment_cleanup_loop())

    yield

    # Shutdown
    logger.info("Shutting down vectorAIz API...")

    # BQ-123A: Persist issue tracker state
    issue_tracker.persist()

    # BQ-123A: Cancel resource monitor
    resource_task.cancel()
    try:
        await resource_task
    except asyncio.CancelledError:
        pass

    # BQ-ALLAI-FILES: Cancel attachment cleanup
    attachment_cleanup_task.cancel()
    try:
        await attachment_cleanup_task
    except asyncio.CancelledError:
        pass

    # BQ-110: Cancel queue processor gracefully
    queue_task.cancel()
    try:
        await queue_task
    except asyncio.CancelledError:
        logger.info("Queue processor cancelled")

    # BQ-127: Only close stripe proxy in connected mode
    if settings.mode == "connected":
        from app.services.stripe_connect_proxy import close_proxy_client
        await close_proxy_client()
    close_db()
    executor.shutdown(wait=False)

    # BQ-116: Release single-worker lock
    try:
        import fcntl
        fcntl.flock(_lock_file, fcntl.LOCK_UN)
        _lock_file.close()
    except Exception:
        pass

    logger.info("Database connection closed")


def create_app() -> FastAPI:
    """Create and configure the FastAPI application."""
    
    app = FastAPI(
        title=API_TITLE,
        description=API_DESCRIPTION,
        version=API_VERSION,
        openapi_tags=TAGS_METADATA,
        docs_url="/docs",
        redoc_url="/redoc",
        openapi_url="/openapi.json",
        lifespan=lifespan,  # Use lifespan for startup/shutdown
        contact={
            "name": "AI.Market Support",
            "url": "https://ai.market/support",
            "email": "support@ai.market",
        },
        license_info={
            "name": "Proprietary",
            "url": "https://ai.market/license",
        },
    )
    
    # CORS middleware
    app.add_middleware(
        CORSMiddleware,
        allow_origins=settings.cors_origins,
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )

    # BQ-123A: Correlation ID middleware (request_id + correlation_id in every log)
    app.add_middleware(CorrelationMiddleware)

    # BQ-123A: Structured error handler for VectorAIzError
    app.add_exception_handler(VectorAIzError, vectoraiz_error_handler)

    # Catch-all handler so unhandled exceptions return JSON (not bare text)
    @app.exception_handler(Exception)
    async def _unhandled_exception_handler(request: Request, exc: Exception):
        logger.error("Unhandled exception on %s %s: %s", request.method, request.url.path, exc, exc_info=True)
        return JSONResponse(
            status_code=500,
            content={"detail": "Internal Server Error"},
        )

    # Protected Routes Dependency
    protected_route_dependency = [Depends(get_current_user)]

    # ------------------------------------------------------------------
    # BQ-127: Register routers — stock (always) vs connected (conditional)
    # ------------------------------------------------------------------

    # ALWAYS mount — stock features (work in both standalone and connected)
    app.include_router(health.router, prefix="/api", tags=["health"])
    app.include_router(diagnostics.router, prefix="/api", tags=["diagnostics"])
    app.include_router(docs.router, prefix="/api/docs", tags=["documentation"])
    app.include_router(
        auth_router_module.router,
        prefix="/api/auth",
        tags=["auth"],
    )
    app.include_router(
        search.router,
        prefix="/api/search",
        tags=["search"],
    )

    # Protected stock routers
    app.include_router(
        datasets.router,
        prefix="/api/datasets",
        tags=["datasets"],
        dependencies=protected_route_dependency,
    )
    app.include_router(
        vectors.router,
        prefix="/api/vectors",
        tags=["vectors"],
        dependencies=protected_route_dependency,
    )
    app.include_router(
        sql.router,
        prefix="/api/sql",
        tags=["sql"],
        dependencies=protected_route_dependency,
    )
    app.include_router(
        pii.router,
        prefix="/api/pii",
        tags=["pii"],
        dependencies=protected_route_dependency,
    )
    app.include_router(
        llm_admin.router,
        prefix="/api/admin/llm",
        tags=["LLM Admin"],
        dependencies=protected_route_dependency,
    )

    # BQ-128: Co-Pilot routers (REST + WebSocket) — always mounted
    from app.routers.copilot import router as copilot_rest_router, ws_router as copilot_ws_router
    app.include_router(
        copilot_rest_router,
        prefix="/api/copilot",
        tags=["copilot"],
        dependencies=protected_route_dependency,
    )
    app.include_router(copilot_ws_router)  # WebSocket at /ws/copilot (no prefix)

    # BQ-127 (C5): CONNECTED MODE ONLY — lazy imports to avoid loading
    # premium deps (Stripe, billing, integrations) in standalone mode.
    if settings.mode == "connected":
        from app.routers import allai, billing, integrations, webhooks

        app.include_router(
            webhooks.router,
            prefix="/api/webhooks",
            tags=["webhooks"],
        )
        app.include_router(
            allai.router,
            prefix="/api/allai",
            tags=["allai"],
        )
        app.include_router(
            billing.router,
            prefix="/api",
            tags=["billing", "api-keys"],
            dependencies=protected_route_dependency,
        )
        app.include_router(
            integrations.router,
            prefix="/api/integrations",
            tags=["integrations"],
            dependencies=protected_route_dependency,
        )
        logger.info("Connected mode: premium routers mounted (allai, billing, integrations, webhooks)")
    else:
        logger.info("Standalone mode: premium routers NOT mounted")

    # BQ-MCP-RAG Phase 3: Connectivity management endpoints (always mounted for Settings UI)
    from app.routers.connectivity_mgmt import router as connectivity_mgmt_router
    app.include_router(
        connectivity_mgmt_router,
        prefix="/api/connectivity",
        tags=["Connectivity Management"],
        dependencies=protected_route_dependency,
    )

    # BQ-MCP-RAG: External LLM Connectivity — conditionally mount
    if settings.connectivity_enabled:
        from app.routers.ext import router as ext_router
        from app.routers.mcp import mount_mcp_sse

        app.include_router(ext_router, tags=["External Connectivity"])
        mount_mcp_sse(app)
        logger.info("BQ-MCP-RAG: External connectivity routers mounted (REST + MCP SSE)")
    else:
        logger.info("BQ-MCP-RAG: External connectivity disabled (CONNECTIVITY_ENABLED=false)")

    # Root endpoint
    @app.get("/", tags=["health"], summary="API Root", description="Returns basic API information and links to documentation.")
    async def root():
        return {
            "name": API_TITLE,
            "version": API_VERSION,
            "mode": settings.mode,
            "status": "running",
            "docs": {
                "swagger": "/docs",
                "redoc": "/redoc",
                "openapi": "/openapi.json",
                "postman": "/api/docs/postman",
                "guide": "/api/docs/guide",
            },
        }
    
    return app


# Create the app instance
app = create_app()
