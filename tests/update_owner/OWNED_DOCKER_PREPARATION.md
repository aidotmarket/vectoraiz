# R374 owned Docker preparation (source only)

This recipe prepares one disposable, real vectorAIz replacement observation.
It does not authorize execution. Production source is the reviewed PR38 base
`e719adb9ab6bf927f301ee86dc5b7d6a7c90a563`, tree
`b71392972ae895dd2dd01dca701999f591c5ae97` (source67 accepted). The two separately
approved three-case process-native runs are consumed; neither authorizes these
phases. R289 SQLSandbox/security source, artifacts and import chains, R328 scanner,
and R339 native hold remain. Do not import app main, conftest, or security code
during preparation. No Docker command, build, package download, service, native
test, credential lookup, provider or customer action has been run for this recipe.

## Present contract gap: stop before execution

There is **no truthful private-local-image path through the current owner**:

* `host-updater/protocol.go` fixes the repository to
  `ghcr.io/aidotmarket/vectoraiz` and accepts only numeric release tags, optionally
  `-rc.N`. `dev`, source-SHA tags and private repositories are not targets.
* `registerExisting` in `docker.go` rejects an original app image outside that
  repository, even before setup can adopt it (`channel_image_mismatch`).
* `Docker.resolve` always calls `docker manifest inspect --verbose` on the fixed
  repository's tag and then immutable manifest reference. An image in the local
  daemon cache alone does not supply that registry evidence.
* `Docker.image` requires the fixed repository RepoDigest, exact platform config
  ID and matching `version` label. A local tag or `docker load` by itself cannot
  satisfy those assertions. Cache misses cause a real digest pull. Selection
  writes the fixed repository digest, version and mailbox into a final override.
* App-only activation uses `up -d --no-deps --no-build --pull never vectoraiz`;
  that prevents an activation-time pull but does not disable earlier resolution
  or download. `--channel direct` does not select a private image source.

Consequently, do not fill the driver's release fields with invented releases,
retag the local candidate as a GHCR release, edit registration/selection/status,
rewrite DNS/registry transport, wrap Docker with fabricated manifests, replace
the owner, or substitute a tiny HTTP server. None tests this candidate honestly.
Changing only the original Compose image is insufficient: the final owner
override supersedes it. `VECTORAIZ_OWNER_PACKAGE_DIR` supplies owner package
bytes to the installer; it does not change image selection.

A private local candidate can be built and run directly with a private fixture
Compose override and `--pull never --no-build`, without tagging or touching any
customer image. That proves only direct app startup/data behavior. It cannot
exercise this owner's adoption/update path. No companion fixture helper is
included: automating creation now would produce resources that the reviewed
owner cannot adopt. The manifest below documents exact missing inputs rather
than pretending they already exist.

To proceed with the unchanged owner requires separately authorized, genuine
release distribution of the exact candidate through its existing repository
contract, plus a compatible baseline. That conflicts with this task's no-release,
no-push scope. Alternatively, a truthful private selection contract would need
a separate product decision/change and review; this preparation adds no such
behavior. Do not execute the lifecycle driver until this gap is actually resolved.

## Inputs and isolation for a later bounded run

Use a disposable Linux user/VM with an already available working user systemd
session and an already available Docker daemon, Compose plugin, curl, Python,
RTK, and Go toolchain if compiling. No installation, daemon startup, linger,
reboot, host/customer Docker configuration discovery or credential retrieval.
Use one native architecture shared by owner, image and daemon; no emulation or
cross-host claim. Keep all operator commands and subprocess argv under
`rtk proxy`. Product subprocesses retain their normal direct argv.

Choose a new private absolute root `F` (0700), with spaces/non-ASCII permitted,
and unique project `P = r374-owned-<32 random hex characters>`. `ownership.json`
contains `{"project":"P"}` with the actual value substituted. Use only an
explicit preassigned disposable context `C`; do not switch the global context.
All fixture files, import bind sources, receipts and synthetic auth stay under
F. Named volumes and network are project-scoped. Before creating anything,
check for collisions in that context for the exact proposed project, volume
and network names and chosen loopback port; stop rather than adopt existing
resources. The ownership file is a collision check, not authority over resources.
Record created IDs and Compose project/service/volume/network labels privately.

Use a disposable OS account because normal owner setup installs a user service
and launchers. Merely changing HOME on a real user's active systemd session does
not isolate service-manager effects. Supply an empty fixture-owned Docker config
and existing explicitly supplied context endpoint material, with no customer
auth/credential helper. If registry authentication is needed, that is an
additional named access exception; do not borrow it from the host.

Required concrete inputs, chosen before a later request to execute:

