C1 bounded feasibility harness
==============================

Authority: runbooks `c8699ee5`, `specs/BQ-VECTORAIZ-WEB-SERVER-GATE2.md`
(Pins, C0, C1, A1–A11), Gate 1 and requirements. Accepted native base:
`2af9e945fce07be6935ef3e4445a054d20b0b26c`. This is a spikes-only draft.
No product listener, office enable switch, tenant configuration or cloud path.
The app includes `office/spikes.rs` only under `cfg(all(test,
feature="office-feasibility"))`; the feature is empty/default-off in core.

Run from the checkout with `rtk proxy python3 scripts/office-feasibility.py
--help`, `--self-test`, or `--by-hand`. For actual probes, use a NEW private
directory outside the repo, for example:

```sh
rtk proxy python3 -m venv "$KD_BRIDGE_JOB_EVIDENCE/venv"
rtk proxy "$KD_BRIDGE_JOB_EVIDENCE/venv/bin/python" -m pip install h2==4.3.0 jsonschema==4.25.1
rtk proxy "$KD_BRIDGE_JOB_EVIDENCE/venv/bin/python" scripts/office-feasibility.py \
  --out "$KD_BRIDGE_JOB_EVIDENCE/run-1" --rust \
  --model-assets /path/to/public-models --model-pins spike/fixtures/models.json
```

Models are optional. The runner verifies and copies only pinned model files to
private evidence before native verification writes its sidecar. It never reads
settings/credentials from the source directory. Without models, storage uses
explicitly labelled deterministic vectors and native abort is blocked. Pin
changes require affected reruns. All subprocess argv pass through `rtk proxy`.
The runner writes a versioned manifest and complete command logs, returns 1 on
failures/blocked/unrun probes, and never changes the contracts to get exit 0.
`jsonschema` validates the versioned schema; the stdlib validator also rejects
missing required fields, batch overflow and accidental acceptance of unresolved
bounds. Raw manifests/results, keys, corpus and model copies stay outside git.

What is measured
----------------

* A deterministic generator produces 2,048 synthetic TXT documents by default;
  hashes, byte sizes, pages and exact query oracles are private. `--public-docs`
  inventories up to 1,000 explicitly public local mixed-format documents,
  read-only. It does not silently ingest a tenant folder. Public inventory is
  not counted as indexed/renderer-covered documents.
* The core test builds real SQLite/Lance generations and measures 25 concurrent
  arrivals twice, including the shared-guard queue, real opens and correctness
  against document IDs/source markers. With explicit pinned assets it uses
  native embedding; otherwise vectors are synthetic. Default storage size is
  32 documents, separately recorded from generated corpus size; increase
  `--storage-documents` to measure the costly per-document Lance opens. The
  first pass has OS caches warmed by indexing: it is NOT a cold-disk trial.
* A native-only abort probe checks cancellation between <=32-token prefill
  steps and individual decode steps, joins the blocking thread and measures
  guard release from the signal. Native calls themselves are not interruptible;
  >1s is a design failure. This does not change desktop Ask.
* Cache tests exercise actual Operations with a synthetic engine. The proposed
  full-run lease is test-local, and admission traces name today's read-only MCP
  tools, with desktop/MCP before office reads. The current cancel-generation
  hole is recorded as a contract failure, even though the regression test
  detecting the hole passes. No fictional mutating MCP path exists.
* Separate storage roots keep old-generation searches possible with a new
  writer locked, including identical vector recipes. A test-local pointer is
  renamed atomically. This does not claim `active_index::reindex` supports
  generation Refresh or that product desktop/MCP paths use the proposed lease.
* A minimal compiled C hostile worker runs without inherited host credentials.
  macOS Seatbelt ingest/render test processes must start, read their permitted
  snapshot, then fail BOTH sentinel reads and loopback socket connection with
  EPERM/EACCES. Startup failure is a failure, never successful confinement.
  This is a capability prototype, not actual parser/viewer coverage. Baseline
  probes run the existing network-only worker profile too. Socket creation and
  connection are recorded separately; a denied connection cannot prove that
  socket creation itself failed. Existing desktop worker behaviour is unchanged.
