"""
LLM Settings Service
====================

Business logic for LLM provider configuration management.
Handles encryption, CRUD, connection testing, and usage tracking.

Phase: BQ-125 — Connect Your LLM
Created: 2026-02-12
"""

from __future__ import annotations

import logging
import time
from datetime import datetime, timezone, timedelta
from typing import Optional

from sqlmodel import select, func

from app.config import settings
from app.core.database import get_session_context
from app.core.llm_key_crypto import encrypt_api_key, decrypt_api_key, decrypt_with_fallback
from app.core.llm_error_adapters import ERROR_ADAPTERS
from app.models.llm_settings import LLMSettings, LLMUsageLog
from app.schemas.llm_settings import (
    LLMSettingsCreate,
    LLMSettingsResponse,
    LLMSettingsListResponse,
    LLMTestResponse,
    LLMProviderInfo,
    LLMModelInfo,
    LLMProvidersResponse,
    LLMUsageSummary,
)

logger = logging.getLogger(__name__)

VALID_PROVIDERS = {"openai", "anthropic", "gemini"}

PROVIDER_CATALOG = {
    "openai": {
        "name": "OpenAI",
        "key_prefix": "sk-",
        "docs_url": "https://platform.openai.com/api-keys",
        "models": [
            {"id": "gpt-4o", "name": "GPT-4o", "context": 128000, "tier": "standard"},
            {"id": "gpt-4o-mini", "name": "GPT-4o Mini", "context": 128000, "tier": "budget"},
        ],
    },
    "anthropic": {
        "name": "Anthropic",
        "key_prefix": "sk-ant-",
        "docs_url": "https://console.anthropic.com/settings/keys",
        "models": [
            {"id": "claude-sonnet-4-20250514", "name": "Claude Sonnet 4", "context": 200000, "tier": "standard"},
            {"id": "claude-haiku-4-5-20251001", "name": "Claude Haiku 4.5", "context": 200000, "tier": "budget"},
        ],
    },
    "gemini": {
        "name": "Google Gemini",
        "key_prefix": "AI",
        "docs_url": "https://aistudio.google.com/apikey",
        "models": [
            {"id": "gemini-2.0-flash", "name": "Gemini 2.0 Flash", "context": 1000000, "tier": "standard"},
            {"id": "gemini-1.5-pro", "name": "Gemini 1.5 Pro", "context": 2000000, "tier": "premium"},
        ],
    },
}


def _row_to_response(row: LLMSettings) -> LLMSettingsResponse:
    return LLMSettingsResponse(
        provider=row.provider,
        model=row.model,
        display_name=row.display_name,
        key_hint=row.key_hint,
        is_active=row.is_active,
        last_tested_at=row.last_tested_at,
        last_test_ok=row.last_test_ok,
        total_requests=row.total_requests,
        total_tokens=row.total_tokens,
    )


def get_settings() -> LLMSettingsListResponse:
    """List all configured providers (masked keys)."""
    with get_session_context() as session:
        stmt = (
            select(LLMSettings)
            .where(LLMSettings.scope == "instance")
            .order_by(LLMSettings.created_at)
        )
        rows = session.exec(stmt).all()

    providers = [_row_to_response(r) for r in rows]
    active = next((p for p in providers if p.is_active), None)

    return LLMSettingsListResponse(
        configured=len(providers) > 0,
        active_provider=active.provider if active else None,
        providers=providers,
    )


