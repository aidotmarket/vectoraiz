C1 bounded feasibility harness
==============================

Authority: runbooks `origin/main` (pin its SHA in each private evidence manifest), `specs/BQ-VECTORAIZ-WEB-SERVER-GATE2.md`
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
  native embedding; otherwise vectors are synthetic. Baseline instrumentation
  separates inference-guard wait, arrival/scheduling delay, native context
  creation/encode, State open (nonblocking exclusive lock, SQLite and recovery),
  document JSON reads, per-document Lance opens/queries and ranking. The C1
  harness uses one Tokio runtime; an isolated microprobe measures desktop/MCP's
  per-request runtime construction/destruction separately.
  `storage::office::Snapshot` is compiled only with `office-feasibility`. It
  prepares one consolidated Lance/FTS table and chunk lookup per published
  office generation, retains one table handle, and releases the inference guard
  before storage queries. It uses the same queries, top-k and correctness oracle;
  global FTS corpus statistics can change ordering, so exact ranking equivalence
  is not claimed. Build cost is reported separately: preparation must finish
  before a generation accepts searches. Readers retain an Arc across promotion;
  real generation retirement/recipe binding and product call sites remain work.
  `VZ_OFFICE_SKIP_BASELINE=1` explicitly selects an after-only measurement; keep
  a separately captured baseline when using it. Default storage size is
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
  detecting the hole passes. A separate protocol test signals cancellation
  immediately and defers generation invalidation until the full-run lease
  drains; native pinned-engine batches must retain their cache and then reload
  after the run. This does not integrate the desktop mutation call sites. No fictional mutating MCP path exists.
* Separate storage roots keep old-generation searches possible with a new
  writer locked, including identical vector recipes. A test-local pointer is
  renamed atomically. This does not claim `active_index::reindex` supports
  generation Refresh or that product desktop/MCP paths use the proposed lease.
* A minimal compiled C hostile worker runs without inherited host credentials.
  macOS Seatbelt ingest/render test processes must start, read their permitted
  snapshot, then fail BOTH sentinel reads and socket creation with EPERM/EACCES.
  A denied loopback connect is also recorded but cannot substitute for denied
  socket creation. The profile permits the literal root directory needed by
  dyld startup, without recursively granting host files; paths are canonical.
  An unconfined positive control must read the sentinel and connect. Startup failure is a failure, never successful confinement.
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
  unrun. The second test-only transport variant retires the oldest connection
  that has not authenticated when the unchanged global cap is full. Completed
  test phases drain before the stalled-TLS attack; a barrier releases 25 new
  h1/h2 clients per protocol against that attack. Retiring unknown connections
  proves only finite-stall progress: sustained churn can still evict a legitimate
  handshake before authentication, so this is not an accepted reserve/fairness
  design. Only 127.0.0.1 is bound.

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

S1927 approved delta rerun (manifest v2)
--------------------------------------

Authority: runbooks `c45acd1a37d00f5ceda88aaeaf9837f265960dd4`, complete
"C1 design delta (S1927)" and C1. Revalidated integration base:
`af145f7280bf162e25809f380bcc95f8b638e105`, merged into the feasibility branch.
The earlier sections document v1 evidence; v2 supersedes the described prototype
mechanisms, without claiming C1 acceptance or changing any acceptance bound.

* Snapshot storage is owned by the immutable generation and retires with its
  last Arc. Publication binds scope/recipe/generation. Same-scope Refresh keeps
  old readers; delivery retries once with the current recipe. Scope changes,
  Off and revocation cancel readers via watch, remove the published snapshot,
  and refuse replacement until the old readers drain. The five-second deadline
  covers both attempts. Native synchronous embedding still requires the
  separately measured cancellation seam; this is not installed qualification.
* v2 runs native 32- and 2,048-document synthetic measurements, and, when
  `--public-docs` is supplied, native extraction/embedding/snapshot measurements
  for its first 1,000 public files. Every source hash, extraction failure,
  per-query result and chunk/page count is private. Public correctness compares
  exact first-chunk content, permitting duplicate content; human relevance,
  viewer completeness and true cold-cache trials remain owed. RSS values are
  process high-water marks, not instantaneous or GPU-exclusive allocations.
* The candidate Seatbelt profile starts with deny-default. It grants only
  runtime assets, literal root, worker, read-only snapshot and per-job scratch.
  Both role-labelled capability workers probe host reads/metadata/list/write/
  create/rename/unlink; socket creation, connect/bind/listen/sendto/connectx/
  sendmsg, TCP/UDP IPv4/IPv6, loopback, route gateway LAN and public resolver Internet
  addresses (external destinations only under confinement); AF_UNIX, AF_SYSTEM, multicast, DNS, Mach/XPC/LaunchServices broker
  lookup, children and sanitized inherited descriptors plus deliberately inherited file/socket bypass diagnostics. Non-EPERM errors and
  unexplained broker errors fail. Socket creation remains a separate strict
  result. Real native ingest ordinary-format controls are also attempted;
  there is no actual render worker at this C1 base. The capability workers
  apply DATA=1.5 GiB, NOFILE=64, CPU=2 s and CORE=0; the parent deadline is 3 s.
* Admission and classification share one lock. Eviction selects the oldest
  unauthenticated connection from the source with most pending connections;
  live authenticated sessions are excluded. Eight connections/person and eight
  pending/source are experimental. Global registry and draining-worker permits
  are both bounded at 64 (plus one transient accept candidate). TLS/header idle
  deadline is 2 s; header bytes are bounded at 16 KiB. Finite 64-arrival stalls
  are followed by at least 3 s per h1/h2 of 12-worker raw-TLS/header/pre-auth
  churn. Each protocol records three rounds of 25 established, 25 new-auth and
  25 pre-auth sign-in requests. Sign-in success is separate; none is protected
  before authentication. Invalid/expired session state demotes protection.
* The fair full-run lease precedes start and inference acquisition. All four
  named cache mutations and cancellation race every batch/yield/drain boundary.
  Actual activation and verified import/download promotion APIs are additionally
  wrapped in the harness lease. Cancellation signals immediately, then performs
  generation-scoped invalidation after drain without re-signalling. Duplicate
  deferred invalidation cannot cancel a successor. Product call sites remain
  unchanged and therefore unqualified. The baseline product cancel gap remains
  a recorded failure. Current read-only MCP paths are never labelled mutations.
* Refusal traces now run desktop indexing and Refresh (two batches of at most
  32 chunks), desktop Ask, and all three concurrently. They retain separate
  1 ms sampling traces and null accepted maxima. A1 remains each correct search
  <=5 s; busy never counts as success. No cap or refusal bound is accepted here.

By-hand items above remain unchanged. Tenant trials are specifically pending
Max's Partner Center check. All evidence, fixture/model copies and command
logs must be under `$KD_BRIDGE_JOB_EVIDENCE`, outside the checkout. Use a new
`--out` per run; `manifest-v1.schema.json` remains the historical validator and
`manifest-v2.schema.json` pins this rerun base. No product listener or enabled
build is created by the test-only, default-off feature.
