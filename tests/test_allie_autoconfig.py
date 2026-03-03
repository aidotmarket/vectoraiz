"""
Tests for allAI copilot auto-configuration after serial activation.

Verifies:
- Activation writes allie_config.json
- get_allie_provider picks up config file when env vars are empty
- Token refresh updates allie_config.json
- Explicit env var overrides config file
- MockProvider fallback when neither exists
- AiMarketAllieProvider accepts kwargs

BQ-VZ-SERIAL-CLIENT
"""

import json
import os

import pytest
from unittest.mock import AsyncMock, MagicMock, patch

from app.services.allie_provider import (
    AiMarketAllieProvider,
    MockAllieProvider,
    get_allie_provider,
    read_allie_config,
    reset_provider,
    write_allie_config,
    ALLIE_CONFIG_FILENAME,
)


@pytest.fixture(autouse=True)
def clean_provider_singleton():
    """Reset the provider singleton before and after each test."""
    reset_provider()
    yield
    reset_provider()


@pytest.fixture
def tmp_data_dir(tmp_path):
    """Temp directory that replaces settings.serial_data_dir."""
    return str(tmp_path)


@pytest.fixture
def patch_data_dir(tmp_data_dir):
    """Patch _allie_config_path to point to a temp directory."""
    config_path = os.path.join(tmp_data_dir, ALLIE_CONFIG_FILENAME)
    with patch("app.services.allie_provider._allie_config_path", return_value=config_path):
        yield tmp_data_dir


class TestWriteAllieConfig:
    def test_writes_valid_json(self, patch_data_dir):
        write_allie_config(
            serial_number="VZ-test1234-abcd5678",
            install_token="vzit_test_token_123",
            ai_market_url="https://ai.market.test",
        )
        path = os.path.join(patch_data_dir, ALLIE_CONFIG_FILENAME)
        assert os.path.exists(path)

        with open(path) as f:
            data = json.load(f)

        assert data["serial_number"] == "VZ-test1234-abcd5678"
        assert data["install_token"] == "vzit_test_token_123"
        assert data["ai_market_url"] == "https://ai.market.test"
        assert data["provider"] == "aimarket"

    def test_file_permissions(self, patch_data_dir):
        write_allie_config("VZ-s", "vzit_t", "https://test")
        path = os.path.join(patch_data_dir, ALLIE_CONFIG_FILENAME)
        mode = os.stat(path).st_mode & 0o777
        assert mode == 0o600

    def test_overwrites_existing(self, patch_data_dir):
        write_allie_config("VZ-old", "vzit_old", "https://old")
        write_allie_config("VZ-new", "vzit_new", "https://new")

        path = os.path.join(patch_data_dir, ALLIE_CONFIG_FILENAME)
        with open(path) as f:
            data = json.load(f)
        assert data["install_token"] == "vzit_new"


class TestReadAllieConfig:
    def test_reads_valid_config(self, patch_data_dir):
        write_allie_config("VZ-serial", "vzit_key", "https://url")
        config = read_allie_config()
        assert config is not None
        assert config["serial_number"] == "VZ-serial"
        assert config["install_token"] == "vzit_key"
        assert config["ai_market_url"] == "https://url"
        assert config["provider"] == "aimarket"

    def test_returns_none_when_missing(self, patch_data_dir):
        assert read_allie_config() is None

    def test_returns_none_for_invalid_json(self, patch_data_dir):
        path = os.path.join(patch_data_dir, ALLIE_CONFIG_FILENAME)
        with open(path, "w") as f:
            f.write("not json {{{")
        assert read_allie_config() is None

    def test_returns_none_for_missing_fields(self, patch_data_dir):
        path = os.path.join(patch_data_dir, ALLIE_CONFIG_FILENAME)
        with open(path, "w") as f:
            json.dump({"provider": "aimarket"}, f)
        assert read_allie_config() is None

    def test_returns_none_for_wrong_provider(self, patch_data_dir):
        path = os.path.join(patch_data_dir, ALLIE_CONFIG_FILENAME)
        with open(path, "w") as f:
            json.dump({"provider": "other", "install_token": "x", "serial_number": "y"}, f)
        assert read_allie_config() is None


