# Releasing vectorAIz

## Prerequisites

| Requirement | Check | Install |
|---|---|---|
| On `main` branch | `git branch --show-current` | `git checkout main` |
| Clean working tree | `git status` | `git stash` or `git commit` |
| Docker CLI | `docker version` | Install Docker Desktop or OrbStack |
| GitHub CLI | `gh auth status` | `brew install gh && gh auth login` |
| GHCR access | `docker manifest inspect ghcr.io/aidotmarket/vectoraiz:latest` | Set `GITHUB_TOKEN` env var or configure Doppler |

## Usage

```bash
# Release a specific version
./scripts/release.sh 1.17.0

# Auto-bump from latest git tag
./scripts/release.sh patch    # 1.16.0 → 1.16.1
./scripts/release.sh minor    # 1.16.0 → 1.17.0
./scripts/release.sh major    # 1.16.0 → 2.0.0
```

The version argument does **not** take a `v` prefix — the script adds it.

## What the script does

| Step | Action | Verification |
|---|---|---|
| Pre-flight | Checks branch, clean tree, docker, gh, GHCR auth | Aborts with specific fix instructions |
| 1. Update compose | `sed` the version default in `docker-compose.customer.yml` | `grep` confirms new version appears |
| 2. Commit + push | Commits the compose change, pushes to `origin/main` | `curl` the raw GitHub URL and confirms version (3 retries, 5s delay) |
| 3. Tag + push | Creates annotated tag `v{VERSION}`, pushes to origin | `gh release view` or `git ls-remote` confirms tag exists |
| 4. Wait for image | Polls `docker manifest inspect` for the versioned GHCR tag | Up to 30 minutes, 30s intervals. Aborts with clear message on timeout |
| 5. Smoke test | Checks install compose URL resolves to new version, `:latest` digest matches | Warns (non-fatal) if CDN hasn't propagated yet |

## Recovery: what to do if a step fails

### Pre-flight fails

Nothing has been modified. Fix the reported issue (wrong branch, dirty tree, missing tool) and re-run.

### Step 1 fails (compose update)

The file may be partially edited. Check `docker-compose.customer.yml` and ensure the `VECTORAIZ_VERSION` default is set correctly, then re-run the script.

### Step 2 fails (commit/push)

The commit was created locally but may not have been pushed.

```bash
# Check what happened
git log --oneline -3
git status

# If committed but not pushed:
git push origin main

# Then re-run the script — it will skip the commit (no changes) and continue
./scripts/release.sh <version>
```

### Step 3 fails (tag/push)

The tag may exist locally but not on the remote.

```bash
# Push the tag manually
git push origin v<VERSION>

# If you need to re-create the tag:
git tag -d v<VERSION>
git tag -a v<VERSION> -m "Release <VERSION>"
git push origin v<VERSION>
```

### Step 4 fails (image timeout)

The tag was pushed and GitHub Actions should be building the image. Check the workflow:

```bash
gh run list --workflow=docker-publish.yml
gh run view <run-id> --log
```

Once the image appears on GHCR, the release is functionally complete. Run the script again to execute the smoke test, or verify manually:

```bash
docker manifest inspect ghcr.io/aidotmarket/vectoraiz:v<VERSION>
```

### Step 5 fails (smoke test)

The release is already complete — smoke test failures are warnings, not blockers. CDN caching can delay raw.githubusercontent.com updates. The `:latest` tag may take a moment to update if Actions pushes it separately.

## Important rules

- **NEVER manually tag** without running this script. The script ensures `docker-compose.customer.yml` is updated before the tag is created, so customers pulling the install script always get the correct version default.
- **Version source of truth is git tags.** No hardcoded version strings anywhere in the codebase except the compose default (which this script manages).
- **The Docker image is built by GitHub Actions**, not locally. The script waits for the image to appear on GHCR after pushing the tag.
