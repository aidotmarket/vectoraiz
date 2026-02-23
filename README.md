<p align="center">
  <img src="backend/frontend/public/vectoraiz-logo-sm.png" alt="vectorAIz" width="200" />
</p>

<h1 align="center">vectorAIz</h1>

<p align="center">
  <strong>Turn your data into AI-ready assets — locally, privately, for free.</strong>
</p>

<p align="center">
  <a href="#install">Install</a> · <a href="#features">Features</a> · <a href="https://vectoraiz.com">Website</a> · <a href="#architecture">Architecture</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/status-public%20beta-teal" alt="Public Beta" />
  <img src="https://img.shields.io/badge/license-ELv2-blue" alt="License" />
  <img src="https://img.shields.io/badge/docker-required-2496ED?logo=docker&logoColor=white" alt="Docker" />
  <img src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows%20(WSL)-lightgrey" alt="Platforms" />
</p>

---

vectorAIz is a free, self-hosted tool that processes your documents locally, generates vector embeddings, and makes your data queryable with natural language. Upload files, ask questions, get AI-powered answers — everything runs on your machine. Your data never leaves your network.

## Install

### One-command install (all platforms)

```bash
git clone https://github.com/maxrobbins/vectoraiz.git && cd vectoraiz && ./start.sh
```

**Requirements:** [Docker](https://docs.docker.com/get-docker/) (Docker Desktop, OrbStack, or docker.io)  
**Works on:** macOS (Apple Silicon & Intel) · Linux (Ubuntu, Debian, Fedora) · Windows (WSL)

The installer will:
1. Check that Docker is installed and running
2. Find an available port (prefers 80, falls back to 8080, 3000, etc.)
3. Generate secure credentials on first run
4. Build and start all containers
5. Open your browser when ready

To stop: `./stop.sh`

### Platform installers

| Platform | Download | Notes |
|----------|----------|-------|
| **macOS** | [vectorAIz-Installer.dmg](https://github.com/maxrobbins/vectoraiz/releases/latest) | Checks for Docker/OrbStack, installs if needed |
| **Windows** | [install-vectoraiz.ps1](https://github.com/maxrobbins/vectoraiz/releases/latest) | Requires Docker Desktop + WSL2 |
| **Linux** | `curl -fsSL https://get.vectoraiz.com/install.sh \| bash` | Auto-installs Docker if missing |

### Allie AI Assistant (optional)

vectorAIz includes Allie, an AI-powered data assistant. To enable her, run:

```bash
./start.sh --setup-allie
```

You'll need an API key from [ai.market](https://ai.market). Without Allie, vectorAIz runs in standalone mode with full functionality using your own LLM keys.

## Features

- **Local-first processing** — Your data never leaves your machine. All parsing, chunking, and embedding happens locally.
- **Multi-format ingestion** — PDF, DOCX, XLSX, CSV, TXT, HTML, Markdown, and more. Drag and drop any document.
- **Vector search** — Automatic chunking and embedding into Qdrant for fast semantic search.
- **RAG queries** — Ask natural-language questions against your documents and get cited, context-aware answers.
- **BYO LLM** — Bring your own API key from OpenAI, Anthropic, Google, or any compatible provider.
- **Data attestation** — Cryptographic hashing and metadata tracking for data provenance and integrity.
- **MCP server** — Expose your processed data to AI assistants via the Model Context Protocol.
- **Privacy scoring** — Automatic PII detection with GDPR, CCPA, and HIPAA compliance reporting.
- **Clean UI** — Web-based interface for uploading, searching, managing datasets, and configuring providers.

## Architecture

```
┌─────────────────────────────────────────────────┐
│               Web Interface (frontend/)          │
│            React + Vite + Tailwind CSS           │
└──────────────────────┬──────────────────────────┘
                       │ REST API
┌──────────────────────▼──────────────────────────┐
│               vectorAIz API (backend/)           │
│          FastAPI · Python · Uvicorn               │
│                                                   │
│  ┌─────────┐  ┌──────────┐  ┌────────────────┐  │
│  │ Upload  │  │  Search  │  │  RAG Query     │  │
│  │ Pipeline│  │  Engine  │  │  Engine        │  │
│  └────┬────┘  └────┬─────┘  └───────┬────────┘  │
│       │            │                │            │
│  ┌────▼────────────▼────────────────▼─────────┐  │
│  │           Qdrant Vector Database           │  │
│  └────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

## Quick Start

Once running, visit **http://localhost** (or the port shown in the terminal) to:

1. **Upload documents** — Drag and drop PDFs, CSVs, Excel files, or any supported format
2. **Browse datasets** — View uploaded data, metadata, and processing status
3. **Search** — Run natural-language queries across all your indexed data
4. **Configure LLMs** — Add your API keys for OpenAI, Anthropic, Google, etc.
5. **API access** — Full REST API at `/docs` (Swagger UI)

## Project Structure

```
vectoraiz/
├── start.sh              # ← Run this to get started
├── stop.sh               # ← Run this to stop
├── backend/
│   ├── app/              # FastAPI application source
│   ├── frontend/         # Embedded web UI
│   ├── tests/            # Test suite
│   ├── alembic/          # Database migrations
│   ├── docker-compose.customer.yml
│   ├── start.sh          # Core installer script
│   └── Dockerfile
├── frontend/             # Standalone frontend (development)
├── LICENSE               # Elastic License v2 (ELv2)
└── README.md
```

## Contributing

We welcome contributions! Please open an issue or pull request.

## License

Licensed under [Elastic License v2 (ELv2)](LICENSE). You can use, copy, distribute, and modify the software — but you can't provide it as a managed service to third parties.

## Links

- [vectoraiz.com](https://vectoraiz.com)
- [@vectoraiz](https://x.com/vectoraiz) on X
- [Vectoraiz](https://linkedin.com/company/111660309) on LinkedIn
- [ai.market](https://ai.market) — Data marketplace for AI

---

<p align="center">Built with ❤️ by the vectorAIz team</p>
