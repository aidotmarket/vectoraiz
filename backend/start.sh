#!/bin/bash
# =============================================================================
# vectorAIz — One-Click Setup
# =============================================================================
# Usage: ./start.sh
#
# What this does:
#   1. Checks Docker is installed and running
#   2. Checks for port conflicts and finds a free port
#   3. Generates secrets if first run
#   4. Configures Allie AI assistant (if API key provided)
#   5. Builds and starts all containers
#   6. Waits for the app to be healthy
#   7. Creates a desktop shortcut
#   8. Opens your browser
# =============================================================================

set -e

# --- Configuration ---
COMPOSE_FILE="docker-compose.customer.yml"
APP_NAME="vectorAIz"
SHORTCUT_NAME="vectorAIz"
PREFERRED_PORTS=(80 8080 3000 8888 9000)
AI_MARKET_URL="https://ai-market-backend-production.up.railway.app"

# --- Colors ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# --- Helpers ---
print_banner() {
    echo ""
    echo -e "${CYAN}${BOLD}"
    echo "  ╔═══════════════════════════════════════════╗"
    echo "  ║                                           ║"
    echo "  ║           ⚡ vectorAIz Setup ⚡           ║"
    echo "  ║                                           ║"
    echo "  ║   Self-hosted data processing & search    ║"
    echo "  ║                                           ║"
    echo "  ╚═══════════════════════════════════════════╝"
    echo -e "${NC}"
}

print_ready() {
    local allie_status="$1"
    echo ""
    echo -e "${GREEN}${BOLD}"
    echo "  ╔═══════════════════════════════════════════╗"
    echo "  ║                                           ║"
    echo "  ║          ✅ vectorAIz is Ready!           ║"
    echo "  ║                                           ║"
    echo "  ║   Open your browser to:                   ║"
    echo "  ║                                           ║"
    echo -e "  ║   ${BOLD}${CYAN}➜  ${URL} $(printf '%*s' $((25 - ${#URL})) '')${GREEN}║"
    echo "  ║                                           ║"
    if [ "$allie_status" = "enabled" ]; then
    echo -e "  ║   ${CYAN}🤖 Allie AI assistant: ON${GREEN}              ║"
    else
    echo -e "  ║   ${DIM}🤖 Allie AI assistant: OFF${GREEN}             ║"
    fi
    echo "  ║                                           ║"
    echo "  ║   To stop: ./stop.sh                      ║"
    echo "  ║                                           ║"
    echo "  ╚═══════════════════════════════════════════╝"
    echo -e "${NC}"
}

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

is_port_free() {
    local port=$1
    if command -v lsof &>/dev/null; then
        ! lsof -i :"$port" -sTCP:LISTEN &>/dev/null
    elif command -v ss &>/dev/null; then
        ! ss -tlnp | grep -q ":${port} "
    elif command -v netstat &>/dev/null; then
        ! netstat -tlnp 2>/dev/null | grep -q ":${port} "
    else
        ! (echo >/dev/tcp/127.0.0.1/"$port") 2>/dev/null
    fi
}

get_port_process() {
    local port=$1
    if command -v lsof &>/dev/null; then
        lsof -i :"$port" -sTCP:LISTEN -t 2>/dev/null | head -1 | xargs -I{} ps -p {} -o comm= 2>/dev/null
    fi
}

make_url() {
    local port=$1
    if [ "$port" = "80" ]; then
        echo "http://localhost"
    else
        echo "http://localhost:${port}"
    fi
}

# --- Main ---
print_banner

# ─── Step 1: Check Docker ───────────────────────────────────────
info "Checking Docker..."
if ! command -v docker &>/dev/null; then
    fail "Docker is not installed.\n\n  Install Docker Desktop: https://docker.com/get-started\n  Or OrbStack (recommended for Mac): https://orbstack.dev"
fi

if ! docker info &>/dev/null 2>&1; then
    fail "Docker is not running. Please start Docker Desktop or OrbStack first."
