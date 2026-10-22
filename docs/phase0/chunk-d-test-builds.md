# Phase 0 chunk D test builds

The workspace Cargo version is authoritative; `scripts/native-version.py --write` derives the UI, npm lockfile, Cargo.lock workspace-member and Tauri versions. CI rejects drift. Stage 1 starts at `0.0.1`.

`native-ci.yml` builds on hosted `macos-14` and `windows-2022`. Artifacts contain the ad-hoc signed macOS DMG and `.app.tar.gz`, unsigned per-machine Windows MSI (including the silent offline WebView2 installer), notices and provenance/hash/size manifests. All macOS Mach-O leaves and the app container are signed and verified. The Windows MSI itself is the updater artifact.

Models are excluded. Packaging clears stale generated resources and copies only the platform PDFium runtime library, model metadata and notices. Parser worker, CLI and MCP binaries are included; macOS includes the Vision OCR helper. CI's `.assets` model cache is never copied. For this spike, import the pinned GGUF files named in `spike/fixtures/models.json` into `~/vectorAIz/models/` (Windows: `%USERPROFILE%\vectorAIz\models`). The installed app uses this user-owned directory and copies PDFium there as needed. Automatic model download/import UI is Phase 1 work.

Fresh-runner stand-ins install the downloaded artifacts, drive the installed WebView self-test and uninstall while checking a persistent data sentinel. Every external step has a deadline and prints progress immediately. Windows uses the native System32 `msiexec.exe` with `/qn /norestart /l*v`; MSI and UI logs are retained even on failure. WebView2 test profiles use a writable user data directory. Windows Server is explicitly a stand-in for Windows 10/11.

Notices include Rust licence text, bundled npm licence text, PDFium, llama.cpp and provisional model notices. `LICENSE` is unchanged.

## Signed publish dry run

Install UI dependencies with `cd app && npm ci`; install Minisign on Koskadeux. Then run:

```sh
scripts/publish-test-build.sh --dry-run --version 0.0.1 \
  --commit <full-PR-head-SHA> --run-id <successful-native-ci-run-id> \
  --output /var/tmp/vectoraiz-dry-run-<unique-suffix>
```

The output directory must be new. The script checks workflow/commit provenance, both platform manifests and hashes, and Docker release-stream invariants through GitHub's API. It generates a throwaway Tauri key pair, signs both updater artifacts with version-bound signatures, verifies with Minisign, and writes `latest.json`, `.sig` files, `SHA256SUMS` and evidence containing the verification public key. It lists assets without uploading. It never reads the real private key. Temporary key files are deleted when the command exits.

The embedded public key is the 2026-10-22 replacement. A future explicitly authorized `--publish` run accepts only successful `native` push builds and reads the two approved updater secrets from Infisical Cloud (`ai-market`, project `4a79065b-76db-4c4a-917d-ae6dc93f2686`, `prod`, `/koskadeux`) through the isolated Cloud helper with legacy environment overrides removed. It derives the signing public key and refuses any mismatch with the embedded key before signing or uploading. Key material is passed through process environment or private temporary files with signer output suppressed. No retired vault is used. The release target is exclusively `aidotmarket/vectoraiz-test-builds`; Docker latest and first prerelease tags are checked through the API. No `/rc` HTTP probe is made.

## Update acceptance

`native-update-test.yml` accepts `from_version`, `to_version` and `dry_check` via `workflow_dispatch`. Its default dry check also runs on PRs and native build-branch pushes on both hosted platforms, validating ascending stage 1 versions and the exact configured latest.json endpoint without requiring unpublished releases. A real run installs the published from-version, invokes the app's signed updater, verifies the expected to-version/self-test and preserved data, then uninstalls and checks persistence again. Reports and logs are uploaded.

The first real releases (`native-v0.0.1`, then `native-v0.0.2`), real update execution, and clean macOS/Windows client VM screenshots remain release-validation work after Gate 3. No real publication is authorized in this chunk.
