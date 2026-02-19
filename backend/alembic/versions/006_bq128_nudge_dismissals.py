"""BQ-128 Phase 3: Nudge dismissals table

Creates nudge_dismissals table for storing per-user permanent
nudge dismissals ("Don't show again" feature).

Revision ID: 006_bq128_nudge_dismissals
Revises: 005_bq127_local_auth
Create Date: 2026-02-14
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "006_bq128_nudge_dismissals"
down_revision: Union[str, None] = "005_bq127_local_auth"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "nudge_dismissals",
        sa.Column("id", sa.String(36), primary_key=True),
        sa.Column("user_id", sa.String(255), nullable=False),
        sa.Column("trigger_type", sa.String(50), nullable=False),
        sa.Column("permanent", sa.Boolean, nullable=False, server_default=sa.text("false")),
        sa.Column("created_at", sa.DateTime, nullable=False, server_default=sa.func.now()),
        sa.UniqueConstraint("user_id", "trigger_type", name="uq_nudge_user_trigger"),
    )
    op.create_index("ix_nudge_dismissals_user_id", "nudge_dismissals", ["user_id"])


def downgrade() -> None:
    op.drop_index("ix_nudge_dismissals_user_id", table_name="nudge_dismissals")
    op.drop_table("nudge_dismissals")
