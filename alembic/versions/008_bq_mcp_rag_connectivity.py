"""BQ-MCP-RAG: Add connectivity_tokens table and externally_queryable column

New table: connectivity_tokens (§10)
Modified table: dataset_records — add externally_queryable BOOLEAN DEFAULT FALSE

Revision ID: 008_bq_mcp_rag_connectivity
Revises: 007_bq128_p4_idempotency
Create Date: 2026-02-16
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "008_bq_mcp_rag_connectivity"
down_revision: Union[str, None] = "007_bq128_p4_idempotency"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # -- New table: connectivity_tokens --
    op.create_table(
        "connectivity_tokens",
        sa.Column("id", sa.String(length=8), primary_key=True),
        sa.Column("label", sa.String(length=255), nullable=False),
        sa.Column("hmac_hash", sa.String(length=255), nullable=False),
        sa.Column("secret_last4", sa.String(length=4), nullable=False),
        sa.Column(
            "scopes",
            sa.Text(),
            nullable=False,
            server_default='["ext:search","ext:sql","ext:schema","ext:datasets"]',
        ),
        sa.Column("created_at", sa.DateTime(), nullable=False, server_default=sa.func.now()),
        sa.Column("expires_at", sa.DateTime(), nullable=True),
        sa.Column("last_used_at", sa.DateTime(), nullable=True),
        sa.Column("request_count", sa.Integer(), nullable=False, server_default="0"),
        sa.Column("is_revoked", sa.Boolean(), nullable=False, server_default="0"),
        sa.Column("revoked_at", sa.DateTime(), nullable=True),
    )
    op.create_index("idx_connectivity_tokens_revoked", "connectivity_tokens", ["is_revoked"])
    op.create_index("idx_connectivity_tokens_last_used", "connectivity_tokens", ["last_used_at"])

    # -- Add externally_queryable to dataset_records --
    op.add_column(
        "dataset_records",
        sa.Column(
            "externally_queryable",
            sa.Boolean(),
            nullable=False,
            server_default="0",
        ),
    )


def downgrade() -> None:
    op.drop_column("dataset_records", "externally_queryable")
    op.drop_index("idx_connectivity_tokens_last_used", table_name="connectivity_tokens")
    op.drop_index("idx_connectivity_tokens_revoked", table_name="connectivity_tokens")
    op.drop_table("connectivity_tokens")
