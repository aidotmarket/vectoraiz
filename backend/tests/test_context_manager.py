"""
Tests for CoPilotContextManager — Runtime state injection.
============================================================

Covers:
- Context includes screen/route from StateSnapshot
- Context includes active dataset
- Context includes system state (connected_mode, Qdrant status)
- Context includes user preferences (tone mode, quiet mode)
- Route-to-screen mapping
- Capability resolution
- Default context when no snapshot

PHASE: BQ-128 Phase 2 — Personality + Context Engine (Task 2.2)
CREATED: 2026-02-14
"""

import pytest
from unittest.mock import patch

from app.models.copilot import StateSnapshot
from app.services.context_manager_copilot import CoPilotContextManager
from app.services.prompt_factory import AllieContext


@pytest.fixture
def ctx_manager():
    return CoPilotContextManager()


@pytest.fixture
def sample_snapshot():
    return StateSnapshot(
        current_route="/datasets/ds_abc/preview",
        page_title="Data Preview",
        active_dataset_id="ds_abc",
        timestamp="2026-02-14T12:00:00Z",
    )


# ---------------------------------------------------------------------------
# Basic Context Building
# ---------------------------------------------------------------------------

class TestContextBuilding:
    """Tests for context assembly from various sources."""

    @pytest.mark.asyncio
    async def test_context_includes_screen_and_route(self, ctx_manager, sample_snapshot):
        ctx = await ctx_manager.build_context(state_snapshot=sample_snapshot)
        assert ctx.screen == "data_preview"
        assert ctx.route == "/datasets/ds_abc/preview"

    @pytest.mark.asyncio
    async def test_context_includes_active_dataset(self, ctx_manager, sample_snapshot):
        ctx = await ctx_manager.build_context(state_snapshot=sample_snapshot)
        assert ctx.selection.get("dataset_id") == "ds_abc"
        assert ctx.dataset_summary is not None
        assert ctx.dataset_summary["dataset_id"] == "ds_abc"

    @pytest.mark.asyncio
    @patch("app.services.context_manager_copilot.is_local_only", return_value=False)
    async def test_context_includes_system_state(self, _mock, ctx_manager):
        ctx = await ctx_manager.build_context()
        assert ctx.connected_mode is True
        assert ctx.qdrant_status == "healthy"
        assert ctx.local_only is False

    @pytest.mark.asyncio
    @patch("app.services.context_manager_copilot.is_local_only", return_value=True)
    async def test_context_local_only_mode(self, _mock, ctx_manager):
        ctx = await ctx_manager.build_context()
        assert ctx.connected_mode is False
        assert ctx.local_only is True

    @pytest.mark.asyncio
    async def test_context_includes_user_preferences(self, ctx_manager):
        prefs = {"tone_mode": "surfer", "quiet_mode": True}
        ctx = await ctx_manager.build_context(user_preferences=prefs)
        assert ctx.tone_mode == "surfer"
        assert ctx.quiet_mode is True

    @pytest.mark.asyncio
    async def test_context_default_tone_friendly(self, ctx_manager):
        ctx = await ctx_manager.build_context()
        assert ctx.tone_mode == "friendly"

    @pytest.mark.asyncio
    async def test_context_default_quiet_false(self, ctx_manager):
        ctx = await ctx_manager.build_context()
        assert ctx.quiet_mode is False


# ---------------------------------------------------------------------------
# Route-to-Screen Mapping
# ---------------------------------------------------------------------------

class TestRouteToScreen:
    """Tests for route-to-screen mapping logic."""

    def test_datasets_list(self, ctx_manager):
        assert ctx_manager._route_to_screen("/datasets") == "datasets_list"

    def test_dataset_preview(self, ctx_manager):
        assert ctx_manager._route_to_screen("/datasets/ds_abc/preview") == "data_preview"

    def test_dataset_query(self, ctx_manager):
        assert ctx_manager._route_to_screen("/datasets/ds_abc/query") == "query_builder"

    def test_dataset_upload(self, ctx_manager):
        assert ctx_manager._route_to_screen("/datasets/ds_abc/upload") == "upload_wizard"

    def test_dataset_detail(self, ctx_manager):
        assert ctx_manager._route_to_screen("/datasets/ds_abc") == "dataset_detail"

    def test_settings(self, ctx_manager):
        assert ctx_manager._route_to_screen("/settings") == "settings"

    def test_dashboard(self, ctx_manager):
        assert ctx_manager._route_to_screen("/dashboard") == "dashboard"

    def test_unknown_route(self, ctx_manager):
        assert ctx_manager._route_to_screen("/some/unknown/page") == "unknown"

    def test_trailing_slash(self, ctx_manager):
        assert ctx_manager._route_to_screen("/datasets/") == "datasets_list"


# ---------------------------------------------------------------------------
# Capability Resolution
# ---------------------------------------------------------------------------

class TestCapabilities:
    """Tests for capability resolution based on deployment mode."""

    def test_connected_mode_capabilities(self, ctx_manager):
        caps = ctx_manager._resolve_capabilities(connected_mode=True, local_only=False)
        assert caps["can_preview_rows"] is True
        assert caps["can_push_to_marketplace"] is True

    def test_local_only_capabilities(self, ctx_manager):
        caps = ctx_manager._resolve_capabilities(connected_mode=False, local_only=True)
        assert caps["can_preview_rows"] is True
        assert caps["can_push_to_marketplace"] is False

    def test_disconnected_capabilities(self, ctx_manager):
        caps = ctx_manager._resolve_capabilities(connected_mode=False, local_only=False)
        assert caps["can_push_to_marketplace"] is False


# ---------------------------------------------------------------------------
# No Snapshot / Edge Cases
# ---------------------------------------------------------------------------

class TestEdgeCases:
    """Tests for edge cases."""

    @pytest.mark.asyncio
    async def test_no_snapshot_defaults(self, ctx_manager):
        ctx = await ctx_manager.build_context()
        assert ctx.screen == "unknown"
        assert ctx.route == "/"
        assert ctx.selection == {}

    @pytest.mark.asyncio
    async def test_snapshot_without_dataset(self, ctx_manager):
        snapshot = StateSnapshot(
            current_route="/settings",
            page_title="Settings",
            timestamp="2026-02-14T12:00:00Z",
        )
        ctx = await ctx_manager.build_context(state_snapshot=snapshot)
        assert ctx.screen == "settings"
        assert ctx.dataset_summary is None

    @pytest.mark.asyncio
    async def test_context_is_allie_context(self, ctx_manager):
        ctx = await ctx_manager.build_context()
        assert isinstance(ctx, AllieContext)

    @pytest.mark.asyncio
    @patch("app.services.context_manager_copilot.is_local_only", return_value=False)
    async def test_rate_limits_in_connected_mode(self, _mock, ctx_manager):
        ctx = await ctx_manager.build_context()
        assert ctx.remaining_tokens_today is not None
        assert ctx.daily_token_limit is not None

    @pytest.mark.asyncio
    @patch("app.services.context_manager_copilot.is_local_only", return_value=True)
    async def test_no_rate_limits_in_local_mode(self, _mock, ctx_manager):
        ctx = await ctx_manager.build_context()
        assert ctx.remaining_tokens_today is None
        assert ctx.daily_token_limit is None

    @pytest.mark.asyncio
    async def test_quiet_mode_from_env(self, ctx_manager, monkeypatch):
        monkeypatch.setenv("ALLAI_QUIET_MODE", "true")
        ctx = await ctx_manager.build_context()
        assert ctx.quiet_mode is True