class TestGetAllieProviderConfigFile:
    """Test that get_allie_provider picks up the config file."""

    def test_uses_config_file_when_no_env_var(self, patch_data_dir):
        write_allie_config("VZ-serial123", "vzit_token456", "https://ai.test")

        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("VECTORAIZ_ALLIE_PROVIDER", None)
            provider = get_allie_provider()

        assert isinstance(provider, AiMarketAllieProvider)
        assert provider.api_key == "vzit_token456"
        assert provider.serial == "VZ-serial123"
        assert provider.base_url == "https://ai.test"

    def test_falls_back_to_mock_when_no_config(self, patch_data_dir):
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("VECTORAIZ_ALLIE_PROVIDER", None)
            provider = get_allie_provider()

        assert isinstance(provider, MockAllieProvider)

    def test_env_var_mock_overrides_config_file(self, patch_data_dir):
        write_allie_config("VZ-serial", "vzit_token", "https://url")

        with patch.dict(os.environ, {"VECTORAIZ_ALLIE_PROVIDER": "mock"}):
            provider = get_allie_provider()

        assert isinstance(provider, MockAllieProvider)

    def test_env_var_aimarket_uses_env_settings(self):
        """When env var says aimarket, it uses settings (not config file)."""
        with patch.dict(os.environ, {"VECTORAIZ_ALLIE_PROVIDER": "aimarket"}):
            # Patch settings inside AiMarketAllieProvider.__init__
            with patch("app.config.settings") as mock_settings:
                mock_settings.ai_market_url = "https://env.ai.market"
                mock_settings.internal_api_key = "vzit_env_key"
                mock_settings.serial = "VZ-env-serial"
                provider = get_allie_provider()

        assert isinstance(provider, AiMarketAllieProvider)
        assert provider.api_key == "vzit_env_key"


class TestAiMarketAllieProviderKwargs:
    def test_accepts_kwargs(self):
        provider = AiMarketAllieProvider(
            serial="VZ-kwarg-serial",
            api_key="vzit_kwarg_key",
            base_url="https://kwarg.test/",
        )
        assert provider.serial == "VZ-kwarg-serial"
        assert provider.api_key == "vzit_kwarg_key"
        assert provider.base_url == "https://kwarg.test"  # trailing slash stripped

    def test_raises_without_api_key(self):
        with patch("app.config.settings") as mock_settings:
            mock_settings.internal_api_key = None
            mock_settings.ai_market_url = "https://test"
            mock_settings.serial = None
            with pytest.raises(ValueError, match="API key required"):
                AiMarketAllieProvider()


class TestResetProviderAfterActivation:
    """Test that activation triggers config write + provider reset."""

    @pytest.mark.asyncio
    async def test_activation_writes_config_and_resets_provider(self, patch_data_dir):
        from app.services.activation_manager import ActivationManager
        from app.services.serial_store import SerialStore
        from app.services.serial_client import ActivateResult

        # Setup: serial store with PROVISIONED state
        store_path = os.path.join(patch_data_dir, "serial.json")
        with open(store_path, "w") as f:
            json.dump({
                "serial": "VZ-activate-test1234",
                "bootstrap_token": "vzbt_boot_token",
                "state": "provisioned",
            }, f)
        store = SerialStore(path=store_path)

        # Mock client returning successful activation
        mock_client = MagicMock()
        mock_client.activate = AsyncMock(return_value=ActivateResult(
            success=True,
            install_token="vzit_new_install_token",
            status_code=200,
        ))

        manager = ActivationManager(store=store, client=mock_client)

        with patch("app.services.activation_manager.settings") as mock_settings:
            mock_settings.app_version = "1.0.0"
            mock_settings.aimarket_url = "https://ai.market.activated"
            await manager._attempt_activation()

        # Verify config file was written
        config = read_allie_config()
        assert config is not None
        assert config["serial_number"] == "VZ-activate-test1234"
        assert config["install_token"] == "vzit_new_install_token"
        assert config["ai_market_url"] == "https://ai.market.activated"

    @pytest.mark.asyncio
    async def test_refresh_updates_config(self, patch_data_dir):
        from app.services.activation_manager import ActivationManager
        from app.services.serial_store import SerialStore
        from app.services.serial_client import RefreshResult

        # Setup: serial store with ACTIVE state
        store_path = os.path.join(patch_data_dir, "serial.json")
        with open(store_path, "w") as f:
            json.dump({
                "serial": "VZ-refresh-test1234",
                "install_token": "vzit_old_token",
                "state": "active",
            }, f)
        store = SerialStore(path=store_path)

        # Write initial config
        write_allie_config("VZ-refresh-test1234", "vzit_old_token", "https://ai.market.test")

        # Mock client returning successful refresh
        mock_client = MagicMock()
        mock_client.refresh = AsyncMock(return_value=RefreshResult(
            success=True,
            install_token="vzit_refreshed_token",
            status_code=200,
        ))

        manager = ActivationManager(store=store, client=mock_client)

        await manager._attempt_refresh()

        # Verify config file was updated with new token
        config = read_allie_config()
        assert config is not None
        assert config["install_token"] == "vzit_refreshed_token"

    @pytest.mark.asyncio
    async def test_provider_switches_from_mock_after_activation(self, patch_data_dir):
        """End-to-end: provider starts as Mock, activation writes config, reset picks up AiMarket."""
        # Before activation: no config, should be mock
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("VECTORAIZ_ALLIE_PROVIDER", None)
            provider = get_allie_provider()
            assert isinstance(provider, MockAllieProvider)

        # Simulate activation writing config
        write_allie_config("VZ-e2e-serial", "vzit_e2e_token", "https://e2e.test")
        reset_provider()

        # After reset: should now pick up the config file
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("VECTORAIZ_ALLIE_PROVIDER", None)
            provider = get_allie_provider()
            assert isinstance(provider, AiMarketAllieProvider)
            assert provider.api_key == "vzit_e2e_token"
