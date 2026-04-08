# BQ-VZ-DATA-CHANNEL — AIM Data Channel for vectorAIz

**Status:** Gate 1 — Needs Determination (R2)
**Author:** Vulcan (S415)
**Estimate:** 40–60h (to be re-evaluated after M1/M5 resolution)
**Repo:** aidotmarket/vectoraiz (monorepo)
**Council Reviews:** MP REVISE (6 mandates), AG APPROVE_WITH_MANDATES (2 mandates)

---

## 1. Problem Statement

vectorAIz currently serves two personas via the channel system (`direct` and `marketplace`). A third persona — the **data seller** — needs a purpose-built experience focused on getting data listed and sold on ai.market. This is the "AIM Data" product: **same codebase, same Docker image, different channel configuration**.

## 2. Solution: `aim-data` Channel

### 2.1 Architecture Decision

**One codebase, one Docker image, channel-driven menu configuration.** `VECTORAIZ_CHANNEL=aim-data` activates the seller-oriented experience.

### 2.2 Key Architectural Clarification (MP-M1)

**Optional vectorization is a product-wide capability, NOT channel-gated.** The existing channel contract is explicitly presentation-only (enforced in `channel_config.py`, `useChannel.ts`, and `test_channel_presentation_only.py`). This BQ does NOT violate that invariant.

What differs per channel is **presentation priority**:
- `direct`: Vectorization is the primary workflow, raw file listing is secondary
- `marketplace`: Listing creation is primary, vectorization supports listing quality
- `aim-data`: Data upload and marketplace listing are primary, vectorization is an optional enhancement

The underlying capability for raw file management and optional vectorization already exists via the raw file/listing stack (see §3.2). No new feature gating by channel is introduced.

### 2.3 Channel Behavior (Presentation Only)

| Aspect | `direct` | `marketplace` | `aim-data` |
|--------|----------|---------------|------------|
| Primary persona | Data engineer | Data seller (from ai.market) | Data seller (standalone) |
| UI emphasis | Vectorize → Query | Enrich → Publish | Upload → Manage → Publish |
| allAI focus | Data processing copilot | Listing creation copilot | Data management + publishing copilot |
| Default landing | Dashboard | ai.market page | My Data |
| Vectorization in UI | Primary action | Supporting action | Optional enhancement |

### 2.4 Menu Order (`aim-data` channel)

**Primary section:**
1. Dashboard (landing = "My Data" view)
2. Datasets (upload + manage)
3. ai.market (publish + earnings)
4. Data Requests ("I Need Data")

**Secondary section:**
5. Search
6. SQL Query
7. Artifacts
8. Databases

**Bottom section:**
9. Earnings
10. Billing
11. Data Types
12. Settings

Note: No relabeled route duplicates. Each nav item maps to a unique existing route. The "Upload" and "Manage" flows are accessed within the Datasets page, not as separate nav items. This preserves the existing `Map(path→item)` data structure in `Sidebar.tsx` (MP-M3).

## 3. Backend Changes

### 3.1 Channel Config Extension

Add `aim_data` to `ChannelType` enum in `app/core/channel_config.py`. Parse `VECTORAIZ_CHANNEL=aim-data` → `ChannelType.aim_data`. Add channel prompt templates in `channel_prompts.py`.

### 3.2 Reuse Existing Raw File/Listing Stack (MP-M5)

The repo already has a complete raw file listing infrastructure (750 lines, from BQ-VZ-RAW-LISTINGS):
- `app/models/raw_file.py` — RawFile model (filename, content hash, file_path)
- `app/models/raw_listing.py` — RawListing model (draft → listed → delisted lifecycle)
- `app/services/raw_file_service.py` — File registration, metadata extraction
- `app/services/raw_listing_service.py` — Listing creation, marketplace publishing
- `app/routers/raw_listings.py` — REST endpoints

**This BQ builds on this existing stack.** No new `vectorization_status` field on the dataset model. Instead:
- Raw files use the raw_file/raw_listing flow (already supports non-vectorized data)
- Datasets that go through vectorization use the existing dataset/listing flow
- The `aim-data` channel UI surfaces the raw file flow more prominently

### 3.3 allAI Metadata Extraction Enhancement

Extend `raw_file_service.py` with allAI-assisted metadata extraction for non-tabular formats:
- PDF: title, author, page count, topic extraction
- Images: dimensions, format, EXIF data, allAI-generated description
- Audio: duration, format, sample rate, summary
- Generic: file size, MIME type, allAI-generated description

Uses existing allAI infrastructure (proxied Gemini key). No new LLM integrations.

## 4. Frontend Changes

### 4.1 Complete Channel Fan-Out Points (MP-M2)

All files requiring `aim-data` channel support:

