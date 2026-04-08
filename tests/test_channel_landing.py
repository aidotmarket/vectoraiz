"""
Tests for channel-aware landing redirect in App.tsx.
"""

from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
APP_PATH = REPO_ROOT / "frontend" / "src" / "App.tsx"


def test_channel_landing_redirects_aim_data_to_datasets():
    assert APP_PATH.exists(), f"App.tsx not found at {APP_PATH}"
    content = APP_PATH.read_text()

    assert "const ChannelLanding = () => {" in content
    assert 'const channel = useChannel();' in content
    assert 'const target = channel === "marketplace" ? "/ai-market" : "/datasets";' in content
    assert 'return <Navigate to={target} replace />;' in content

    # aim-data follows the non-marketplace branch and lands on /datasets.
    assert 'marketplace → /ai-market, aim-data/direct → /datasets' in content
