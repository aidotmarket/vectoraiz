"""
BQ-125: Tests for LLM settings — encryption, CRUD, connection testing,
error adapters, usage tracking, provider catalog, and LLMService lazy-load.

Covers 20 tests across all BQ-125 components.
"""

import time
import uuid
from datetime import datetime, timezone
from unittest.mock import patch, MagicMock, AsyncMock

import pytest

from app.core.llm_key_crypto import (
    encrypt_api_key,
    decrypt_api_key,
    decrypt_with_fallback,
)
from app.core.llm_error_adapters import (
    BaseErrorAdapter,
    OpenAIErrorAdapter,
    AnthropicErrorAdapter,
    GeminiErrorAdapter,
    ERROR_ADAPTERS,
)
from app.schemas.llm_settings import (
    LLMSettingsCreate,
    LLMSettingsResponse,
    LLMSettingsListResponse,
    LLMTestResponse,
    LLMProvidersResponse,
    LLMUsageSummary,
)
from app.models.llm_settings import LLMSettings, LLMUsageLog


# Test secret keys
SECRET_A = "test-secret-key-alpha-32chars!!!"
SECRET_B = "test-secret-key-bravo-32chars!!!"


# ═══════════════════════════════════════════════════════════════════════
# 1-4. Encryption Tests
# ═══════════════════════════════════════════════════════════════════════

class TestEncryption:
    """Tests for AES-256-GCM encryption of API keys."""

    def test_encrypt_decrypt_roundtrip(self):
        """1. Encryption round-trip: encrypt -> decrypt -> matches original."""
        plaintext = "sk-proj-abc123def456"
        ciphertext, iv, tag = encrypt_api_key(plaintext, SECRET_A)
        decrypted = decrypt_api_key(ciphertext, iv, tag, SECRET_A)
        assert decrypted == plaintext

    def test_different_versions_produce_different_ciphertexts(self):
        """2. Different key versions produce different ciphertexts."""
        plaintext = "sk-proj-abc123def456"
        ct1, iv1, tag1 = encrypt_api_key(plaintext, SECRET_A, key_version=1)
        ct2, iv2, tag2 = encrypt_api_key(plaintext, SECRET_A, key_version=2)
        # Even with same plaintext and secret, different versions -> different derived keys
        # (iv is random too, but the derived key itself differs)
        # Verify both decrypt correctly with their own version
        assert decrypt_api_key(ct1, iv1, tag1, SECRET_A, 1) == plaintext
        assert decrypt_api_key(ct2, iv2, tag2, SECRET_A, 2) == plaintext
        # Cross-version decrypt should fail
        with pytest.raises(Exception):
            decrypt_api_key(ct1, iv1, tag1, SECRET_A, 2)

    def test_dual_decrypt_fallback(self):
        """3. Dual-decrypt fallback: current + previous secret."""
        plaintext = "sk-ant-old-key-here"
        # Encrypt with old secret
        ct, iv, tag = encrypt_api_key(plaintext, SECRET_A)
        # Decrypt with fallback (new secret fails, falls back to old)
        result = decrypt_with_fallback(ct, iv, tag, SECRET_B, SECRET_A)
        assert result == plaintext

    def test_decrypt_wrong_key_raises(self):
        """4. Decrypt with wrong key raises error."""
        plaintext = "sk-test-key"
        ct, iv, tag = encrypt_api_key(plaintext, SECRET_A)
        with pytest.raises(Exception):
            decrypt_api_key(ct, iv, tag, "completely-wrong-secret-key!!!!!")


# ═══════════════════════════════════════════════════════════════════════
# 5-8. Settings CRUD Tests (via service layer with mocked DB)
# ═══════════════════════════════════════════════════════════════════════

