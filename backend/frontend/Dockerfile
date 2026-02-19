# vectorAIz Frontend — Railway Deployment Dockerfile
# Build context: repo root (Railway sets this when using RAILWAY_DOCKERFILE_PATH)
# VITE_API_URL is injected at build time by Railway from env vars.

FROM node:22-slim AS builder
WORKDIR /app

# Copy package files from frontend/ subdirectory
COPY frontend/package.json frontend/package-lock.json ./

# Install dependencies
RUN npm install

# Copy frontend source
COPY frontend/ .

# Vite needs env vars at build time
ARG VITE_API_URL
ENV VITE_API_URL=$VITE_API_URL

# Build
RUN npm run build

# --- Runtime ---
FROM node:22-slim
RUN npm install -g serve@14
WORKDIR /app
COPY --from=builder /app/dist ./dist
EXPOSE ${PORT:-3000}
CMD serve dist -s -l ${PORT:-3000}
