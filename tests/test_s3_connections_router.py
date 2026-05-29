from contextlib import contextmanager
from datetime import datetime, timezone
from typing import Optional
from uuid import uuid4

import boto3
import pytest
from botocore.stub import Stubber
from fastapi.testclient import TestClient
from sqlalchemy import create_engine, event
from sqlalchemy.pool import StaticPool
from sqlmodel import SQLModel, Session

from app.auth.api_key_auth import AuthenticatedUser, get_current_user
from app.main import app
from app.models.dataset import DatasetRecord  # noqa: F401
from app.models.s3_connection import S3Connection
from app.models.s3_object_metadata import S3ObjectMetadata  # noqa: F401
from app.models.s3_scan_job import S3ScanJob  # noqa: F401
from app.routers import s3_connections
from app.services import s3_scan_service

USER_A = AuthenticatedUser(user_id="user-a", key_id="key-a", scopes=["read", "write"], valid=True)
USER_B = AuthenticatedUser(user_id="user-b", key_id="key-b", scopes=["read", "write"], valid=True)


@pytest.fixture
def s3_engine():
    engine = create_engine(
        "sqlite:///:memory:",
        connect_args={"check_same_thread": False},
        poolclass=StaticPool,
    )

    @event.listens_for(engine, "connect")
    def _enable_foreign_keys(dbapi_conn, _connection_record):
        cursor = dbapi_conn.cursor()
        cursor.execute("PRAGMA foreign_keys=ON")
        cursor.close()

    SQLModel.metadata.create_all(engine)
    return engine


@pytest.fixture
def client(s3_engine, monkeypatch):
    @contextmanager
    def _session_context():
        with Session(s3_engine) as session:
            yield session

    monkeypatch.setattr(s3_connections, "get_session_context", _session_context)
    monkeypatch.setattr(s3_scan_service, "get_session_context", _session_context)
    monkeypatch.setattr(s3_connections.settings, "ai_market_aws_account_id", "123456789012")
    app.dependency_overrides[get_current_user] = lambda: USER_A
    yield TestClient(app)
    app.dependency_overrides.pop(get_current_user, None)


def _create_connection(client: TestClient) -> dict:
    response = client.post(
        "/api/s3-connections/",
        json={
            "name": "Seller bucket",
            "bucket": "seller-bucket",
            "region": "us-east-1",
            "prefix": "exports/",
        },
    )
    assert response.status_code == 201
    return response.json()


def _configured_row(
    s3_engine,
    *,
    prefix: Optional[str] = "exports/",
    owner_id: Optional[str] = "user-a",
) -> S3Connection:
    connection = S3Connection(
        id=str(uuid4()),
        owner_id=owner_id,
        name="Seller bucket",
        bucket="seller-bucket",
        region="us-east-1",
        prefix=prefix,
        role_arn="arn:aws:iam::210987654321:role/aim-data",
        external_id=str(uuid4()),
        status="configured",
    )
    with Session(s3_engine) as session:
        session.add(connection)
        session.commit()
        session.refresh(connection)
        session.expunge(connection)
    return connection