* PEM: RSA2048 PKCS8 converted to traditional RSA for Keychain import,
  ephemeral CA, SAN/serverAuth, explicit trust, isolated
  disposable Keychain import/export/reload and cleanup. No default/search-list
  Keychain or system trust modification. Export uses a disposable wrapping
  secret and reload checks the matching certificate/key. Real company CA/browser trust and
  ECDSA/encrypted-key compatibility remain unqualified.
* TLS 1.2+ h2/h1 loopback processes test completed pre-auth exhaustion against
  25 simultaneous synthetic-token requests, bounded connections and pre-auth
  slots. Token headers are test authentication, not OIDC. Provisional test
  caps are NOT final: a separate stalled-handshake/global-cap attack must retain
  authenticated admission too. Its failure is reported even when completed
  pre-auth requests passed. Shared NAT/VPN/VDI and identity/session abuse remain
  unrun. Only 127.0.0.1 is bound.

Unresolved exit evidence
------------------------

`T_search_refusal_max` is longest continuous office-search refusal, not query
latency or the <=5s busy deadline. Desktop indexing, office Refresh, desktop Ask
and combined have separate 1ms-sampled guard traces for the synthetic native
workload. Product scheduling and the agreed workload remain unqualified. The manifest never invents
an accepted bound; null means unmeasured/unaccepted, not zero. Normal A1 load
excludes desktop Ask, and busy never counts as a correct A1 result. C1 cannot
exit until the agreed mixed-format workload, answers/indexing/Refresh and both
hosts are qualified. A timing/content/confinement failure returns to design.

By hand — UNRUN, do not execute without separately provisioned host/IT tenant
--------------------------------------------------------------------------

1. On the confirmed Windows x64 PC, pin OS/CPU/GPU/RAM/power/model/recipe and
   rerun search/abort/lease/import/h1/h2. Prove actual AppContainer or equivalent
   ingest AND render confinement, with positive snapshot and negative host
   sentinel/socket probes. Job Object memory limits alone are insufficient.
2. IT supplies actual Entra tenant/Workspace customer, immutable user binding,
   API/minimum read permissions and consent. Pin action/check-start/check-end/
   revocation/delivery timestamps for disable/delete/suspend/archive/move,
   re-enable, 403/429, delayed propagation and outages. Enforce <=15m from IT's
   action; OIDC/token expiry alone is not proof. Re-enable requires new sign-in.
3. Behind shared NAT, VPN and VDI, run 25 identities plus abusive pre-auth and
   session/tab floods. Exercise fourth-session own-LRU eviction, authenticated
   reserve, stalled TLS/header/body and per-person fairness. Accept final caps
   only after these trials, not from this loopback prototype.
4. Roam/lock/lid/battery/Modern Standby/sleep/wake; deny macOS network-identity
   permission. Uncertainty stops delivery/listening. Use sleep-inclusive clocks;
   recover <=2m with original deadlines, no new network fallback, Off stays Off.
5. Per-format PDF/DOCX/XLSX/TXT/Markdown/HTML manifest and expected complete
   content: every page/sheet/table/image/text, accessible equivalents, resource
   bounds and hostile inputs. Exercise Chrome/Edge/Safari/Firefox, VoiceOver/
   NVDA, keyboard and zoom. Missing content returns to design.
6. IT hostname/DNS/firewall/CA/client trust, renewal/expiry/replacement and port
   collision. No warning bypass. Installed A1–A11, privacy/audit retention and
   desktop key/consent regressions belong to later C2–C6 qualification.

C0 remains unresolved; live CORE v9.30 is unverified. A1–A11 are not waived.
No enabled distributable office feature follows from any local spike success.
