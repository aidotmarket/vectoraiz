#!/bin/bash
# =============================================================================
# vectorAIz — Universal Installer
# =============================================================================
# Auto-detects your OS and runs the appropriate installer.
#
# Usage:
#   curl -fsSL https://get.vectoraiz.com/install.sh | bash
#
# Or from GitHub:
#   curl -fsSL https://raw.githubusercontent.com/aidotmarket/vectoraiz/main/install.sh | bash
# =============================================================================

set -e
cd "$HOME" 2>/dev/null || cd /tmp

CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

GITHUB_RAW="https://raw.githubusercontent.com/aidotmarket/vectoraiz/main"

echo ""
echo -e "${CYAN}${BOLD}  vectorAIz — Universal Installer${NC}"
echo ""

case "$(uname -s)" in
    Darwin)
        echo -e "  ${CYAN}▸${NC} Detected: macOS"
        echo ""
        curl -fsSL "${GITHUB_RAW}/installers/mac/install-mac.sh" | bash
        ;;
    Linux)
        echo -e "  ${CYAN}▸${NC} Detected: Linux"
        echo ""
        curl -fsSL "${GITHUB_RAW}/installers/linux/install-linux.sh" | bash
        ;;
    MINGW*|MSYS*|CYGWIN*)
        echo -e "  ${CYAN}▸${NC} Detected: Windows (via Git Bash / MSYS2)"
        echo ""
        echo "  Windows requires the PowerShell installer. Run this command in PowerShell:"
        echo ""
        echo -e "  ${BOLD}irm ${GITHUB_RAW}/installers/windows/install-vectoraiz.ps1 | iex${NC}"
        echo ""
        echo -e "  ${DIM}Or download from: https://github.com/aidotmarket/vectoraiz/releases${NC}"
        echo ""
        ;;
    *)
        echo "  Unsupported operating system: $(uname -s)"
        echo ""
        echo "  Please install Docker manually and use docker-compose:"
        echo "  https://github.com/aidotmarket/vectoraiz#manual-installation"
        echo ""
        exit 1
        ;;
esac