def put_settings(data: LLMSettingsCreate) -> LLMSettingsResponse:
    """Encrypt key, upsert provider config, invalidate LLMService cache."""
    if data.provider not in VALID_PROVIDERS:
        raise ValueError(f"Invalid provider: {data.provider}")

    secret = settings.get_secret_key()
    ciphertext, iv, tag = encrypt_api_key(
        data.api_key, secret, provider_id=data.provider, scope="instance",
    )
    key_hint = data.api_key[-4:] if len(data.api_key) >= 4 else data.api_key
    now = datetime.now(timezone.utc)

    with get_session_context() as session:
        # Upsert: find existing row for this scope+provider
        stmt = (
            select(LLMSettings)
            .where(LLMSettings.scope == "instance")
            .where(LLMSettings.user_id.is_(None))  # type: ignore[union-attr]
            .where(LLMSettings.provider == data.provider)
        )
        existing = session.exec(stmt).first()

        if existing:
            existing.model = data.model
            existing.display_name = data.display_name
            existing.encrypted_key = ciphertext
            existing.key_iv = iv
            existing.key_tag = tag
            existing.key_version = 1
            existing.key_hint = key_hint
            existing.updated_at = now
            if data.set_active:
                # Deactivate other providers
                _deactivate_others(session, data.provider)
                existing.is_active = True
            session.add(existing)
            session.commit()
            session.refresh(existing)
            row = existing
        else:
            row = LLMSettings(
                scope="instance",
                user_id=None,
                provider=data.provider,
                model=data.model,
                display_name=data.display_name,
                encrypted_key=ciphertext,
                key_iv=iv,
                key_tag=tag,
                key_hint=key_hint,
                is_active=data.set_active,
                created_at=now,
                updated_at=now,
            )
            if data.set_active:
                _deactivate_others(session, data.provider)
            session.add(row)
            session.commit()
            session.refresh(row)

    # Invalidate LLMService cache
    _invalidate_llm_service()

    return _row_to_response(row)


def delete_settings(provider: str) -> None:
    """Remove provider config."""
    with get_session_context() as session:
        stmt = (
            select(LLMSettings)
            .where(LLMSettings.scope == "instance")
            .where(LLMSettings.provider == provider)
        )
        row = session.exec(stmt).first()
        if not row:
            raise ValueError(f"Provider '{provider}' not configured")
        session.delete(row)
        session.commit()

    _invalidate_llm_service()


def test_connection(provider: str) -> LLMTestResponse:
    """Load stored key, decrypt, create temp provider, ping."""
    with get_session_context() as session:
        stmt = (
            select(LLMSettings)
            .where(LLMSettings.scope == "instance")
            .where(LLMSettings.provider == provider)
        )
        row = session.exec(stmt).first()
        if not row:
            return LLMTestResponse(
                ok=False,
                provider=provider,
                message=f"Provider '{provider}' not configured. Save settings first.",
                error_code="not_configured",
            )

        # Decrypt key
        secret = settings.get_secret_key()
        previous = settings.previous_secret_key
        try:
            api_key = decrypt_with_fallback(
                row.encrypted_key, row.key_iv, row.key_tag,
                secret, previous, row.key_version,
                provider_id=row.provider, scope=row.scope,
            )
        except Exception:
            logger.error("Failed to decrypt key for provider=%s", provider)
            return LLMTestResponse(
                ok=False,
                provider=provider,
                message="Failed to decrypt stored API key. SECRET_KEY may have changed.",
                error_code="decrypt_failed",
            )

        model = row.model
        settings_id = row.id

    # Attempt a cheap 1-token completion
    start = time.time()
    try:
        import asyncio
        result = asyncio.get_event_loop().run_until_complete(
            _ping_provider(provider, api_key, model)
        )
        latency_ms = int((time.time() - start) * 1000)

        # Update test status
        _update_test_status(settings_id, True)

        return LLMTestResponse(
            ok=True,
            provider=provider,
            model=model,
            latency_ms=latency_ms,
            message="Connection successful",
        )
    except Exception as exc:
        latency_ms = int((time.time() - start) * 1000)
        adapter = ERROR_ADAPTERS.get(provider, ERROR_ADAPTERS.get("openai"))
        error_code, user_message = adapter.normalize(exc)
        # Log only exception type and error code — raw exception may contain API keys
        logger.warning("LLM test failed: provider=%s error=%s exc_type=%s", provider, error_code, type(exc).__name__)

        _update_test_status(settings_id, False)

        return LLMTestResponse(
            ok=False,
            provider=provider,
            model=model,
            latency_ms=latency_ms,
            message=user_message,
            error_code=error_code,
        )


async def _ping_provider(provider: str, api_key: str, model: str) -> str:
    """Do a cheap 1-token completion to verify the key works."""
    if provider == "openai":
        from openai import AsyncOpenAI
        client = AsyncOpenAI(api_key=api_key)
        resp = await client.chat.completions.create(
            model=model,
            messages=[{"role": "user", "content": "ping"}],
            max_tokens=1,
        )
        return resp.choices[0].message.content or ""

    elif provider == "anthropic":
        import anthropic
        client = anthropic.AsyncAnthropic(api_key=api_key)
        resp = await client.messages.create(
            model=model,
            max_tokens=1,
            messages=[{"role": "user", "content": "ping"}],
        )
        return resp.content[0].text if resp.content else ""

    elif provider == "gemini":
        import google.generativeai as genai
        genai.configure(api_key=api_key)
        gen_model = genai.GenerativeModel(model)
        resp = await gen_model.generate_content_async(
            "ping",
            generation_config=genai.GenerationConfig(max_output_tokens=1),
        )
        return resp.text or ""

    raise ValueError(f"Unsupported provider: {provider}")


