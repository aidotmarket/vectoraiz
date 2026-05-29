from pathlib import Path
from uuid import uuid4

import sqlalchemy as sa
from alembic import command
from alembic.config import Config


def _alembic_config(repo_root: Path) -> Config:
    config = Config(str(repo_root / "alembic.ini"))
    config.set_main_option("script_location", str(repo_root / "alembic"))
    return config


def test_s3_owner_migration_applies_to_prod_shape_legacy_rows(tmp_path, monkeypatch):
    repo_root = Path(__file__).resolve().parents[1]
    db_path = tmp_path / "s3_owner_migration.db"
    monkeypatch.setenv("DATABASE_URL", f"sqlite:///{db_path}")
    config = _alembic_config(repo_root)

    command.upgrade(config, "020_s3_sts_connector_schema")

    operator_id = str(uuid4())
    engine = sa.create_engine(f"sqlite:///{db_path}")
    with engine.begin() as conn:
        conn.execute(
            sa.text(
                """
                INSERT INTO local_users (id, username, password_hash, role, is_active)
                VALUES (:id, 'operator', 'hash', 'admin', 1)
                """
            ),
            {"id": operator_id},
        )
        conn.execute(
            sa.text(
                """
                INSERT INTO s3_connection
                    (id, name, bucket, region, status, created_at, updated_at)
                VALUES
                    (:id, 'Legacy onboarding', 'legacy-bucket', 'us-east-1',
                     'onboarding', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
                """
            ),
            {"id": str(uuid4())},
        )
        conn.execute(
            sa.text(
                """
                INSERT INTO s3_connection
                    (id, name, bucket, region, role_arn, external_id, status, created_at, updated_at)
                VALUES
                    (:id, 'Configured', 'configured-bucket', 'us-east-1',
                     'arn:aws:iam::210987654321:role/aim-data', :external_id,
                     'configured', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
                """
            ),
            {"id": str(uuid4()), "external_id": str(uuid4())},
        )

    command.upgrade(config, "head")

    with engine.connect() as conn:
        rows = conn.execute(sa.text("SELECT owner_id FROM s3_connection")).fetchall()

    assert rows
    assert {row[0] for row in rows} == {operator_id}
