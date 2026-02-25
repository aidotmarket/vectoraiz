"""
Feedback Model
==============

Stores user feedback, bug reports, and suggestions submitted via allAI chat.

Phase: Feedback/Support Tool
Created: 2026-02-19
"""

import uuid
from datetime import datetime, timezone
from typing import Optional

from sqlmodel import Field, SQLModel


class Feedback(SQLModel, table=True):
    __tablename__ = "feedback"

    id: str = Field(
        default_factory=lambda: uuid.uuid4().hex[:8], primary_key=True
    )
    category: str  # bug, suggestion, question, other
    summary: str
    details: Optional[str] = None
    user_id: Optional[str] = None
    created_at: datetime = Field(
        default_factory=lambda: datetime.now(timezone.utc)
    )
    forwarded: bool = Field(default=False)
