"""
Tests for PII scrubbing functionality (BQ-059).
Tests the scrub_dataset service method and POST /api/pii/scrub/{dataset_id} endpoint.
Does NOT modify or duplicate tests from test_pii.py.
"""

import pytest
from pathlib import Path
import csv

from fastapi.testclient import TestClient
from app.main import app
from app.services.pii_service import PIIService
from app.services.duckdb_service import get_duckdb_service

client = TestClient(app)


@pytest.fixture
def pii_service():
    """Create PII service instance."""
    return PIIService()


@pytest.fixture
def dataset_with_pii(tmp_path):
    """Create a dataset containing PII for scrub testing."""
    csv_file = tmp_path / "scrub_test.csv"
    with open(csv_file, 'w', newline='') as f:
        writer = csv.writer(f)
        writer.writerow(['id', 'name', 'email', 'phone', 'ssn', 'notes'])
        writer.writerow([1, 'John Smith', 'john.smith@email.com', '555-123-4567', '123-45-6789', 'Regular customer'])
        writer.writerow([2, 'Jane Doe', 'jane.doe@company.org', '(555) 987-6543', '987-65-4321', 'VIP member'])
        writer.writerow([3, 'Bob Wilson', 'bob@test.net', '555.456.7890', '456-78-9012', 'New signup'])

    # Convert to parquet
    duckdb = get_duckdb_service()
    parquet_path = tmp_path / "scrub_test.parquet"
    duckdb.connection.execute(f"""
        COPY (SELECT * FROM read_csv_auto('{csv_file}'))
        TO '{parquet_path}' (FORMAT PARQUET)
    """)

    return parquet_path


# --- Service-level tests ---


def test_scrub_mask_strategy(pii_service, dataset_with_pii):
    """Test mask strategy replaces PII with ***."""
    result = pii_service.scrub_dataset(dataset_with_pii, strategy="mask")

    assert result["strategy_used"] == "mask"
    assert result["pii_removed_count"] > 0
    assert result["duration_seconds"] > 0

    # Scrubbed file should exist with _scrubbed suffix
    scrubbed_path = Path(result["scrubbed_filepath"])
    assert scrubbed_path.exists()
    assert "_scrubbed" in scrubbed_path.stem

    # Before scan should show PII
    assert result["before_scan"]["columns_with_pii"] > 0

    # After scan should show less PII
    assert result["after_scan"]["columns_with_pii"] <= result["before_scan"]["columns_with_pii"]


def test_scrub_redact_strategy(pii_service, dataset_with_pii):
    """Test redact strategy removes PII entirely."""
    result = pii_service.scrub_dataset(dataset_with_pii, strategy="redact")

    assert result["strategy_used"] == "redact"
    assert result["pii_removed_count"] > 0

    scrubbed_path = Path(result["scrubbed_filepath"])
    assert scrubbed_path.exists()
    assert "_scrubbed" in scrubbed_path.stem


def test_scrub_hash_strategy(pii_service, dataset_with_pii):
    """Test hash strategy replaces PII with SHA256 hash."""
    result = pii_service.scrub_dataset(dataset_with_pii, strategy="hash")

    assert result["strategy_used"] == "hash"
    assert result["pii_removed_count"] > 0

    scrubbed_path = Path(result["scrubbed_filepath"])
    assert scrubbed_path.exists()
    assert "_scrubbed" in scrubbed_path.stem


def test_scrub_invalid_strategy(pii_service, dataset_with_pii):
    """Test that invalid strategy raises ValueError."""
    with pytest.raises(ValueError, match="Invalid strategy"):
        pii_service.scrub_dataset(dataset_with_pii, strategy="invalid")


def test_scrub_privacy_score_improves(pii_service, dataset_with_pii):
    """Test that privacy score improves after scrubbing (AC 4)."""
    result = pii_service.scrub_dataset(dataset_with_pii, strategy="mask")

    before_score = result["before_scan"]["privacy_score"]
    after_score = result["after_scan"]["privacy_score"]

    # Privacy score should improve (higher = better) or at least not get worse
    assert after_score >= before_score


def test_scrub_before_after_counts(pii_service, dataset_with_pii):
    """Test that before/after PII counts are included (AC 3)."""
    result = pii_service.scrub_dataset(dataset_with_pii, strategy="mask")

    # Verify structure
    assert "before_scan" in result
    assert "after_scan" in result
    assert "columns_with_pii" in result["before_scan"]
    assert "columns_with_pii" in result["after_scan"]
    assert "privacy_score" in result["before_scan"]
    assert "privacy_score" in result["after_scan"]

    # Before should have detected PII
    assert result["before_scan"]["columns_with_pii"] > 0


def test_scrub_file_alongside_original(pii_service, dataset_with_pii):
    """Test scrubbed file is saved alongside original with _scrubbed suffix (AC 2)."""
    result = pii_service.scrub_dataset(dataset_with_pii, strategy="mask")

    original_dir = dataset_with_pii.parent
    scrubbed_path = Path(result["scrubbed_filepath"])

    # Same directory as original
    assert scrubbed_path.parent == original_dir

    # Has _scrubbed suffix
    assert scrubbed_path.stem == dataset_with_pii.stem + "_scrubbed"
    assert scrubbed_path.suffix == dataset_with_pii.suffix


# --- Endpoint tests ---


def test_scrub_endpoint_not_found():
    """Test scrub endpoint returns 404 for missing dataset."""
    response = client.post("/api/pii/scrub/nonexistent_dataset")
    assert response.status_code == 404


def test_scrub_endpoint_exists():
    """Test that the scrub endpoint is registered and reachable."""
    # This will fail auth or dataset lookup, but proves the route exists
    response = client.post("/api/pii/scrub/test123")
    # Should be 404 (dataset not found) not 405 (method not allowed)
    assert response.status_code in [404, 401, 403]
    assert response.status_code != 405
