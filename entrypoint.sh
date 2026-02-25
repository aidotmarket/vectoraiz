#!/bin/sh
# vectorAIz Backend — Railway Entrypoint
# Ensures /data directories exist and are writable before starting the app.

set -e

# Create data directories (volume may mount as empty root-owned dir)
mkdir -p /data/uploads /data/processed /data/temp /data/keystore 2>/dev/null || true

# Start the application
exec uvicorn app.main:app \
    --host 0.0.0.0 \
    --port "${PORT:-8000}" \
    --workers "${WORKERS:-1}" \
    --log-level info
