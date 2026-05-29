# BQ-AIM-DATA-REMOVE-STANDALONE-MODE-S726

**Pillar:** AIM-Channel (vectorAIz shared codebase, AIM Data customer build)
**Type:** operational / customer-facing — product-definition
**Session:** 726
**Branch base:** origin/main (cite SHA used at build time)

## Intent
AIM Data has **no standalone/local mode**. It is always connected to ai.market,
and allAI must be available immediately on login. The current codebase inherits
vectorAIz's dual-mode machinery (`mode: "standalone" | "connected"`) which
defaults to "standalone" and SUPPRESSES allAI + connected features until a
backend call flips it. On the AIM Data build this is wrong by definition and is
why (a) allAI is invisible after login and (b) the UI shows "standalone / not
connected".

This BQ removes the standalone concept for the AIM Data build so the app is
always connected with allAI mounted. Scope is the mode machinery ONLY. Feature
removals (billing, API keys, processing, connectivity, portal, data types,
allAI credits, search reframing) are SEPARATE BQs — do not touch them here.

## Ground truth (traced S726, cite current SHAs when building)
- Mode authority: backend `app/routers/health.py::system_info()` returns
  `"mode": settings.mode`, `"features": {"allai": settings.allai_enabled and
  settings.mode != "standalone", ...}`, `"marketplace_api_url": ... if
  settings.mode != "standalone" else None`. Also `system_mode()` same file.
- Frontend authority: `frontend/src/contexts/ModeContext.tsx` —
  `useState<Mode>("standalone")` default; fetches `/api/system/info`; exposes
  `isStandalone`, `isConnected`, `hasFeature`.
- 20 consumers of `useMode`/`isStandalone`/`ModeProvider` (App.tsx,
  CoPilotContext, MarketplaceContext, CoPilotFab, ChatPanel, EmptyState,
  Sidebar, DashboardRequestsPage, DatasetDetail, Dashboard, SettingsPage,
  DataRequestDetailPage, CreateDataRequestPage, DataRequestsPage, EarningsPage,
  RawFileDetail, Datasets, AiMarketPage, useChannel.ts, ModeContext.tsx).
- allAI gating: `CoPilotFab.tsx` `if (isStandalone && !allieAvailable) return null;`

## Required changes
### Backend
1. For the AIM Data build/runtime, `settings.mode` must resolve to `connected`
   (not "standalone"). Implement so the AIM Data channel/brand forces
   `mode=connected` — do NOT rely on an env var a customer could leave unset.
   The `aim-data` channel (see channel_config) implies connected.
2. `system_info()` / `system_mode()`: when channel is aim-data, report
   `mode="connected"` and `features.allai=true` (gated only on
   `settings.allai_enabled`, NOT on mode). Keep behavior intact for the
   non-aim-data (vectorAIz) build.

### Frontend
3. For the AIM Data brand build, eliminate the standalone branch:
   - allAI/co-pilot always mounted on login (remove the
     `isStandalone && !allieAvailable` suppression for aim-data).
   - Remove "standalone"/"not connected" UI affordances (the Item-#4 "Backend
     connection" indicator is part of this — vestigial once always-connected;
     remove it).
   - `MarketplaceContext` must use the real provider (no no-op standalone path)
     for aim-data.
4. Cleanest implementation: keep `ModeContext` for the shared codebase but make
   the aim-data brand hard-pin `isConnected=true / isStandalone=false` and
   `features.allai=true`, so all 20 consumers behave connected without editing
   each call site's logic. Do NOT delete ModeContext outright (vectorAIz build
   still needs it). Verify EVERY consumer renders correctly under connected.

## Out of scope (separate BQs — do NOT touch)
Billing/Stripe, API-key management UI, Processing, External Connectivity,
Shared Search portal, Data Types, allAI credits, search semantic→filename
reframe, the VECTORAIZ_*→AIM_DATA_* env rename.

## Acceptance
- AIM Data build: login → connected, no standalone/not-connected text, allAI
  co-pilot visible immediately.
- vectorAIz build: unchanged (still mode-driven).
- `npm run build` (frontend) clean; backend `py_compile` + app import clean.
- No consumer of useMode throws; no dead-import errors.