| Input | Meaning |
| --- | --- |
| source | Read-only exact base checkout above; preparation branch changes docs only |
| baseline source/image | Exact approved protocol-compatible baseline source and immutable platform/config IDs; not an assumed published old release |
| candidate image | Full customer app built from exact reviewed source, immutable platform/config IDs, truthful version identity and distribution path; currently unavailable to owner privately |
| owner | Absolute persistent path to actual `host-updater` executable and source/build provenance; never `native_run_probe.go` |
| F, P, C | New disposable root/project and preassigned owned Docker endpoint |
| port | One unused loopback-only host port mapped to app :80 |
| auth | Newly created fixture-local account/key with admin or update:submit; no provider login |
| data | Newly generated synthetic settings/files, serial state, keystore, Postgres rows, Qdrant vector/payload |

These are executable dependencies and collision protection, not new release
forms, signing schemes or mandatory metadata. No values or binaries are supplied
by the existing lifecycle driver.

## Phase 1: source and build preparation (separate approval)

Keep an immutable source export of the exact reviewed tree outside F's data and
build outputs. Copy `docker-compose.customer.yml` byte-for-byte to F; use its
three-service topology, original service names, health checks, data targets and
restart policies. For this disposable local topology only, edit the fixture
copy to bind `127.0.0.1:<chosen-port>:80`, omit the Docker socket mount, and set
the build context to the immutable source export. Keep the original checked-in
file unchanged. The owner uses the host Docker CLI; its app mailbox needs no
socket. This selects the existing no-socket variant, not a containment claim.
Set HOST_IMPORT_DIR to an absolute newly created F/import directory.

Build the full unmodified `Dockerfile.customer` from that export. A prospective
private build command, **not executable under current authority**, is:

```sh
rtk proxy docker --context "$C" build --file "$SOURCE/Dockerfile.customer" --build-arg VERSION=dev --tag "localhost/$P/app:source-e719adb9" "$SOURCE"
```

`dev` is truthful for this unreleased source build, and intentionally fails the
owner release contract. Do not substitute a numeric published version to make
it pass. Inspect and retain the actual resulting config ID/platform and bind it
to the source/build logs; the tag alone proves nothing. A direct fixture override
may use that private tag, with no pulls and no builds during Compose up. This
optional direct-start path must stop before owner adoption and is not needed
until the image-selection gap has a disposition.

The customer build includes nginx/UI, actual Python API, entrypoint and Alembic
migrations, not a synthetic replacement service. It runs npm ci/install/build,
apt, pip (including the PyTorch CPU index), numerical library imports, NLTK/model
downloads and cloudflared's current latest download. Base images are
node:22-slim and python:3.11.11-slim-bookworm. Record resolved base digests and full
build logs; the Dockerfile's mutable inputs do not imply reproducible output.
Network destinations include container registries, Debian mirrors, npm/PyPI,
download.pytorch.org, NLTK/model hosts and GitHub/cloudflared redirects. This
requires explicit package/download/network authority and held-import-chain
disposition; it is not covered by a Docker-only exception.

If no approved actual owner binary exists, compile the unmodified complete
`host-updater` package (all production Go files, no test/probe entrypoint) with an
existing compatible Go toolchain, `CGO_ENABLED=0 GOPROXY=off GOSUMDB=off
GOTOOLCHAIN=local`, private GOCACHE and native GOOS/GOARCH, `go build -o <F>/bin/owner
.` from its copied production module. Record toolchain, source and binary digest.
Do not run tests or fetch modules/toolchains. No owner build occurred here.

Budget for a later build: one native platform, one candidate image (baseline
only if separately specified), up to 60 minutes and 40 GiB private build/cache
storage on a 4-vCPU/16-GiB worker; these are proposed ceilings, not measured
requirements or product guarantees. Stop on timeout/resource failure, preserve
logs, and request a revised bound rather than retrying or downloading tools.
This phase proves artifact provenance only, never activation.

## Phase 2: create and seed the real fixture (separate approval)

Only after the selection gap is resolved, prepare the actual compatible baseline
in the original context. Use the fixture copy of customer Compose plus a final
fixture override for approved immutable image identities and feature environment.
Pin Postgres 16-alpine and Qdrant v1.13.2 by their approved platform digests in
that override. Do not silently refresh tags or run Compose build. Use original
ordered files and explicit `--env-file F/fixture.env`, `--project-directory F`,
`-p P`, `--context C`; these exact inputs must later go to owner setup.

Keep auth enabled. Use existing standalone settings and existing AllAI/marketplace
disabled options. These options alone do not guarantee absence of outbound
traffic: constrain the disposable VM's runtime egress externally to the explicitly
approved registry access needed by the owner; deny app provider/serial calls.
App, Postgres and Qdrant need their private Compose network and local health
traffic; host curl needs the loopback port. No Redis or external DB is required
by this three-service fixture; an unexpected dependency is a stop condition.

