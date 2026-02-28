# Releasing vectorAIz

## Overview

Releases are a two-part process: a local script that prepares the release, and GitHub Actions that builds, verifies, and publishes it.

**NEVER manually tag, build, or push Docker images.** Always use `release.sh`.

## Prerequisites

- On `main` branch with clean working tree
- `docker` CLI available (OrbStack or Docker Desktop)
- `gh` CLI installed and authenticated (`brew install gh && gh auth login`)
- GHCR access (script handles auth via Doppler or GITHUB_TOKEN)

## Usage

```bash
cd ~/Projects/vectoraiz/vectoraiz-monorepo

# Bump patch: 1.16.0 → 1.16.1
./scripts/release.sh patch

# Bump minor: 1.16.0 → 1.17.0
./scripts/release.sh minor

# Specific version
./scripts/release.sh 2.0.0
```

## What happens

### Local (release.sh)

1. **Pre-flight checks** — main branch, clean tree, docker, gh, GHCR auth
2. **Update compose** — sets `VECTORAIZ_VERSION:-v1.17.0` in `docker-compose.customer.yml`
3. **Commit + push** — commits compose change, pushes to main, verifies GitHub raw URL
4. **Tag + push tag** — creates `v1.17.0` tag, pushes to origin
5. **Wait for image** — polls GHCR until the image is available (up to 30 min)
6. **Smoke test** — verifies install URL and `:latest` tag match

### GitHub Actions (triggered by tag push)

1. **verify-release** — confirms compose file has correct version, install scripts have health gates, Dockerfile exists
2. **build-push** — builds Docker image from `Dockerfile.customer`, pushes to GHCR as `v1.17.0` + `latest`
3. **smoke-test** — pulls image, verifies digests match, simulates install, runs container startup health check
4. **create-release** — creates GitHub Release with installer scripts (only if smoke test passes)

### CI (on every push to main)

- Verifies compose version matches latest tag
- Shell script syntax checks
- Install script safety checks (health gates present)
- v-prefix convention enforcement

## Critical rules

- **GHCR tags always have `v` prefix**: `v1.16.0`, never `1.16.0`
- **Compose file always references v-prefixed version**: `VECTORAIZ_VERSION:-v1.16.0`
- **Install scripts must gate success banner on health check**: no false "installed!" messages
- **GitHub Actions is the enforcement layer**: even if release.sh has a bug, Actions will catch it

## Recovery procedures

### Tag pushed but image build failed
```bash
# Check what went wrong
gh run list --workflow=release.yml --limit 5

# If compose was wrong, fix and force-update:
# 1. Fix docker-compose.customer.yml
# 2. git commit and push
# 3. Delete the tag: git tag -d v1.17.0 && git push origin :refs/tags/v1.17.0
# 4. Re-run: ./scripts/release.sh 1.17.0
```

### Compose file has wrong version
```bash
# Edit docker-compose.customer.yml manually
sed -i '' 's/VECTORAIZ_VERSION:-v.*/VECTORAIZ_VERSION:-v1.17.0}/' docker-compose.customer.yml
git add docker-compose.customer.yml
git commit -m "fix: correct compose version to v1.17.0"
git push origin main
```

### Install script serving old version (CDN cache)
GitHub raw CDN can take 5-10 minutes to propagate. The CI workflow will warn but not block. If it persists beyond 10 minutes, check that the commit actually reached `main`.

### Health check fails in smoke test
The container startup test runs in GitHub Actions. If it fails:
1. Check the Actions log for container logs
2. The image was already pushed to GHCR but the GitHub Release was NOT created
3. Fix the issue, bump a new patch version
