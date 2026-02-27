# vectorAIz — Build Guide

## Docker Images

There are TWO Dockerfiles. Using the wrong one is the #1 recurring build error.

| File | Purpose | Port | Includes Frontend? | When to use |
|------|---------|------|-------------------|-------------|
| `Dockerfile.customer` | **Customer deployment** | 80 (nginx) | YES — frontend + nginx + backend | GHCR pushes, docker-compose.customer.yml, anything a customer runs |
| `Dockerfile` | Railway backend only | 8000 (uvicorn) | NO | Railway auto-deploy only. Never push to GHCR. |

## Build & Push to GHCR

```bash
cd /Users/max/Projects/vectoraiz/vectoraiz-monorepo

# ALWAYS use Dockerfile.customer for GHCR
docker build -f Dockerfile.customer \
  -t ghcr.io/aidotmarket/vectoraiz:VERSION \
  -t ghcr.io/aidotmarket/vectoraiz:latest .

docker push ghcr.io/aidotmarket/vectoraiz:VERSION
docker push ghcr.io/aidotmarket/vectoraiz:latest
```

## Version Strings — ALL THREE must match

1. `app/main.py` → `API_VERSION = "x.y.z"`
2. `app/config.py` → `app_version: str = "x.y.z"`
3. `docker-compose.customer.yml` → `VECTORAIZ_VERSION:-x.y.z`

## Verification — From the Customer's Perspective

After pushing, ALWAYS test like a customer would:

```bash
cd ~/vectoraiz
docker compose -f docker-compose.customer.yml pull
docker compose -f docker-compose.customer.yml up -d
# Wait 30s for migrations + nginx startup
curl -s http://localhost:8080/           # Must return HTML (frontend)
curl -s http://localhost:8080/api/health # Must return JSON (API via nginx proxy)
```

If localhost:8080 returns ERR_EMPTY_RESPONSE, you used the wrong Dockerfile.

## Docker binary on this machine

OrbStack: `/Users/max/.orbstack/bin/docker`
