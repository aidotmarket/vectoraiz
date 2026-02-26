#!/bin/bash
set -e

# Air-gap: prevent HuggingFace Hub from phoning home (model is baked into image)
export HF_HUB_OFFLINE=1
export TRANSFORMERS_OFFLINE=1

echo ""
echo "  ╔═══════════════════════════════════════════╗"
echo "  ║                                           ║"
echo "  ║       ⚡ vectorAIz v${VECTORAIZ_VERSION:-1.8.1}               ║"
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

# Auto-detect workers based on CPU cores if not explicitly set
if [ -z "$VECTORAIZ_WORKERS" ]; then
    CPU_CORES=$(nproc 2>/dev/null || echo 4)
    VECTORAIZ_WORKERS=$(python3 -c "import os; cores=os.cpu_count() or 4; print(min(max(cores//4, 2), 4))")
    echo "[INFO] Auto-detected $CPU_CORES CPU cores → $VECTORAIZ_WORKERS workers"
fi

# Start uvicorn (foreground — tini handles signals)
echo "[INFO] Starting API server with $VECTORAIZ_WORKERS workers..."
echo ""
exec uvicorn app.main:app \
    --host 0.0.0.0 \
    --port 8000 \
    --workers ${VECTORAIZ_WORKERS} \
    --log-level ${VECTORAIZ_LOG_LEVEL:-info}
