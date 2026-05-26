# BQ-AIM-DATA-S3-STS-FULFILLMENT-S711 — Gate 1 Spec v1

**Branch:** `spec/bq-aim-data-s3-sts-fulfillment-s711-gate1`
**Pillar:** AIM-Channel (primary) + ai.market + allAI
**Priority:** P0
**Predecessor:** BQ-AIM-DATA-S3-SOURCE-CONNECTOR-S684 (PARKED; surviving elements per stash `infra:worker-artifact-stash:S711.W:r3-t4-c-sts-surviving-element-extraction`)
**Authored by:** Mars Worker S711.W round 4 — 2026-05-26
**Predecessor reads:** vectoraiz-monorepo `cd8bb5dc` + ai-market-backend `ef2e6d07` on origin/main

---

## §1 Problem statement

The ai.market non-custodial-data-marketplace promise is that seller data never transits ai.market servers. Today AIM-Data covers AI-queryable listings (upload → DuckDB extraction → vector index) and raw-file listings (upload → local storage with tunneled gatekeeper delivery). Neither path serves the case where a seller's data already lives in their own S3 bucket and they don't want to upload it — Sergey's case, and the general data-owner case for sellers with bulk catalogued data on cloud object storage.

Two distinct delivery requirements need to be served:

1. **Buyer fulfillment.** At purchase time the buyer receives a short-lived URL that streams bytes directly from the seller's S3 bucket. ai.market servers never see the bytes.
2. **Agent QA access.** At listing-creation time, and on buyer questions later, ai-market's allAI agents need to read sample data from the seller's bucket for quality scoring, metadata enrichment, and answer-first response generation. Same non-custodial constraint: ai.market processes bytes only in-memory at scoring time; nothing is persisted.

Both requirements are solved by the same AWS primitive: **STS AssumeRole + presigned URLs**. The seller creates a single IAM role in their AWS account with a two-policy structure (trust policy + permissions policy). ai-market's platform identity AssumeRole-s into the seller's role on demand to either (a) generate buyer-facing presigned URLs at purchase time or (b) get short-lived S3 client credentials for agent runtime use. Seller IAM access keys are NEVER created — only the role exists. ai-market never persists seller secrets at rest. CloudTrail in the seller's AWS account audits every cross-account access.

The implementation spans nine chunks across four surfaces (vectorAIz, ai-market-backend, ai-market-frontend, allAI). The largest single chunk is a new allAI conversational onboarding agent that walks sellers through IAM role setup with diagnostic feedback on the four most common failure modes: ExternalId mismatch, MaxSessionDuration too low, insufficient bucket permissions, role-trust-policy drift.

## §2 Architecture overview + per-pillar surface

References cite source-file:line on origin/main as of HEAD `cd8bb5dc` (vectoraiz-monorepo) and `ef2e6d07` (ai-market-backend).

### §2.1 AIM-Channel — vectorAIz (seller's machine)

Components added:

- `app/models/s3_connection.py` — `S3Connection` SQLModel storing seller bucket pointer + role ARN + ExternalId. No secret material at rest. Mirrors connection-record pattern at `app/models/database_connection.py:18` `class DatabaseConnection` but replaces `password_encrypted: str = Field(sa_column=Column(Text))` with `role_arn: str` + `external_id: str`.
- `app/models/s3_scan_job.py` — `S3ScanJob` tracks paginated bucket scan progress (continuation_token + status). Lighter than the predecessor `S3IngestJob` (no per-file download status).
- `app/models/s3_object_metadata.py` — `S3ObjectMetadata` (one row per S3 object). `dataset_id` FK lives on the per-object row, not on the scan job, per M7 cardinality mandate carried forward from BQ-S684 R3 fold.
- `app/services/sts_credential_broker.py` — STS AssumeRole + boto3 client cache. Mirrors `app/services/llm_providers/bedrock.py:40-75` (ai-market-backend) pattern verbatim: `sts_client = boto3.client("sts", ...)` → `assumed_role = sts_client.assume_role(RoleArn=..., RoleSessionName=...)` → `boto3.client("s3", aws_session_token=credentials["SessionToken"], ...)`. Per-seller RoleSessionName per locked-decision format.
- `app/services/s3_connector.py` — Reshaped from predecessor design (`app/services/db_connector.py:108` `class DatabaseConnector` 695 LoC pattern). Keeps paginated `list_objects_v2` + continuation_token persistence + boto3 client cache + single `threading.Lock` + `asyncio.to_thread` bridge (M8 mandate carries forward). Byte-download paths removed.
- `app/routers/s3.py` — REST router mirroring `app/routers/database.py:133-223` CRUD shape (`@router.post("/connections")` create + GET list + GET/{id} detail + DELETE + POST /test). Adds `POST /{id}/test-assume-role` (probes STS path without bucket access), `POST /{id}/scan` (kicks off async scan), `POST /{id}/sign-url` (gatekeeper endpoint — see §2.2 for invocation).

