#!/bin/bash
# =============================================================================
# vectorAIz — Linux Uninstaller
# =============================================================================

set -e

# --- Configuration ---
INSTALL_DIR="$HOME/vectoraiz"
COMPOSE_FILE="docker-compose.customer.yml"

# --- Colors ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

fail() {
    echo -e "\n  ${RED}${BOLD}ERROR:${NC} $1\n"
    exit 1
}

info() {
    echo -e "  ${BLUE}▸${NC} $1"
}

success() {
    echo -e "  ${GREEN}✓${NC} $1"
}

warn() {
    echo -e "  ${YELLOW}⚠${NC} $1"
}

echo ""
echo -e "${CYAN}${BOLD}"
echo "  ╔═══════════════════════════════════════════╗"
echo "  ║                                           ║"
echo "  ║     vectorAIz — Linux Uninstaller         ║"
echo "  ║                                           ║"
echo "  ╚═══════════════════════════════════════════╝"
echo -e "${NC}"

# ─── Step 1: Stop containers ─────────────────────────────────────
if [ -f "$INSTALL_DIR/$COMPOSE_FILE" ] && command -v docker &>/dev/null && docker info &>/dev/null 2>&1; then
    info "Stopping vectorAIz containers..."
    cd "$INSTALL_DIR"
    docker compose -f "$COMPOSE_FILE" down 2>/dev/null || true
    success "Containers stopped"
else
    info "No running containers found (skipping)"
fi

# ─── Step 2: Ask about volumes ───────────────────────────────────
echo ""
echo -e "  ${YELLOW}${BOLD}WARNING:${NC} Removing Docker volumes will permanently delete all your data"
echo -e "  ${DIM}(uploaded files, databases, vector indices, user accounts)${NC}"
echo ""
read -rp "  Delete all data volumes? [y/N]: " DELETE_VOLUMES
echo ""

if [[ "$DELETE_VOLUMES" =~ ^[Yy]$ ]]; then
    if command -v docker &>/dev/null && docker info &>/dev/null 2>&1 && [ -f "$INSTALL_DIR/$COMPOSE_FILE" ]; then
        info "Removing Docker volumes..."
        cd "$INSTALL_DIR" && docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
        success "Docker volumes removed"
    fi
else
    info "Keeping Docker volumes (your data is preserved)"
    echo -e "  ${DIM}To remove volumes later: docker volume rm vectoraiz_vectoraiz-data vectoraiz_postgres-data vectoraiz_qdrant-storage${NC}"
fi

# ─── Step 3: Remove systemd service ──────────────────────────────
SYSTEMD_SERVICE="$HOME/.config/systemd/user/vectoraiz.service"
if [ -f "$SYSTEMD_SERVICE" ]; then
    info "Removing systemd service..."
    systemctl --user disable vectoraiz.service 2>/dev/null || true
    systemctl --user stop vectoraiz.service 2>/dev/null || true
    rm -f "$SYSTEMD_SERVICE"
    systemctl --user daemon-reload 2>/dev/null || true
    success "Removed systemd service"
fi

# ─── Step 4: Remove desktop entries ──────────────────────────────
DESKTOP_ENTRY="$HOME/.local/share/applications/vectoraiz.desktop"
if [ -f "$DESKTOP_ENTRY" ]; then
    rm -f "$DESKTOP_ENTRY"
    success "Removed application menu entry"
fi

if [ -f "$HOME/Desktop/vectoraiz.desktop" ]; then
    rm -f "$HOME/Desktop/vectoraiz.desktop"
    success "Removed desktop shortcut"
fi

# Update desktop database
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
fi

# ─── Step 5: Remove install directory ─────────────────────────────
if [ -d "$INSTALL_DIR" ]; then
    info "Removing install directory..."
    rm -rf "$INSTALL_DIR"
    success "Removed $INSTALL_DIR"
else
    info "Install directory not found (skipping)"
fi

echo ""
echo -e "  ${GREEN}${BOLD}vectorAIz has been uninstalled.${NC}"
echo ""
if [[ ! "$DELETE_VOLUMES" =~ ^[Yy]$ ]]; then
    echo -e "  ${DIM}Note: Docker volumes were preserved. To fully clean up:${NC}"
    echo -e "  ${DIM}docker volume rm vectoraiz_vectoraiz-data vectoraiz_postgres-data vectoraiz_qdrant-storage${NC}"
    echo ""
fi