class TestSettingsCRUD:
    """Tests for settings create/read/update/delete using the service layer."""

    def _make_settings_row(self, provider="openai", is_active=True):
        """Helper: create a mock LLMSettings row."""
        plaintext = "sk-test-1234567890abcdef"
        ct, iv, tag = encrypt_api_key(plaintext, SECRET_A)
        return LLMSettings(
            id=str(uuid.uuid4()),
            scope="instance",
            provider=provider,
            model="gpt-4o",
            display_name="Test Key",
            encrypted_key=ct,
            key_iv=iv,
            key_tag=tag,
            key_hint="cdef",
            is_active=is_active,
            total_requests=100,
            total_tokens=50000,
            created_at=datetime.now(timezone.utc),
            updated_at=datetime.now(timezone.utc),
        )

    @patch("app.services.llm_settings_service.get_session_context")
    @patch("app.services.llm_settings_service.settings")
    def test_put_settings_encrypts_key(self, mock_settings, mock_ctx):
        """5. Settings create via PUT: key is encrypted, hint saved."""
        from app.services.llm_settings_service import put_settings

        mock_settings.get_secret_key.return_value = SECRET_A

        # Mock session that returns no existing row
        mock_session = MagicMock()
        mock_session.exec.return_value.first.return_value = None
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)
        mock_ctx.return_value = mock_session

        # Track what gets added to session
        added_rows = []
        original_add = mock_session.add

        def capture_add(row):
            added_rows.append(row)
            return original_add(row)

        mock_session.add = capture_add

        data = LLMSettingsCreate(
            provider="openai",
            api_key="sk-proj-testkey1234",
            model="gpt-4o",
            display_name="My OpenAI",
            set_active=True,
        )

        with patch("app.services.llm_settings_service._invalidate_llm_service"):
            result = put_settings(data)

        # Verify a row was added with encrypted data
        assert len(added_rows) > 0
        new_row = added_rows[0]
        assert new_row.provider == "openai"
        assert new_row.key_hint == "1234"
        assert isinstance(new_row.encrypted_key, bytes)
        assert len(new_row.encrypted_key) > 0
        assert new_row.key_iv is not None
        assert new_row.key_tag is not None

    @patch("app.services.llm_settings_service.get_session_context")
    def test_get_settings_no_raw_key(self, mock_ctx):
        """6. Settings read via GET: key never in response, only hint."""
        from app.services.llm_settings_service import get_settings

        row = self._make_settings_row()
        mock_session = MagicMock()
        mock_session.exec.return_value.all.return_value = [row]
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)
        mock_ctx.return_value = mock_session

        result = get_settings()
        assert result.configured is True
        assert result.active_provider == "openai"
        assert len(result.providers) == 1
        resp = result.providers[0]
        # Key hint present, no raw key
        assert resp.key_hint == "cdef"
        resp_dict = resp.model_dump()
        assert "api_key" not in resp_dict
        assert "encrypted_key" not in resp_dict

    @patch("app.services.llm_settings_service.get_session_context")
    @patch("app.services.llm_settings_service.settings")
    def test_update_settings_reencrypts(self, mock_settings, mock_ctx):
        """7. Settings update: re-encrypt with new key."""
        from app.services.llm_settings_service import put_settings

        mock_settings.get_secret_key.return_value = SECRET_A
        existing = self._make_settings_row()

        mock_session = MagicMock()
        mock_session.exec.return_value.first.return_value = existing
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)
        mock_ctx.return_value = mock_session

        data = LLMSettingsCreate(
            provider="openai",
            api_key="sk-proj-newkey5678",
            model="gpt-4o-mini",
            set_active=True,
        )

        with patch("app.services.llm_settings_service._invalidate_llm_service"):
            result = put_settings(data)

        # Existing row should be updated
        assert existing.model == "gpt-4o-mini"
        assert existing.key_hint == "5678"

    @patch("app.services.llm_settings_service._invalidate_llm_service")
    @patch("app.services.llm_settings_service.get_session_context")
    def test_delete_settings(self, mock_ctx, mock_invalidate):
        """8. Settings delete."""
        from app.services.llm_settings_service import delete_settings

        row = self._make_settings_row()
        mock_session = MagicMock()
        mock_session.exec.return_value.first.return_value = row
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)
        mock_ctx.return_value = mock_session

        delete_settings("openai")
        mock_session.delete.assert_called_once_with(row)
        mock_session.commit.assert_called_once()
        mock_invalidate.assert_called_once()


# ═══════════════════════════════════════════════════════════════════════
# 9-10. Connection Test (mocked provider)
# ═══════════════════════════════════════════════════════════════════════

