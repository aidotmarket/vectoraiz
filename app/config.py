"""
vectorAIz Application Configuration
=====================================

PURPOSE:
    Pydantic-Settings based configuration for the vectorAIz backend.
    All settings can be overridden via environment variables (VECTORAIZ_ prefix).

UPDATED:
    S94 (2026-02-07) - BQ-066 Sub-task 1: Added SECRET_KEY with Fernet
        auto-generation for API key encryption at rest.
    S130 (2026-02-13) - BQ-127: Air-Gap Architecture — added VECTORAIZ_MODE,
        local auth secrets, connected fallback, premium feature flags.
"""

import logging
from pydantic_settings import BaseSettings
from typing import List, Optional, Literal
from cryptography.fernet import Fernet

logger = logging.getLogger(__name__)

# BQ-127: Default ai.market URL used for mode inference
_DEFAULT_AI_MARKET_URL = "https://ai-market-backend-production.up.railway.app"


def _generate_fernet_key() -> str:
    """Generate a Fernet-compatible key for encryption at rest.

    WARNING: Auto-generated keys are ephemeral — they change on each restart.
    In production, set VECTORAIZ_SECRET_KEY env var to a persistent Fernet key.
    Generate one with: python -c "from cryptography.fernet import Fernet; print(Fernet.generate_key().decode())"
    """
    return Fernet.generate_key().decode()


