#!/bin/bash
# =============================================================================
# vectorAIz Customer Image Verification Script
# =============================================================================
# Run after EVERY customer image build to catch regressions before push.
# This is the mechanical enforcement — no build is "done" until this passes.
#
# Usage:
#   ./scripts/verify-customer-image.sh [image_tag]              # standalone
#   ./scripts/verify-customer-image.sh [image_tag] --compose    # CI (compose stack)
#
# Standalone: spins up ephemeral container on port 18080
# Compose:    tests against already-running compose stack on port 8080
# =============================================================================

set -euo pipefail

TAG="${1:-latest}"
MODE="${2:-}"
IMAGE="ghcr.io/aidotmarket/vectoraiz:${TAG}"
PASS=0
FAIL=0
WARN=0

log_pass() { echo "  ✅ $1"; PASS=$((PASS+1)); }
log_fail() { echo "  ❌ $1"; FAIL=$((FAIL+1)); }
log_warn() { echo "  ⚠️  $1"; WARN=$((WARN+1)); }

echo "============================================"
echo " vectorAIz Customer Image Verification"
echo " Image: ${IMAGE}"
echo " Mode:  ${MODE:-standalone}"
echo " Date:  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "============================================"
echo ""

if [ "$MODE" = "--compose" ]; then
  # ── Compose mode: test against running stack on port 8080 ──
  TEST_PORT=8080
  CONTAINER=""
  cleanup() { true; }
  trap cleanup EXIT

  echo "0. Compose stack reachable:"
  if curl -sf "http://localhost:${TEST_PORT}/api/health" >/dev/null 2>&1; then
    log_pass "Compose stack healthy on port ${TEST_PORT}"
  else
    log_fail "Compose stack NOT reachable on port ${TEST_PORT}"
    echo ""; echo "RESULT: FAIL (0 pass, 1 fail)"; exit 1
  fi
else
  # ── Standalone mode: ephemeral container on port 18080 ──
  TEST_PORT=18080
  CONTAINER="vz-verify-$$"

  echo "0. Image exists:"
  if docker image inspect "${IMAGE}" &>/dev/null; then
    log_pass "Image ${IMAGE} found"
  else
    log_fail "Image ${IMAGE} NOT FOUND — build first"
    echo ""; echo "RESULT: FAIL (0 pass, 1 fail)"; exit 1
  fi

  echo ""
  echo "Starting verification container..."
  docker run -d --name "${CONTAINER}" -p ${TEST_PORT}:80 "${IMAGE}" >/dev/null 2>&1
  sleep 8

  cleanup() { docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true; }
  trap cleanup EXIT
fi

# === CHECK 1: Correct Dockerfile (nginx present) ===
echo ""
echo "1. Built from Dockerfile.customer (not Dockerfile):"
if [ -n "$CONTAINER" ]; then
  if docker exec "${CONTAINER}" which nginx &>/dev/null; then
    log_pass "nginx binary present"
  else
    log_fail "nginx MISSING — built from wrong Dockerfile!"
  fi

  if docker exec "${CONTAINER}" test -d /usr/share/nginx/html/assets; then
    log_pass "Frontend static files present"
  else
    log_fail "Frontend assets MISSING — wrong Dockerfile or build failed"
  fi
else
  log_warn "Skipping container-level checks in compose mode"
fi

# === CHECK 2: API health ===
echo ""
echo "2. API health:"
HEALTH=$(curl -sf "http://localhost:${TEST_PORT}/api/health" 2>/dev/null || echo '{}')
VERSION=$(echo "$HEALTH" | python3 -c "import sys,json; print(json.load(sys.stdin).get('version','MISSING'))" 2>/dev/null || echo "MISSING")
STATUS=$(echo "$HEALTH" | python3 -c "import sys,json; print(json.load(sys.stdin).get('status','MISSING'))" 2>/dev/null || echo "MISSING")

if [ "$STATUS" = "ok" ]; then log_pass "Health status: ok"; else log_fail "Health status: ${STATUS}"; fi
if [ "$VERSION" != "MISSING" ] && [ "$VERSION" != "0.0.0" ]; then
  log_pass "Version reported: ${VERSION}"
else
  log_fail "Version MISSING or 0.0.0 from /api/health"
fi

# === CHECK 3: Frontend serves HTML ===
echo ""
echo "3. Frontend:"
HTTP_CODE=$(curl -sf -o /dev/null -w "%{http_code}" "http://localhost:${TEST_PORT}/" 2>/dev/null || echo "000")
if [ "$HTTP_CODE" = "200" ]; then log_pass "index.html serves (HTTP 200)"; else log_fail "index.html HTTP ${HTTP_CODE}"; fi

LOGIN_CODE=$(curl -sf -o /dev/null -w "%{http_code}" "http://localhost:${TEST_PORT}/login" 2>/dev/null || echo "000")
if [ "$LOGIN_CODE" = "200" ]; then log_pass "/login route serves (SPA fallback works)"; else log_fail "/login HTTP ${LOGIN_CODE}"; fi

# === CHECK 4: Cache-Control headers ===
echo ""
echo "4. Cache headers:"
HTML_CC=$(curl -sf -I "http://localhost:${TEST_PORT}/" 2>/dev/null | grep -i "cache-control" | tr -d '\r')
if echo "$HTML_CC" | grep -qi "no-cache"; then
  log_pass "HTML: Cache-Control no-cache"
else
  log_warn "HTML: Missing no-cache (got: '${HTML_CC:-none}')"
fi

# === SUMMARY ===
echo ""
echo "============================================"
echo " RESULT: ${PASS} pass, ${FAIL} fail, ${WARN} warn"
echo "============================================"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