Generate synthetic POSTGRES_PASSWORD, bootstrap/account material and keystore
passphrase locally under F. Leave HMAC/encryption env inputs empty so the normal
entrypoint creates and subsequently reuses its persisted secret files. Never
overwrite or regenerate them after baseline. Use valid synthetic local serial
state from an approved existing fixture in the real serial format; no activation
or provider request. If that format/state is unavailable without held-source
work, record it as another unresolved input rather than writing `{}` and claiming
serial preservation. Use the app's normal keystore handling, never a dummy file
claimed as a working identity.

Start exactly vectoraiz/postgres/qdrant with explicit images, `--no-build --pull
never`; approved missing dependency images may be pulled once by exact digest
in a separate logged preparation step. Wait for all three real health checks.
Use the normal first-run local setup UI/API to create a synthetic admin and
save its returned API key as the private header file expected by the driver.
This is synthetic credential creation, not customer credential retrieval. Its
auth/security imports and DB migrations need the explicit bounded exception
described below; no app/auth code is executed during source preparation.

Seed actual contents before replacement:

* `/data/r374-owned-probe` and `/data/import/r374-owned-probe`: different known
  nonempty bytes; retain originals privately, plus one representative real
  app-created synthetic data record with its file payload.
* `/data/serial.json`, `/data/.vectoraiz_hmac_secret`,
  `/data/.vectoraiz_secret_key`, app keystore and fixture env/settings: retain
  exact original bytes privately, access modes and paths. Capture a normal
  authenticated read of the app record/settings using the original key.
* In the fixture Postgres only, execute
  `CREATE TABLE r374_owned_probe(id integer PRIMARY KEY, payload text NOT NULL);`
  and `INSERT INTO r374_owned_probe VALUES (1, 'owned row one'), (2, 'owned row two');`.
  Retain exact ordered rows as well as the real app-created account/data rows.
* In fixture Qdrant, PUT `/collections/r374_owned_probe` with
  `{"vectors":{"size":4,"distance":"Cosine"}}`, then PUT
  `/collections/r374_owned_probe/points?wait=true` with
  `{"points":[{"id":1,"vector":[0.1,0.2,0.3,0.4],"payload":{"fixture":"owned payload","count":7}}]}`.
  Read point 1 with `?with_payload=true&with_vector=true` and retain its full
  decoded vector/payload, not merely a collection count or metadata digest.

Use scoped docker exec/HTTP calls to these exact owned container IDs/endpoints;
retain SQL/HTTP output privately. Never seed customer databases or host paths.
Cap seed/startup at 10 minutes, synthetic payloads at 10 MiB, runtime at 4 vCPU,
12 GiB RAM and 10 GiB data space. Failure is UNKNOWN with retained inventory.
This phase proves a real populated baseline exists, not replacement preservation.

## Phase 3: normal adoption and one actual API replacement (blocked)

The manifest consumed by `run_owned_lifecycle.py` is JSON with these keys:

```text
fixture_root: F (absolute); project: P; docker: absolute real Docker CLI
context: C; project_directory: F
compose_files: [F/customer.yml, F/fixture.override.yml] (original order)
env_files: [F/fixture.env]
owner_binary: F/bin/owner (actual production executable, kept in place)
adoption_tag: genuine compatible baseline release (UNRESOLVED)
target_tag: genuine exact candidate release (UNRESOLVED)
expected_image_id: candidate platform config sha256 ID (UNRESOLVED)
app_url: http://127.0.0.1:<chosen-port>
watchdog_seconds: 3000; request_key: one fresh 32-hex idempotency key
```

This is a field description, not runnable JSON or an invented release receipt.
Create `F/synthetic-auth-headers.txt` (0600) with the newly issued synthetic
account's normal auth header. Do not hand-construct a signed owner request or
change auth policy. The API route requires admin or update:submit and submits
the authenticated principal via the real owner client/mailbox.

After explicit approval and R339 boundary release, invoke the existing driver
once for each phase, using fresh receipt directories `F/receipts/<phase>`:

```sh
rtk proxy python3 "$SOURCE/tests/update_owner/run_owned_lifecycle.py" --mode owned-docker --phase baseline --fixture-manifest "$F/manifest.json" --receipt-dir "$F/receipts/baseline" --r339-released
rtk proxy python3 "$SOURCE/tests/update_owner/run_owned_lifecycle.py" --mode owned-docker --phase adopt --fixture-manifest "$F/manifest.json" --receipt-dir "$F/receipts/adopt" --r339-released
rtk proxy "$F/bin/owner" wait --root "$F/.vectoraiz-owner"
```

