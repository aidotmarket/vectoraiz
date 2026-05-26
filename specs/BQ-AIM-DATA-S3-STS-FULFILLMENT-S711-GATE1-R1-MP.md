# MP R1 Review — BQ-AIM-DATA-S3-STS-FULFILLMENT-S711 Gate 1

Reviewed branch SHA: `0beb7c42e96a` (`spec/bq-aim-data-s3-sts-fulfillment-s711-gate1`)
Base checked: `cd8bb5dc`
ai-market-backend predecessor checked: `ef2e6d07`

Verdict: **CHANGES_REQUESTED**

## Findings

### HIGH — C6 cannot be implemented from the specified contracts because the marketplace-side S3 fulfillment metadata is undefined

The spec adds only `SELLER_S3_PRESIGNED_URL` to ai-market-backend schema (§3.3) but C6 requires ai-market-backend to resolve "seller's S3Connection record from listing snapshot" and generate a URL for the "buyer-purchased S3ObjectMetadata" (§2.2, §3.4). There is no contract for how `role_arn`, `external_id`, `region`, `bucket`, `object_key`, `connection_id`, or a selected `s3_object_metadata_id` get from vectorAIz into ai-market listing/order state. The vectorAIz tables are local to AIM-Channel, while ai-market-backend has no S3 connection table or listing metadata delta beyond the enum.

This also makes the chunk dependencies incoherent: C6 declares a dependency on C4's vectorAIz gatekeeper contract, but §2.2/§3.4 say C6 directly assumes the seller role and signs the S3 URL platform-side. Those are different ownership models. Gate 2 needs one explicit contract: either platform signs from platform-stored role/bucket/object metadata, or vectorAIz signs through `POST /sign-url`, with exact request/response auth and listing snapshot fields.

### HIGH — The fulfillment-token predecessor contract is misstated; the named function/path does not exist

The spec repeatedly says the S3 URL is wired through `app/core/security.py:200` `issue_fulfillment_token` and a `FulfillmentDownloadToken` issuance path (§2.2, §3.4, C6). At `ef2e6d07`, `app/core/security.py` has `create_delivery_token` at line 188, not `issue_fulfillment_token`; line 200 is inside that function's docstring. The order endpoint issues tokens through `order_service.issue_download_token` (`orders.py:214`) which calls `create_delivery_token` at `order_service.py:599`. The `FulfillmentDownloadToken` model at `app/models/fulfillment_download_token.py:39` is a separate DB token model for proxied transfer finalization, not the JWT delivery-token path described in C6.

Gate 2 implementers would branch against a nonexistent API and conflate two token systems. The spec must rename the predecessor contract accurately and define whether the new S3 URL is returned in the existing JWT delivery-token response, stored in `delivery_config`, represented by `FulfillmentDownloadToken`, or handled by a new S3-specific access-token type.

### MEDIUM — Presigned S3 URL revocation/replay mitigation is incorrect once the buyer has the direct URL

R4 says revocation is enforced "at gatekeeper layer via FulfillmentDownloadToken invalidation" and that `jti` preserves replay prevention upstream of S3. That is true only before the direct S3 URL is minted. Once the buyer has a presigned S3 URL, S3 will honor it until its own expiry unless credentials expire first or the underlying object/policy changes. ai-market JWT invalidation, order revocation, and download counters cannot revoke an already-issued S3 URL for the remaining 15-minute window.

This is probably acceptable as a bounded exposure if explicitly accepted, but it must be stated that revocation is best-effort until URL expiry. If stronger revocation is required, the design needs a stateful gatekeeper/proxy or much shorter URL TTLs.

### MEDIUM — Backend STS cache key conflicts with per-order CloudTrail/session-name claims

§3.4 defines the ai-market-backend broker cache key as `(role_arn, region)`, while §8 locks `role_session_name_format = "aim-{seller_id}-{order_id_or_purpose}"` and §2.2 claims S3 GetObject CloudTrail will show `aim-{seller_id}-{order_id}`. With a cached S3 client shared by role+region, the RoleSessionName is fixed at the AssumeRole call that populated the cache; later orders signed with the same cached client cannot get per-order session names.

The design must choose between per-order CloudTrail readability and credential reuse. If per-order auditability is required, include session purpose/order id in the cache key or avoid caching order-signing clients. If reuse is desired, weaken the CloudTrail claim and define how order-to-URL audit correlation is logged in ai-market instead.

### LOW — Test plan misses named edge cases from the risk section

The test plan covers happy path, ExternalId mismatch, MaxSessionDuration, malformed ARN, and a generic expiry simulation, but it does not explicitly cover allAI walkthrough abandonment/resume from the onboarding pre-row, nor an integration-level large-bucket pagination restart/resume across multiple pages. R6 only has unit-level continuation-token coverage, and R5 has no abandonment/resume test. Add explicit tests for abandoned onboarding checkpoint recovery and process-restart scan resume after at least two S3 pages.

## Citation Audit

Sampled more than 50% of predecessor citations across both repos.

Verified as accurate or materially accurate:

- vectoraiz `cd8bb5dc`: `app/models/database_connection.py:18`, `app/services/db_connector.py:108`, `app/routers/database.py:133-223`, `app/services/processing_service.py:294`, `app/config.py:80`, `app/config.py:140`, `app/services/db_credential_service.py:45,50`.
- ai-market-backend `ef2e6d07`: `app/models/marketplace.py:37`, `app/models/marketplace.py:119-126`, `app/schemas/listing.py:31`, `app/api/v1/endpoints/orders.py:619`, `app/services/order_service.py:147`, `app/services/llm_providers/bedrock.py:40-75`, `app/models/fulfillment_download_token.py:39`.
- allAI predecessor lines in ai-market-backend `ef2e6d07`: `app/allai/base_agent.py:90`, `app/allai/agents/matchmaker.py:55`, `app/allai/agents/marketing_ops.py:65`, `app/allai/agents/crm_steward.py:157`, `app/allai/agent_registry.py:25`, `app/allai/agent_manifest.py:79`.

Citation drift / unsubstantiated references:

- `app/core/security.py:200-258` is a real JWT delivery-token function body, but the function is `create_delivery_token`, not `issue_fulfillment_token`.
- `app/api/v1/endpoints/orders.py:200` is documentation text inside `request_download_token`; the actual token call is at `orders.py:214`.

## Open Questions

The Living State locked decisions at `build:bq-aim-data-s3-sts-fulfillment-s711.body.decisions_locked_s711_12_50_utc` cover the big architecture defaults: 3600s STS sessions, 900s presigned URLs, ExternalId format, RoleSessionName format, required MaxSessionDuration, 9 chunks, and platform-side allAI placement. They do not resolve the marketplace listing/order metadata contract, platform-vs-vectorAIz signing ownership, or the per-order CloudTrail versus STS cache tradeoff. Those should remain open until the Gate 1 spec is revised.

## Schema / Chunk Notes

The proposed vectorAIz tables are additive and avoid long-lived AWS secrets. The ai-market enum extension is additive, but the migration must be isolated and account for the deployed PostgreSQL version's `ALTER TYPE ... ADD VALUE IF NOT EXISTS` behavior.

Chunk decomposition is mostly plausible after C1-C3. C4 and C5 can ship in parallel after C3. C6 and C7 are not truly parallel-shippable until the C6 metadata/signing contract is fixed because C7's `sts_probe` and onboarding persistence depend on knowing which side owns role metadata and probe execution.
