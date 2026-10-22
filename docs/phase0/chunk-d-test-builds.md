# Chunk D: stage 1 build and release contract

The 2026-10-22 chunk D dispatch amends Gate 2 sections 2 and 4.4: publishing runs on Koskadeux, not Actions. The existing Native CI builds workflow artifacts on pushes to `native`, on feature pushes, and on PRs into `native`. It has read-only repository permissions and no updater secrets. There is no release workflow, tag trigger, GHCR build or new Actions secret.

`Cargo.toml` workspace version is authoritative. Run `python3 scripts/native-version.py --write` after changing it, then commit the derived UI, Tauri and lockfile versions. CI checks consistency and runs negative tests that reject drift and reject real release checks with the placeholder public key. The initial stage 1 version is `0.0.1`.

## CI artifacts and hosted stand-ins

`native-installers-macos-14` contains an ad-hoc-signed Apple Silicon app in a DMG and `.app.tar.gz`, notices and a provenance/hash manifest. Every Mach-O leaf is signed before signing and verifying the app container. `native-installers-windows-2022` contains the unsigned per-machine MSI with offline WebView2, notices and its provenance/hash manifest. The MSI itself is the Windows updater artifact for Tauri 2; no legacy `.msi.zip` is used. Both include the provisional models, PDFium, parser worker and MCP/CLI binaries; macOS also includes the Vision OCR helper. llama.cpp is statically linked in this configuration; any Mach-O libraries in the bundle are signed by the same walk.

Phase 0 installers intentionally package the tested **debug** profile with the `smoke-test` feature, keeping the existing CPU pipeline evidence and avoiding a second full native compilation. These are test builds, not optimized distribution candidates. A production-profile build remains a later validation item.

Separate fresh `macos-14` and `windows-2022` jobs download the installer artifact, install it, launch the installed WebView from a temporary directory outside the checkout, click its real self-test control, require an `OK` report, and uninstall. The self-test checks installed resources and creates a persistent marker under the actual user data folder (`~/vectorAIz` / `%USERPROFILE%\vectorAIz`). Uninstall must remove the executable and leave that marker unchanged. MSI logs and the self-test result are uploaded as evidence. These are **hosted-runner stand-ins**, and Windows Server does not prove Windows 10/11 behavior.

Rust licence text comes from cargo-about. npm production dependencies are enumerated transitively with license-checker, but the output contains each package's actual licence text, name and version, not its JSON metadata or runner paths. Missing npm licence files fail the build. PDFium licence files, the llama.cpp licence and provisional model source/licence notices are appended. The complete notices are bundled and uploaded separately.

## Key freeze and dry run

The pre-rotation updater pair is treated as exposed. The checked-in public key is deliberately invalid and the app does not initialize the updater while it is present. No signing key is read, used, generated or stored by CI or dry-run. No Infisical access happens on these paths.

After CI on the selected commit is green, run on Koskadeux:

```sh
scripts/publish-test-build.sh --dry-run --version 0.0.1 \
  --commit <full-PR-head-SHA> --run-id <Native-CI-run-id> \
  --output /var/tmp/vectoraiz-dry-run-<unique-suffix>
```

The output directory must not already exist. The script verifies the completed workflow and exact source commit, both artifact provenance manifests and hashes, and agreement with the requested version. It reads the Docker repository's latest release and first non-draft prerelease and requires `v*` tags. It writes `latest.json`, explicit `UNSIGNED-DRY-RUN-NO-KEY-ACCESSED` `.sig` placeholders, `SHA256SUMS` and a machine-readable evidence file, and lists the proposed upload. This succeeds as an **unsigned assembly dry run**. It does **not** satisfy cryptographic signing/verification acceptance: that is deferred under the dispatch's prohibition on any signing key.

The future `--publish` path fails before network/secret access while the placeholder remains. After Gate 3 and incident unfreeze, Minerva must supply the rotated public key and replacement secret names, verify the names against the rotation dispatch, and authorize publishing. The script refuses the old exposed names. That path reads only those two secrets from Infisical `koskadeux-mcp`/`prod` using the sysadmin token, passes key material only through the signer's environment with output suppressed, signs version-bound Tauri signatures and verifies them with minisign before uploading. It only accepts successful push runs from `native`; it creates a full release with a tag on the test-build repository's default branch, names the source commit in its notes, and checks both repositories' release streams afterward. No source-repository tag or release is created.

## Acceptance still requiring evidence

Record final PR head, both CI run URLs, artifact names and the unsigned assembly dry-run output in the PR. Real signature verification, first real releases (`native-v0.0.1` then `native-v0.0.2`), update proof, and clean macOS/Windows client VM runs with screenshots remain for Minerva after Gate 3 and key rotation. No real release is authorized in chunk D. `LICENSE` stays unchanged.
