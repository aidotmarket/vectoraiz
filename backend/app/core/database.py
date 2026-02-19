"""
Database Configuration
======================

Persistent SQL database for vectorAIz state.

Supports two backends via DATABASE_URL env var:
  - SQLite (default): sqlite:///data/vectoraiz.db
  - PostgreSQL: postgresql://user:pass@host/db

SQLite extras: WAL mode, busy_timeout=5000, check_same_thread=False,
retry on SQLITE_BUSY up to 3× with jittered backoff.

Phase: BQ-111 — Persistent State
Created: 2026-02-12
"""

import logging
import os
import random
import time
from contextlib import contextmanager
from pathlib import Path
from typing import Generator, Optional

from sqlalchemy import event, text
from sqlalchemy.engine import Engine
from sqlalchemy.exc import OperationalError
from sqlmodel import Session as SQLModelSession, SQLModel, create_engine

from app.config import settings

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# DATABASE_URL resolution
# ---------------------------------------------------------------------------
_DEFAULT_SQLITE_PATH = Path(settings.data_directory) / "vectoraiz.db"
DATABASE_URL: str = os.environ.get(
    "DATABASE_URL",
    f"sqlite:///{_DEFAULT_SQLITE_PATH}",
)

_is_sqlite = DATABASE_URL.startswith("sqlite")

# ---------------------------------------------------------------------------
# Engine (lazy singleton)
# ---------------------------------------------------------------------------
_engine: Optional[Engine] = None

# Legacy compat: keep the old state DB path for init_db()
_LEGACY_DATABASE_DIR = Path(settings.data_directory)
_LEGACY_DATABASE_FILE = _LEGACY_DATABASE_DIR / "vai_state.db"
_LEGACY_DATABASE_URL = f"sqlite:///{_LEGACY_DATABASE_FILE}"
_legacy_engine: Optional[Engine] = None


def _build_engine(url: str) -> Engine:
    """Create an engine appropriate for the database backend."""
    if url.startswith("sqlite"):
        # Ensure parent directory exists
        db_path = url.replace("sqlite:///", "")
        Path(db_path).parent.mkdir(parents=True, exist_ok=True)

        engine = create_engine(
            url,
            echo=settings.debug,
            connect_args={"check_same_thread": False},
        )

        @event.listens_for(engine, "connect")
        def _set_sqlite_pragmas(dbapi_conn, _connection_record):
            cursor = dbapi_conn.cursor()
            cursor.execute("PRAGMA journal_mode=WAL")
            cursor.execute("PRAGMA busy_timeout=5000")
            cursor.close()

        return engine

    # PostgreSQL (or other)
    return create_engine(
        url,
        echo=settings.debug,
        pool_size=5,
        max_overflow=10,
    )


def get_engine() -> Engine:
    """Get or create the primary database engine (BQ-111 tables)."""
    global _engine
    if _engine is None:
        _engine = _build_engine(DATABASE_URL)
        logger.info("Database engine created: %s", DATABASE_URL.split("@")[-1] if "@" in DATABASE_URL else DATABASE_URL)
    return _engine


def get_legacy_engine() -> Engine:
    """Get or create the legacy vai_state.db engine (sessions, messages, prefs)."""
    global _legacy_engine
    if _legacy_engine is None:
        _LEGACY_DATABASE_DIR.mkdir(parents=True, exist_ok=True)
        _legacy_engine = create_engine(
            _LEGACY_DATABASE_URL,
            echo=settings.debug,
            connect_args={"check_same_thread": False},
        )
        logger.info("Legacy database engine created: %s", _LEGACY_DATABASE_FILE)
    return _legacy_engine


# ---------------------------------------------------------------------------
# SQLite retry helper
# ---------------------------------------------------------------------------
_SQLITE_MAX_RETRIES = 3
_SQLITE_BACKOFF_MIN_MS = 100
_SQLITE_BACKOFF_MAX_MS = 500