class TestConnectionTest:
    """Tests for LLM connection testing."""

    @patch("app.services.llm_settings_service._update_test_status")
    @patch("app.services.llm_settings_service._ping_provider", new_callable=AsyncMock)
    @patch("app.services.llm_settings_service.settings")
    @patch("app.services.llm_settings_service.get_session_context")
    def test_connection_success(self, mock_ctx, mock_settings, mock_ping, mock_update):
        """9. Test connection success (mock provider)."""
        from app.services.llm_settings_service import test_connection

        mock_settings.get_secret_key.return_value = SECRET_A
        mock_settings.previous_secret_key = None

        plaintext = "sk-test-key-here"
        ct, iv, tag = encrypt_api_key(plaintext, SECRET_A, provider_id="openai", scope="instance")
        row = LLMSettings(
            id="test-id",
            scope="instance",
            provider="openai",
            model="gpt-4o",
            encrypted_key=ct,
            key_iv=iv,
            key_tag=tag,
            key_version=1,
            created_at=datetime.now(timezone.utc),
            updated_at=datetime.now(timezone.utc),
        )

        mock_session = MagicMock()
        mock_session.exec.return_value.first.return_value = row
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)
        mock_ctx.return_value = mock_session

        mock_ping.return_value = "pong"

        result = test_connection("openai")
        assert result.ok is True
        assert result.provider == "openai"
        assert result.model == "gpt-4o"
        assert result.latency_ms is not None
        assert "successful" in result.message.lower()

    @patch("app.services.llm_settings_service._update_test_status")
    @patch("app.services.llm_settings_service._ping_provider", new_callable=AsyncMock)
    @patch("app.services.llm_settings_service.settings")
    @patch("app.services.llm_settings_service.get_session_context")
    def test_connection_auth_failure(self, mock_ctx, mock_settings, mock_ping, mock_update):
        """10. Test connection auth failure (mock, returns sanitized error)."""
        from app.services.llm_settings_service import test_connection

        mock_settings.get_secret_key.return_value = SECRET_A
        mock_settings.previous_secret_key = None

        plaintext = "sk-bad-key"
        ct, iv, tag = encrypt_api_key(plaintext, SECRET_A, provider_id="openai", scope="instance")
        row = LLMSettings(
            id="test-id",
            scope="instance",
            provider="openai",
            model="gpt-4o",
            encrypted_key=ct,
            key_iv=iv,
            key_tag=tag,
            key_version=1,
            created_at=datetime.now(timezone.utc),
            updated_at=datetime.now(timezone.utc),
        )

        mock_session = MagicMock()
        mock_session.exec.return_value.first.return_value = row
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)
        mock_ctx.return_value = mock_session

        mock_ping.side_effect = Exception("401 Unauthorized: invalid_api_key")

        result = test_connection("openai")
        assert result.ok is False
        assert result.error_code == "auth_failed"
        assert "api key" in result.message.lower()


# ═══════════════════════════════════════════════════════════════════════
# 11. Rate Limiting
# ═══════════════════════════════════════════════════════════════════════

class TestRateLimiting:
    """Test rate limiting for /test endpoint."""

    def test_rate_limit_fourth_call_429(self):
        """11. Test endpoint rate limiting: 4th call -> 429."""
        from app.routers.llm_admin import _check_rate_limit, _test_rate_limits
        from fastapi import HTTPException

        # Clear any existing state
        test_key = f"test:rate-limit-{uuid.uuid4()}"
        _test_rate_limits.pop(test_key, None)

        # First 3 calls should succeed
        for _ in range(3):
            _check_rate_limit(test_key)

        # 4th call should raise 429
        with pytest.raises(HTTPException) as exc_info:
            _check_rate_limit(test_key)
        assert exc_info.value.status_code == 429


# ═══════════════════════════════════════════════════════════════════════
# 12-13. Error Adapter Normalization
# ═══════════════════════════════════════════════════════════════════════