def _stubbed_clients(connection: S3Connection, *, s3_error: bool = False):
    sts_client = boto3.client(
        "sts",
        region_name=connection.region,
        aws_access_key_id="test",
        aws_secret_access_key="test",
        aws_session_token="test",
    )
    s3_client = boto3.client(
        "s3",
        region_name=connection.region,
        aws_access_key_id="test",
        aws_secret_access_key="test",
        aws_session_token="test",
    )
    sts_stubber = Stubber(sts_client)
    s3_stubber = Stubber(s3_client)
    sts_stubber.add_response(
        "assume_role",
        {
            "Credentials": {
                "AccessKeyId": "ASIAIOSFODNN7EXAMPLE",
                "SecretAccessKey": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                "SessionToken": "session-token",
                "Expiration": datetime.now(timezone.utc),
            },
            "AssumedRoleUser": {
                "AssumedRoleId": "AROA123EXAMPLE:aim-data-verify",
                "Arn": connection.role_arn,
            },
        },
        {
            "RoleArn": connection.role_arn,
            "RoleSessionName": "aim-data-verify",
            "ExternalId": connection.external_id,
        },
    )
    if s3_error:
        s3_stubber.add_client_error(
            "list_objects_v2",
            service_error_code="AccessDenied",
            service_message="Access denied",
            expected_params={
                "Bucket": connection.bucket,
                "Prefix": connection.prefix or "",
                "MaxKeys": 1,
            },
        )
    else:
        s3_stubber.add_response(
            "list_objects_v2",
            {"IsTruncated": False, "KeyCount": 1, "Contents": [{"Key": "exports/file.csv"}]},
            {
                "Bucket": connection.bucket,
                "Prefix": connection.prefix or "",
                "MaxKeys": 1,
            },
        )
    sts_stubber.activate()
    s3_stubber.activate()
    return sts_client, s3_client, sts_stubber, s3_stubber


def test_post_creates_row_and_returns_substituted_policies(client):
    data = _create_connection(client)

    assert data["external_id"]
    assert data["status"] == "onboarding"
    assert data["trust_policy"]["Statement"][0]["Principal"]["AWS"] == "arn:aws:iam::123456789012:root"
    assert data["trust_policy"]["Statement"][0]["Condition"]["StringEquals"]["sts:ExternalId"] == data["external_id"]
    assert data["permission_policy"]["Statement"][0]["Resource"] == "arn:aws:s3:::seller-bucket"
    assert data["permission_policy"]["Statement"][0]["Condition"]["StringLike"]["s3:prefix"] == ["exports/*"]
    assert data["permission_policy"]["Statement"][1]["Resource"] == "arn:aws:s3:::seller-bucket/exports/*"


def test_post_stamps_owner_id(client, s3_engine):
    data = _create_connection(client)

    with Session(s3_engine) as session:
        stored = session.get(S3Connection, data["id"])
        assert stored.owner_id == "user-a"


def test_get_lists_only_caller_owned_rows(client, s3_engine):
    created = _create_connection(client)
    foreign = _configured_row(s3_engine, owner_id="user-b")

    response = client.get("/api/s3-connections/")

    assert response.status_code == 200
    assert [row["id"] for row in response.json()] == [created["id"]]
    assert foreign.id not in [row["id"] for row in response.json()]
    assert "trust_policy" not in response.json()[0] or response.json()[0]["trust_policy"] is None


def test_unauthenticated_request_returns_401(client, monkeypatch):
    app.dependency_overrides.pop(get_current_user, None)
    monkeypatch.setenv("VECTORAIZ_AUTH_ENABLED", "true")

    response = client.get("/api/s3-connections/")

    assert response.status_code == 401


def test_get_missing_returns_404(client):
    response = client.get(f"/api/s3-connections/{uuid4()}")

    assert response.status_code == 404


def test_put_role_arn_rejects_malformed_and_accepts_valid(client):
    created = _create_connection(client)

    bad = client.put(f"/api/s3-connections/{created['id']}/role-arn", json={"role_arn": "bad"})
    assert bad.status_code == 400

    good = client.put(
        f"/api/s3-connections/{created['id']}/role-arn",
        json={"role_arn": "arn:aws:iam::210987654321:role/aim-data"},
    )
    assert good.status_code == 200
    assert good.json()["role_arn"] == "arn:aws:iam::210987654321:role/aim-data"
    assert good.json()["status"] == "configured"


