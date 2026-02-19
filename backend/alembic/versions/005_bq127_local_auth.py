"""BQ-127: Local auth tables for standalone mode

Creates local_users and local_api_keys tables for air-gapped
authentication when VECTORAIZ_MODE=standalone.

- local_users: admin accounts with bcrypt password hashes
- local_api_keys: API keys with HMAC-SHA256 hashed secrets, scoped permissions

Revision ID: 005_bq127_local_auth
Revises: 004_bq108_109
Create Date: 2026-02-13
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "005_bq127_local_auth"
down_revision: Union[str, None] = "004_bq108_109"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # --- local_users ---
    op.create_table(
        "local_users",
        sa.Column("id", sa.String(36), primary_key=True),
        sa.Column("username", sa.String(255), unique=True, nullable=False),
        sa.Column("password_hash", sa.String(255), nullable=False),
        sa.Column("role", sa.String(50), nullable=False, server_default="admin"),
        sa.Column("is_active", sa.Boolean, nullable=False, server_default=sa.true()),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False, server_default=sa.func.now()),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False, server_default=sa.func.now()),
    )

    # --- local_api_keys ---
    op.create_table(
        "local_api_keys",
        sa.Column("id", sa.String(36), primary_key=True),
        sa.Column(
            "user_id", sa.String(36),
            sa.ForeignKey("local_users.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column("key_id", sa.String(16), unique=True, nullable=False),
        sa.Column("key_hash", sa.String(255), nullable=False),
        sa.Column("label", sa.String(255), nullable=True),
        sa.Column("scopes", sa.Text, nullable=False, server_default='["read","write"]'),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False, server_default=sa.func.now()),
        sa.Column("last_used_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("revoked_at", sa.DateTime(timezone=True), nullable=True),
    )

    op.create_index("idx_local_api_keys_key_id", "local_api_keys", ["key_id"])
    op.create_index("idx_local_api_keys_user_id", "local_api_keys", ["user_id"])


def downgrade() -> None:
    op.drop_index("idx_local_api_keys_user_id", table_name="local_api_keys")
    op.drop_index("idx_local_api_keys_key_id", table_name="local_api_keys")
    op.drop_table("local_api_keys")
    op.drop_table("local_users")