Data flow at scan time: vectorAIz cron or seller-triggered `POST /scan` → S3Connector calls STSCredentialBroker.get_or_create_client(connection_id) → STSCredentialBroker AssumeRole-s into seller's role with ExternalId from `S3Connection` → paginated `list_objects_v2` walks bucket → per object, `S3ObjectMetadata` row inserted → per batch, scan orchestrator invokes `processing_service.create_dataset` (`app/services/processing_service.py:294`) whose signature is `def create_dataset(self, original_filename: str, file_type: str) -> DatasetRecord` and which expects **local file** input at `self.upload_dir / safe_filename`.

Because C-STS does NOT download bytes for storage, the C5 scan orchestrator uses an **ephemeral-download adapter**: for each object that scan elects to extract metadata from, generate presigned URL, fetch object bytes into `self.upload_dir / safe_filename`, call `create_dataset`, persist `dataset_id` to the `S3ObjectMetadata` row, then immediately delete the local copy. Bytes touch the seller's local AIM-Data installation for at most one extraction cycle. Bytes NEVER leave the seller's machine; the buyer-fulfillment path is a separate pure pass-through.

### §2.2 ai.market — ai-market-backend (platform)

Components added:

- `app/services/sts_credential_broker.py` — mirror of vectorAIz side, same boto3 pattern from `app/services/llm_providers/bedrock.py:40-75`. Lives on platform infrastructure.
- `app/services/order_service.py:147` `async def create_order` — branched: when `listing_snapshot["fulfillment_type"] == FulfillmentType.SELLER_S3_PRESIGNED_URL.value`, the order-create path calls STSCredentialBroker to AssumeRole into the seller's role and generates a presigned URL for the buyer-purchased S3ObjectMetadata. URL is included in the FulfillmentDownloadToken issuance.
- `app/api/v1/endpoints/orders.py:619` `@router.post("/{order_id}/refresh") async def refresh_access` — already wired in current code for vectorAIz fulfillment refresh; extended to call STSCredentialBroker re-sign for SELLER_S3_PRESIGNED_URL when buyer's URL has expired.

Schema delta:

- `app/models/marketplace.py:37` `class FulfillmentType(str, enum.Enum)` extended with `SELLER_S3_PRESIGNED_URL = "seller_s3_presigned_url"`.
- `app/schemas/listing.py:31` matching extension.
- Alembic migration `ALTER TYPE fulfillment_type_enum ADD VALUE IF NOT EXISTS 'seller_s3_presigned_url'`. Note `create_type=False` at `app/models/marketplace.py:122` — the enum is managed externally by migration; PostgreSQL ALTER TYPE is non-transactional so migration runs alone in its own transaction.