| File | Change Required |
|------|----------------|
| `app/core/channel_config.py` | Add `aim_data` to `ChannelType` enum, update `parse_channel()` |
| `app/prompts/channel_prompts.py` | Add `aim_data` entries to `CHANNEL_GREETINGS` and `CHANNEL_SYSTEM_CONTEXTS` |
| `frontend/src/hooks/useChannel.ts` | Add `"aim-data"` to `Channel` type union |
| `frontend/src/components/layout/Sidebar.tsx` | Add `NAV_ORDER_AIM_DATA`, `SEPARATOR_INDEX_AIM_DATA` |
| `frontend/src/contexts/ModeContext.tsx` | Handle `aim-data` channel in mode context initialization |
| `frontend/src/App.tsx` | Add `aim-data` landing page routing |
| `frontend/src/components/onboarding/OnboardingWizard.tsx` | Add `aim-data` onboarding flow variant |
| `frontend/src/contexts/CoPilotContext.tsx` | Add `aim-data` greeting (or deduplicate to use `channel_prompts` from backend) |

### 4.2 Upload Experience Enhancement

The `aim-data` channel should emphasize raw file upload:
- Prominent drop zone on Dashboard
- Progress tracking with allAI metadata extraction status
- Post-upload: preview + "Publish to ai.market" CTA
- Optional "Vectorize for AI queries" as secondary action

### 4.3 Dataset Detail View

Enhanced for raw files:
- File preview (image thumbnails, PDF first page, audio player)
- Metadata editor (allAI-suggested fields, manual override)
- Listing readiness indicator
- "Vectorize" button (opt-in, shows what it enables)

## 5. Deployment Reconciliation (MP-M4, AG-M1, AG-M2)

### 5.1 docker-compose.aim-data.yml

Current state uses a separate `ghcr.io/aidotmarket/aim-data` image with `AIM_DATA_*` env namespace. This contradicts the single-image architecture.

**Required changes:**
- Replace `ghcr.io/aidotmarket/aim-data` with `ghcr.io/aidotmarket/vectoraiz`
- Replace `AIM_DATA_*` env vars with standard `VECTORAIZ_*` vars + `VECTORAIZ_CHANNEL=aim-data`
- Remove `AIM_DATA_MODE`, `AIM_DATA_MARKETPLACE_ENABLED` (feature gating violates C1)
- Update service name from `aim-data` to `vectoraiz` (or keep as alias for clarity)
- Update installers if they reference the separate image

### 5.2 Image/CI Pipeline

No separate `ghcr.io/aidotmarket/aim-data` Docker build workflow needed. AIM Data deploys the standard vectoraiz image. The existing VZ CI/CD pipeline handles this.

## 6. Conditions & Constraints

- **C1:** Channel is presentation-only — no feature gating, auth, or billing changes (preserves existing invariant from BQ-VZ-CHANNEL)
- **C2:** All existing `direct` and `marketplace` channel behavior unchanged
- **C3:** Optional vectorization is product-wide, not channel-specific
- **C4:** No new Docker image — `aim-data` uses the existing VZ image
- **C5:** Raw file/listing flow uses existing BQ-VZ-RAW-LISTINGS infrastructure
- **C6:** allAI metadata extraction uses existing allAI infrastructure
- **C7:** No relabeled route duplicates in sidebar (preserves `Map(path→item)` structure)

## 7. Test Scope (MP-M6)

### Required test updates:
- `tests/test_channel_config.py` — Add `aim_data` parsing tests (valid value, env var)
- `tests/test_channel_prompts.py` — Add `aim_data` greeting and system context tests
- `tests/test_channel_presentation_only.py` — Extend to verify `aim-data` doesn't gate features
- New: Sidebar ordering test for `aim-data` channel
- New: Onboarding wizard test for `aim-data` flow
- New: Landing page routing test for `aim-data`
- New: docker-compose.aim-data.yml smoke test (image reference, env vars, channel config)

## 8. Slices (Revised)

| Slice | Scope | Estimate | Dependencies |
|-------|-------|----------|-------------|
| A | Channel config: enum, prompts, ALL frontend fan-out points (ModeContext, App.tsx, OnboardingWizard, CoPilotContext, Sidebar, useChannel), tests | 10-14h | None |
| B | Upload UX: prominent drop zone, progress tracking, raw file flow emphasis | 8-10h | A |
| C | allAI metadata extraction: extend raw_file_service with PDF/image/audio/generic metadata | 8-10h | A |
| D | Dataset detail: preview enhancements, metadata editor, listing readiness, vectorize CTA | 6-8h | A, B |
| E | Deployment reconciliation: docker-compose.aim-data.yml refactor, installer updates, smoke tests | 6-8h | A |

**Total: 38-50h**

## 9. Risks

- **R1:** allAI metadata extraction quality varies by format. Mitigation: always allow manual override.
- **R2:** Existing raw_file/raw_listing stack may need enhancements beyond what's documented. Mitigation: Slice C includes discovery/gap analysis.
- **R3:** CoPilotContext greeting duplication may need refactoring to use backend channel_prompts. Mitigation: Flag in Slice A, fix if trivial, defer if complex.

## 10. Success Criteria

1. `VECTORAIZ_CHANNEL=aim-data` activates seller-focused menu ordering and allAI persona
2. Raw files can be uploaded and listed on ai.market using existing raw_file/raw_listing flow
3. allAI extracts useful metadata from common non-tabular formats
4. Existing direct/marketplace channel behavior unaffected
5. `test_channel_presentation_only.py` passes with `aim-data` channel
6. docker-compose.aim-data.yml uses standard VZ image with `VECTORAIZ_CHANNEL=aim-data`
