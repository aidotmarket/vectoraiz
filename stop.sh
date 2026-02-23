#!/bin/bash
# vectorAIz — Stop Wrapper
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/backend/stop.sh" "$@"