Data flow at purchase: buyer completes Stripe checkout → `order_service.create_order(buyer_id, seller_id, listing_id, ..., listing_snapshot)` → branch on `fulfillment_type == SELLER_S3_PRESIGNED_URL` → STSCredentialBroker.get_or_create_s3_client(seller's S3Connection ref) → `s3.generate_presigned_url('get_object', Params={'Bucket', 'Key'}, ExpiresIn=900)` → URL returned via the existing `app/core/security.py:200` `issue_fulfillment_token` pathway (JWT-based; preserves replay-prevention `jti` per security.py:240 + the `seller_gatekeeper_url` injection pattern at security.py:258). Buyer presents JWT to ai-market refresh endpoint when URL expires; refresh path re-signs via the same broker.

Non-custodial proof chain: seller IAM user credentials are NEVER created. Seller's role trusts ai-market platform identity only via the seller-specific ExternalId. STS-derived credentials live in memory on ai-market-backend for the duration of the URL-signing operation (~10ms). The presigned URL contains a non-reversible signature derived from the temporary credentials. Buyer receives only the URL. Seller's CloudTrail logs every AssumeRole event + every S3 GetObject with `userIdentity.arn: arn:aws:sts::SELLER:assumed-role/aim-data-fulfillment/aim-{seller_id}-{order_id}` — full auditability.

### §2.3 allAI — onboarding agent

New agent at `app/allai/agents/aim_data_onboarding/agent.py` subclassing `app/allai/base_agent.py:90` `BaseAgent`. Mirrors the existing 8-agent registry pattern (`matchmaker.py:55`, `marketing_ops.py:65`, `crm_steward.py:157`, etc., per `r3-t5-allai-onboarding-flow-predecessor-read`). Co-located tools:

- `iam_policy_templates.py` — generates trust policy JSON + permissions policy JSON with `{bucket_name}`, `{external_id}`, `{platform_arn}` templated per seller. Trust policy enforces four required conditions: single Principal (the ai-market platform role ARN), `sts:AssumeRole` action only, `StringEquals` ExternalId condition (confused-deputy mitigation per AWS docs), no wildcards.
- `sts_probe.py` — invokes the platform-side STSCredentialBroker test path to validate seller-provided role ARN + ExternalId match the trust policy.
- `diagnostics.py` — classifies AssumeRole error responses. AccessDenied → ExternalId mismatch OR wrong Principal OR insufficient permissions. ValidationError → MaxSessionDuration too low OR malformed role ARN. Each classification ships with a remediation snippet the seller can paste into the IAM console.

Walkthrough conversation flow: (1) Generate per-seller ExternalId following the locked format `aim-data-seller-{uuid}-{32hex}`. Persist to a pre-row `S3Connection` with status=onboarding. (2) Generate trust + permissions policy JSON with seller's bucket name. Display via allAI chat with copy-to-clipboard buttons. (3) Wait for seller to paste role ARN. (4) Invoke `sts_probe` against the pasted ARN. (5) On success: write S3Connection row to vectorAIz instance via `POST /api/v1/s3-connections` then transition to bucket browser. (6) On AccessDenied: `diagnostics.classify_assume_role_error` produces structured remediation (e.g., "Your role's trust policy is missing the ExternalId condition. Add this StringEquals block: ..."). Surface to seller. (7) Idempotent flow with checkpoint state — seller can walk away and resume from the pre-row.

### §2.4 ai-market-frontend — buyer download UX

Component added:

- `src/components/PresignedDownloadButton.tsx` — when order has `fulfillment_type == SELLER_S3_PRESIGNED_URL`, render an `<a download href={presigned_url}>` element. The buyer's browser navigates directly to S3; no CORS configuration is required on the seller's bucket. On 403/expired URL: invoke the `app/api/v1/endpoints/orders.py:619` refresh endpoint, receive a fresh URL, retry the download.

## §3 Schema deltas + API contracts

### §3.1 vectorAIz schema (three new tables)

`S3Connection` (`app/models/s3_connection.py`): `id` (str primary key, 36 chars), `name` (str 255), `bucket` (str 255), `region` (str 64), `role_arn` (str 512), `external_id` (str 128), `prefix` (optional str 512), `status` (str 32, default "configured"), `error_message` (optional Text), `last_scanned_at` (optional datetime), `continuation_token` (optional Text), `created_at`, `updated_at`. No secret columns.

`S3ScanJob` (`app/models/s3_scan_job.py`): `id`, `connection_id` (FK), `status` (pending | running | completed | failed), `started_at`, `completed_at` (optional), `continuation_token` (optional), `error_message` (optional), `objects_enumerated` (int default 0).

`S3ObjectMetadata` (`app/models/s3_object_metadata.py`): `id`, `connection_id` (FK), `scan_job_id` (FK), `object_key` (str 1024), `size_bytes` (int), `content_type` (str 128), `last_modified` (datetime), `etag` (str 128), `dataset_id` (optional FK to `datasets.dataset_id` — per M7 cardinality on the per-object row), `metadata_extracted_at` (optional datetime).

Alembic migration `alembic/versions/<date>_001_s3_sts_connector_schema.py`: CREATE TABLE for all three; indexes on `(connection_id, object_key)`, `(connection_id, scan_job_id)`; FK cascades on connection delete.

### §3.2 vectorAIz REST API (new endpoints)

`POST /api/v1/s3-connections` — create. Body `{name, bucket, region, role_arn, external_id, prefix?}`. Returns 201 with full record.
`GET /api/v1/s3-connections` — list seller's connections.
`GET /api/v1/s3-connections/{id}` — detail.
`PUT /api/v1/s3-connections/{id}` — update name/prefix only. Immutable: bucket, region, role_arn, external_id.
`DELETE /api/v1/s3-connections/{id}` — soft-delete (status → archived).
`POST /api/v1/s3-connections/{id}/test-assume-role` — probe path. Returns success or structured error.
`POST /api/v1/s3-connections/{id}/scan` — kick off async scan job. Returns `scan_job_id`.
`POST /api/v1/s3-connections/{id}/sign-url` — gatekeeper endpoint. Internal-only JWT-authenticated. Body `{object_key, expires_in_seconds?}`. Returns `{presigned_url, expires_at}`.

### §3.3 ai-market-backend schema delta

- `app/models/marketplace.py:37` enum extension: add `SELLER_S3_PRESIGNED_URL = "seller_s3_presigned_url"`.
- `app/schemas/listing.py:31` mirror extension.
- Alembic migration: `ALTER TYPE fulfillment_type_enum ADD VALUE IF NOT EXISTS 'seller_s3_presigned_url';` Run standalone (non-transactional).

### §3.4 ai-market-backend service signatures

`STSCredentialBroker` (`app/services/sts_credential_broker.py` NEW): `async def get_or_create_s3_client(role_arn: str, external_id: str, region: str, session_purpose: str) -> boto3.client`. Cache key `(role_arn, region)`. Refresh 2-min before STS session expiry. Single `threading.Lock` + `asyncio.to_thread` bridge.

`order_service.create_order` (`app/services/order_service.py:147` BRANCH): on `fulfillment_type == SELLER_S3_PRESIGNED_URL`, resolve seller's S3Connection record from listing snapshot, invoke broker for client, call `s3.generate_presigned_url('get_object', Params, ExpiresIn=900)`, wire URL into `app/core/security.py:200` `issue_fulfillment_token` (JWT pathway) for return to buyer.

`refresh_access` (`app/api/v1/endpoints/orders.py:619` EXTENSION): on expired URL for SELLER_S3_PRESIGNED_URL order, re-sign via broker. Existing JWT validation + downloads-remaining checks unchanged.

## §4 Per-chunk acceptance criteria

### C1 — Schema + models + migration (vectorAIz)
Scope: three SQLModel tables per §3.1 + Alembic migration. Files: `app/models/s3_connection.py`, `app/models/s3_scan_job.py`, `app/models/s3_object_metadata.py`, `alembic/versions/<date>_001_s3_sts_connector_schema.py`. Deps: none. Parallel: no (foundation). LoC: ~150. ACs: (a) `alembic upgrade head` creates the three tables idempotently. (b) `S3Connection` constraints reject empty `role_arn` or empty `external_id`. (c) `S3ObjectMetadata.dataset_id` lives on the per-object row (not on `S3ScanJob`) per M7. (d) FK cascades: deleting an `S3Connection` cascades to its scan jobs + object metadata.

### C2 — STS credential broker (vectorAIz)
Scope: `app/services/sts_credential_broker.py` mirroring `app/services/llm_providers/bedrock.py:40-75` (ai-market-backend) verbatim. Deps: C1. Parallel: no. LoC: ~200. ACs: (a) `get_or_create_s3_client(connection_id)` returns a fresh boto3 S3 client. (b) Cache keyed by `(connection_id, region)`. Refresh 2-min before STS session expiry. (c) Single `threading.Lock` + `asyncio.to_thread` bridge for cache safety (M8 carried). (d) RoleSessionName per locked-decision format `aim-{seller_id}-{order_id_or_purpose}`. (e) AssumeRole failure raises classified exception subclasses (ExternalIdMismatch, TrustPolicyPrincipalWrong, BucketPermissionsInsufficient, RoleMaxSessionTooLow, MalformedRoleArn).

### C3 — S3 connector backend service (vectorAIz)
Scope: `app/services/s3_connector.py` reshaped from predecessor `app/services/db_connector.py:108`. Deps: C2. Parallel: no. LoC: ~200. ACs: (a) `scan_bucket(connection_id)` paginated `list_objects_v2` with continuation_token persistence on `S3Connection.continuation_token`. (b) Resume from last good token after process restart. (c) `generate_presigned_url(connection_id, object_key, expires_in=900)` wraps STSCredentialBroker. (d) `test_assume_role(connection_id)` probes STS only — does not touch bucket. (e) `list_objects_preview(connection_id, prefix?, max_results=20)` for bucket browser UI.

### C4 — Management API + gatekeeper endpoint (vectorAIz)
Scope: `app/routers/s3.py` per §3.2. Files: `app/routers/s3.py`, `app/main.py` router registration. Deps: C3. Parallel: YES with C5. LoC: ~200. ACs: (a) CRUD endpoints mirror `app/routers/database.py:133-223` shape. (b) `POST /sign-url` requires internal JWT auth — not exposed to public buyers. (c) `POST /test-assume-role` returns 200 with diagnostic or 400 with classified error. (d) OpenAPI schema correctly types all request/response models. (e) Rate-limit `POST /sign-url` to 10/sec/connection.

### C5 — Scan orchestrator (vectorAIz)
Scope: `app/services/s3_scan_orchestrator.py` (NEW). Walks bucket, populates `S3ObjectMetadata`, ephemerally downloads each object for `processing_service.create_dataset(original_filename, file_type)` (signature per `app/services/processing_service.py:294`) invocation. Deps: C3. Parallel: YES with C4. LoC: ~250. ACs: (a) End-to-end scan of 10-object moto bucket produces 10 `S3ObjectMetadata` rows + 10 `DatasetRecord` rows. (b) Ephemeral download cleanup: after each `create_dataset` returns, temp file deleted from `upload_dir`. (c) Continuation_token persisted; resume from last position. (d) Per-object error tolerated; failed objects logged but scan continues. (e) `S3ScanJob.status` transitions pending → running → completed/failed correctly.

### C6 — ai-market-backend purchase-flow integration + STS service
Scope: per §2.2 + §3.3 + §3.4. Files: `app/services/sts_credential_broker.py` (mirror of vectorAIz C2), `app/services/order_service.py:147` branch, `app/models/marketplace.py:37` enum, `app/schemas/listing.py:31` enum, `alembic/versions/<date>_alter_fulfillment_type_enum_seller_s3_sts.py`, `app/api/v1/endpoints/orders.py:619` refresh wire-up. Deps: C4 gatekeeper contract. Parallel: YES with C7. LoC: ~200. ACs: (a) `alembic upgrade head` adds enum value idempotently. (b) `order_service.create_order` for SELLER_S3_PRESIGNED_URL listing returns a `FulfillmentDownloadToken` with the presigned URL injected via existing `issue_fulfillment_token` path. (c) Refresh endpoint at `orders.py:619` re-signs via broker on expired URL. (d) STS broker is platform-side (uses ai-market platform IAM identity) — does NOT store seller secrets. (e) Buyer JWT signature validation preserves existing `app/core/security.py:200` patterns including `jti` replay prevention.

### C7 — allAI onboarding agent
Scope: per §2.3. Files: `app/allai/agents/aim_data_onboarding/agent.py`, `iam_policy_templates.py`, `sts_probe.py`, `diagnostics.py`, `__init__.py`. Deps: C1 (schema) + C4 (test-assume-role endpoint). Parallel: YES with C6. LoC: ~300 (largest). ACs: (a) Agent registered in `app/allai/agent_registry.py:25` `AgentRegistry` following existing pattern. (b) Agent manifest at `app/allai/agent_manifest.py:79` style. (c) `generate_trust_policy_json(seller_id, external_id, platform_arn)` produces valid JSON enforcing the four required conditions. (d) `probe_role_arn(role_arn, external_id)` invokes platform-side STS broker test path; returns structured success or failure. (e) `classify_assume_role_error(error_response)` distinguishes 5+ distinct failure modes. (f) Walkthrough flow is idempotent and checkpointable via pre-row S3Connection with status=onboarding.

### C8 — Frontends (vectoraiz-frontend + ai-market-frontend)
Scope: per §2.4 + vectorAIz seller side. Files: `vectoraiz-frontend/src/pages/S3ConnectionPage.tsx`, `vectoraiz-frontend/src/components/S3BucketBrowser.tsx`, `vectoraiz-frontend/src/components/S3OnboardingChat.tsx`, `ai-market-frontend/src/components/PresignedDownloadButton.tsx`. Deps: C4 + C6 + C7. Parallel: no. LoC: ~200. ACs: (a) Seller creates S3 connection via guided allAI chat. (b) Buyer download uses `<a download href={url}>` pattern (no CORS required). (c) Refresh-on-expiry handled via `orders.py:619` refresh endpoint. (d) Bucket browser preview surfaces first 20 objects via `POST /list_objects_preview` proxy.

### C9 — Integration tests
Scope: end-to-end with moto STS+S3 fixtures. Files: `vectoraiz/tests/integration/test_s3_sts_scan_end_to_end.py`, `vectoraiz/tests/integration/test_s3_sts_sign_url.py`, `ai-market-backend/tests/integration/test_s3_sts_purchase_flow.py`, `ai-market-backend/tests/integration/test_aim_data_onboarding_agent.py`. Deps: C5 + C6 + C7. Parallel: no. LoC: ~250. ACs: (a) Full scan-to-sign-to-buyer-fetch flow passes against moto. (b) Walkthrough agent unit-tested for all error classification paths. (c) STS session expiry simulated; refresh path verified. (d) ExternalId mismatch surfaces correct diagnostic.

## §5 Test plan

**Unit tests:**

- `tests/unit/test_sts_credential_broker.py`: mocked boto3 client. Verify cache key tuple `(connection_id, region)`, 2-min refresh-buffer logic, RoleSessionName format compliance with locked decision.
- `tests/unit/test_s3_connector.py`: mocked client, paginator behavior, continuation_token resume, error path on missing connection.
- `tests/unit/test_iam_policy_templates.py`: assert generated trust JSON has Principal=single ARN, Action=sts:AssumeRole only, Condition.StringEquals.sts:ExternalId required, no wildcards anywhere. Assert generated permissions JSON has s3:GetObject + s3:ListBucket scoped to specified bucket + bucket/*.
- `tests/unit/test_diagnostics.py`: 5+ distinct AssumeRole failure types map to 5+ distinct classified diagnostics with remediation snippets.
- `tests/unit/test_order_service_create_order_branch.py`: assert SELLER_S3_PRESIGNED_URL branch invokes broker exactly once per order; assert FulfillmentDownloadToken contains presigned URL.

**Integration tests (moto fixtures):**

- `tests/integration/test_s3_sts_scan_end_to_end.py`: spin up moto S3 + STS, configure fake role with trust policy, end-to-end scan produces N `S3ObjectMetadata` rows. Verify ephemeral download cleanup.
- `tests/integration/test_s3_sts_sign_url.py`: assert sign-url endpoint returns URL with `X-Amz-Signature` query param. Assert URL is fetchable from moto and returns expected bytes.
- `tests/integration/test_s3_sts_purchase_flow.py`: simulate Stripe checkout → order create → FulfillmentDownloadToken contains presigned URL → mock fetch via moto returns object bytes. Verify CloudTrail-style event log emitted.
- `tests/integration/test_aim_data_onboarding_agent.py`: walkthrough happy path, ExternalId mismatch path, MaxSessionDuration-too-low path, malformed-ARN path. Each path produces correct classified diagnostic.

**End-to-end scenarios:**

- Onboarding happy path: seller initiates chat → role-ARN paste → probe success → S3Connection persisted → bucket scan kicked off → 10 objects enumerated → listing published with SELLER_S3_PRESIGNED_URL fulfillment_type.
- Error remediation: seller pastes role-ARN with missing ExternalId → diagnostic surfaced with remediation snippet → seller corrects → probe success.
- URL refresh on expiry: buyer order delivered → wait 16 min (URL expired) → buyer hits refresh endpoint → fresh URL returned with new X-Amz-Signature.

## §6 Risk analysis

**R1 — STS session expiry mid-request (MEDIUM).** STS session might expire between AssumeRole and presigned URL signing. **Mitigation:** Cache with 2-min refresh buffer per §6 of `infra:worker-artifact-stash:S711.W:r3-t3-aws-sts-presigned-url-semantics-reference`. **Monitoring:** Log STS refresh frequency; alert on >5/sec/connection (indicates abuse or bug).

**R2 — ExternalId mismatch (HIGH-PROBABILITY, LOW-IMPACT).** Most common onboarding failure. AccessDenied response from AssumeRole does NOT specify ExternalId as the cause (AWS security behavior). **Mitigation:** Walkthrough always passes ExternalId; diagnostic offers ExternalId verification as the primary failure mode. Display seller's ExternalId prominently with copy-to-clipboard.

**R3 — Role-trust-policy drift (LOW-PROBABILITY, HIGH-IMPACT).** Seller edits their trust policy after onboarding to break the trust relationship. **Mitigation:** Periodic background `test-assume-role` job (daily). On failure, mark `S3Connection.status = degraded` and surface to seller dashboard. Suspend listings linked to degraded connections; resume on next successful probe.

**R4 — Presigned URL replay (LOW-IMPACT).** URLs are reusable within their 15-min validity. **Mitigation:** Short expiry (15 min); revocation enforced at gatekeeper layer via FulfillmentDownloadToken invalidation. JWT-based pattern at `app/core/security.py:200` preserves `jti` for replay prevention upstream of S3.

**R5 — allAI walkthrough abandoned mid-flow (MEDIUM-PROBABILITY).** Seller starts onboarding, walks away, returns later. **Mitigation:** Idempotent agent flow with explicit checkpoint state. ExternalId generated at flow start and persisted to a pre-row `S3Connection` (status=onboarding); resume reads pre-row.

**R6 — Large-bucket pagination resume (MEDIUM-PROBABILITY).** Scan of a 10M-object bucket may take hours; process restart should resume. **Mitigation:** `S3Connection.continuation_token` persisted on every page boundary. On scan resume, load token and continue.

**R7 — Confused-deputy attack (LOW-PROBABILITY, HIGH-IMPACT).** Seller A registers seller B's role ARN as their own. Without ExternalId, ai-market platform would assume seller B's role for seller A's listings. **Mitigation:** ExternalId scheme is per-seller unique (`aim-data-seller-{uuid}-{32hex}`); trust policy enforces `StringEquals` ExternalId; mismatched ExternalId fails AssumeRole.

**R8 — Cross-region latency for sign + download (LOW-IMPACT).** Bucket in eu-west-1, platform in us-east-1 — signing works but adds round-trip. **Mitigation:** Pre-cache `S3Connection.region` at onboarding; construct boto3 S3 client with correct `region_name` so URL host targets correct region endpoint.

## §7 Dependencies + sibling BQs

**Predecessor:** BQ-AIM-DATA-S3-SOURCE-CONNECTOR-S684 — PARKED. Surviving elements per `infra:worker-artifact-stash:S711.W:r3-t4-c-sts-surviving-element-extraction`: `S3Connection` model shape, boto3 client cache + lock infrastructure (M8 carried), paginated `list_objects_v2` logic, frontend UI shape, `processing_service.create_dataset` integration target. MOOT under C-STS: M9 partial-download cleanup, M10 `local_temp_path` persistence.

**Sibling (already merged):** BQ-AIM-DATA-NON-VECTOR-LISTING-METADATA-RELAX-S684 — Gate 2 APPROVED_AND_MERGED via S691/S692/S693 parallel track (verified Mars S703.W round-2 verbatim source-read on ai-market-backend `52d5f1d4` + vectoraiz `a0afb37` + alembic `20260522_001_relax_nullable_privacy_quality_s691.py`). The PREVIEW_READY status precondition relax enables listing metadata generation for S3-sourced non-vector datasets.

**Sub-BQ (recommend filing separately):** ai-market platform IAM identity setup. One-time ops work — create `arn:aws:iam::AI_MARKET_PLATFORM_ACCT:role/aim-data-platform-identity`, configure backend env to AssumeRole into this identity at startup. Should ship before C6 but has separate ops checklist.

**Cross-pillar dependency:** ai-market-frontend C8 buyer-flow chunk requires C6 backend contract finalized (FulfillmentType enum value + refresh endpoint shape).

## §8 Open questions resolved

Per `build:bq-aim-data-s3-sts-fulfillment-s711.body.decisions_locked_s711_12_50_utc` (Max-locked 2026-05-26 12:50 UTC, entity version 10):

- `session_duration_seconds = 3600`
- `presigned_url_expiry_seconds = 900`
- `external_id_format = "aim-data-seller-{uuid}-{32hex}"`
- `role_session_name_format = "aim-{seller_id}-{order_id_or_purpose}"`
- `required_role_max_session_duration_seconds = 3600`
- 9-chunk C-STS decomp APPROVED per `infra:worker-artifact-stash:S711.W:r3-t4-c-sts-surviving-element-extraction`
- allAI onboarding agent placement: PLATFORM SIDE (ai-market-backend allAI registry)

No new open questions surfaced in this spec. Implementation chunks proceed against these locked decisions.

## §9 References

**vectoraiz-monorepo origin/main HEAD `cd8bb5dc`:**

- `app/config.py:80` — `class Settings(BaseSettings)`
- `app/config.py:140` — `AliasChoices("AIM_DATA_KEYSTORE_PASSPHRASE", "VECTORAIZ_KEYSTORE_PASSPHRASE")` post-PR #5 canonicalization
- `app/services/db_connector.py:108` — `class DatabaseConnector` predecessor pattern
- `app/services/db_credential_service.py:45,50` — `encrypt_password` / `decrypt_password` (Fernet pattern; REPLACED in C-STS)
- `app/models/database_connection.py:18` — `class DatabaseConnection` predecessor schema
- `app/routers/database.py:133-223` — CRUD route handler shape
- `app/services/processing_service.py:294` — `def create_dataset(self, original_filename: str, file_type: str) -> DatasetRecord` integration target

**ai-market-backend origin/main HEAD `ef2e6d07`:**

- `app/models/marketplace.py:37` — `class FulfillmentType(str, enum.Enum)`
- `app/models/marketplace.py:119-126` — `fulfillment_type` Column with `name="fulfillment_type_enum", create_type=False`
- `app/schemas/listing.py:31` — schema-side FulfillmentType
- `app/core/security.py:200-258` — `issue_fulfillment_token` JWT-based gatekeeper pattern
- `app/models/fulfillment_download_token.py:39` — `class FulfillmentDownloadToken`
- `app/api/v1/endpoints/orders.py:200` — token issue
- `app/api/v1/endpoints/orders.py:619` — `@router.post("/{order_id}/refresh")` `refresh_access`
- `app/services/order_service.py:147` — `async def create_order`
- `app/services/llm_providers/bedrock.py:40-75` — STS AssumeRole + downstream S3 client construction pattern (mirror target)

**Mars stash references:**

- `infra:worker-artifact-stash:S711.W:r3-t1-gha-tag-trigger-diagnostic`
- `infra:worker-artifact-stash:S711.W:r3-t2-ai-market-backend-purchase-flow-predecessor-read` (v=2 with STS supplement)
- `infra:worker-artifact-stash:S711.W:r3-t3-aws-presigned-url-semantics-reference`
- `infra:worker-artifact-stash:S711.W:r3-t3-aws-sts-presigned-url-semantics-reference`
- `infra:worker-artifact-stash:S711.W:r3-t4-c-sts-surviving-element-extraction`
- `infra:worker-artifact-stash:S711.W:r3-t5-allai-onboarding-flow-predecessor-read`

**AWS documentation:**

- STS AssumeRole API: https://docs.aws.amazon.com/STS/latest/APIReference/API_AssumeRole.html
- External ID best practices: https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_create_for-user_externalid.html
- Confused Deputy Problem: https://docs.aws.amazon.com/IAM/latest/UserGuide/confused-deputy.html
- S3 generate_presigned_url: https://boto3.amazonaws.com/v1/documentation/api/latest/reference/services/s3/client/generate_presigned_url.html
- CloudTrail S3 Data Events: https://docs.aws.amazon.com/AmazonS3/latest/userguide/enable-cloudtrail-events.html
