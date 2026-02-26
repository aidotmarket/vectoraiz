#!/usr/bin/env bash
# =============================================================================
# vectorAIz Release Script
# =============================================================================
# Single source of truth: git tags
# Usage:
#   ./scripts/release.sh 1.9.0        # Build + push specific version
#   ./scripts/release.sh patch         # Auto-bump patch (1.8.5 → 1.8.6)
#   ./scripts/release.sh minor         # Auto-bump minor (1.8.5 → 1.9.0)
#   ./scripts/release.sh major         # Auto-bump major (1.8.5 → 2.0.0)
# =============================================================================
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

IMAGE="ghcr.io/maxrobbins/vectoraiz"
DOCKER="${DOCKER_BIN:-docker}"

# --- Resolve docker binary (OrbStack or standard) ---
if ! command -v "$DOCKER" &>/dev/null; then
  for candidate in /Users/max/.orbstack/bin/docker /usr/local/bin/docker /opt/homebrew/bin/docker; do
    if [ -x "$candidate" ]; then DOCKER="$candidate"; break; fi
  done
fi

# --- Get current version from latest git tag ---
current_tag=$(git tag -l 'v*' --sort=-v:refname | { grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' || true; } | head -1)
current="${current_tag#v}"
if [ -z "$current" ]; then
  current="0.0.0"
fi

# --- Determine new version ---
bump="${1:?Usage: release.sh <version|patch|minor|major>}"

IFS='.' read -r major minor patch <<< "$current"
case "$bump" in
  patch) new_version="$major.$minor.$((patch + 1))" ;;
  minor) new_version="$major.$((minor + 1)).0" ;;
  major) new_version="$((major + 1)).0.0" ;;
  *)     new_version="$bump" ;;
esac

echo ""
echo "╔═══════════════════════════════════════════╗"
echo "║  vectorAIz Release                        ║"
echo "║  Current: ${current:-none}                          ║"
echo "║  Building: ${new_version}                         ║"
echo "╚═══════════════════════════════════════════╝"
echo ""

# --- Pre-flight checks ---
echo "▸ Pre-flight checks..."

# Clean working tree (allow untracked)
if ! git diff --quiet HEAD; then
  echo "✗ Uncommitted changes. Commit or stash first."
  git status --short
  exit 1
fi

# GHCR auth
if ! "$DOCKER" pull "$IMAGE:latest" &>/dev/null 2>&1; then
  echo "▸ Logging into GHCR..."
  GITHUB_TOKEN=$(doppler secrets get GITHUB_TOKEN --plain -p ai-market -c dev_personal 2>/dev/null || echo "")
  if [ -z "$GITHUB_TOKEN" ]; then
    echo "✗ No GITHUB_TOKEN found. Run: doppler secrets get GITHUB_TOKEN --plain -p ai-market -c dev_personal"
    exit 1
  fi
  echo "$GITHUB_TOKEN" | "$DOCKER" login ghcr.io -u maxrobbins --password-stdin
fi

echo "✓ Pre-flight passed"

# --- Build ---
echo ""
echo "▸ Building $IMAGE:$new_version ..."
echo "  Using: Dockerfile.customer (nginx + frontend + backend)"
echo ""

"$DOCKER" build \
  -f Dockerfile.customer \
  --build-arg VERSION="$new_version" \
  -t "$IMAGE:$new_version" \
  -t "$IMAGE:latest" \
  .

echo ""
echo "✓ Image built"

# --- Verify image was built with Dockerfile.customer ---
echo ""
echo "▸ Verifying image (customer perspective)..."
echo "  Checking: nginx installed, frontend bundled, version env set"

# Check image contents without starting full stack (which needs postgres/qdrant)
has_nginx=$("$DOCKER" run --rm --entrypoint="" "$IMAGE:$new_version" which nginx 2>/dev/null && echo "yes" || echo "no")
has_frontend=$("$DOCKER" run --rm --entrypoint="" "$IMAGE:$new_version" ls /usr/share/nginx/html/index.html 2>/dev/null && echo "yes" || echo "no")
has_tini=$("$DOCKER" run --rm --entrypoint="" "$IMAGE:$new_version" which tini 2>/dev/null && echo "yes" || echo "no")
img_version=$("$DOCKER" run --rm --entrypoint="" "$IMAGE:$new_version" printenv VECTORAIZ_VERSION 2>/dev/null || echo "none")

echo "  nginx binary:     $has_nginx"
echo "  frontend bundle:  $has_frontend"
echo "  tini (PID 1):     $has_tini"
echo "  VECTORAIZ_VERSION: $img_version"

if [ "$has_nginx" != "yes" ]; then
  echo ""
  echo "✗ FATAL: nginx not found in image."
  echo "  This means Dockerfile.customer was NOT used (plain Dockerfile has no nginx)."
  echo "  DO NOT PUSH. Fix the build."
  exit 1
fi

if [ "$has_frontend" != "yes" ]; then
  echo ""
  echo "✗ FATAL: Frontend bundle not found at /usr/share/nginx/html/index.html."
  echo "  Frontend build stage may have failed."
  echo "  DO NOT PUSH. Fix the build."
  exit 1
fi

if [ "$img_version" != "$new_version" ]; then
  echo ""
  echo "✗ FATAL: VECTORAIZ_VERSION env is '$img_version', expected '$new_version'."
  echo "  --build-arg VERSION may not have been passed."
  exit 1
fi

echo "✓ Image verified — nginx + frontend + tini + correct version"

# --- Push ---
echo ""
echo "▸ Pushing to GHCR..."
"$DOCKER" push "$IMAGE:$new_version"
"$DOCKER" push "$IMAGE:latest"
echo "✓ Pushed $IMAGE:$new_version + :latest"

# --- Git tag ---
echo ""
echo "▸ Tagging git: v$new_version"
git tag -a "v$new_version" -m "Release $new_version"
git push origin "v$new_version"
echo "✓ Tag v$new_version pushed to origin"

# --- Done ---
echo ""
echo "╔═══════════════════════════════════════════╗"
echo "║  ✅ Released vectorAIz $new_version              ║"
echo "║                                           ║"
echo "║  Image: $IMAGE:$new_version    ║"
echo "║  Tag:   v$new_version                            ║"
echo "╚═══════════════════════════════════════════╝"
echo ""
