"""BQ-111: initial persistent state tables

Revision ID: 001_initial
Revises: None
Create Date: 2026-02-12
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "001_initial"
down_revision: Union[str, None] = None
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # --- dataset_records ---
    op.create_table(
        "dataset_records",
        sa.Column("id", sa.String(36), primary_key=True),
        sa.Column("original_filename", sa.String(512), nullable=False),
        sa.Column("storage_filename", sa.String(512), nullable=False),
        sa.Column("file_type", sa.String(32), nullable=False),
        sa.Column("file_size_bytes", sa.Integer, nullable=False, server_default="0"),
        sa.Column("status", sa.String(32), nullable=False, server_default="uploading"),
        sa.Column("processed_path", sa.String(1024), nullable=True),
        sa.Column("metadata_json", sa.Text, nullable=False, server_default="{}"),
        sa.Column("created_at", sa.DateTime, nullable=False),
        sa.Column("updated_at", sa.DateTime, nullable=False),
    )
    op.create_index("ix_dataset_records_status", "dataset_records", ["status"])

    # --- billing_usage ---
    op.create_table(
        "billing_usage",
        sa.Column("id", sa.Integer, primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.String(128), nullable=False),
        sa.Column("service", sa.String(128), nullable=False),
        sa.Column("amount_cents", sa.Integer, nullable=False, server_default="0"),
        sa.Column("idempotency_key", sa.String(255), nullable=False, unique=True),
        sa.Column("created_at", sa.DateTime, nullable=False),
    )
    op.create_index("ix_billing_usage_user_id", "billing_usage", ["user_id"])

    # --- billing_subscriptions ---
    op.create_table(
        "billing_subscriptions",
        sa.Column("id", sa.Integer, primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.String(128), nullable=False, unique=True),
        sa.Column("stripe_subscription_id", sa.String(255), nullable=True),
        sa.Column("plan", sa.String(64), nullable=False, server_default="metered"),
        sa.Column("status", sa.String(32), nullable=False, server_default="active"),
        sa.Column("balance_cents", sa.Integer, nullable=False, server_default="0"),
        sa.Column("created_at", sa.DateTime, nullable=False),
        sa.Column("updated_at", sa.DateTime, nullable=False),
    )
    op.create_index("ix_billing_subscriptions_user_id", "billing_subscriptions", ["user_id"])

    # --- api_keys ---
    op.create_table(
        "api_keys",
        sa.Column("id", sa.Integer, primary_key=True, autoincrement=True),
        sa.Column("user_id", sa.String(128), nullable=False),
        sa.Column("key_prefix", sa.String(16), nullable=False),
        sa.Column("key_hash", sa.String(128), nullable=False),
        sa.Column("label", sa.String(255), nullable=True),
        sa.Column("scopes", sa.Text, nullable=False, server_default='["read","write"]'),
        sa.Column("is_active", sa.Boolean, nullable=False, server_default="1"),
        sa.Column("last_used_at", sa.DateTime, nullable=True),
        sa.Column("created_at", sa.DateTime, nullable=False),
        sa.Column("revoked_at", sa.DateTime, nullable=True),
    )
    op.create_index("ix_api_keys_user_id", "api_keys", ["user_id"])
    op.create_index("ix_api_keys_key_hash", "api_keys", ["key_hash"])
    op.create_index("ix_api_keys_is_active", "api_keys", ["is_active"])


def downgrade() -> None:
    op.drop_table("api_keys")
    op.drop_table("billing_subscriptions")
    op.drop_table("billing_usage")
    op.drop_table("dataset_records")
