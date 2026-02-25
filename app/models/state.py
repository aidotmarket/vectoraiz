"""
State Management Models
=======================

SQLModel classes for local state persistence:
- Session: Conversation containers
- Message: Individual chat messages
- UserPreferences: Per-user LLM settings + Allie personality

Phase: 3.W.1
Created: 2026-01-25
Updated: BQ-128 Phase 1 — Added user_id, MessageKind, usage tracking, idempotency
Updated: BQ-128 Phase 2 Audit — Per-user prefs, idempotency constraint
"""

from datetime import datetime, timezone
from enum import Enum
from typing import List, Optional, Any, Dict
import uuid
from sqlmodel import Field, Relationship, SQLModel, Column, JSON


class MessageRole(str, Enum):
    """Role of a message in the conversation."""
    USER = "user"
    ASSISTANT = "assistant"
    SYSTEM = "system"


class MessageKind(str, Enum):
    """Discriminator for message source (BQ-128)."""
    CHAT = "chat"          # User <-> Allie conversation
    NUDGE = "nudge"        # Server-initiated proactive nudge
    SYSTEM = "system"      # System messages (session start, config changes)


class TimestampMixin(SQLModel):
    """Mixin for created_at and updated_at timestamps."""
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    updated_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))


# =============================================================================
# User Preferences (per-user)
# =============================================================================

class UserPreferences(TimestampMixin, table=True):
    """
    Per-user LLM + Allie personality preferences.
    Keyed by user_id (unique). Created on first access.
    """
    __tablename__ = "user_preferences"

    id: int = Field(default=None, primary_key=True)

    # Per-user key — each user gets their own row
    user_id: str = Field(index=True, unique=True)

    # LLM Provider Settings
    llm_provider: str = Field(default="gemini")  # gemini, openai
    llm_model: str = Field(default="gemini-1.5-flash")
    temperature: float = Field(default=0.2)
    max_tokens: int = Field(default=1024)

    # Custom system prompt (overrides default RAG prompt)
    system_prompt_override: Optional[str] = Field(default=None, nullable=True)

    # API Keys (stored locally, never sent to ai.market)
    # Note: In production, consider encryption at rest
    gemini_api_key: Optional[str] = Field(default=None, nullable=True)
    openai_api_key: Optional[str] = Field(default=None, nullable=True)

    # BQ-128 Phase 2: Allie personality preferences
    tone_mode: str = Field(default="friendly")  # professional | friendly | surfer
    quiet_mode: bool = Field(default=False)
    has_seen_intro: bool = Field(default=False)


# =============================================================================
# Chat Sessions
# =============================================================================

class Session(TimestampMixin, table=True):
    """
    A conversation session containing multiple messages.
    Supports soft delete via 'archived' flag.
    """
    __tablename__ = "sessions"

    id: uuid.UUID = Field(default_factory=uuid.uuid4, primary_key=True)

    # BQ-128: Session ownership — all queries MUST be scoped to user_id
    user_id: Optional[str] = Field(default=None, nullable=True, index=True)

    # Title auto-generated from first user message if not set
    title: Optional[str] = Field(default=None, nullable=True, max_length=255)

    # Soft delete - archived sessions hidden by default
    archived: bool = Field(default=False, index=True)

    # Denormalized for performance (avoid COUNT queries)
    # Tracks total persisted messages (user + assistant), not turns
    total_message_count: int = Field(default=0)

    # Optional: track which dataset(s) this session is about
    dataset_id: Optional[str] = Field(default=None, nullable=True, index=True)

    # Relationships
    messages: List["Message"] = Relationship(
        back_populates="session",
        sa_relationship_kwargs={"cascade": "all, delete-orphan", "order_by": "Message.created_at"}
    )


# =============================================================================
# Chat Messages
# =============================================================================