fi
success "Docker is running"

# ─── Step 2: Find compose file ──────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [ ! -f "$COMPOSE_FILE" ]; then
    fail "Cannot find $COMPOSE_FILE in $(pwd)"
fi

# ─── Step 3: Port detection ─────────────────────────────────────
info "Checking for available port..."

if [ -n "$VECTORAIZ_PORT" ]; then
    PORT="$VECTORAIZ_PORT"
elif [ -f ".env" ] && grep -q "^VECTORAIZ_PORT=" .env 2>/dev/null; then
    PORT=$(grep "^VECTORAIZ_PORT=" .env | cut -d'=' -f2 | tr -d ' "'"'"'')
fi

if [ -n "$PORT" ]; then
    if is_port_free "$PORT"; then
        success "Port $PORT is available"
    else
        OCCUPANT=$(get_port_process "$PORT")
        warn "Port $PORT is in use${OCCUPANT:+ by $OCCUPANT}"
        echo ""
        echo -e "    ${BOLD}1)${NC} Pick a free port automatically"
        echo -e "    ${BOLD}2)${NC} Enter a specific port"
        echo -e "    ${BOLD}3)${NC} Abort"
        echo ""
        read -rp "  Choice [1/2/3]: " CHOICE
        case "$CHOICE" in
            1) PORT="" ;;
            2)
                read -rp "  Enter port number: " PORT
                if ! is_port_free "$PORT"; then
                    fail "Port $PORT is also in use."
                fi
                success "Port $PORT is available"
                ;;
            *) echo ""; info "Run again after freeing the port."; exit 0 ;;
        esac
    fi
fi

if [ -z "$PORT" ]; then
    for TRY_PORT in "${PREFERRED_PORTS[@]}"; do
        if is_port_free "$TRY_PORT"; then
            PORT="$TRY_PORT"
            break
        fi
    done
    if [ -z "$PORT" ]; then
        PORT=$(python3 -c "import socket; s=socket.socket(); s.bind(('',0)); print(s.getsockname()[1]); s.close()" 2>/dev/null || echo "8080")
    fi
    if [ "$PORT" != "80" ]; then
        info "Port 80 is in use — using port ${PORT} instead"
    fi
    success "Using port $PORT"
fi

URL=$(make_url "$PORT")
export VECTORAIZ_PORT="$PORT"

# ─── Step 4: Generate .env ──────────────────────────────────────
FIRST_RUN=false

if [ ! -f ".env" ]; then
    FIRST_RUN=true
    info "First run detected — generating configuration..."
    POSTGRES_PW=$(openssl rand -hex 16)
    cat > .env <<EOF
# vectorAIz Configuration
# Generated on $(date -u +"%Y-%m-%d %H:%M:%S UTC")

# Database password (auto-generated, keep this safe)
POSTGRES_PASSWORD=${POSTGRES_PW}

# Port to serve on
VECTORAIZ_PORT=${PORT}

# Mode: standalone (default) or connected (with Allie AI)
VECTORAIZ_MODE=standalone
EOF
    success "Generated .env with secure defaults"
else
    if grep -q "^VECTORAIZ_PORT=" .env 2>/dev/null; then
        sed -i.bak "s/^VECTORAIZ_PORT=.*/VECTORAIZ_PORT=${PORT}/" .env && rm -f .env.bak
    else
        echo "VECTORAIZ_PORT=${PORT}" >> .env
    fi
    success "Using existing .env (port: ${PORT})"
fi

# ─── Step 5: Allie AI Setup ─────────────────────────────────────
ALLIE_ENABLED=false

# Check if already configured
if grep -q "^VECTORAIZ_INTERNAL_API_KEY=aim_" .env 2>/dev/null; then
    ALLIE_ENABLED=true
    success "Allie AI assistant: already configured"