def _sqlite_retry(fn):
    """Execute *fn* with SQLite BUSY retry (up to 3×, jittered backoff)."""
    last_exc: Optional[Exception] = None
    for attempt in range(_SQLITE_MAX_RETRIES):
        try:
            return fn()
        except OperationalError as exc:
            if "database is locked" not in str(exc).lower():
                raise
            last_exc = exc
            delay = random.randint(_SQLITE_BACKOFF_MIN_MS, _SQLITE_BACKOFF_MAX_MS) / 1000
            logger.warning(
                "SQLITE_BUSY retry %d/%d — sleeping %.3fs",
                attempt + 1,
                _SQLITE_MAX_RETRIES,
                delay,
            )
            time.sleep(delay)
    raise last_exc  # type: ignore[misc]


# ---------------------------------------------------------------------------
# Session helpers
# ---------------------------------------------------------------------------

def get_session() -> Generator[SQLModelSession, None, None]:
    """
    FastAPI dependency — yields a DB session for the BQ-111 database.

    Usage::

        @app.get("/items")
        def get_items(db: Session = Depends(get_session)):
            ...
    """
    engine = get_engine()
    with SQLModelSession(engine) as session:
        yield session


@contextmanager
def get_session_context():
    """
    Context manager for non-FastAPI code.

    Usage::

        with get_session_context() as session:
            session.exec(...)
    """
    engine = get_engine()
    with SQLModelSession(engine) as session:
        yield session


def get_legacy_session() -> Generator[SQLModelSession, None, None]:
    """FastAPI dependency for the legacy vai_state.db (sessions, prefs)."""
    engine = get_legacy_engine()
    with SQLModelSession(engine) as session:
        yield session


@contextmanager
def get_legacy_session_context():
    """Context manager for the legacy vai_state.db."""
    engine = get_legacy_engine()
    with SQLModelSession(engine) as session:
        yield session


# ---------------------------------------------------------------------------
# Initialization
# ---------------------------------------------------------------------------

def init_db() -> None:
    """
    Initialize databases at application startup.

    1. Legacy vai_state.db — sessions, messages, user_preferences (SQLite, non-fatal).
    2. BQ-111 tables — datasets, billing, api_keys, deductions (via Alembic on DATABASE_URL).
    """
    # --- Legacy tables (sessions, messages, prefs) — non-fatal ---
    # This is ephemeral session data. If the SQLite path is not writable
    # (e.g. Railway volume permissions), the app can still function.
    try:
        legacy_engine = get_legacy_engine()
        from app.models.state import Session, Message, UserPreferences  # noqa: F401
        SQLModel.metadata.create_all(legacy_engine)
        _ensure_default_preferences(legacy_engine)
        _migrate_legacy_bq128(legacy_engine)
        logger.info("Legacy database tables initialized (vai_state.db)")
    except Exception as exc:
        logger.warning(
            "Legacy database init failed (non-fatal, sessions will not persist): %s", exc
        )

    # --- BQ-111+ tables (managed by Alembic on DATABASE_URL) ---
    _run_alembic_upgrade()


