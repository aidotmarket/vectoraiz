#!/bin/bash
# vectorAIz — Quick Start Wrapper
# Delegates to backend/start.sh
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/backend/start.sh" "$@"