class TestErrorAdapters:
    """Tests for provider error normalization."""

    def test_openai_401_auth_failed(self):
        """12. Error adapter normalization: OpenAI 401 -> auth_failed."""
        adapter = OpenAIErrorAdapter()
        code, msg = adapter.normalize(Exception("Error 401: invalid_api_key"))
        assert code == "auth_failed"
        assert "api key" in msg.lower()

    def test_unknown_error_fallback(self):
        """13. Error adapter normalization: unknown error -> unknown_error."""
        adapter = BaseErrorAdapter()
        code, msg = adapter.normalize(Exception("something totally unexpected"))
        assert code == "unknown_error"
        assert "check logs" in msg.lower()

    def test_anthropic_auth_error(self):
        """Extra: Anthropic auth normalization."""
        adapter = AnthropicErrorAdapter()
        code, msg = adapter.normalize(Exception("401 invalid x-api-key"))
        assert code == "auth_failed"

    def test_gemini_403_auth_error(self):
        """Extra: Gemini 403 normalization."""
        adapter = GeminiErrorAdapter()
        code, msg = adapter.normalize(Exception("403 API key not valid"))
        assert code == "auth_failed"

    def test_registry_has_all_providers(self):
        """All providers have adapters in registry."""
        assert "openai" in ERROR_ADAPTERS
        assert "anthropic" in ERROR_ADAPTERS
        assert "gemini" in ERROR_ADAPTERS


# ═══════════════════════════════════════════════════════════════════════
# 14. Providers Endpoint
# ═══════════════════════════════════════════════════════════════════════

class TestProvidersCatalog:
    """Test providers catalog."""

    def test_providers_returns_catalog(self):
        """14. Providers endpoint returns catalog."""
        from app.services.llm_settings_service import get_providers

        result = get_providers()
        assert isinstance(result, LLMProvidersResponse)
        assert len(result.providers) == 3

        provider_ids = {p.id for p in result.providers}
        assert provider_ids == {"openai", "anthropic", "gemini"}

        for p in result.providers:
            assert len(p.models) >= 1
            assert p.key_prefix
            assert p.docs_url
            for m in p.models:
                assert m.id
                assert m.name
                assert m.context > 0
                assert m.tier in ("budget", "standard", "premium")


# ═══════════════════════════════════════════════════════════════════════
# 15-16. Usage Tracking
# ═══════════════════════════════════════════════════════════════════════

class TestUsageTracking:
    """Tests for usage logging and aggregation."""

    @patch("app.services.llm_settings_service.get_session_context")
    def test_log_usage_append(self, mock_ctx):
        """15. Usage log append."""
        from app.services.llm_settings_service import log_usage

        mock_session = MagicMock()
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)

        # Mock the get call for incrementing counters
        settings_row = MagicMock()
        settings_row.total_requests = 10
        settings_row.total_tokens = 5000
        mock_session.get.return_value = settings_row
        mock_ctx.return_value = mock_session

        log_usage(
            settings_id="test-settings-id",
            operation="rag_query",
            model="gpt-4o",
            input_tokens=100,
            output_tokens=50,
            latency_ms=200,
            success=True,
        )

        # Verify a log entry was added
        mock_session.add.assert_called()
        mock_session.commit.assert_called_once()

        # Verify counters incremented
        assert settings_row.total_requests == 11
        assert settings_row.total_tokens == 5150

    @patch("app.services.llm_settings_service.get_session_context")
    def test_usage_summary_aggregation(self, mock_ctx):
        """16. Usage summary aggregation."""
        from app.services.llm_settings_service import get_usage

        row = MagicMock()
        row.id = "test-id"
        row.provider = "openai"
        row.total_requests = 500
        row.total_tokens = 250000

        mock_session = MagicMock()
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)
        mock_session.exec.return_value.all.return_value = [row]
        # For the error count query
        mock_session.exec.return_value.one.return_value = 3
        mock_ctx.return_value = mock_session

        result = get_usage()
        assert len(result) == 1
        assert result[0].provider == "openai"
        assert result[0].total_requests == 500
        assert result[0].total_tokens == 250000


# ═══════════════════════════════════════════════════════════════════════
# 17. Status Endpoint
# ═══════════════════════════════════════════════════════════════════════

