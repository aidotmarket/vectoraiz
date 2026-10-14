"""Primary DB driver regressions; engines are constructed without PG connections."""

import importlib.util
import runpy
import sys
from pathlib import Path
from types import ModuleType, SimpleNamespace
from unittest.mock import MagicMock

import pytest
from alembic.config import Config
from sqlalchemy import MetaData, make_url

from app.core.database_url import normalize_database_url

REPO = Path(__file__).resolve().parents[1]
ENCODED = "postgresql://test%40user:p%25ss%3Aword@invalid.example/db%20name?application_name=a%2Fb&options=-c%20search_path%3Dpublic"


@pytest.mark.parametrize("url", [
    "postgresql://test:test@invalid.example/db",
    "postgresql://invalid.example/db",
    ENCODED,
])
def test_bare_postgresql_selects_declared_driver(url):
    result = normalize_database_url(url)
    assert result == "postgresql+psycopg2://" + url[len("postgresql://"):]
    assert make_url(result).set(drivername="postgresql") == make_url(url)


@pytest.mark.parametrize("url", [
    "postgresql+psycopg2://test:test@invalid.example/db",
    "postgresql+psycopg://test:test@invalid.example/db",
    "postgresql+asyncpg://test:test@invalid.example/db",
    "sqlite:///data/vectoraiz.db",
    "sqlite+pysqlite:///:memory:",
    "mysql+pymysql://test:test@invalid.example/db",
])
def test_explicit_drivers_and_other_backends_unchanged(url):
    assert normalize_database_url(url) == url


@pytest.fixture
def database(monkeypatch, tmp_path):
    # Avoid loading application settings or reading any .env/credentials.
    config = ModuleType("app.config")
    config.settings = SimpleNamespace(data_directory=str(tmp_path), debug=False)
    monkeypatch.setitem(sys.modules, "app.config", config)
    monkeypatch.setenv("DATABASE_URL", "sqlite:///:memory:")
    spec = importlib.util.spec_from_file_location("r241_database", REPO / "app/core/database.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    yield module
    module.close_db()


@pytest.mark.parametrize("url", [ENCODED, "postgresql://test:test@invalid.example/db",
                                  "postgresql+psycopg2://test:test@invalid.example/db"])
def test_runtime_constructs_psycopg2_engine_without_connection(database, url):
    engine = database._build_engine(url)
    try:
        assert engine.dialect.driver == "psycopg2"
        assert engine.url == make_url(normalize_database_url(url))
    finally:
        engine.dispose()


def test_primary_get_engine_normalizes_without_logging_url(database, monkeypatch, caplog):
    monkeypatch.setattr(database, "DATABASE_URL", ENCODED)
    with caplog.at_level("INFO"):
        assert database.get_engine().dialect.driver == "psycopg2"
    assert "Database engine created: postgresql" in caplog.text
    assert "invalid.example" not in caplog.text
    assert "application_name" not in caplog.text
    assert "p%25ss" not in caplog.text


def test_sqlite_and_legacy_engines_preserved(database):
    engine = database._build_engine("sqlite:///:memory:")
    try:
        assert engine.dialect.name == "sqlite"
    finally:
        engine.dispose()
    assert database.get_legacy_engine().url.database.endswith("vai_state.db")


@pytest.mark.parametrize("url", [ENCODED, "sqlite:///:memory:",
                                  "postgresql+psycopg://invalid.example/db"])
def test_programmatic_alembic_url_preserves_encoding(database, monkeypatch, url):
    from alembic import command
    captured = []
    monkeypatch.setattr(database, "DATABASE_URL", url)
    monkeypatch.setattr(command, "upgrade", lambda cfg, revision: captured.append((cfg, revision)))
    database._run_alembic_upgrade()
    cfg, revision = captured[0]
    assert cfg.get_main_option("sqlalchemy.url") == normalize_database_url(url)
    assert revision == "head"


@pytest.mark.parametrize("environment_url,offline", [
    (ENCODED, True), (ENCODED, False), (None, True), (None, False),
    ("postgresql+psycopg://invalid.example/db", True),
    ("sqlite:///:memory:", True),
])
def test_alembic_env_normalizes_environment_and_config(monkeypatch, environment_url, offline):
    from alembic import context
    import sqlalchemy
    cfg = Config()
    cfg.set_main_option("sqlalchemy.url", ENCODED.replace("%", "%%"))
    if environment_url is None:
        monkeypatch.delenv("DATABASE_URL", raising=False)
    else:
        monkeypatch.setenv("DATABASE_URL", environment_url)
    expected = normalize_database_url(environment_url or ENCODED)
    monkeypatch.setattr(context, "config", cfg, raising=False)
    monkeypatch.setattr(context, "is_offline_mode", lambda: offline)
    configure = MagicMock()
    monkeypatch.setattr(context, "configure", configure)
    monkeypatch.setattr(context, "begin_transaction", MagicMock())
    monkeypatch.setattr(context, "run_migrations", MagicMock())
    # Metadata registration is unrelated to driver selection. Isolate these
    # imports from provider/configuration dependencies and shared ORM metadata.
    for name, symbols in {
        "dataset": ["DatasetRecord"], "billing": ["BillingUsage", "BillingSubscription"],
        "api_key": ["APIKey"], "local_auth": ["LocalUser", "LocalAPIKey"],
        "user": ["User"], "connectivity": ["ConnectivityTokenRecord"],
        "raw_file": ["RawFile"], "raw_listing": ["RawListing"],
    }.items():
        module = ModuleType("app.models." + name)
        for symbol in symbols:
            setattr(module, symbol, object())
        monkeypatch.setitem(sys.modules, module.__name__, module)
    deductions = ModuleType("app.services.deduction_queue")
    deductions.deductions_metadata = MetaData()
    monkeypatch.setitem(sys.modules, deductions.__name__, deductions)
    captured = []
    real_constructor = sqlalchemy.engine_from_config
    def construct_without_connect(section, **kwargs):
        engine = real_constructor(section, **kwargs)
        captured.append(engine)
        return MagicMock()
    monkeypatch.setattr(sqlalchemy, "engine_from_config", construct_without_connect)
    try:
        runpy.run_path(str(REPO / "alembic/env.py"))
        assert cfg.get_main_option("sqlalchemy.url") == expected
        if offline:
            assert configure.call_args.kwargs["url"] == expected
        else:
            assert len(captured) == 1
            assert captured[0].dialect.driver == "psycopg2"
            assert captured[0].url == make_url(expected)
    finally:
        for engine in captured:
            engine.dispose()
