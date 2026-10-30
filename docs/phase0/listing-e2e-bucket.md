# Bucket listing end to end (Phase 0 chunk E, throwaway)

Spec: runbooks `specs/BQ-VECTORAIZ-NATIVE-DESKTOP-PHASE0-GATE2.md` §4.5 (runbooks
`origin/main` `7c2c328e`). Run on Koskadeux by Minerva (S1837) on 2026-10-30,
12:16-12:46 CET (11:16-11:46 UTC). Times below are CET (UTC+1) with UTC in
brackets where an API or log reported UTC.

**Result: steps 1-6 passed.** A synthetic seller listed a vector pack from a
dedicated R2 bucket through Seller Workspace in a scripted browser, a synthetic
buyer bought it with a mock payment and downloaded it directly from R2, and the
downloaded pack matched its manifest and answered the example query. One
cleanup item is owed: deleting the consumed bucket needs Max's approval
(secrets-broker.md R2 step 6).

Production only, synthetic `is_test` accounts only (`seller-01`, `buyer-03`),
no money moved. No secret value was printed, logged or put on a command line;
the R2 keys, passwords and TOTP seed were read from Infisical Cloud into process
memory and passed to child processes through the environment or stdin.

## Authority

| What | Reference |
|---|---|
| R2 fixture bucket and tokens | Max's approval of broker plan `1b281ce3`, Event Ledger `fba08548` ("Yes, approve 1b281ce3 (Recommended)"); broker attempt `s1837-vz-e-1`, `mint-complete` before the run |
| E2E purchase window (arm, buy, disarm) | Max's standing GO, Event Ledger `68376220` (covers E2E test-purchase windows for chunk E with synthetic accounts and mock payment; START/END on the peer bus). Read before arming. |
| Window announcement | Peer bus START #24357 (12:29 CET), END #24367 (12:40 CET) |

Runbooks followed: `runbooks/connector-seller-chunk5-release-proof-s1796.md`
(fixture pattern), `aim-data-gateway.md` "Production self-test" steps 4-5 (synthetic
accounts, `arm.py` E2E window, checkout request shape), `e2e-browser-runner.md`
(seller-01 TOTP sign-in and the scripted seller-session pattern),
`secrets-broker.md` "R2 fixture bucket" steps 4-6 (status, retire, proof),
`seller-workspace-cloud-listing-delivery.md` and
`runbooks/seller-workspace-operator-guide.md` (capabilities and guided listing
steps).

## Production preconditions (design 7.2), read-only

| Precondition | Value | Evidence source |
|---|---|---|
| `sources_enabled` | on | Railway `ai-market-backend` production variable `SELLER_WORKSPACE_SOURCES_ENABLED=true` (GraphQL `variables`, read 11:56 CET); seller-01's `GET /api/v1/seller-workspace/capabilities` at 11:59:18 CET (10:59:18Z): `sources: enabled, available` |
| `drafts_enabled` | on | `SELLER_WORKSPACE_DRAFTS_ENABLED=true` (same read); capabilities `drafts: enabled, available` |
| R2 connections enabled | on | `SELLER_WORKSPACE_R2_CONNECT_ENABLED`, `_R2_PUBLISH_ENABLED`, `_R2_DELIVERY_ENABLED` all `true`, master `SELLER_WORKSPACE_ENABLED=true`; capabilities `providers.r2.connect`, `.discovery`, `.publish`, `.delivery` all `enabled, available`; `providers.r2.profile` `unavailable, not_implemented` (not needed) |

Defaults for all of these are false in backend `app/core/seller_workspace_config.py`
(backend main `f5ada1ee`), so the variables are what turn them on.

## Step 1: build, index, pack — PASS

- Source: vectoraiz `native` `a33eefe6f18f7f852286c31231d432884d9dd6b2`.
- `cargo build --locked --release -p vz-cli -p vz-parser-worker` finished
  12:16 CET (27 min 42 s). SHA-256: `vectoraiz-cli`
  `1df549e0…72d6`, `vz-parser-worker` `f6399be6…cccf3`.
- `vectoraiz-cli --cpu --state <fresh dir> index spike/fixtures/documents`:
  `completed 20, failed 0, total 20`, exit 0.