Adoption is a mutating update in its own right. `setup` invokes real registration,
normal integrate/startSupervisor and release selection; it creates the uniquely
named user service and launcher files, then may replace the app. The driver's
120-second adoption subprocess timeout does not bound the continuing owner.
Authorize the normal disposable-user setup explicitly; no fabricated Integrated
marker, manual owner serve substitute or test-only product switch. Do not run
the release-downloading installer wrapper for this custom invocation.

Compare all original bytes/payloads/settings after adoption. Retain the initial
baseline, then capture another `baseline` phase into `F/receipts/pre-submit`.
This second baseline ensures submit/observe measures the API-triggered
replacement rather than counting adoption's container change as API success.

```sh
rtk proxy python3 "$SOURCE/tests/update_owner/run_owned_lifecycle.py" --mode owned-docker --phase submit --fixture-manifest "$F/manifest.json" --receipt-dir "$F/receipts/submit" --r339-released
rtk proxy python3 "$SOURCE/tests/update_owner/run_owned_lifecycle.py" --mode owned-docker --phase observe --fixture-manifest "$F/manifest.json" --receipt-dir "$F/receipts/observe" --baseline "$F/receipts/pre-submit/baseline.json" --r339-released
```

Use a target with a different config ID from pre-submit. Capture the actual
202/operation correlation and status independently. Do not retry on lost ACK,
unknown, timeout or setup failure in this single-run scope. Allow up to 50 minutes
each for adoption and requested replacement, plus 10 minutes for final reads.
An external operator deadline must stop further experiment steps and record
still-running owned work; a driver timeout does not cancel the owner/service.

## Phase 4: independent readback, receipts and owned cleanup

The driver already reads real file bytes, ordered SQL values and a Qdrant point,
but stores hashes. It does not assert full settings/keystore or app-level data
access, and its Qdrant GET does not request vectors. Supplement its observations:
compare the retained original file bytes/settings, keystore bytes/modes and
decoded SQL rows and Qdrant payload **and vector**, then read the original real
app record/settings with the original synthetic key through the candidate API.
No fresh key, serial provisioning or DB reseed is allowed after replacement.
Capture full mount objects, restart policy, dependency container/config/image
identities and exact candidate config ID; confirm API health/version from inside
that pinned container and continued normal authenticated access on loopback.
Distinguish expected owner version/mailbox selection from other settings changes.

Only those actual observations can establish preserved data and working auth
through app replacement. Hashes of environment/config metadata, owner success
text, port health from another server or unchanged volume names are insufficient.
Do not label driver pass alone as full preservation acceptance.

Keep full uncropped stdout/stderr (including failures), original and pre-submit
baselines, operation/status, source/build identities, image inspection, app/API
and DB readbacks privately at 0700/0600. Share only bounded relevant excerpts
and booleans without secret values. Driver result says cleanup NOT_RUN and must
remain so until cleanup is observed independently.

After preserving evidence, stop/disable only the exact owner user unit derived
from this registration's installation ID, wait for its owned work to finish,
and remove only that unit and its fixture launchers. Do not disable other users'
services or delete unknown launchers. In C, recheck inventory IDs and project
labels, then use the exact original Compose argv/project to remove this fixture's
three containers and network; remove its three named volumes only after verifying
their recorded identities/ownership and approval to discard synthetic payloads.
Never prune, reset, use global image cleanup, or run down in an arbitrary cwd.
Retain private receipts and failed-state inventory; remove only exclusively
created local image tags/cache if specifically included, never shared base or
dependency images. Budget 10 minutes; report cleanup failure separately.

## Concrete future exception to request

No approval is requested or consumed by this document. Once the image-selection
gap has a lawful disposition, a bounded request must name this source/tree,
actual owner build/image/baseline identities, disposable Linux user/VM/context,
F/P/loopback port, and the phases above. It would permit one full customer build
with the listed package/model network effects if needed; three real containers,
one normal owner user-service setup/adoption, one authenticated API update,
synthetic local auth/serial/keystore/data creation and readback, scoped Docker
exec/SQL/Qdrant operations, and the exact owned teardown. Proposed total ceiling:
200 minutes including build, 4 vCPU, 16 GiB RAM, 50 GiB disk, no retries.

It must explicitly release R339 for this Docker/normal-setup boundary and address
the real app's R289-held auth/security/import/migration chains for this isolated
fixture. Denying provider egress does not grant import-chain authority. R328
scanner execution, security probes, unrelated native tests and prior consumed
process cases remain excluded. If that exception cannot be granted, leave the
real app phases NOT_RUN. No reviewer dispatch, PR/publication, release tag/push,
credential retrieval, customer mutation, reboot, installed/native supervisor
acceptance, browser/UI click acceptance or customer acceptance is implied.
