#!/bin/sh
# =============================================================================
# vectorAIz Installer
# =============================================================================
# Usage: curl -fsSL https://raw.githubusercontent.com/maxrobbins/vectoraiz/main/install.sh | sh
#
# Works on stock macOS (zsh) and Linux.
# Only requires: curl (pre-installed on macOS and most Linux)
# Installs Docker automatically if missing.
# =============================================================================

set -e

# --- Colors (POSIX-safe) ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

info()    { printf "  ${CYAN}▸${NC} %s\n" "$1"; }
success() { printf "  ${GREEN}✓${NC} %s\n" "$1"; }
warn()    { printf "  ${YELLOW}⚠${NC} %s\n" "$1"; }
fail()    { printf "\n  ${RED}${BOLD}ERROR:${NC} %s\n\n" "$1"; exit 1; }

REPO="maxrobbins/vectoraiz"
BRANCH="main"
INSTALL_DIR="$HOME/vectoraiz"

printf "\n"
printf "  ${CYAN}${BOLD}⚡ vectorAIz Installer${NC}\n"
printf "\n"

# ─── Check curl ──────────────────────────────────────────────────
if ! command -v curl >/dev/null 2>&1; then
    fail "curl is required but not found. Install it with your package manager."
fi

# ─── Detect OS ───────────────────────────────────────────────────
OS="unknown"
case "$(uname -s)" in
    Darwin*) OS="macos" ;;
    Linux*)  OS="linux" ;;
esac

# ─── Install Docker if missing ───────────────────────────────────
install_docker_macos() {
    ARCH="$(uname -m)"
    if [ "$ARCH" = "arm64" ]; then
        DMG_URL="https://desktop.docker.com/mac/main/arm64/Docker.dmg"
        info "Detected Apple Silicon Mac"
    else
        DMG_URL="https://desktop.docker.com/mac/main/amd64/Docker.dmg"
        info "Detected Intel Mac"
    fi

    info "Downloading Docker Desktop..."
    DMG_PATH="/tmp/Docker.dmg"
    curl -fSL "$DMG_URL" -o "$DMG_PATH" || fail "Failed to download Docker Desktop."
    success "Downloaded Docker Desktop"

    info "Installing Docker Desktop (you may be prompted for your password)..."
    hdiutil attach "$DMG_PATH" -quiet -nobrowse -mountpoint /tmp/docker-mount || fail "Failed to mount Docker DMG."
    sudo cp -R /tmp/docker-mount/Docker.app /Applications/ 2>/dev/null || cp -R /tmp/docker-mount/Docker.app /Applications/
    hdiutil detach /tmp/docker-mount -quiet 2>/dev/null
    rm -f "$DMG_PATH"
    success "Docker Desktop installed"

    info "Starting Docker Desktop..."
    open /Applications/Docker.app

    printf "\n"
    printf "  ${CYAN}${BOLD}Docker is starting up...${NC}\n"
    printf "  ${DIM}This takes 30-60 seconds on first launch.${NC}\n"
    printf "\n"

    # Wait for Docker to be ready (up to 120s)
    WAITED=0
    MAX_WAIT=120
    while [ $WAITED -lt $MAX_WAIT ]; do
        if docker info >/dev/null 2>&1; then
            success "Docker is ready"
            return 0
        fi
        sleep 3
        WAITED=$((WAITED + 3))
        printf "\r  ${CYAN}⏳${NC} Waiting for Docker to start... (%ds)" "$WAITED"
    done
    printf "\n"

    # If we get here, Docker didn't start in time
    printf "\n"
    warn "Docker is still starting. Please wait for the Docker icon in your menu bar,"
    warn "then re-run this installer:"
    printf "\n"
    printf "    curl -fsSL https://raw.githubusercontent.com/maxrobbins/vectoraiz/main/install.sh | sh\n"
    printf "\n"
    exit 0
}

install_docker_linux() {
    info "Installing Docker via official script..."
    printf "  ${DIM}(You may be prompted for your sudo password)${NC}\n"
    printf "\n"

    curl -fsSL https://get.docker.com | sudo sh || fail "Docker installation failed."

    # Add current user to docker group
    if command -v usermod >/dev/null 2>&1; then
        sudo usermod -aG docker "$USER" 2>/dev/null || true
    fi

    # Start Docker
    if command -v systemctl >/dev/null 2>&1; then
        sudo systemctl start docker 2>/dev/null || true
        sudo systemctl enable docker 2>/dev/null || true
    fi

    success "Docker installed"

    # Check if it works without sudo
    if ! docker info >/dev/null 2>&1; then
        printf "\n"
        warn "Docker was installed, but you need to log out and back in"
        warn "for group permissions to take effect. Then re-run:"
        printf "\n"
        printf "    curl -fsSL https://raw.githubusercontent.com/maxrobbins/vectoraiz/main/install.sh | sh\n"
        printf "\n"
        exit 0
    fi
}