def _update_test_status(settings_id: str, success: bool) -> None:
    now = datetime.now(timezone.utc)
    with get_session_context() as session:
        row = session.get(LLMSettings, settings_id)
        if row:
            row.last_tested_at = now
            row.last_test_ok = success
            row.updated_at = now
            session.add(row)
            session.commit()


def get_providers() -> LLMProvidersResponse:
    """Return static provider/model catalog."""
    providers = []
    for pid, info in PROVIDER_CATALOG.items():
        providers.append(
            LLMProviderInfo(
                id=pid,
                name=info["name"],
                models=[LLMModelInfo(**m) for m in info["models"]],
                key_prefix=info["key_prefix"],
                docs_url=info["docs_url"],
            )
        )
    return LLMProvidersResponse(providers=providers)


def get_usage(provider: Optional[str] = None) -> list[LLMUsageSummary]:
    """Aggregate usage from llm_usage_log."""
    with get_session_context() as session:
        # Get settings for aggregation
        stmt = select(LLMSettings).where(LLMSettings.scope == "instance")
        if provider:
            stmt = stmt.where(LLMSettings.provider == provider)
        rows = session.exec(stmt).all()

        results = []
        cutoff = datetime.now(timezone.utc) - timedelta(hours=24)

        for row in rows:
            # Count recent errors
            err_stmt = (
                select(func.count())
                .select_from(LLMUsageLog)
                .where(LLMUsageLog.settings_id == row.id)
                .where(LLMUsageLog.success == False)  # noqa: E712
                .where(LLMUsageLog.ts >= cutoff)
            )
            recent_errors = session.exec(err_stmt).one()

            results.append(
                LLMUsageSummary(
                    provider=row.provider,
                    total_requests=row.total_requests,
                    total_tokens=row.total_tokens,
                    recent_errors=recent_errors,
                )
            )

    return results


def get_status() -> dict:
    """Is any provider configured + active?"""
    with get_session_context() as session:
        stmt = (
            select(LLMSettings)
            .where(LLMSettings.scope == "instance")
            .where(LLMSettings.is_active == True)  # noqa: E712
        )
        active = session.exec(stmt).first()

    if active:
        return {
            "configured": True,
            "active_provider": active.provider,
            "active_model": active.model,
            "last_tested_at": active.last_tested_at.isoformat() if active.last_tested_at else None,
            "last_test_ok": active.last_test_ok,
        }
    return {"configured": False, "active_provider": None}


def log_usage(
    settings_id: str,
    operation: str,
    model: str,
    input_tokens: int = 0,
    output_tokens: int = 0,
    latency_ms: Optional[int] = None,
    success: bool = True,
    error_code: Optional[str] = None,
) -> None:
    """Append to usage log + increment counters on llm_settings."""
    now = datetime.now(timezone.utc)
    with get_session_context() as session:
        log_entry = LLMUsageLog(
            settings_id=settings_id,
            ts=now,
            operation=operation,
            model=model,
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            latency_ms=latency_ms,
            success=success,
            error_code=error_code,
        )
        session.add(log_entry)

        # Increment counters
        row = session.get(LLMSettings, settings_id)
        if row:
            row.total_requests += 1
            row.total_tokens += input_tokens + output_tokens
            row.updated_at = now
            session.add(row)

        session.commit()


def _deactivate_others(session, active_provider: str) -> None:
    """Deactivate all instance-scope providers except the given one."""
    stmt = (
        select(LLMSettings)
        .where(LLMSettings.scope == "instance")
        .where(LLMSettings.provider != active_provider)
        .where(LLMSettings.is_active == True)  # noqa: E712
    )
    for row in session.exec(stmt).all():
        row.is_active = False
        row.updated_at = datetime.now(timezone.utc)
        session.add(row)


def _invalidate_llm_service() -> None:
    """Invalidate LLMService provider cache after config change."""
    try:
        from app.services.llm_service import LLMService
        LLMService.invalidate()
    except Exception:
        pass
