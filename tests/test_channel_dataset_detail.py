"""
Tests for aim-data CTA styling in DatasetDetail.tsx.
"""

from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
DATASET_DETAIL_PATH = REPO_ROOT / "frontend" / "src" / "pages" / "DatasetDetail.tsx"


def test_aim_data_channel_gets_primary_publish_button_variant():
    assert DATASET_DETAIL_PATH.exists(), f"DatasetDetail.tsx not found at {DATASET_DETAIL_PATH}"
    content = DATASET_DETAIL_PATH.read_text()

    assert 'variant={channel === "marketplace" || channel === "aim-data" ? "default" : "ghost"}' in content


def test_aim_data_channel_shows_publish_to_aimarket_label():
    content = DATASET_DETAIL_PATH.read_text()
    assert '{channel === "marketplace" || channel === "aim-data" ? "Publish to ai.market" : "Publish"}' in content


def test_aim_data_channel_gets_ring_two_styling():
    content = DATASET_DETAIL_PATH.read_text()
    assert 'channel === "marketplace" || channel === "aim-data" ? " ring-2 ring-primary/30" : ""' in content