if ! command -v docker >/dev/null 2>&1; then
    printf "\n"
    info "Docker is not installed. vectorAIz needs Docker to run."
    printf "\n"

    # Prompt user
    printf "  Install Docker now? [Y/n] "
    read -r REPLY < /dev/tty 2>/dev/null || REPLY="y"
    REPLY="${REPLY:-y}"

    case "$REPLY" in
        [Nn]*)
            printf "\n"
            info "You can install Docker manually:"
            if [ "$OS" = "macos" ]; then
                printf "    https://orbstack.dev  ${DIM}(recommended for Mac)${NC}\n"
                printf "    https://docker.com/get-started\n"
            else
                printf "    https://docs.docker.com/engine/install/\n"
            fi
            printf "\n"
            info "Then re-run this installer."
            printf "\n"
            exit 0
            ;;
    esac

    if [ "$OS" = "macos" ]; then
        install_docker_macos
    elif [ "$OS" = "linux" ]; then
        install_docker_linux
    else
        fail "Unsupported OS. Please install Docker manually: https://docs.docker.com/get-started/"
    fi
elif ! docker info >/dev/null 2>&1; then
    # Docker installed but not running
    if [ "$OS" = "macos" ]; then
        info "Docker is installed but not running. Starting it..."
        open /Applications/Docker.app 2>/dev/null || open -a OrbStack 2>/dev/null || true

        WAITED=0
        while [ $WAITED -lt 90 ]; do
            if docker info >/dev/null 2>&1; then
                success "Docker is running"
                break
            fi
            sleep 3
            WAITED=$((WAITED + 3))
            printf "\r  ${CYAN}⏳${NC} Waiting for Docker... (%ds)" "$WAITED"
        done
        printf "\n"

        if ! docker info >/dev/null 2>&1; then
            fail "Docker didn't start. Please start Docker Desktop manually and re-run."
        fi
    else
        fail "Docker is installed but not running. Start it with: sudo systemctl start docker"
    fi
fi

success "Docker is ready"

# ─── Check unzip (install if missing) ────────────────────────────
if ! command -v unzip >/dev/null 2>&1; then
    info "Installing unzip..."
    if command -v apt-get >/dev/null 2>&1; then
        sudo apt-get install -y unzip >/dev/null 2>&1
    elif command -v yum >/dev/null 2>&1; then
        sudo yum install -y unzip >/dev/null 2>&1
    elif command -v dnf >/dev/null 2>&1; then
        sudo dnf install -y unzip >/dev/null 2>&1
    else
        fail "unzip is required. Install it with your package manager."
    fi
    success "unzip installed"
fi

# ─── Download ────────────────────────────────────────────────────
if [ -d "$INSTALL_DIR/backend" ]; then
    info "Existing installation found — updating..."
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

EXTRACTED_DIR="$TMPDIR_DL/vectoraiz-${BRANCH}"
if [ ! -d "$EXTRACTED_DIR" ]; then
    EXTRACTED_DIR=$(find "$TMPDIR_DL" -maxdepth 1 -type d -name "vectoraiz*" | head -1)
fi
if [ -z "$EXTRACTED_DIR" ] || [ ! -d "$EXTRACTED_DIR" ]; then
    fail "Failed to extract archive."
fi

# Preserve .env if updating
if [ "$EXISTING" = true ] && [ -f "$INSTALL_DIR/backend/.env" ]; then
    cp "$INSTALL_DIR/backend/.env" "$TMPDIR_DL/.env.backup"
fi

mkdir -p "$INSTALL_DIR"
cp -R "$EXTRACTED_DIR/"* "$INSTALL_DIR/"

if [ -f "$TMPDIR_DL/.env.backup" ]; then
    cp "$TMPDIR_DL/.env.backup" "$INSTALL_DIR/backend/.env"
    success "Preserved existing configuration"
fi

rm -r "$TMPDIR_DL"
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
