# vectorAIz — Claude Code Context

## Project
Local-first data processing tool. Upload docs, vectorize, search, query via LLM. Docker-packaged, runs on customer hardware. Never phones home with customer data.

## Monorepo Layout
```
app/              → FastAPI backend (Python 3.9+)
  routers/        → API endpoints
  services/       → Business logic
  models/         → SQLAlchemy + Pydantic models
  core/           → Auth, DB, config, middleware
frontend/         → React + Vite + Tailwind + shadcn/ui
  src/pages/      → Page components
  src/api/        → API client functions
  src/components/ → Shared UI components
  src/hooks/      → Custom React hooks
tests/            → pytest test suite
alembic/          → DB migrations (SQLite)
scripts/          → release.sh, verify scripts
installers/       → mac/linux/windows install scripts
deploy/           → nginx.conf, entrypoint.sh
specs/            → BQ spec documents (read-only reference)
```

## Tech Stack
- **Backend:** FastAPI, SQLAlchemy, DuckDB, Qdrant (vectors), SQLite (state/auth/audit)
- **Frontend:** React 18, Vite, TypeScript, Tailwind CSS, shadcn/ui
- **Infrastructure:** Docker (multi-arch), nginx reverse proxy
- **Two Dockerfiles:** `Dockerfile` = Railway dev deploy, `Dockerfile.customer` = customer install (nginx + frontend on port 80)

## Commands
```bash
# Tests
cd /path/to/repo && python -m pytest tests/ -x -q
python -m pytest tests/test_search.py -x -q        # single file
python -m pytest tests/ -k "test_name" -x -q        # single test

# Run locally
uvicorn app.main:app --host 0.0.0.0 --port 8100 --reload

# Frontend dev
cd frontend && npm run dev

# Releases — NEVER run from Claude Code (gh CLI not in PATH)
# Releases are handled externally via scripts/release.sh
```

## Conventions
- **Commits:** `feat:`, `fix:`, `docs:`, `refactor:`, `test:` prefixes
- **Branch:** main only (no feature branches)
- **Tests:** pytest with async support. Always run tests before committing.
- **Imports:** absolute from `app.` (e.g. `from app.services.search_service import SearchService`)
- **API routes:** `/api/v1/` prefix for all endpoints, `/api/copilot/` for allAI
- **Frontend API calls:** centralized in `frontend/src/api/` — one file per domain
- **Error handling:** FastAPI HTTPException for API errors, never raw exceptions
- **Simplicity First:** Prefer simple code. Don't abstract until a pattern repeats. Fewer files, fewer layers, fewer indirections. If a feature can be 50 lines instead of 200 with a new service/factory/registry, write the 50 lines.

## Key Architectural Rules
1. **Data never leaves the customer's machine.** VZ is non-custodial. No telemetry, no cloud storage.
2. **allAI is optional.** Everything must work without it (standalone mode).
3. **BYO LLM.** Customer provides their own API key for RAG/search. allAI key is separate (metered by ai.market).
4. **System API key ("Admin setup") is sacred.** Frontend↔backend auth. Never expose to users, never make deletable.
5. **Null guard everything.** Datasets can be in processing/failed state with null schema/rows. Frontend must handle gracefully.

## What NOT to Do
- Do NOT run `scripts/release.sh` — it requires `gh` CLI which is not in CC's PATH
- Do NOT modify `.github/workflows/` without explicit approval
- Do NOT add external network calls from the backend without checking standalone mode
- Do NOT hardcode versions — version comes from git tags
- Do NOT add new Python dependencies without checking `requirements.txt` compatibility
