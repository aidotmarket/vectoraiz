"""Add owner_id to S3 connections.

Revision ID: 021_s3_connection_owner_id
Revises: 020_s3_sts_connector_schema
Create Date: 2026-05-29
"""

from typing import Optional, Sequence, Union

from alembic import op
import sqlalchemy as sa


revision: str = "021_s3_connection_owner_id"
down_revision: Union[str, None] = "020_s3_sts_connector_schema"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def _table_names(bind) -> set[str]:
    return set(sa.inspect(bind).get_table_names())


def _single_operator_user_id(bind) -> Optional[str]:
    tables = _table_names(bind)
    candidates: set[str] = set()

    if "users" in tables:
        rows = bind.execute(
            sa.text("SELECT id FROM users WHERE role = :role AND is_active = :active"),
            {"role": "admin", "active": True},
        ).fetchall()
        candidates.update(str(row[0]) for row in rows if row[0])

    if "local_users" in tables:
        rows = bind.execute(
            sa.text("SELECT id FROM local_users WHERE role = :role AND is_active = :active"),
            {"role": "admin", "active": True},
        ).fetchall()
        candidates.update(str(row[0]) for row in rows if row[0])

    if len(candidates) == 1:
        return next(iter(candidates))
    return None


def upgrade() -> None:
    bind = op.get_bind()
    op.add_column("s3_connection", sa.Column("owner_id", sa.String(64), nullable=True))
    op.create_index("ix_s3_connection_owner_id", "s3_connection", ["owner_id"])

    owner_id = _single_operator_user_id(bind)
    if owner_id:
        bind.execute(
            sa.text("UPDATE s3_connection SET owner_id = :owner_id WHERE owner_id IS NULL"),
            {"owner_id": owner_id},
        )


def downgrade() -> None:
    op.drop_index("ix_s3_connection_owner_id", table_name="s3_connection")
    op.drop_column("s3_connection", "owner_id")