elif [ "$FIRST_RUN" = true ]; then
    # First run — ask about Allie
    echo ""
    echo -e "  ${CYAN}${BOLD}─── Allie AI Assistant ────────────────────${NC}"
    echo ""
    echo -e "  Allie is your AI-powered data assistant. She can help"
    echo -e "  you understand, clean, and optimize your datasets."
    echo ""
    echo -e "  To enable Allie, you need an API key from ${BOLD}ai.market${NC}."
    echo -e "  ${DIM}(Your beta invitation email includes this key)${NC}"
    echo ""
    read -rp "  Enter your API key (or press Enter to skip): " API_KEY
    
    if [ -n "$API_KEY" ]; then
        # Validate key format
        if [[ "$API_KEY" == aim_* ]]; then
            # Write Allie config to .env
            cat >> .env <<EOF

# Allie AI Assistant (connected mode)
VECTORAIZ_MODE=connected
VECTORAIZ_AI_MARKET_URL=${AI_MARKET_URL}
VECTORAIZ_ALLIE_PROVIDER=aimarket
VECTORAIZ_INTERNAL_API_KEY=${API_KEY}
EOF
            ALLIE_ENABLED=true
            success "Allie enabled! She'll be ready when vectorAIz starts."
        else
            warn "API key should start with 'aim_' — skipping Allie setup."
            echo -e "  ${DIM}You can enable Allie later by re-running ./start.sh --setup-allie${NC}"
        fi
    else
        info "Skipping Allie — running in standalone mode."
        echo -e "  ${DIM}You can enable Allie later by re-running ./start.sh --setup-allie${NC}"
    fi
    echo ""
fi

# Handle --setup-allie flag for re-runs
if [[ "$1" == "--setup-allie" ]] && [ "$ALLIE_ENABLED" = false ]; then
    echo ""
    echo -e "  ${CYAN}${BOLD}─── Allie AI Setup ───────────────────────${NC}"
    echo ""
    read -rp "  Enter your API key: " API_KEY
    
    if [ -n "$API_KEY" ] && [[ "$API_KEY" == aim_* ]]; then
        # Remove old mode line, add connected config
        sed -i.bak '/^VECTORAIZ_MODE=/d' .env && rm -f .env.bak
        sed -i.bak '/^VECTORAIZ_AI_MARKET_URL=/d' .env && rm -f .env.bak
        sed -i.bak '/^VECTORAIZ_ALLIE_PROVIDER=/d' .env && rm -f .env.bak
        sed -i.bak '/^VECTORAIZ_INTERNAL_API_KEY=/d' .env && rm -f .env.bak
        sed -i.bak '/^# Allie AI/d' .env && rm -f .env.bak
        cat >> .env <<EOF

# Allie AI Assistant (connected mode)
VECTORAIZ_MODE=connected
VECTORAIZ_AI_MARKET_URL=${AI_MARKET_URL}
VECTORAIZ_ALLIE_PROVIDER=aimarket
VECTORAIZ_INTERNAL_API_KEY=${API_KEY}
EOF
        ALLIE_ENABLED=true
        success "Allie enabled!"
    else
        fail "Invalid API key. Keys start with 'aim_'."
    fi
    echo ""
fi

# ─── Step 6: Build and start ────────────────────────────────────
info "Starting vectorAIz..."
echo ""

docker compose -f "$COMPOSE_FILE" pull 2>&1 | while IFS= read -r line; do
    case "$line" in
        *"Pulling"*|*"Downloaded"*|*"Pull"*|*"Image"*|*"Up to date"*|*"digest"*)
            echo -e "  ${CYAN}│${NC} $line"
            ;;
    esac
done

docker compose -f "$COMPOSE_FILE" up -d 2>&1 | while IFS= read -r line; do
    case "$line" in
        *"Created"*|*"Started"*|*"Running"*)
            echo -e "  ${CYAN}│${NC} $line"
            ;;
    esac
done

echo ""