class Message(SQLModel, table=True):
    """
    A single message in a conversation session.
    Stores role (user/assistant/system), content, and metadata.

    Idempotency: partial unique index (session_id, client_message_id)
    WHERE client_message_id IS NOT NULL — created in database migration,
    plus app-level check before insert.
    """
    __tablename__ = "messages"

    id: uuid.UUID = Field(default_factory=uuid.uuid4, primary_key=True)
    session_id: uuid.UUID = Field(foreign_key="sessions.id", index=True)

    # Message content
    role: MessageRole
    content: str

    # BQ-128: Message kind discriminator
    kind: str = Field(default=MessageKind.CHAT, index=True)

    # BQ-128: Idempotency key — (session_id, client_message_id) unique
    client_message_id: Optional[str] = Field(default=None, nullable=True, max_length=64)

    # Token tracking for context window management
    token_count: Optional[int] = Field(default=None)

    # BQ-128: Usage tracking per message (nullable — populated for assistant messages)
    input_tokens: Optional[int] = Field(default=None, nullable=True)
    output_tokens: Optional[int] = Field(default=None, nullable=True)
    cost_cents: Optional[int] = Field(default=None, nullable=True)
    provider: Optional[str] = Field(default=None, nullable=True, max_length=32)
    model: Optional[str] = Field(default=None, nullable=True, max_length=64)

    # Timestamp
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))

    # Metadata for assistant messages (citations, sources, timing)
    # SQLite stores as TEXT, SQLModel handles JSON serialization
    metadata_: Dict[str, Any] = Field(
        default_factory=dict,
        sa_column=Column("metadata", JSON)
    )

    # Relationships
    session: Session = Relationship(back_populates="messages")


# =============================================================================
# API Models (non-table)
# =============================================================================

class SessionCreate(SQLModel):
    """Request model for creating a new session."""
    title: Optional[str] = None
    dataset_id: Optional[str] = None


class SessionUpdate(SQLModel):
    """Request model for updating a session."""
    title: Optional[str] = None
    archived: Optional[bool] = None


class SessionRead(SQLModel):
    """Response model for session (without messages)."""
    id: uuid.UUID
    user_id: Optional[str] = None
    title: Optional[str]
    archived: bool
    total_message_count: int
    dataset_id: Optional[str]
    created_at: datetime
    updated_at: datetime


class MessageCreate(SQLModel):
    """Request model for adding a message."""
    role: MessageRole
    content: str
    token_count: Optional[int] = None
    metadata_: Dict[str, Any] = Field(default_factory=dict)


class MessageRead(SQLModel):
    """Response model for a message."""
    id: uuid.UUID
    session_id: uuid.UUID
    role: MessageRole
    content: str
    kind: str = MessageKind.CHAT
    client_message_id: Optional[str] = None
    token_count: Optional[int] = None
    input_tokens: Optional[int] = None
    output_tokens: Optional[int] = None
    cost_cents: Optional[int] = None
    provider: Optional[str] = None
    model: Optional[str] = None
    created_at: datetime
    metadata_: Dict[str, Any] = Field(default_factory=dict)


# =============================================================================
# Nudge Dismissals (BQ-128 Phase 3)
# =============================================================================

class NudgeDismissal(SQLModel, table=True):
    """
    Per-user permanent nudge dismissals ("Don't show again").
    Unique on (user_id, trigger_type).

    Tenant boundary: vectorAIz is single-tenant per instance.
    user_id is unique within the instance — no cross-tenant risk.
    If multi-tenant is ever added, add tenant_id column and
    update unique constraint to (tenant_id, user_id, trigger_type).
    """
    __tablename__ = "nudge_dismissals"

    id: str = Field(default_factory=lambda: str(uuid.uuid4()), primary_key=True)
    user_id: str = Field(index=True)
    trigger_type: str = Field(max_length=50)
    permanent: bool = Field(default=False)
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))


class UserPreferencesUpdate(SQLModel):
    """Request model for updating preferences."""
    llm_provider: Optional[str] = None
    llm_model: Optional[str] = None
    temperature: Optional[float] = None
    max_tokens: Optional[int] = None
    system_prompt_override: Optional[str] = None
    gemini_api_key: Optional[str] = None
    openai_api_key: Optional[str] = None
    tone_mode: Optional[str] = None
    quiet_mode: Optional[bool] = None


class UserPreferencesRead(SQLModel):
    """Response model for preferences (masks API keys)."""
    id: int
    user_id: str
    llm_provider: str
    llm_model: str
    temperature: float
    max_tokens: int
    system_prompt_override: Optional[str]
    gemini_api_key_set: bool = False
    openai_api_key_set: bool = False
    tone_mode: str = "friendly"
    quiet_mode: bool = False
    has_seen_intro: bool = False
    updated_at: datetime
