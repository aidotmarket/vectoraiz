"""BQ-VZ-DATA-CHANNEL Slice C: add metadata column to raw_files

Revision ID: 018_raw_files_metadata
Revises: 017_sync_state
Create Date: 2026-04-09
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


revision: str = "018_raw_files_metadata"
down_revision: Union[str, None] = "017_sync_state"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column("raw_files", sa.Column("metadata", sa.JSON(), nullable=True))


def downgrade() -> None:
    op.drop_column("raw_files", "metadata")
