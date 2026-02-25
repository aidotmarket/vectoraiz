"""
Database Connection Model
=========================

SQLModel table for external database connection metadata.
Credentials are Fernet-encrypted at rest.

Phase: BQ-VZ-DB-CONNECT — Database Connectivity
Created: 2026-02-25
"""

from datetime import datetime, timezone
from typing import Optional

from sqlmodel import Field, SQLModel, Column, Text


class DatabaseConnection(SQLModel, table=True):
    """Persistent record of an external database connection."""

    __tablename__ = "database_connections"

    id: str = Field(primary_key=True, max_length=36)
    name: str = Field(max_length=255)
    db_type: str = Field(max_length=32)  # "postgresql" | "mysql"
    host: str = Field(max_length=512)
    port: int
    database: str = Field(max_length=255)
    username: str = Field(max_length=255)
    password_encrypted: str = Field(sa_column=Column(Text))
    ssl_mode: str = Field(default="prefer", max_length=32)
    extra_options: Optional[str] = Field(default=None, sa_column=Column(Text, nullable=True))

    # Metadata
    last_connected_at: Optional[datetime] = Field(default=None, nullable=True)
    last_sync_at: Optional[datetime] = Field(default=None, nullable=True)
    table_count: Optional[int] = Field(default=None, nullable=True)
    status: str = Field(default="configured", max_length=32)
    error_message: Optional[str] = Field(default=None, sa_column=Column(Text, nullable=True))

    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    updated_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