def test_verify_success_sets_verified_and_last_scanned_at(client, s3_engine, monkeypatch):
    connection = _configured_row(s3_engine)
    sts_client, s3_client, sts_stubber, s3_stubber = _stubbed_clients(connection)

    monkeypatch.setattr(
        s3_connections,
        "_boto3_client",
        lambda service_name, **_kwargs: sts_client if service_name == "sts" else s3_client,
    )

    response = client.post(f"/api/s3-connections/{connection.id}/verify")

    assert response.status_code == 200
    assert response.json()["status"] == "verified"
    assert response.json()["verified_at"]
    sts_stubber.assert_no_pending_responses()
    s3_stubber.assert_no_pending_responses()
    with Session(s3_engine) as session:
        stored = session.get(S3Connection, connection.id)
        assert stored.status == "verified"
        assert stored.last_scanned_at is not None


def test_verify_sts_failure_sets_error(client, s3_engine, monkeypatch):
    connection = _configured_row(s3_engine)
    sts_client = boto3.client(
        "sts",
        region_name=connection.region,
        aws_access_key_id="test",
        aws_secret_access_key="test",
        aws_session_token="test",
    )
    sts_stubber = Stubber(sts_client)
    sts_stubber.add_client_error(
        "assume_role",
        service_error_code="AccessDenied",
        service_message="Cannot assume role",
        expected_params={
            "RoleArn": connection.role_arn,
            "RoleSessionName": "aim-data-verify",
            "ExternalId": connection.external_id,
        },
    )
    sts_stubber.activate()
    monkeypatch.setattr(s3_connections, "_boto3_client", lambda *_args, **_kwargs: sts_client)

    response = client.post(f"/api/s3-connections/{connection.id}/verify")

    assert response.status_code == 200
    assert response.json()["status"] == "error"
    assert "Cannot assume role" in response.json()["error_message"]
    sts_stubber.assert_no_pending_responses()


def test_verify_s3_failure_after_sts_success_sets_error(client, s3_engine, monkeypatch):
    connection = _configured_row(s3_engine)
    sts_client, s3_client, sts_stubber, s3_stubber = _stubbed_clients(connection, s3_error=True)
    monkeypatch.setattr(
        s3_connections,
        "_boto3_client",
        lambda service_name, **_kwargs: sts_client if service_name == "sts" else s3_client,
    )

    response = client.post(f"/api/s3-connections/{connection.id}/verify")

    assert response.status_code == 200
    assert response.json()["status"] == "error"
    assert "Access denied" in response.json()["error_message"]
    sts_stubber.assert_no_pending_responses()
    s3_stubber.assert_no_pending_responses()


def test_user_cannot_get_scan_or_list_objects_on_foreign_connection(client, s3_engine, monkeypatch):
    connection = _configured_row(s3_engine, owner_id="user-b")
    scan_job = S3ScanJob(id=str(uuid4()), connection_id=connection.id, status="completed")
    with Session(s3_engine) as session:
        session.add(scan_job)
        session.add(
            S3ObjectMetadata(
                id=str(uuid4()),
                connection_id=connection.id,
                scan_job_id=scan_job.id,
                object_key="exports/foreign.csv",
                size_bytes=1,
                content_type="text/csv",
                last_modified=datetime.now(timezone.utc),
                etag="etag",
            )
        )
        session.commit()

    scan_called = False

    def _scan_connection(_self, _connection_id):
        nonlocal scan_called
        scan_called = True
        raise AssertionError("foreign connection should be rejected before scan")

    monkeypatch.setattr(s3_connections.S3ScanService, "scan_connection", _scan_connection)

    assert client.get(f"/api/s3-connections/{connection.id}").status_code == 403
    assert client.post(f"/api/s3-connections/{connection.id}/scan").status_code == 403
    assert client.get(f"/api/s3-connections/{connection.id}/objects").status_code == 403
    assert scan_called is False


def test_delete_removes_row(client):
    created = _create_connection(client)

    deleted = client.delete(f"/api/s3-connections/{created['id']}")
    assert deleted.status_code == 204

    missing = client.get(f"/api/s3-connections/{created['id']}")
    assert missing.status_code == 404