- `vectoraiz-cli --cpu --state <same> pack <dir> --query "What is the orbital
  archive retention policy?"`: exit 0. Pack id `pack-23e55b6b007fb426c0f837e6`,
  20 documents, 20 chunks, `originals: true`, `format_version 1`, recipe
  BGE-small 384-d, CLS pooling, L2, llama.cpp `b8635075`, model SHA-256
  `f0b2fef9…e999`.

Manifest (`files`; `manifest.json` excludes itself):

| File | Bytes | SHA-256 |
|---|---:|---|
| `README.md` | 504 | `3765b2eee94449307ec76f71af9f2ed415b1ef9227707cc7ad2b30b7893dbd46` |
| `chunks.parquet` | 52,280 | `143abfe5d34f3a143e669735d0ab7a050d80cc2df20724b2ed4c902389471473` |
| `documents.parquet` | 6,445 | `02c3106066b1379b8d4fe49eabd774d76d81224cc543565506581c7938bcc4b4` |
| `examples/query.json` | 10,505 | `3869b17cee540155e7e36302964ce82ff2746c0ead661d0a7dd938e67243a5ef` |
| `examples/query.py` | 2,377 | `8d0a36c455e08aeedd14e6d66168b9619269c940846e748d237575701f7e9051` |
| `originals-0001.zip` | 10,311 | `e2ff53d6e7e6d30672456a78d53a551eb33a9c406a94cc5e167a6ba33be275d9` |
| `manifest.json` | 1,565 | `b60ce8eec66d9ef6769288017525320731d72d40f215c2bfc94827c69582e822` |

## Step 2: upload — PASS

`vectoraiz-cli upload <pack> --endpoint
https://d5346d3e0f8f344c5f4915aaca689adf.r2.cloudflarestorage.com --bucket
aimarket-fixture-vz-s1837 --region auto --prefix
vz/pack-23e55b6b007fb426c0f837e6`, with the read/write slot in
`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` of the child only. 12:19:30-12:19:32
CET, exit 0.

Check with the read-only key: listing the bucket showed exactly the seven pack
objects under `vz/pack-23e55b6b007fb426c0f837e6/`, each with the size and
SHA-256 above (objects downloaded and hashed). A `PutObject` with the read-only
key was refused `AccessDenied`.

## Step 3: seller connects and publishes — PASS

Headless Chromium (Playwright from the e2e-harness venv) on `https://ai.market`
as `seller-01`, signed in with password and TOTP (login 200, `/auth/2fa/verify`
200). Every step went through the website's Seller Workspace UI.

| CET | Action | Result |
|---|---|---|
| 12:20:07 | Connect your storage → Add Cloudflare R2 connection: account `d5346d3e…`, location default, bucket `aimarket-fixture-vz-s1837`, folder `vz`, read-only keys, "Object Read only" box | `POST /seller-workspace/connections/r2` **201**, connection `0245d291-52c1-4ccb-81a8-fb321a9844ad`, `status verified`, `prefix vz` |
| 12:21:21 | Choose your files: the seven files under `vz/pack-23e55b6b…/` ticked one by one, Save, Confirm selection (7 files, 82 KB) | `PUT /seller-workspace/listing-source` 200 |
| 12:21:46 | Choose a licence: Standard, AI/ML training allowed, signer "Synthetic Seller" / "Test", covenant box | `PUT /seller-workspace/listing-draft` 200 |
| 12:22:23 | Describe and price, typed by hand (no Allai): title "Sample document vector pack (test fixture)", neutral description, category AI & Machine Learning, tags "vector pack, embeddings, test fixture", price **$25** (the priced pattern of the gateway self-test); "This description matches my files"; Save private draft | `PUT /seller-workspace/listing-draft` 200 |
| 12:22:48 | Review: 7 files, 83,987 bytes, licence hash `4b05dbcd…63c6`; "I confirm all of the above"; Approve | `POST /seller-workspace/listing-approval` 200 |
| 12:23:11 | Publish → Prepare for publishing | `POST /seller-workspace/listing-preparation` 200, preparation `903a0771`, `prepared` |
| 12:24:09 | Sign 1 licences | **403 `RECENT_LOGIN_REQUIRED`** ("Sign in again within the last 15 minutes"); sign-in had been 25 min earlier |
| 12:26:19 | Log out, sign in again with TOTP, Sign 1 licences | `POST /seller-workspace/listing-licences/sign` 200, `signed`, coverage `6c7c3993…0bca` |
| 12:26:34 | Publish this listing | `POST /seller-workspace/listing-publication` 200: listing `0db1ff53-21eb-4a44-82db-2aafb761b1c3`, version `3110500d`, publication `16a6149c`, `published`, `is_listed true` |

