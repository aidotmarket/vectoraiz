"""BQ-D1: Fulfillment log table + listing_id on dataset_records

Revision ID: 010_fulfillment_log
Revises: 009_feedback_table
Create Date: 2026-02-23
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "010_fulfillment_log"
down_revision: Union[str, None] = "009_feedback_table"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # --- Fulfillment log table (§6.2) ---
    op.create_table(
        "fulfillment_log",
        sa.Column("id", sa.String(36), primary_key=True),
        sa.Column("transfer_id", sa.String(36), nullable=False, unique=True),
        sa.Column("order_id", sa.String(255), nullable=False),
        sa.Column("listing_id", sa.String(255), nullable=False),
        sa.Column("request_id", sa.String(255), nullable=False),
        sa.Column("status", sa.String(32), nullable=False, server_default="received"),
        sa.Column("started_at", sa.DateTime(), nullable=False, server_default=sa.func.now()),
        sa.Column("completed_at", sa.DateTime(), nullable=True),
        sa.Column("file_size_bytes", sa.BigInteger(), nullable=True),
        sa.Column("chunks_sent", sa.Integer(), nullable=True),
        sa.Column("error_code", sa.String(64), nullable=True),
        sa.Column("error_message", sa.Text(), nullable=True),
    )
    op.create_index("idx_fulfillment_log_transfer_id", "fulfillment_log", ["transfer_id"])
    op.create_index("idx_fulfillment_log_listing_id", "fulfillment_log", ["listing_id"])
    op.create_index("idx_fulfillment_log_status", "fulfillment_log", ["status"])

    # --- Add listing_id to dataset_records (BQ-D1) ---
    op.add_column("dataset_records", sa.Column("listing_id", sa.String(255), nullable=True))
    op.create_index("idx_dataset_records_listing_id", "dataset_records", ["listing_id"])


def downgrade() -> None:
    op.drop_index("idx_dataset_records_listing_id", table_name="dataset_records")
    op.drop_column("dataset_records", "listing_id")
    op.drop_index("idx_fulfillment_log_status", table_name="fulfillment_log")
    op.drop_index("idx_fulfillment_log_listing_id", table_name="fulfillment_log")
    op.drop_index("idx_fulfillment_log_transfer_id", table_name="fulfillment_log")
    op.drop_table("fulfillment_log")
