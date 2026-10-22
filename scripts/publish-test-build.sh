#!/bin/sh
# Koskadeux only. Dry-run never accesses a signing key or writes to GitHub.
set -eu
exec python3 "$(dirname "$0")/publish-test-build.py" "$@"
