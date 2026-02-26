#!/bin/bash
set -e

# Air-gap: prevent HuggingFace Hub from phoning home (model is baked into image)
export HF_HUB_OFFLINE=1
export TRANSFORMERS_OFFLINE=1

echo ""
echo "  ╔═══════════════════════════════════════════╗"
echo "  ║                                           ║"
echo "  ║       ⚡ vectorAIz v${VECTORAIZ_VERSION:-1.8.4}               ║"
echo "  ║                                           ║"
echo "  ║   Mode: $(printf '%-34s' "${VECTORAIZ_MODE:-standalone}")║"
echo "  ║                                           ║"
echo "  ║   ➜  http://localhost                     ║"
echo "  ║                                           ║"
echo "  ╚═══════════════════════════════════════════╝"
echo ""

# Auto-generate HMAC secret if not provided
if [ -z "$VECTORAIZ_APIKEY_HMAC_SECRET" ]; then
    HMAC_FILE="/data/.vectoraiz_hmac_secret"
    if [ -f "$HMAC_FILE" ]; then
        export VECTORAIZ_APIKEY_HMAC_SECRET=$(cat "$HMAC_FILE")
        echo "[INFO] Using existing HMAC secret"
    else
        export VECTORAIZ_APIKEY_HMAC_SECRET=$(python3 -c "import secrets; print(secrets.token_hex(32))")
        echo "$VECTORAIZ_APIKEY_HMAC_SECRET" > "$HMAC_FILE"
        chmod 600 "$HMAC_FILE"
        echo "[INFO] Generated HMAC secret"
    fi
fi

# Auto-generate SECRET_KEY if not provided
if [ -z "$VECTORAIZ_SECRET_KEY" ]; then
    SECRET_FILE="/data/.vectoraiz_secret_key"
    if [ -f "$SECRET_FILE" ]; then
        export VECTORAIZ_SECRET_KEY=$(cat "$SECRET_FILE")
        echo "[INFO] Using existing encryption key"
    else
        export VECTORAIZ_SECRET_KEY=$(python3 -c "from cryptography.fernet import Fernet; print(Fernet.generate_key().decode())")
        echo "$VECTORAIZ_SECRET_KEY" > "$SECRET_FILE"
        chmod 600 "$SECRET_FILE"
        echo "[INFO] Generated encryption key"
    fi
fi

# Run database migrations
echo "[INFO] Running database migrations..."
cd /app && python -m alembic upgrade head
echo "[INFO] Migrations complete"

# Start nginx in background
echo "[INFO] Starting web server..."
nginx

# Co-Pilot requires single-worker mode (file lock enforced).
# Multi-worker support would require switching Co-Pilot to Redis pub/sub.
# nginx handles concurrent connections; uvicorn single worker handles async I/O.
VECTORAIZ_WORKERS=1
echo "[INFO] Starting API server..."
echo ""
exec uvicorn app.main:app \
    --host 0.0.0.0 \
    --port 8000 \
    --workers ${VECTORAIZ_WORKERS} \
    --log-level ${VECTORAIZ_LOG_LEVEL:-info}
