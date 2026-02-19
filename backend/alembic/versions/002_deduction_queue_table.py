"""BQ-113: deduction queue table for exactly-once billing

Revision ID: 002_deduction_queue
Revises: 001_initial
Create Date: 2026-02-12
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "002_deduction_queue"
down_revision: Union[str, None] = "001_initial"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "deductions",
        sa.Column("id", sa.Integer, primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.String(128), nullable=False),
        sa.Column("idempotency_key", sa.String(255), nullable=False, unique=True),
        sa.Column("payload", sa.Text, nullable=False),
        sa.Column("created_at", sa.DateTime, nullable=False),
        sa.Column("next_retry_at", sa.DateTime, nullable=False),
        sa.Column("attempt_count", sa.Integer, nullable=False, server_default="0"),
        sa.Column("last_error", sa.Text, nullable=True),
        sa.Column("status", sa.String(32), nullable=False, server_default="pending"),
        sa.Column("worker_id", sa.String(64), nullable=True),
        sa.Column("leased_at", sa.DateTime, nullable=True),
    )
    op.create_index("idx_status_retry", "deductions", ["status", "next_retry_at"])
    op.create_index("idx_status_leased", "deductions", ["status", "leased_at"])


def downgrade() -> None:
    op.drop_table("deductions")
