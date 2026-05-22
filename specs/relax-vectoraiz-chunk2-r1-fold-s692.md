# AIM Data vectoraiz chunk 2 R1 fold (Mars S691.W)

**Canonical body:** state entity `infra:worker-artifact-stash:S691.W:T3:vectoraiz-chunk2-r1-fold` v=1
**BQ:** BQ-AIM-DATA-NON-VECTOR-LISTING-METADATA-RELAX-S684
**Pillar:** AIM-Channel
**Base ref:** vectoraiz origin/main b6387f30 (stable since R1)
**Predecessor R1 spec:** infra:worker-artifact-stash:S688.W2:T3:gate2-chunk2-relax-vectoraiz-spec-draft v=1
**Ship dependency:** Chunk 2 MUST ship AFTER ai-market-backend chunk 1 (relax BQ) merges and deploys. Spec authoring and review are parallel-eligible.
**AG R2 verdict (task ca8918dc):** APPROVE clean — all 6 mandates verified at b6387f30 with no new HIGH/MED/LOW concerns; one trivial implementation note (ValidationError import for new validation test).

## Mandates folded (6 total: 0 HIGH + 4 MED + 2 LOW)

### MP_MED1 — endpoint count correction
**Finding:** Spec said 9 strict-other endpoints; actual is 13. There are 14 endpoints in `app/routers/datasets.py` with `record.status != ProcessingStatus.READY` checks; excluding line 1041 (which relaxes), 13 others remain strict.
**Delta:** AC-C2-1 narrative + acceptance criteria text updated with correct list (lines 783, 819, 872, 920, 954, 989, 1016, 1069, 1104, 1140, 1182, 1308, 1354 = 13 strict; 1041 = 1 relax; 14 total).

### CONVERGENT_MED — schema constraints (MP + AG)
**Finding:** `app/models/listing_metadata_schemas.py:29` verbatim `privacy_score: float = Field(1.0, description="1.0 = no PII detected, 0.0 = high PII risk")` has stale 0-1 description on a 0-10-canonical pipeline. Spec proposed Optional[float] = None but did not mandate ge/le constraints or replace description.
**Delta:** Change to `privacy_score: Optional[float] = Field(None, ge=0.0, le=10.0, description="0-10 scale, 10.0 = no PII detected, 0.0 = high PII risk; None = not scanned")` + add `from typing import Optional, List` import if missing.

### MP_LOW — exact path correction
**Finding:** Spec used approximate `app/models/listing_metadata.py [or wherever...]`.
**Delta:** Exact path is `app/models/listing_metadata_schemas.py:29`.

### AG_MED3 — _compute_privacy_score fallback fix
**Finding:** `app/services/listing_metadata_service.py:340` verbatim `return float(pii_data.get("privacy_score", 1.0))`. The 1.0 fallback is wrong on 0-10 scale (1.0 = critical risk, not perfect privacy).
**Delta:** Fallback returns None (propagates unscored signal). Return type annotation `Optional[float]`. Catch-block also returns None.

### AG_MED4 — test sample regression
**Finding:** `tests/test_marketplace_push.py:37` SAMPLE privacy_score=0.9 + assertion at :115 expects 9.0 via stale *10 multiplier. Removing the multiplier in marketplace_push_service.py breaks the test.
**Delta:** SAMPLE 0.9 → 9.0; multiplier comment removed (assertion value unchanged at 9.0).

### AG_LOW2 — range validation test
**Finding:** New ge=0.0/le=10.0 constraints need test coverage.
**Delta:** Add `test_listing_metadata_validation_fails_for_out_of_range_privacy_score` asserting pydantic raises ValidationError on -0.1 and 11.0. Tests 10 → 11.

## Post-fold acceptance criteria (excerpted; full body in canonical stash)

- AC-C2-1: datasets.py:1041 precondition relaxes to accept PREVIEW_READY; 13 strict-other endpoints unchanged
- AC-C2-3: _compute_privacy_score returns None when no PII scan exists + emits log warning; missing privacy_score field also returns None (not 1.0)
- AC-C2-4: marketplace_push_service.py:180 REMOVES `* 10` multiplier and stale comment; pass-through directly (or None); emit payload.privacy_scan_status='scanned' when value present, 'not_scanned' when None
- AC-C2-5: pii_service emission scale RESOLVED as 0-10 canonical per AG R1 verification + S687.W2 audit
- AC-C2-6: listing_metadata_schemas.py:29 Optional[float] Field(None, ge=0.0, le=10.0, description="0-10 scale...")
- AC-C2-9: new validation test asserts ValidationError on out-of-range

## Verification evidence at b6387f30 (verbatim cites)

- `app/routers/datasets.py`: 14 endpoints with `record.status != ProcessingStatus.READY` confirmed at lines 783/819/872/920/954/989/1016/1041/1069/1104/1140/1182/1308/1354
- `app/models/listing_metadata_schemas.py:29` verbatim `privacy_score: float = Field(1.0, description="1.0 = no PII detected, 0.0 = high PII risk")`
- `app/services/listing_metadata_service.py:169` verbatim `privacy_score=round(privacy_score, 4),`
- `app/services/listing_metadata_service.py:340` verbatim `return float(pii_data.get("privacy_score", 1.0))`
- `tests/test_marketplace_push.py:37` verbatim `"privacy_score": 0.9,` in SAMPLE_LISTING_METADATA
- `tests/test_marketplace_push.py:115` verbatim `assert payload["privacy_score"] == 9.0  # 0.9 * 10`
- `app/services/marketplace_push_service.py:179-180` verbatim `# Map privacy_score: vectoraiz uses 0.0-1.0, ai.market uses 0-10` + `privacy_score_10 = round(listing_metadata.privacy_score * 10, 1)`

## R2 review request

For each of the 6 mandates: verify the delta correctly addresses the original R1 finding (no semantic regression) and cross-check against vectoraiz origin/main b6387f30. Identify any new HIGH/MED/LOW concerns introduced by the fold.

Return APPROVE / APPROVE_WITH_MANDATES / REJECT verdict with rationale and new mandates owed for R3 if applicable.

AG R2 (task ca8918dc): APPROVE clean. MP R2 provides second-positive lock for Gate 2 R3-readiness.
