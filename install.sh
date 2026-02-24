#!/bin/sh
# =============================================================================
# vectorAIz Installer
# =============================================================================
# Usage: curl -fsSL https://raw.githubusercontent.com/maxrobbins/vectoraiz/main/install.sh | sh
#
# Works on stock macOS (zsh), Linux, and Windows (WSL).
# Only requires: curl, unzip, Docker
# =============================================================================

set -e

# --- Colors (POSIX-safe) ---
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()    { printf "  ${CYAN}▸${NC} %s\n" "$1"; }
success() { printf "  ${GREEN}✓${NC} %s\n" "$1"; }
fail()    { printf "\n  ${RED}${BOLD}ERROR:${NC} %s\n\n" "$1"; exit 1; }

REPO="maxrobbins/vectoraiz"
BRANCH="main"
INSTALL_DIR="$HOME/vectoraiz"

printf "\n"
printf "  ${CYAN}${BOLD}⚡ vectorAIz Installer${NC}\n"
printf "\n"

# ─── Check prerequisites ────────────────────────────────────────
info "Checking prerequisites..."

if ! command -v curl >/dev/null 2>&1; then
    fail "curl is required but not found."
fi

if ! command -v unzip >/dev/null 2>&1; then
    fail "unzip is required but not found."
fi

if ! command -v docker >/dev/null 2>&1; then
    printf "\n"
    printf "  ${RED}${BOLD}Docker is not installed.${NC}\n"
    printf "\n"
    printf "  vectorAIz runs in Docker containers. Please install one of:\n"
    printf "\n"
    printf "    ${BOLD}macOS:${NC}    https://orbstack.dev  (recommended)\n"
    printf "              https://docker.com/get-started\n"
    printf "\n"
    printf "    ${BOLD}Linux:${NC}    https://docs.docker.com/engine/install/\n"
    printf "\n"
    printf "    ${BOLD}Windows:${NC}  https://docs.docker.com/desktop/install/windows/\n"
    printf "              (use WSL2 backend)\n"
    printf "\n"
    printf "  After installing Docker, run this installer again.\n"
    printf "\n"
    exit 1
fi

if ! docker info >/dev/null 2>&1; then
    fail "Docker is installed but not running. Please start Docker Desktop or OrbStack first."
fi

success "All prerequisites met (curl, unzip, Docker)"

# ─── Download ────────────────────────────────────────────────────
if [ -d "$INSTALL_DIR/backend" ]; then
    info "Existing installation found at $INSTALL_DIR"
    info "Updating..."
    EXISTING=true
else
    EXISTING=false
fi

info "Downloading vectorAIz..."
TMPDIR_DL=$(mktemp -d)
ZIPFILE="$TMPDIR_DL/vectoraiz.zip"

curl -fsSL "https://github.com/${REPO}/archive/refs/heads/${BRANCH}.zip" -o "$ZIPFILE" \
    || fail "Failed to download. Check your internet connection."

success "Downloaded"

# ─── Extract ─────────────────────────────────────────────────────
info "Extracting..."

unzip -qo "$ZIPFILE" -d "$TMPDIR_DL"

# GitHub zips extract to repo-branch/ directory
EXTRACTED_DIR="$TMPDIR_DL/vectoraiz-${BRANCH}"

if [ ! -d "$EXTRACTED_DIR" ]; then
    # Try alternative naming
    EXTRACTED_DIR=$(find "$TMPDIR_DL" -maxdepth 1 -type d -name "vectoraiz*" | head -1)
fi

if [ -z "$EXTRACTED_DIR" ] || [ ! -d "$EXTRACTED_DIR" ]; then
    fail "Failed to extract archive."
fi

# Move to install dir (preserve .env if exists)
if [ "$EXISTING" = true ] && [ -f "$INSTALL_DIR/backend/.env" ]; then
    cp "$INSTALL_DIR/backend/.env" "$TMPDIR_DL/.env.backup"
fi

mkdir -p "$INSTALL_DIR"
cp -R "$EXTRACTED_DIR/"* "$INSTALL_DIR/"

if [ -f "$TMPDIR_DL/.env.backup" ]; then
    cp "$TMPDIR_DL/.env.backup" "$INSTALL_DIR/backend/.env"
    success "Preserved existing configuration"
fi

# Cleanup
rm -rf "$TMPDIR_DL"

success "Installed to $INSTALL_DIR"

# ─── Make scripts executable ─────────────────────────────────────
chmod +x "$INSTALL_DIR/backend/start.sh" "$INSTALL_DIR/backend/stop.sh" 2>/dev/null
chmod +x "$INSTALL_DIR/start.sh" "$INSTALL_DIR/stop.sh" 2>/dev/null

# ─── Launch ──────────────────────────────────────────────────────
printf "\n"
info "Starting vectorAIz..."
printf "\n"

cd "$INSTALL_DIR/backend"
exec ./start.sh
