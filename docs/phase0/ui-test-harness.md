# Phase 0 macOS UI harness

Decision (Minerva; to be confirmed at Gate 3): WebdriverIO with
`@wdio/tauri-service` and `driverProvider: 'embedded'`, using
`tauri-plugin-wdio-webdriver` 1.5.1. The embedded server runs inside the real
Tauri application and needs neither an external driver nor a paid CrabNebula key.

[Tauri's WebDriver guidance](https://v2.tauri.app/develop/tests/webdriver/)
recommends this service for macOS. Direct `tauri-driver` supports Windows/Linux;
macOS has no WKWebView driver. The alternative accessibility-driver smoke test
needs Accessibility (TCC) permission for the runner, which hosted macOS runners
do not grant without workarounds. See also the
[service setup](https://github.com/webdriverio/desktop-mobile/blob/main/packages/tauri-service/docs/plugin-setup.md).

## Run locally on macOS

Use the repository's pinned Rust toolchain and Node version (`.nvmrc`), Xcode
command-line tools (macOS 13+ deployment target), and protocol compiler (`python3 scripts/install-protoc.py`).
From the repository root:

```sh
cd app
npm ci
npm run tauri -- build --debug --ci --features ui-test,custom-protocol --bundles app --config src-tauri/ui-test/tauri.conf.json
npx wdio run wdio.conf.mjs
cd ..
python3 scripts/native-ui-test-gate.py
```

The UI-test and stage-1 builds share `target/debug/bundle/macos/vectorAIz.app`, so rebuild stage-1 before using that path for shipping acceptance.

The config launches `target/debug/bundle/macos/vectorAIz.app/Contents/MacOS/vectoraiz`
(the executable inside the debug bundle), one main window and one worker. It creates a fresh temporary model directory, bundles only the
model manifest, skips the startup update request, and never clicks download or
loads models. No model assets or inference helper are needed. The smoke checks
the real `vectorAIz` heading and Models first-run screen, waits for model status
IPC to settle, and asserts no frontend errors. A debug-only initialization script
records uncaught exceptions, unhandled promise rejections and `console.error`
from before React starts. It also rejects and records external frontend fetches;
Tauri's local IPC transport remains available. This uses ordinary WebDriver
script execution, so the additional WDIO API/mocking plugin is unnecessary.

`ui-test-macos` runs on `macos-14`. On failure it uploads WDIO/backend logs,
`frontend.json`, a screenshot and page HTML from `app/ui-test-results/`.
To verify failure detection, temporarily change the heading expectation in
`app/ui-tests/smoke.e2e.mjs`, push to the PR, observe the failed smoke job, then
revert that assertion. The PR records both Native CI run IDs.

## Shipping boundary and limits

`ui-test` is an optional Cargo feature. Registration, error observation, offline
startup behavior and the separate `ui-test/capability.json` are guarded by both
the feature and `debug_assertions`; optimized builds with `ui-test` are rejected.
The capability is outside the auto-discovered shipping capabilities directory
and is added only by that guarded code. Existing default/self-test ACLs stay
unchanged. The CI gate combines shipping workflow feature lists with `build.features` from
the base, stage-1 and generated packaging configs and checks the default and
stage-1 Cargo dependency graphs, including the debug stage-1 package, contain no
WDIO plugin; no shipping workflow or build command enables it.
The embedded server is loopback-only (`127.0.0.1:4445`, or the port set by `TAURI_WEBDRIVER_PORT`) and unauthenticated, and `ui-test` builds must not run outside the harness.

This proves first paint and real first-run IPC on macOS, not model downloads,
inference, accessibility or a full interaction suite. Windows is not wired yet;
the embedded provider supports extending the same harness there later.