Seller Workspace accepted the synthetic seller's R2 connection; §4.5's open
question is answered yes.

## Step 4: purchase — PASS

Before arming: no objection or other backend window on the bus; Railway showed
no paused builds (status page: only new sign-ups paused after the 2026-10-21
incident; backend deploys were succeeding); backend `E2E_TEST_ROUTES_ENABLED=false`;
deployed backend main `f5ada1ee56dbad26c45c693ce1e4f4a768b5b3ea` (deployment
`91f0c366`).

| CET | Step | Evidence |
|---|---|---|
| 12:29 | START on the bus | #24357 |
| 12:34:08 | `arm.py arm f5ada1ee…` (`/Users/max/worktrees/gwtest-s1837/arm.py`) | `E2E_TEST_ROUTES_ENABLED=true`, `E2E_ARMED_AT=2026-10-30T11:34:08Z`; deployment `d0f23cf2` SUCCESS 12:37:06 |
| 12:37:12 | buyer-03 `POST /api/v1/checkout/create` (Origin `https://ai.market`, CSRF, `checkout_request_id`, licence `4b05dbcd…63c6`, covenant `a91234b6…31af`, `authority_confirmed`) | **200**, order `258c4c55-515e-4a72-945d-ba92c4e8a27f`, `checkout_url null` (mock payment) |
| 12:37:30 | `arm.py disarm f5ada1ee…` | variable `false`, `E2E_ARMED_AT` removed; deployment `6d1cd345` (commit `f5ada1ee`) SUCCESS 12:40:30 |
| 12:40:43 | Disarm verified | Railway readback `E2E_TEST_ROUTES_ENABLED=false`, `E2E_ARMED_AT` absent; `GET /api/v1/e2e/teardown/status/<uuid>` and `POST /api/v1/e2e/reset/account` both **404** (the router mounts `/e2e` only when the flag is true); `/health` healthy; `d0f23cf2` REMOVED |
| 12:40:59 | END on the bus | #24367 |

The armed window lasted 6 min 22 s (arm to disarm request), 9 min 35 s to the
disarmed deployment.

Buyer download, after the disarm (12:41-12:42 CET):

- `POST /seller-workspace/orders/258c4c55…/prepare` 200 `workspace_direct`, ready.
- `POST /seller-workspace/orders/258c4c55…/download` 200, session `b7521f6b`,
  7 files, `download_number 1`, `downloads_remaining 2`.
- One grant per file (`POST …/download/<session>/files/<i>` 200), each a
  presigned URL valid about 5 minutes; files fetched straight from R2. URLs were
  never printed or stored.

## Step 5: verify — PASS

- **Hashes.** All seven downloaded files have the SHA-256 of the table in step 1;
  the six manifest payload files match the manifest and `manifest.json` matches
  the uploaded one (`b60ce8ee…e822`).
