# vectorAIz

**Your data. Your infrastructure. AI-ready.**

vectorAIz transforms your corporate data into searchable, AI-optimized assets — completely private, running entirely on your hardware. Connect your own LLM, upload your files, and query everything with natural language.

[![License: ELv2](https://img.shields.io/badge/License-ELv2-blue.svg)](LICENSE)
[![Python 3.11+](https://img.shields.io/badge/python-3.11+-blue.svg)](https://www.python.org/downloads/)
[![Docker](https://img.shields.io/badge/docker-required-blue.svg)](https://www.docker.com/)

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

### Option 2: Platform installers

Download from the [latest release](https://github.com/aidotmarket/vectoraiz/releases/latest):

| Platform | Download | Run |
|----------|----------|-----|
| **macOS** | `install-mac.sh` | `chmod +x install-mac.sh && ./install-mac.sh` |
| **Linux** | `install-linux.sh` | `chmod +x install-linux.sh && ./install-linux.sh` |
| **Windows** | `install-vectoraiz.ps1` | Run in PowerShell as Administrator |

### Option 3: Docker Compose (manual)

```bash
git clone https://github.com/aidotmarket/vectoraiz.git
cd vectoraiz
docker-compose up --build
```

Once running:

- **vectorAIz UI** → [http://localhost:8000](http://localhost:8000)
- **API docs** → [http://localhost:8000/docs](http://localhost:8000/docs)
- **Health check** → [http://localhost:8000/api/health](http://localhost:8000/api/health)

## First-Time Setup

1. **Launch vectorAIz** — open `http://localhost:8000` in your browser
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
│  │  :8000    │  │   :6333    │           │
│  │           │  └────────────┘           │
│  │           │  ┌────────────┐           │
│  │           │──│   DuckDB   │           │
│  │           │  │ (tabular)  │           │
│  └─────┬─────┘  └────────────┘           │
│        │                                 │
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
- **DuckDB** — embedded analytical database for tabular data storage and queries
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
open http://localhost:8000/docs
```

The `docker-compose.yml` mounts `app/` as a volume — code changes reflect immediately without rebuilding.

## Connect to ai.market (optional)

vectorAIz can optionally connect to [ai.market](https://ai.market) to publish your dataset metadata and make it discoverable by AI agents and buyers. This is entirely opt-in — no data is shared, only metadata you explicitly publish.

## Requirements

- **Docker** and **Docker Compose** (v2+)
- **4GB RAM** minimum (8GB recommended)
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
