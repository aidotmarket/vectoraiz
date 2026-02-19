#!/bin/bash
# =============================================================================
# vectorAIz — Stop
# =============================================================================
# Usage: ./stop.sh
# =============================================================================

set -e

COMPOSE_FILE="docker-compose.customer.yml"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo ""
echo -e "${CYAN}${BOLD}  ⚡ vectorAIz — Stopping...${NC}"
echo ""

if [ ! -f "$COMPOSE_FILE" ]; then
    echo -e "${RED}Cannot find $COMPOSE_FILE${NC}"
    exit 1
fi

docker compose -f "$COMPOSE_FILE" down 2>&1 | while IFS= read -r line; do
    echo -e "  ${BLUE}│${NC} $line"
done

echo ""
echo -e "${GREEN}${BOLD}  ✅ vectorAIz stopped.${NC}"
echo ""
echo -e "  Your data is preserved. Run ${BOLD}./start.sh${NC} to start again."
echo ""
