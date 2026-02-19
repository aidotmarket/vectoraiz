"""BQ-125: LLM settings and usage log tables

Revision ID: 003_llm_settings
Revises: 002_deduction_queue
Create Date: 2026-02-12
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "003_llm_settings"
down_revision: Union[str, None] = "002_deduction_queue"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # llm_settings table
    op.create_table(
        "llm_settings",
        sa.Column("id", sa.String(36), primary_key=True),
        sa.Column("scope", sa.String(16), nullable=False, server_default="instance"),
        sa.Column("user_id", sa.String(36), nullable=True),
        sa.Column("provider", sa.String(32), nullable=False),
        sa.Column("model", sa.String(64), nullable=False),
        sa.Column("display_name", sa.String(128), nullable=True),
        sa.Column("encrypted_key", sa.LargeBinary, nullable=False),
        sa.Column("key_iv", sa.LargeBinary, nullable=False),
        sa.Column("key_tag", sa.LargeBinary, nullable=False),
        sa.Column("key_version", sa.Integer, nullable=False, server_default="1"),
        sa.Column("key_hint", sa.String(16), nullable=True),
        sa.Column("is_active", sa.Boolean, nullable=False, server_default="1"),
        sa.Column("last_tested_at", sa.DateTime, nullable=True),
        sa.Column("last_test_ok", sa.Boolean, nullable=True),
        sa.Column("total_requests", sa.BigInteger, nullable=False, server_default="0"),
        sa.Column("total_tokens", sa.BigInteger, nullable=False, server_default="0"),
        sa.Column("created_at", sa.DateTime, nullable=False),
        sa.Column("updated_at", sa.DateTime, nullable=False),
        sa.Column("created_by", sa.String(64), nullable=False, server_default="admin"),
        sa.UniqueConstraint("scope", "user_id", "provider", name="uq_llm_scope_provider"),
        sa.CheckConstraint("scope IN ('instance', 'user')", name="chk_scope"),
        sa.CheckConstraint("provider IN ('openai', 'anthropic', 'gemini')", name="chk_provider"),
    )
    op.create_index(
        "idx_llm_active",
        "llm_settings",
        ["scope", "is_active"],
    )

    # llm_usage_log table
    op.create_table(
        "llm_usage_log",
        sa.Column("id", sa.Integer, primary_key=True, autoincrement=True),
        sa.Column("settings_id", sa.String(36), sa.ForeignKey("llm_settings.id"), nullable=False),
        sa.Column("ts", sa.DateTime, nullable=False),
        sa.Column("operation", sa.String(32), nullable=False),
        sa.Column("model", sa.String(64), nullable=False),
        sa.Column("input_tokens", sa.Integer, nullable=False, server_default="0"),
        sa.Column("output_tokens", sa.Integer, nullable=False, server_default="0"),
        sa.Column("latency_ms", sa.Integer, nullable=True),
        sa.Column("success", sa.Boolean, nullable=False, server_default="1"),
        sa.Column("error_code", sa.String(32), nullable=True),
    )
    op.create_index("idx_usage_ts", "llm_usage_log", [sa.text("ts DESC")])
    op.create_index("idx_usage_settings", "llm_usage_log", ["settings_id"])


def downgrade() -> None:
    op.drop_table("llm_usage_log")
    op.drop_table("llm_settings")
