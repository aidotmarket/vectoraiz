# vectorAIz Backend

Data processing and serving API for AI.Market.

## Quick Start

1. Clone the repository
2. Copy `.env.example` to `.env`
3. Run with Docker Compose:
```bash
docker-compose up --build
```

4. API available at http://localhost:8001
5. API docs at http://localhost:8001/docs
6. Qdrant dashboard at http://localhost:6333/dashboard

## Development

The `docker-compose.yml` is configured for hot reloading. Edit files in `app/` and changes will reflect immediately.

## API Endpoints

- `GET /` - Root endpoint
- `GET /api/health` - Health check
- `GET /api/health/ready` - Readiness check
- `GET /api/datasets` - List datasets (stub)
- `POST /api/datasets/upload` - Upload dataset (stub)
- `GET /api/search?q=query` - Search datasets (stub)
- `POST /api/sql/query` - Execute SQL (stub)
