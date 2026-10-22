#!/bin/sh
# Koskadeux only. Dry-run uses a generated throwaway signing key; --publish writes to GitHub.
set -eu
exec python3 "$(dirname "$0")/publish-test-build.py" "$@"
