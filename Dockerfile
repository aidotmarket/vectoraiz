# =============================================================================
# vectorAIz Backend — Production Dockerfile
# =============================================================================
# BQ-089: Hardened for self-hosting
# S123: Updated for Railway PostgreSQL deployment
# - Multi-stage build (builder + runtime)
# - Non-root user (vectoraiz:1001)
# - Pinned base image
# - No dev dependencies in final image
# - Alembic migrations run on startup via app.core.database.init_db()
# =============================================================================

# ---- Stage 1: Builder ----
FROM python:3.11.11-slim-bookworm AS builder

WORKDIR /build

# Install build-time system deps
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    libpq-dev \
    && rm -rf /var/lib/apt/lists/*

COPY requirements.txt .
RUN pip install --no-cache-dir --prefix=/install -r requirements.txt

# Pre-download NLTK data
RUN PYTHONPATH=/install/lib/python3.11/site-packages \
    python -c "import nltk; nltk.download('punkt', download_dir='/install/nltk_data'); nltk.download('averaged_perceptron_tagger', download_dir='/install/nltk_data')"


# ---- Stage 2: Runtime ----
FROM python:3.11.11-slim-bookworm AS runtime

LABEL maintainer="ai.market <ops@ai.market>"
LABEL description="vectorAIz Backend — Self-hosted data processing engine"

WORKDIR /app

# Runtime system deps only (no build-essential)
RUN apt-get update && apt-get install -y --no-install-recommends \
    libpq5 \
    poppler-utils \
    tesseract-ocr \
    libreoffice \
    pandoc \
    libmagic1 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy installed Python packages from builder
COPY --from=builder /install /usr/local
COPY --from=builder /install/nltk_data /usr/share/nltk_data

# Create non-root user
RUN groupadd -g 1001 vectoraiz && \
    useradd -u 1001 -g vectoraiz -m -s /bin/bash vectoraiz

# Create data directories with correct ownership
RUN mkdir -p /data/uploads /data/processed /data/temp /data/keystore && \
    chown -R vectoraiz:vectoraiz /data

# Copy application code
COPY --chown=vectoraiz:vectoraiz app/ ./app/

# Copy Alembic migration files (BQ-111+)
COPY --chown=vectoraiz:vectoraiz alembic.ini ./alembic.ini
COPY --chown=vectoraiz:vectoraiz alembic/ ./alembic/

# Copy entrypoint script
COPY --chown=root:root entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# Ensure non-root user owns the app directory
RUN chown -R vectoraiz:vectoraiz /app

USER vectoraiz

EXPOSE 8000

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 \
    CMD curl -f http://localhost:${PORT:-8000}/api/health || exit 1

# Production entrypoint — creates /data dirs, runs uvicorn
# Alembic migrations run automatically on startup via app.core.database.init_db()
CMD ["/entrypoint.sh"]