# ─── Step 7: Wait for healthy ───────────────────────────────────
info "Waiting for vectorAIz to be ready..."
MAX_WAIT=180
WAITED=0
while [ $WAITED -lt $MAX_WAIT ]; do
    if curl -sf "http://localhost:${PORT}/api/health" >/dev/null 2>&1; then
        break
    fi
    
    CONTAINER_STATUS=$(docker compose -f "$COMPOSE_FILE" ps --format json 2>/dev/null | grep vectoraiz | grep -o '"State":"[^"]*"' | cut -d'"' -f4)
    if [ "$CONTAINER_STATUS" = "restarting" ] && [ $WAITED -gt 30 ]; then
        echo ""
        warn "vectorAIz container is restarting. Checking logs..."
        echo ""
        docker compose -f "$COMPOSE_FILE" logs vectoraiz 2>&1 | tail -10 | while IFS= read -r line; do
            echo -e "  ${RED}│${NC} $line"
        done
        echo ""
        fail "Container failed to start. Check full logs with: docker compose -f $COMPOSE_FILE logs vectoraiz"
    fi
    
    printf "\r  ${BLUE}⏳${NC} Waiting for services to initialize... (%ds)" "$WAITED"
    sleep 3
    WAITED=$((WAITED + 3))
done
printf "\r                                                          \r"

if [ $WAITED -ge $MAX_WAIT ]; then
    warn "Timed out waiting for health check."
    echo -e "  Check logs: ${BOLD}docker compose -f $COMPOSE_FILE logs${NC}"
    echo -e "  The app may still be starting. Try opening ${BOLD}$URL${NC} in a minute."
else
    success "All services healthy"
fi

# ─── Step 8: Desktop shortcut ───────────────────────────────────
create_webloc() {
    local dir="$1"
    local shortcut_path="${dir}/${SHORTCUT_NAME}.webloc"
    if [ -d "$dir" ]; then
        cat > "$shortcut_path" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>URL</key>
    <string>${URL}</string>
</dict>
</plist>
EOF
        return 0
    fi
    return 1
}

create_desktop_file() {
    local filepath="$1"
    cat > "$filepath" <<EOF
[Desktop Entry]
Type=Link
Name=vectorAIz
URL=${URL}
Icon=applications-internet
EOF
    chmod +x "$filepath" 2>/dev/null
}

if [[ "$OSTYPE" == "darwin"* ]]; then
    if create_webloc "$HOME/Desktop"; then
        success "Desktop shortcut created: ~/Desktop/${SHORTCUT_NAME}.webloc"
    fi
    if create_webloc "$SCRIPT_DIR"; then
        success "Local shortcut created: ./${SHORTCUT_NAME}.webloc"
    fi
elif [[ "$OSTYPE" == "linux"* ]]; then
    if [ -d "$HOME/Desktop" ]; then
        create_desktop_file "${HOME}/Desktop/${SHORTCUT_NAME}.desktop"
        success "Desktop shortcut created"
    fi
    create_desktop_file "${SCRIPT_DIR}/${SHORTCUT_NAME}.desktop"
fi

# ─── Step 9: Open browser ───────────────────────────────────────
ALLIE_FLAG="disabled"
[ "$ALLIE_ENABLED" = true ] && ALLIE_FLAG="enabled"
print_ready "$ALLIE_FLAG"

if [[ "$OSTYPE" == "darwin"* ]]; then
    sleep 1
    open "$URL" 2>/dev/null || true
elif command -v xdg-open &>/dev/null; then
    sleep 1
    xdg-open "$URL" 2>/dev/null || true
fi

echo -e "  ${CYAN}Tip:${NC} View logs with: docker compose -f $COMPOSE_FILE logs -f"
if [ "$ALLIE_ENABLED" = false ]; then
    echo -e "  ${CYAN}Tip:${NC} Enable Allie later with: ./start.sh --setup-allie"
fi
echo ""
