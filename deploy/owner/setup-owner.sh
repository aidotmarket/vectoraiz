#!/usr/bin/env bash
# Invoked by shipped installers only after original/fresh configuration exists.
# Re-running an adopted installation delegates before any downloads or rewrites.
set -euo pipefail
INSTALL_DIR=$1
COMPOSE_FILE=$2
RELEASE=$3
APP_URL=$4
CHANNEL=${5:-direct}
STATE="$INSTALL_DIR/.vectoraiz-owner"
if [ "$CHANNEL" = aim-data ]; then
    echo 'channel_not_integrated: AIM-Data needs its own release evidence; no vectoraiz owner will be installed.' >&2
    exit 1
fi
if [ -f "$STATE/registration.json" ]; then
    exec "$STATE/owner" launch --root "$STATE" --open
fi
case "$RELEASE" in v[0-9]*.[0-9]*.[0-9]*|[0-9]*.[0-9]*.[0-9]*) ;; *) echo 'An exact verified owner release is required for one-time adoption.' >&2; exit 1;; esac
case "$(uname -s)" in Darwin) OWNER_OS=darwin;; Linux) OWNER_OS=linux;; *) exit 1;; esac
case "$(uname -m)" in arm64|aarch64) OWNER_ARCH=arm64;; x86_64|amd64) OWNER_ARCH=amd64;; *) exit 1;; esac
umask 077
mkdir -p "$STATE"
STAGE=$(mktemp -d "$STATE/.package-XXXXXXXX")
trap 'rm -rf "$STAGE"' EXIT
ASSET="vectoraiz-owner-${OWNER_OS}-${OWNER_ARCH}"
BASE="https://github.com/aidotmarket/vectoraiz/releases/download/$RELEASE"
if [ -n "${VECTORAIZ_OWNER_PACKAGE_DIR:-}" ]; then
    cp "$VECTORAIZ_OWNER_PACKAGE_DIR/$ASSET" "$STAGE/$ASSET"
    cp "$VECTORAIZ_OWNER_PACKAGE_DIR/owner-sha256.txt" "$STAGE/checksums"
else
    curl --fail --silent --show-error --location --proto '=https' --max-time 120 "$BASE/$ASSET" -o "$STAGE/$ASSET"
    curl --fail --silent --show-error --location --proto '=https' --max-time 30 "$BASE/owner-sha256.txt" -o "$STAGE/checksums"
fi
EXPECTED=$(awk -v asset="$ASSET" '$2 == asset {print $1}' "$STAGE/checksums")
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL=$(sha256sum "$STAGE/$ASSET" | awk '{print $1}')
else
    ACTUAL=$(shasum -a 256 "$STAGE/$ASSET" | awk '{print $1}')
fi
if [ "${#EXPECTED}" -ne 64 ] || [ "$ACTUAL" != "$EXPECTED" ]; then
    echo 'Owner package checksum mismatch; existing installation untouched.' >&2; exit 1
fi
chmod 700 "$STAGE/$ASSET"
if [ "$("$STAGE/$ASSET" version)" != "$RELEASE" ]; then
    echo 'Owner package belongs to a different release.' >&2; exit 1
fi
mv "$STAGE/$ASSET" "$STATE/owner"
cd "$INSTALL_DIR"
# This default entry path knows its invocation. Custom deployments run setup
# directly with their actual ordered files/env files/project/context instead.
"$STATE/owner" setup --root "$STATE" --cwd "$INSTALL_DIR" --file "$INSTALL_DIR/$COMPOSE_FILE" --channel "$CHANNEL" --target "$RELEASE" --url "$APP_URL"
"$STATE/owner" wait --root "$STATE"
echo "Adopted release verified. Independent status: $STATE/status.json"