def _migrate_legacy_bq128(engine: Engine) -> None:
    """Add BQ-128 columns to legacy tables if missing (SQLite ALTER TABLE)."""
    from sqlalchemy import inspect
    insp = inspect(engine)

    # Sessions: add user_id
    session_cols = {c["name"] for c in insp.get_columns("sessions")}
    with engine.connect() as conn:
        if "user_id" not in session_cols:
            conn.execute(text("ALTER TABLE sessions ADD COLUMN user_id VARCHAR"))
            conn.commit()
            logger.info("BQ-128: Added user_id column to sessions table")

    # Messages: add kind, client_message_id, usage fields
    msg_cols = {c["name"] for c in insp.get_columns("messages")}
    new_msg_cols = {
        "kind": "VARCHAR DEFAULT 'chat'",
        "client_message_id": "VARCHAR(64)",
        "input_tokens": "INTEGER",
        "output_tokens": "INTEGER",
        "cost_cents": "INTEGER",
        "provider": "VARCHAR(32)",
        "model": "VARCHAR(64)",
    }
    with engine.connect() as conn:
        for col_name, col_def in new_msg_cols.items():
            if col_name not in msg_cols:
                conn.execute(text(f"ALTER TABLE messages ADD COLUMN {col_name} {col_def}"))
                logger.info("BQ-128: Added %s column to messages table", col_name)
        conn.commit()

    # BQ-128 Phase 4: Partial unique index for message idempotency (on legacy DB)
    with engine.connect() as conn:
        try:
            conn.execute(text(
                "CREATE UNIQUE INDEX IF NOT EXISTS ix_messages_session_client_msg_id "
                "ON messages (session_id, client_message_id) "
                "WHERE client_message_id IS NOT NULL"
            ))
            conn.commit()
        except Exception as idx_err:
            logger.warning("BQ-128 P4: Idempotency index creation skipped: %s", idx_err)

    # UserPreferences: add user_id column if missing (migration from singleton to per-user)
    prefs_cols = {c["name"] for c in insp.get_columns("user_preferences")}
    with engine.connect() as conn:
        if "user_id" not in prefs_cols:
            conn.execute(text("ALTER TABLE user_preferences ADD COLUMN user_id VARCHAR"))
            # Backfill existing singleton row with a default user_id
            conn.execute(text("UPDATE user_preferences SET user_id = 'legacy_default' WHERE user_id IS NULL"))
            conn.commit()
            logger.info("BQ-128 Audit: Added user_id column to user_preferences table")

    # Sessions: rename message_count → total_message_count (audit fix 6)
    if "message_count" in session_cols and "total_message_count" not in session_cols:
        with engine.connect() as conn:
            conn.execute(text("ALTER TABLE sessions RENAME COLUMN message_count TO total_message_count"))
            conn.commit()
            logger.info("Audit: Renamed sessions.message_count → total_message_count")
    elif "total_message_count" not in session_cols and "message_count" not in session_cols:
        with engine.connect() as conn:
            conn.execute(text("ALTER TABLE sessions ADD COLUMN total_message_count INTEGER DEFAULT 0"))
            conn.commit()
            logger.info("Audit: Added total_message_count column to sessions table")

    # Messages: add partial unique index for idempotency (session_id, client_message_id)
    existing_indexes = {idx["name"] for idx in insp.get_indexes("messages")}
    if "uq_msg_session_client_id" not in existing_indexes:
        with engine.connect() as conn:
            conn.execute(text(
                "CREATE UNIQUE INDEX IF NOT EXISTS uq_msg_session_client_id "
                "ON messages (session_id, client_message_id) "
                "WHERE client_message_id IS NOT NULL"
            ))
            conn.commit()
            logger.info("BQ-128 Audit: Added partial unique index uq_msg_session_client_id")


def _ensure_default_preferences(engine: Engine) -> None:
    """Migrate singleton row (id=1) to per-user schema if needed. No-op for fresh installs."""
    pass  # Per-user prefs are created on first access via _get_or_create_user_preferences


def _run_alembic_upgrade() -> None:
    """Run ``alembic upgrade head`` programmatically."""
    try:
        from alembic.config import Config
        from alembic import command

        alembic_dir = Path(__file__).resolve().parent.parent.parent / "alembic"
        alembic_ini = alembic_dir.parent / "alembic.ini"

        if not alembic_ini.exists():
            logger.warning("alembic.ini not found at %s — skipping migration", alembic_ini)
            return

        cfg = Config(str(alembic_ini))
        cfg.set_main_option("sqlalchemy.url", DATABASE_URL)
        command.upgrade(cfg, "head")
        logger.info("Alembic migrations applied (upgrade head)")
    except Exception as exc:
        logger.error("Alembic migration failed: %s", exc)
        raise


# ---------------------------------------------------------------------------
# Shutdown
# ---------------------------------------------------------------------------

def close_db() -> None:
    """Dispose both engines at shutdown."""
    global _engine, _legacy_engine
    if _engine is not None:
        _engine.dispose()
        _engine = None
    if _legacy_engine is not None:
        _legacy_engine.dispose()
        _legacy_engine = None
    logger.info("Database connections closed")
