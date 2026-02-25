"""BQ-108+109: Enhanced Upload Pipeline — batch upload + data preview

Adds batch_id, relative_path, preview_text, preview_metadata,
confirmed_at, confirmed_by columns to dataset_records.
Migrates existing status values:
  processing → ready, failed → error, uploading → error.

Revision ID: 004_bq108_109
Revises: 003_llm_settings
Create Date: 2026-02-13
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "004_bq108_109"
down_revision: Union[str, None] = "003_llm_settings"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # Add new columns (all nullable for backward compat)
    op.add_column("dataset_records", sa.Column("batch_id", sa.String(64), nullable=True))
    op.add_column("dataset_records", sa.Column("relative_path", sa.String(1024), nullable=True))
    op.add_column("dataset_records", sa.Column("preview_text", sa.Text, nullable=True))
    op.add_column("dataset_records", sa.Column("preview_metadata", sa.Text, nullable=True))
    op.add_column("dataset_records", sa.Column("confirmed_at", sa.DateTime, nullable=True))
    op.add_column("dataset_records", sa.Column("confirmed_by", sa.String(64), nullable=True))

    # Index on batch_id for batch status queries
    op.create_index("idx_batch_id", "dataset_records", ["batch_id"])

    # Migrate existing status values to new enum
    # processing → ready (these completed background processing)
    op.execute("UPDATE dataset_records SET status = 'ready' WHERE status = 'processing'")
    # failed → error (normalize naming)
    op.execute("UPDATE dataset_records SET status = 'error' WHERE status = 'failed'")
    # uploading → error (stale interrupted uploads)
    op.execute("UPDATE dataset_records SET status = 'error' WHERE status = 'uploading'")


def downgrade() -> None:
    # Reverse status migration
    op.execute("UPDATE dataset_records SET status = 'processing' WHERE status = 'extracting'")
    op.execute("UPDATE dataset_records SET status = 'processing' WHERE status = 'indexing'")
    op.execute("UPDATE dataset_records SET status = 'ready' WHERE status = 'preview_ready'")
    op.execute("UPDATE dataset_records SET status = 'failed' WHERE status = 'error'")
    op.execute("UPDATE dataset_records SET status = 'failed' WHERE status = 'cancelled'")

    op.drop_index("idx_batch_id", table_name="dataset_records")
    op.drop_column("dataset_records", "confirmed_by")
    op.drop_column("dataset_records", "confirmed_at")
    op.drop_column("dataset_records", "preview_metadata")
    op.drop_column("dataset_records", "preview_text")
    op.drop_column("dataset_records", "relative_path")
    op.drop_column("dataset_records", "batch_id")
