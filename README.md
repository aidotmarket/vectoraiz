<p align="center">
  <img src="https://vectoraiz.com/logo.png" alt="vectorAIz" width="200" />
</p>

<h1 align="center">vectorAIz</h1>

<p align="center">
  <strong>Process, vectorize, and make your data searchable by AI systems.</strong>
</p>

<p align="center">
  <a href="https://vectoraiz.com">Website</a> · <a href="#quick-start">Quick Start</a> · <a href="#features">Features</a> · <a href="#architecture">Architecture</a>
</p>

---

vectorAIz is an open-source, privacy-first tool that processes your documents locally, generates vector embeddings, and makes your data queryable through RAG (Retrieval-Augmented Generation). Upload files, ask questions, and get AI-powered answers — all running on your own machine.

## Features

- **Local-first processing** — Your data never leaves your machine. All file parsing, chunking, and embedding happens locally.
- **Multi-format ingestion** — PDF, DOCX, XLSX, CSV, TXT, HTML, Markdown, and more. Drag and drop any document.
- **Vector search with Qdrant** — Automatic chunking and embedding into a local Qdrant instance for fast semantic search.
- **RAG queries** — Ask natural-language questions against your uploaded documents and get cited, context-aware answers.
- **Built-in data attestation** — Cryptographic hashing and metadata tracking for data provenance and integrity.
- **MCP server** — Expose your processed data to AI assistants via the Model Context Protocol.
- **Desktop app** — Clean UI for uploading, searching, managing datasets, and configuring LLM providers.

## Architecture

```
┌─────────────────────────────────────────────────┐
│                  Desktop App (frontend/)         │
│            React + Vite + Tailwind CSS           │
└──────────────────────┬──────────────────────────┘
                       │ REST API
┌──────────────────────▼──────────────────────────┐
│               vectorAIz API (backend/)           │
│          FastAPI · Python · Uvicorn               │
│                                                   │
│  ┌─────────┐  ┌──────────┐  ┌────────────────┐  │
│  │ Upload  │  │  Search  │  │  RAG / allAI   │  │
│  │ Pipeline│  │  Engine  │  │  Query Engine  │  │
│  └────┬────┘  └────┬─────┘  └───────┬────────┘  │
│       │            │                │            │
│  ┌────▼────────────▼────────────────▼─────────┐  │
│  │           Qdrant Vector Database           │  │
│  └────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

## Quick Start

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and Docker Compose

### Run with Docker

```bash
git clone https://github.com/maxrobbins/vectoraiz.git
cd vectoraiz/backend
cp .env.example .env   # Edit with your LLM API keys
docker-compose up --build
```

The API will be available at **http://localhost:8000** and the Qdrant dashboard at **http://localhost:6333/dashboard**.

### API Docs

Once running, visit **http://localhost:8000/docs** for the interactive Swagger UI.

## Project Structure

```
vectoraiz/
├── backend/          # FastAPI backend — file processing, vector search, RAG
│   ├── app/          # Application source
│   ├── tests/        # Test suite
│   ├── alembic/      # Database migrations
│   ├── docker-compose.yml
│   └── Dockerfile
├── frontend/         # Desktop app — React + Vite
│   └── src/
├── LICENSE           # Elastic License v2 (ELv2)
└── README.md
```

## Backend

The backend handles document ingestion, text extraction, chunking, embedding generation, vector storage (Qdrant), semantic search, and RAG query execution. Built with FastAPI and designed to run entirely on your local machine.

See [`backend/README.md`](backend/README.md) for detailed API documentation.

## Frontend

The frontend is a desktop application built with React, Vite, and Tailwind CSS. It provides a clean interface for uploading documents, browsing datasets, running search queries, and managing LLM provider settings.

## Contributing

We welcome contributions! Please open an issue or pull request.

## License

This project is licensed under the [Elastic License v2 (ELv2)](LICENSE). You can use, copy, distribute, and modify the software — but you can't provide it as a managed service to third parties.

## Links

- [vectoraiz.com](https://vectoraiz.com)
- Powered by [allAI](https://allai.com)

---

<p align="center">Built with ❤️ by the vectorAIz team</p>