class Settings(BaseSettings):
    """BQ-127: Settings now include operating mode and local auth configuration."""

    app_name: str = "vectorAIz"
    debug: bool = False  # S100: Default OFF for production safety

    # BQ-127: Operating mode — standalone (air-gapped) or connected (ai.market)
    mode: Literal["standalone", "connected"] = "standalone"

    # BQ-127: Connected fallback behavior (C4) — what happens when ai.market is unreachable
    connected_fallback: Literal["standalone", "refuse"] = "standalone"

    # BQ-127: Local auth secrets (C1 — separate from SECRET_KEY)
    apikey_hmac_secret: Optional[str] = None   # HMAC for local API key hashing
    local_auth_secret: Optional[str] = None    # JWT signing key (Phase 2, not used yet)

    # BQ-127: Premium feature flags (only relevant in connected mode)
    allai_enabled: bool = False
    marketplace_enabled: bool = False

    # ai.market platform integration
    ai_market_url: str = _DEFAULT_AI_MARKET_URL
    auth_enabled: bool = True  # S100: Default ON. Set VECTORAIZ_AUTH_ENABLED=false only for local dev.
    auth_cache_ttl: int = 300 # 5 minutes in seconds

    # Service-to-service auth (for internal endpoints on ai-market-backend)
    internal_api_key: Optional[str] = None
    
    # Encryption key for API keys at rest (BQ-066)
    # If not set, auto-generates a Fernet key.
    # WARNING: Auto-generated keys are ephemeral — encrypted data is lost on restart.
    # In production, always set VECTORAIZ_SECRET_KEY to a persistent Fernet key.
    secret_key: Optional[str] = None

    # BQ-125: Previous SECRET_KEY for dual-decrypt during key rotation.
    # Set VECTORAIZ_PREVIOUS_SECRET_KEY during transition period, remove after re-encryption.
    previous_secret_key: Optional[str] = None

    # BQ-102: Device identity keystore
    # Passphrase for encrypting private keys in the local keystore.
    # REQUIRED in production — startup will fail without it.
    keystore_passphrase: Optional[str] = None  # SecretStr-equivalent via env var
    # Path to keystore file — defaults to persistent data volume for Docker.
    keystore_path: str = "/data/keystore.json"

    # Co-Pilot metering (BQ-073)
    # Markup rate applied to Anthropic wholesale cost.
    # 3.0 = 300% of wholesale → e.g. $0.01 wholesale → $0.03 customer cost.
    copilot_markup_rate: float = 3.0
    # Minimum cost per query in cents (ensures even tiny queries incur a charge)
    copilot_min_cost_cents: int = 1
    # Estimated cost of an average Co-Pilot query in cents (for pre-flight checks)
    copilot_estimated_query_cost_cents: int = 3
    
    # DuckDB settings
    duckdb_memory_limit: str = "12GB"
    duckdb_threads: int = 8
    data_directory: str = "/data"
    
    # Upload settings
    upload_directory: str = "/data/uploads"
    processed_directory: str = "/data/processed"
    chunk_size: int = 1024 * 1024  # 1MB chunks for streaming
    
    # Qdrant settings
    qdrant_host: str = "qdrant"
    qdrant_port: int = 6333
    
    # Document processing (optional premium)
    unstructured_api_key: Optional[str] = None

    # Apache Tika sidecar (BQ-TIKA)
    tika_url: Optional[str] = None
    
    # Stripe billing (BQ-098)
    stripe_secret_key: Optional[str] = None
    stripe_price_id: Optional[str] = None
    stripe_webhook_secret: Optional[str] = None
    billing_markup_rate: float = 3.0
    
    # Public URL for this vectorAIz instance (used in OpenAPI specs for Custom GPT Actions)
    public_url: str = "https://vectoraiz-backend-production.up.railway.app"

    # LLM Settings (BYO-Key)
    llm_provider: Literal["gemini", "openai", "anthropic"] = "gemini"
    llm_model: str = "gemini-1.5-flash"  # Default model
    llm_temperature: float = 0.2  # Lower for factual RAG responses
    llm_max_tokens: int = 1024
    gemini_api_key: Optional[str] = None
    google_genai_use_gca: bool = False  # AG-002: Vertex AI support
    openai_api_key: Optional[str] = None
    anthropic_api_key: Optional[str] = None
    
    # BQ-MCP-RAG: External LLM Connectivity (§4.5)
    connectivity_enabled: bool = False          # Off by default — customer must opt in
    connectivity_bind_host: str = "127.0.0.1"  # Loopback only by default
    connectivity_max_tokens: int = 10
    connectivity_rate_limit_rpm: int = 30       # Per-token requests/min
    connectivity_rate_limit_sql_rpm: int = 10   # Per-token SQL requests/min
    connectivity_rate_limit_global_rpm: int = 120
    connectivity_rate_limit_auth_fail: int = 5  # Auth failures/min per IP before block
    connectivity_max_concurrent: int = 3        # Per-token concurrency cap
    connectivity_sql_timeout_s: int = 10
    connectivity_sql_max_rows: int = 500
    connectivity_sql_memory_mb: int = 256
    connectivity_sql_max_length: int = 4096

    # BQ-VZ-LARGE-FILES: Streaming/chunked processing for large files
    large_file_threshold_mb: int = 100           # Files above this use streaming path
    fallback_max_size_mb: int = 200              # Max file size (MB) for in-memory fallback on streaming failure
    process_worker_memory_limit_mb: int = 2048   # Per-worker memory cap
    process_worker_timeout_s: int = 1800         # 30 min per file default
    process_worker_grace_period_s: int = 60      # Seconds for checkpoint flush after SIGTERM
    process_worker_max_concurrent: int = 2       # Max parallel workers
    duckdb_memory_limit_mb: int = 512            # DuckDB in-memory budget for streaming path
    max_upload_size_gb: int = 1000               # Safety valve only — local app, disk is the real limit
    streaming_queue_maxsize: int = 8             # Backpressure queue depth
    streaming_batch_target_rows: int = 50000     # Target rows per RecordBatch
    parquet_row_group_size_mb: int = 64           # Target row group size for ParquetWriter

    # BQ-VZ-DB-CONNECT: Database extraction limits
    db_extract_max_rows: int = 5_000_000  # Max rows per extraction (M3)

    # BQ-VZ-SERIAL-CLIENT: Serial activation & metering
    aimarket_url: str = _DEFAULT_AI_MARKET_URL  # ai-market serial authority base URL
    app_version: str = os.environ.get("VECTORAIZ_VERSION", "dev")
    serial_data_dir: str = "/data"  # Directory for serial.json + pending_usage.jsonl

    # CORS
    cors_origins: List[str] = ["http://localhost:5173", "http://localhost:3000", "http://localhost:8080", "https://vectoraiz-frontend-production.up.railway.app", "https://dev.vectoraiz.com", "https://vectoraiz.com", "https://www.vectoraiz.com", "https://vectoraiz-website-production.up.railway.app"]
    
    class Config:
        env_file = ".env"
        env_prefix = "VECTORAIZ_"

    def get_secret_key(self) -> str:
        """Return the SECRET_KEY, auto-generating if not set.
        
        Uses Fernet.generate_key() for auto-generation so the key is always
        valid for Fernet encryption/decryption. Logs a warning when auto-generating
        since the key won't survive restarts.
        
        Returns:
            A Fernet-compatible key string.
        """
        if self.secret_key:
            return self.secret_key
        
        # Auto-generate and cache on instance
        logger.warning(
            "SECRET_KEY not set — auto-generating ephemeral Fernet key. "
            "Encrypted data will be LOST on restart. "
            "Set VECTORAIZ_SECRET_KEY in production: "
            "python -c \"from cryptography.fernet import Fernet; print(Fernet.generate_key().decode())\""
        )
        self.secret_key = _generate_fernet_key()
        return self.secret_key


settings = Settings()

# ---------------------------------------------------------------------------
# BQ-127: Mode inference for backward compatibility (C6)
# If VECTORAIZ_MODE is NOT explicitly set but VECTORAIZ_AI_MARKET_URL IS set
# to a non-default value, infer connected mode and log a deprecation warning.
# ---------------------------------------------------------------------------
import os as _os

if not _os.environ.get("VECTORAIZ_MODE") and _os.environ.get("VECTORAIZ_AI_MARKET_URL"):
    if settings.ai_market_url != _DEFAULT_AI_MARKET_URL:
        settings.mode = "connected"
        logger.warning(
            "VECTORAIZ_MODE not set but AI_MARKET_URL detected — defaulting to connected. "
            "Set VECTORAIZ_MODE=connected explicitly. This inference will be removed in v2.0."
        )

logger.info("vectorAIz operating mode: %s", settings.mode)
