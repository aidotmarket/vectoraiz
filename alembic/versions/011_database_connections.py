"""BQ-VZ-DB-CONNECT: Database connections table

Revision ID: 011_database_connections
Revises: 010_fulfillment_log
Create Date: 2026-02-25
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "011_database_connections"
down_revision: Union[str, None] = "010_fulfillment_log"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "database_connections",
        sa.Column("id", sa.String(36), primary_key=True),
        sa.Column("name", sa.String(255), nullable=False),
        sa.Column("db_type", sa.String(32), nullable=False),
        sa.Column("host", sa.String(512), nullable=False),
        sa.Column("port", sa.Integer, nullable=False),
        sa.Column("database", sa.String(255), nullable=False),
        sa.Column("username", sa.String(255), nullable=False),
        sa.Column("password_encrypted", sa.Text, nullable=False),
        sa.Column("ssl_mode", sa.String(32), server_default="prefer"),
        sa.Column("extra_options", sa.Text, nullable=True),
        sa.Column("last_connected_at", sa.DateTime, nullable=True),
        sa.Column("last_sync_at", sa.DateTime, nullable=True),
        sa.Column("table_count", sa.Integer, nullable=True),
        sa.Column("status", sa.String(32), server_default="configured"),
        sa.Column("error_message", sa.Text, nullable=True),
        sa.Column("created_at", sa.DateTime, nullable=False),
        sa.Column("updated_at", sa.DateTime, nullable=False),
    )


def downgrade() -> None:
    op.drop_table("database_connections")