class TestStatus:
    """Tests for LLM status."""

    @patch("app.services.llm_settings_service.get_session_context")
    def test_status_configured(self, mock_ctx):
        """17a. Status endpoint: configured."""
        from app.services.llm_settings_service import get_status

        row = MagicMock()
        row.provider = "openai"
        row.model = "gpt-4o"
        row.last_tested_at = datetime.now(timezone.utc)
        row.last_test_ok = True

        mock_session = MagicMock()
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)
        mock_session.exec.return_value.first.return_value = row
        mock_ctx.return_value = mock_session

        result = get_status()
        assert result["configured"] is True
        assert result["active_provider"] == "openai"

    @patch("app.services.llm_settings_service.get_session_context")
    def test_status_not_configured(self, mock_ctx):
        """17b. Status endpoint: not configured."""
        from app.services.llm_settings_service import get_status

        mock_session = MagicMock()
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)
        mock_session.exec.return_value.first.return_value = None
        mock_ctx.return_value = mock_session

        result = get_status()
        assert result["configured"] is False
        assert result["active_provider"] is None


# ═══════════════════════════════════════════════════════════════════════
# 18. Provider Validation
# ═══════════════════════════════════════════════════════════════════════

class TestProviderValidation:
    """Test validation of provider names."""

    def test_reject_invalid_provider(self):
        """18. Provider validation: reject invalid_provider."""
        from app.services.llm_settings_service import VALID_PROVIDERS

        assert "invalid_provider" not in VALID_PROVIDERS
        assert "openai" in VALID_PROVIDERS
        assert "anthropic" in VALID_PROVIDERS
        assert "gemini" in VALID_PROVIDERS

        # put_settings should raise on invalid provider
        from app.services.llm_settings_service import put_settings

        data = LLMSettingsCreate(
            provider="invalid_provider",
            api_key="sk-test",
            model="gpt-4o",
        )
        with pytest.raises(ValueError, match="Invalid provider"):
            put_settings(data)


# ═══════════════════════════════════════════════════════════════════════
# 19-20. LLMService Lazy-Load
# ═══════════════════════════════════════════════════════════════════════

class TestLLMServiceLazyLoad:
    """Tests for LLMService DB-backed lazy-load and ENV fallback."""

    def setup_method(self):
        """Reset LLMService class state between tests."""
        from app.services.llm_service import LLMService
        LLMService.invalidate()

    @patch("app.services.llm_service.LLMService._create_provider_with_key")
    @patch("app.services.llm_service.LLMService._load_active_settings")
    @pytest.mark.asyncio
    async def test_lazy_load_from_db(self, mock_load, mock_create):
        """19. LLMService lazy-load from DB settings (mock DB)."""
        from app.services.llm_service import LLMService

        mock_settings_row = MagicMock()
        mock_settings_row.id = "settings-123"
        mock_settings_row.provider = "openai"
        mock_settings_row.model = "gpt-4o"
        mock_settings_row.encrypted_key = b"fake-ct"
        mock_settings_row.key_iv = b"fake-iv-1234"
        mock_settings_row.key_tag = b"fake-tag-123456!"
        mock_settings_row.key_version = 1
        mock_settings_row.updated_at = datetime.now(timezone.utc)
        mock_load.return_value = mock_settings_row

        mock_provider = MagicMock()
        mock_provider.generate = AsyncMock(return_value="hello")
        mock_create.return_value = mock_provider

        service = LLMService()

        with patch("app.core.llm_key_crypto.decrypt_with_fallback", return_value="sk-fake"):
            provider = await service._resolve_provider()

        assert provider == mock_provider
        mock_create.assert_called_once()
        assert LLMService._cached_settings_id == "settings-123"

    @patch("app.services.llm_service.LLMService._initialize_provider_from_env")
    @patch("app.services.llm_service.LLMService._load_active_settings")
    @pytest.mark.asyncio
    async def test_fallback_to_env(self, mock_load, mock_env):
        """20. LLMService fallback to ENV when no DB settings."""
        from app.services.llm_service import LLMService

        mock_load.return_value = None  # No DB settings

        mock_provider = MagicMock()
        mock_env.return_value = mock_provider

        service = LLMService()
        provider = await service._resolve_provider()

        assert provider == mock_provider
        mock_env.assert_called_once()
        # No settings_id when using ENV
        assert LLMService._cached_settings_id is None
