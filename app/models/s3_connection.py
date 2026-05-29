"""
S3 Connection Model
===================

SQLModel table for seller-owned S3 bucket connection metadata.
STS role ARN and ExternalId are stored locally; no secret material is stored.
"""

from datetime import datetime, timezone
from typing import TYPE_CHECKING, Optional

from sqlalchemy import CheckConstraint, event
from sqlmodel import Column, Field, Relationship, SQLModel, String, Text

if TYPE_CHECKING:
    from app.models.s3_scan_job import S3ScanJob


class S3Connection(SQLModel, table=True):
    """Persistent record of an S3 STS connection."""

    __tablename__ = "s3_connection"
    __table_args__ = (
        CheckConstraint(
            "(status = 'onboarding') OR (role_arn IS NOT NULL AND external_id IS NOT NULL)",
            name="ck_s3_connection_configured_creds_required",
        ),
    )

    id: str = Field(primary_key=True, max_length=36)
    owner_id: Optional[str] = Field(default=None, max_length=64, nullable=True, index=True)
    name: str = Field(max_length=255)
    bucket: str = Field(max_length=255)
    region: str = Field(max_length=64)
    role_arn: Optional[str] = Field(
        default=None,
        sa_column=Column(String(512), nullable=True),
    )
    external_id: Optional[str] = Field(
        default=None,
        sa_column=Column(String(128), nullable=True),
    )
    prefix: Optional[str] = Field(default=None, max_length=512, nullable=True)
    status: str = Field(default="configured", max_length=32, nullable=False)
    error_message: Optional[str] = Field(default=None, sa_column=Column(Text, nullable=True))
    last_scanned_at: Optional[datetime] = Field(default=None, nullable=True)
    continuation_token: Optional[str] = Field(default=None, sa_column=Column(Text, nullable=True))

    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    updated_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))

    scan_jobs: list["S3ScanJob"] = Relationship(back_populates="connection", cascade_delete=True)


@event.listens_for(S3Connection, "before_insert")
@event.listens_for(S3Connection, "before_update")
def _enforce_configured_credentials(mapper, connection, target):
    if target.status == "configured" and (target.role_arn is None or target.external_id is None):
        raise ValueError("configured S3 connections require role_arn and external_id")
