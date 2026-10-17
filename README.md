# vectorAIz

**Your data. Your infrastructure. AI-ready.**

vectorAIz transforms your corporate data into searchable, AI-optimized assets — completely private, running entirely on your hardware. Connect your own LLM, upload your files, and query everything with natural language.

[![License: ELv2](https://img.shields.io/badge/License-ELv2-3F51B5.svg)](LICENSE)
[![Python 3.11+](https://img.shields.io/badge/python-3.11+-3F51B5.svg)](https://www.python.org/downloads/)
[![Docker](https://img.shields.io/badge/docker-required-3F51B5.svg)](https://www.docker.com/)
[![ai.market](https://img.shields.io/badge/ai.market-product-0F6E56.svg)](https://ai.market)

---

## Why vectorAIz?

Most AI data tools send your data to the cloud. vectorAIz doesn't. Everything runs locally — your files never leave your network.

- **Private by design** — data stays on your machine, always
- **Bring your own LLM** — OpenAI, Anthropic Claude, or Google Gemini
- **Upload anything** — CSV, JSON, TXT, Markdown, HTML, PDF, DOCX, PPTX, XLS, and more
- **Local directory import** — mount a local folder and import files directly, no upload needed
- **Database connectivity** — connect to Postgres and MySQL databases, extract and vectorize table data
- **Natural language queries** — ask questions about your data in plain English
- **AI copilot** — allAI assistant helps you explore and understand your datasets
- **Parallel upload** — concurrent upload workers with real progress tracking
- **Large file streaming** — handles files of any size with chunked processing
- **Auto-update** — automatic software updates for Docker deployments
- **Data preview** — inspect schemas, stats, and samples before vectorizing
- **Diagnostic tools** — structured logging, health checks, one-click diagnostic export

## Quick Start

### Option 1: One-line install (recommended)

```bash
git clone https://github.com/aidotmarket/vectoraiz.git && cd vectoraiz && ./start.sh
```

The updated source scripts preserve existing Docker data volumes on reruns and reuse `.env`. Older versions of `start.sh` and the macOS installer, including v1.20.43, contain automatic cleanup; do not rerun them against an existing installation. Use scripts containing the data-preservation correction before retrying. Keep a backup of the original `.env`: its database password and application secrets are needed to use existing data. If `.env` is missing while data volumes remain, restore the original file to the startup directory (`~/vectoraiz/.env` for the macOS installer). A health timeout is a reason to inspect Compose logs, not to delete volumes.

### Option 2: Platform installers

Download from the [latest release](https://github.com/aidotmarket/vectoraiz/releases/latest):

| Platform | Download | Run |
|----------|----------|-----|
| **macOS** | `install-mac.sh` | `chmod +x install-mac.sh && ./install-mac.sh` |
| **Linux** | `install-linux.sh` | `chmod +x install-linux.sh && ./install-linux.sh` |
| **Windows** | `install-vectoraiz.ps1` | Run in PowerShell as Administrator |

To install the `aim-data` channel instead of the standard deployment:

```bash
# macOS
curl -fsSL https://raw.githubusercontent.com/aidotmarket/vectoraiz/main/installers/mac/install-mac.sh | bash -s -- --channel aim-data

# Linux
curl -fsSL https://raw.githubusercontent.com/aidotmarket/vectoraiz/main/installers/linux/install-linux.sh | bash -s -- --channel aim-data
```

```powershell
# Windows PowerShell
$tmp = Join-Path $env:TEMP 'install-vectoraiz.ps1'
irm https://raw.githubusercontent.com/aidotmarket/vectoraiz/main/installers/windows/install-vectoraiz.ps1 -OutFile $tmp
& $tmp -Channel 'aim-data'
```

### Option 3: Docker Compose (manual)

```bash
git clone https://github.com/aidotmarket/vectoraiz.git
cd vectoraiz
docker compose -f docker-compose.customer.yml up -d
```

Use `docker-compose.customer.yml` for the standard customer deployment, or `docker-compose.aim-data.yml` for the `aim-data` channel:

```bash
# Standard customer deployment
docker compose -f docker-compose.customer.yml up -d

# aim-data channel deployment
docker compose -f docker-compose.aim-data.yml up -d
```

Once running:

- **vectorAIz UI** → [http://localhost:8080](http://localhost:8080)
- **API docs** → [http://localhost:8080/docs](http://localhost:8080/docs)
- **Health check** → [http://localhost:8080/api/health](http://localhost:8080/api/health)

## Host-owned updates (R374 source, unreleased)

This checkout implements one-time host adoption followed by one-click updates from Settings. It has isolated source/fake proof only. Native Docker transport, platform supervision, real authentication, migrations, distribution and retained-data acceptance remain **NOT_RUN**; this is not a released installer announcement. The original Docker socket remains cooperative-only authority, with no containment claim. Installations without that socket use the same host owner transport.

Use the owner-aware installer from the exact verified release when these artifacts are published. The release bundle contains standalone `vectoraiz-owner-{darwin,linux}-{amd64,arm64}` or `vectoraiz-owner-windows-amd64.exe`, checksums and `setup-owner.sh`/`setup-owner.ps1`. Hosts need their existing Docker CLI/context and platform supervisor, with no Python or RTK dependency added. Checksums are retrieved from the same authenticated HTTPS release; independent signing/distribution evidence remains outstanding. Linux requires a working existing user systemd session; macOS uses a user LaunchAgent; Windows uses a per-user logon task. The updater grants no Docker permissions and does not enable linger or install an administrator service.

Back up the original Compose files, `.env`, ordered overrides/env files and installation directory privately. Keep data volumes, serial, HMAC/encryption files, credentials, keystore and original dependency containers. Restore genuinely missing original configuration before adoption; do not generate replacement credentials. Owner-aware reruns delegate before downloading Compose, copying archives, changing ports/channel, provisioning serials or building an accepted release. AIM-Data one-click adoption remains `channel_not_integrated` until its own release evidence exists.

For the standard shipped installation, the installer performs adoption in its original directory. Set `VECTORAIZ_OWNER_RELEASE` to the exact owner-capable release for initial adoption; it must have the corresponding owner assets. `VECTORAIZ_APP_URL` retains an existing remote browser route where needed. One-time adoption activates that exact protocol-capable image and waits for verification. Later authenticated clicks use the displayed chosen tag and a persistent request key; the owner freezes the platform digest, pulls only that reference and activates the app in the original project.

Custom original invocations use the verified binary's `setup` once, under the **original installation user and original interpolation environment**. Repeat `--file` and `--env-file` in their original order; retain the original working directory, project directory/name, context and channel. Example shape (replace placeholders with actual original values):

```text
vectoraiz-owner setup --root "/original install/.vectoraiz-owner" --cwd "/original cwd" --project-directory "/original project" --project actual-project --context existing-context --file "/original/base.yml" --file "/original/override.yml" --env-file "/original/first.env" --env-file "/original/second.env" --channel direct --target vX.Y.Z --url https://existing-app.example
```

Stage the verified binary at `.vectoraiz-owner/owner` (Windows: `owner.exe`) with the restrictive permissions used by the setup helper; Windows state needs the current-user/SYSTEM ACL before setup. Registration privately resolves Compose and compares its configuration hashes with existing project/service containers. It refuses ambiguous/missing resources, changed effective configuration or ownership, unsupported activation flags, and an unwritable root filesystem or absence of a usable non-mounted mailbox. It selects among three fixed image-created private paths, so a custom `/run` mount can retain its configuration and use `/var/lib` or `/opt` instead. A signed challenge must prove the actual app mailbox reader/writer before readiness. Harmless comments, formatting and optional extension metadata can pass after effective configuration/ownership revalidation. Custom launch systems must use the new owner launcher or `launch`; hand-running Compose without the final owner override can intentionally bypass selection.

Use the generated launcher or `vectoraiz-owner launch --root <state>` for subsequent starts. Selection survives stale exported version pins and installer reruns. The owner starts existing stopped dependencies and waits for their health; it never recreates/upgrades dependencies. Its app-only Compose invocation uses the literal final override, `--no-deps --no-build --pull never`. Same-image adoption or repair still activates the app when its selected configuration differs or it is stopped; an already-running app with the exact selected image/configuration skips duplicate activation. Success requires the actual project/service/configuration hash, platform image/config identity, build label, reported version and configured stable health observations.

Before an update, know the independent recovery path on the original Compose client (A), even if Docker/app live on B and the browser is on C:

```text
vectoraiz-owner status --root <installation>/.vectoraiz-owner
```

The private `status.json` remains readable without Docker, the app or a running owner. Stale observations mean unknown current outcome. A loaded browser retains the key and last phase/deadline during disconnection; a full refresh may not load while the app is down. A lost HTTP/ack response is `acceptance_unknown`: look up or retry the **same key**, rather than making a new operation. The Settings page offers **Check update status** and, while acceptance is unknown, **Retry same request**, preserving the original key and target even after a lookup returns 404. Confirmed refusals retain their server reason across polling/reload and allow another attempt after resolving the refusal. Supported cookie-only sessions reach the same authenticated routes as bearer tokens and local API keys; the server decides permissions. Status access does not grant submission permission.

For terminal startup/migration failure, retain the desired image, original restart policy, credentials and volumes. The app may continue restarting under that unchanged policy. There is no automatic app stop, rollback, database downgrade, prune or reset. `retry --target <tag>` starts a new explicit attempt after a confirmed resolution/download failure, including failed initial adoption. `reconcile` observes the original context; `repair --target <same-or-newer-tag>` explicitly starts a forward attempt after revalidating actual state. Every successfully started helper has an asynchronous wait to reap it while the owner survives; the detached helper retains its own deadline and durable outcome independently of polling. An unresolved supervised child blocks conflicting mutation. After its deadline, the owner can seal a lost helper as an unknown outcome only when its OS lock is free and no tracked process group remains (Windows uses a kill-on-close job). It never treats that as command success: inspect the retained original-context state and use explicit forward repair. A live or unidentifiable child continues to block mutation. Never delete lock/journal files to force a retry. Setup interrupted before integration can resume through the staged owner's `launch` without recapturing or overwriting the original context.

Isolated checks are `rtk proxy env GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off go test ./...` in `host-updater`, and `rtk proxy python3 -I tests/update_owner/test_client.py`. These use native-execution/import/network guards and synthetic temporary files. The frontend contract check is `rtk proxy node --permission --allow-fs-read="$(rtk proxy pwd -P)" tests/update_owner/test_frontend.mjs` with no network or child-process grants. `tests/update_owner/run_owned_lifecycle.py` provides separately authorized prepared-fixture adoption/submission/observation phases, including independent data probes; its Docker/native adapters were not executed here. R289/R328/R339 holds remain active; the full sixteen-repository audit remains active and incomplete.

## Deployment Channels

vectorAIz supports multiple deployment channels using the same application image and codebase.

- Standard customer deployments use [`docker-compose.customer.yml`](docker-compose.customer.yml). This is the default installer path.
- `aim-data` is the seller-oriented variant and uses [`docker-compose.aim-data.yml`](docker-compose.aim-data.yml) with `VECTORAIZ_CHANNEL=aim-data`.

If you use an installer, pass `--channel aim-data` to deploy the `aim-data` experience. If you do not set a channel, the standard customer deployment is used.

## First-Time Setup

1. **Launch vectorAIz** — open `http://localhost:8080` in your browser
2. **Create your account** — set up a local admin username and password
3. **Connect your LLM** — go to Settings → LLM and add your API key (OpenAI, Anthropic, or Gemini)
4. **Upload data** — drag and drop files or use bulk upload
5. **Start querying** — ask questions about your data in the chat interface

## Supported File Formats

| Format | Extensions |
|--------|-----------|
| Tabular | `.csv`, `.tsv`, `.json`, `.jsonl` |
| Text | `.txt`, `.md`, `.rst`, `.html` |
| Documents | `.pdf`, `.docx`, `.pptx`, `.xls`, `.xlsx` (via Apache Tika) |

## Architecture

vectorAIz runs as Docker containers on your machine:

```
┌──────────────────────────────────────────┐
│              Your Machine                │
│                                          │
│  ┌───────────┐  ┌────────────┐           │
│  │ vectorAIz │──│   Qdrant   │           │
│  │   API     │  │  (vectors) │           │
│  │  :8080    │  │   :6333    │           │
│  │  (ext)    │  └────────────┘           │
│  │  :80 int  │  ┌────────────┐           │
│  │           │──│ PostgreSQL │           │
│  │           │  │(meta+auth) │           │
│  └─────┬─────┘  │   :5432    │           │
│        │        └────────────┘           │
│        │ Your LLM key                    │
│        ▼                                 │
│  ┌───────────┐                           │
│  │ OpenAI /  │  (external, API calls     │
│  │ Anthropic │   only — no data sent)    │
│  │ / Gemini  │                           │
│  └───────────┘                           │
└──────────────────────────────────────────┘
```

- **vectorAIz API** — FastAPI backend handling uploads, vectorization, search, and the AI copilot
- **Qdrant** — vector database storing embeddings locally
- **PostgreSQL** — metadata storage and authentication
- **Your LLM** — queries go to your chosen provider using your own API key

No data is sent to ai.market or any third party. Only metadata (if you choose to publish) leaves your network.

## Configuration

Environment variables (set in `.env` or `docker-compose.yml`):

| Variable | Default | Description |
|----------|---------|-------------|
| `DEBUG` | `false` | Enable debug logging |
| `VECTORAIZ_AUTH_ENABLED` | `true` | Require authentication |
| `QDRANT_HOST` | `qdrant` | Qdrant hostname |
| `QDRANT_PORT` | `6333` | Qdrant port |

LLM keys are configured through the UI (Settings → LLM) and stored encrypted on disk.

## Development

```bash
# Clone and start with hot reload
git clone https://github.com/aidotmarket/vectoraiz.git
cd vectoraiz
docker-compose up --build

# Run tests
docker-compose exec vectoraiz-api pytest

# API docs (auto-generated)
open http://localhost:8080/docs
```

The `docker-compose.yml` mounts `app/` as a volume — code changes reflect immediately without rebuilding.

## Connect to ai.market (optional)

vectorAIz can optionally connect to [ai.market](https://ai.market) to publish your dataset metadata and make it discoverable by AI agents and buyers. This is entirely opt-in — no data is shared, only metadata you explicitly publish.

## Requirements

- **Docker** and **Docker Compose** (v2+)
- **8 GB RAM** minimum (16 GB recommended for large datasets)
- An API key from OpenAI, Anthropic, or Google (for LLM queries)

## License

Source available under [Elastic License 2.0](LICENSE). Free to use, modify, and run internally. You may not offer vectorAIz as a managed service.

## Links

- 🌐 [vectoraiz.com](https://vectoraiz.com) — project homepage
- 🛒 [ai.market](https://ai.market) — data marketplace
- 📦 [Releases](https://github.com/aidotmarket/vectoraiz/releases) — downloads
- 🐛 [Issues](https://github.com/aidotmarket/vectoraiz/issues) — bug reports

---

Built by [ai.market](https://ai.market) · Made with ❤️ for data teams who take privacy seriously.
