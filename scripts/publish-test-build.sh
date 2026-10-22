#!/bin/sh
# Koskadeux only. Dry-run uses only a generated throwaway signing key or writes to GitHub.
set -eu
exec python3 "$(dirname "$0")/publish-test-build.py" "$@"