- **No identity terms in download URLs.** Every URL host was
  `d5346d3e0f8f344c5f4915aaca689adf.r2.cloudflarestorage.com` (ai.market's R2
  account id, by design 6.6). Checked, after URL-decoding, against the seller's
  identity terms: `seller-01`, `e2e-test`, `ai.market E2E Fixture Seller 01`
  (the seller's legal name on the licence), `Fixture Seller`, `E2E Fixture`,
  `synthetic seller`: **0 hits** in 7 URLs.
- **Query.** The pack rebuilt from the download, `examples/query.py <pack>` exit
  0: `hashes OK`, `citations OK`, top 3 equal `expected_top`. Again with the
  release CLI recomputing the query embedding from the recorded recipe
  (`query.py <pack> vectoraiz-cli`): exit 0, same top 3. Top result, score
  0.836: *"Orbital archive retention. The orbital archive retains signed
  telemetry for **thirty days**…"* (chunk `a2f88eed…-0`, page 1).

**Finding: Workspace delivery drops folders.** The download response names
files by basename only: `examples/query.json` and `examples/query.py` arrive as
`query.json` and `query.py`. A buyer has to rebuild the pack layout from
`manifest.json` by SHA-256 (as this run did) before `examples/query.py` works.
For Phase 3, either the pack keeps every file at the top level, or the README
tells buyers how to restore `examples/`, or Workspace delivery keeps relative
paths. Two files with the same basename in different folders would also be
ambiguous to a buyer.

## Step 6: cleanup

| Item | State | Evidence |
|---|---|---|
| Listing | **Unpublished** | Seller Workspace "Pause new sales", 12:42:57 CET: `POST /seller-workspace/listing-publications/0db1ff53…/visibility` 200, `status unlisted`, `is_listed false`. Afterwards `GET /api/v1/public/listings/<slug>` still answers 200 but with `purchasable false`, `purchase_hold_reason workspace_sales_paused`, `noindex true`; buyer `GET /api/v1/listings/0db1ff53…` 404 |
| Connection and order | Left in place, as §4.5 says | R2 connection `0245d291` (still `verified` in the product; its key is revoked, see below); order `258c4c55` (`delivered`) |
| Sessions | Closed | Browser seller logout; buyer `POST /auth/logout` 200; token cache deleted |
| Broker attempt | `cleanup-pending` | `r2 retire --attempt s1837-vz-e-1` (12:43:44 CET, koskadeux-mcp `57cf999b`, no broker-code change since the mint) printed the generic refusal; `r2 status` then `cleanup-pending`, as expected for a consumed bucket |
| Tokens | **Revoked** | Cloudflare `GET /user/tokens` (12:45:28 CET, HTTP 200, 11 tokens, names and status only): no token named `vz-fixture-s1837-rw`, `vz-fixture-s1837-ro`, `s1802-s1837-vz-e-1-operation-admin` or starting `s1802-s1837-vz-e-1-cleanup-admin-` |
| Vault slots | **Gone** | Infisical Cloud `ai-market` `prod` `/vz-fixture-s1837` (HTTP 200, names only): empty; none of the four `FIXTURE_R2_*` keys present |
| Bucket and 7 objects | **Owed** | Bucket `aimarket-fixture-vz-s1837` was consumed by connection `0245d291`, so the broker's quiesce keeps it. Deleting it needs Max's separate approval and the secrets-broker.md step 6 procedure. Step 6 also requires the connection to be `revoked` first: disconnect connection `0245d291` as seller-01 before that run |
| E2E window | Closed and verified | Step 4 |

## Other observations

- Synthetic-account credentials are now in **Infisical Cloud** project
  `ai-market` (`4a79065b…`), environment `staging`, root
  (`E2E_SYNTHETIC_SELLER_01_EMAIL/_PASSWORD/_TOTP`, buyers `01`-`05`, sellers
  `01`-`05`; only seller-01 has a TOTP seed). The old self-hosted vault
  (`secrets.ai.market`, project `bd272d48`) answered 403 to the sysadmin token,
  so `aim-data-gateway.md`'s location and the S1752 `api.py` helper are stale.
- Licence signing needs a sign-in within the last 15 minutes
  (`RECENT_LOGIN_REQUIRED`); a scripted run should sign in again just before
  signing.
- Anonymous and buyer reads of the published listing showed `purchasable false`
  with `SELLER_PAYOUT_NOT_READY` (the synthetic seller has no Stripe Connect);
  the armed mock checkout still succeeded, as backend #949 intends.
- A Workspace order is `delivered` at checkout (direct delivery), unlike gateway
  orders, which stay `pending_delivery` until receipts arrive.
- The read-only listing of the bucket showed only the seven pack objects, not the
  broker's `broker/s1837-vz-e-1/synthetic-1.txt` that secrets-broker.md says a
  complete mint leaves. Not investigated (UNVERIFIED whether it exists).

## Not covered

- Chunk E used the existing Seller Workspace flow, not the vectorAIz deep-link
  panel (design 7.2 steps 1-5), which is Phase 3 frontend work.
- No probe of the public listing page in a browser after publishing.
- The disarm route probe was taken only after disarming; its 404 is evidence by
  the router code, not by an armed-state comparison.

Private evidence (not committed; synthetic account email appears in
screenshots): `/Users/max/koskadeux-state/s1837/chunk-e/` on Koskadeux
(`evidence/`, `shots/`, helper scripts in `tools/`).
