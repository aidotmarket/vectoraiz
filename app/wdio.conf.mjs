import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

// Config is evaluated in launcher and worker; inherit one fresh data root.
const temporaryDataRoot = process.env.VZ_UI_TEST_DATA_ROOT
  ? undefined
  : mkdtempSync(resolve(tmpdir(), 'vectoraiz-ui-'));
process.env.VZ_UI_TEST_DATA_ROOT ??= temporaryDataRoot;
const artifacts = resolve('ui-test-results');
mkdirSync(artifacts, { recursive: true });

export const config = {
  runner: 'local',
  specs: ['./ui-tests/smoke.e2e.mjs'],
  maxInstances: 1,
  framework: 'mocha',
  reporters: ['spec'],
  logLevel: 'info',
  outputDir: artifacts,
  waitforTimeout: 30_000,
  connectionRetryCount: 0,
  mochaOpts: { timeout: 60_000 },
  services: [['@wdio/tauri-service', {
    driverProvider: 'embedded',
    appBinaryPath: resolve('../target/debug/bundle/macos/vectorAIz.app/Contents/MacOS/vectoraiz'),
    captureFrontendLogs: false,
    captureBackendLogs: true,
  }]],
  capabilities: [{ browserName: 'tauri', 'tauri:options': {} }],
  onComplete: function () {
    if (temporaryDataRoot) rmSync(temporaryDataRoot, { recursive: true, force: true });
  },
  afterTest: async function (_test, _context, { passed }) {
    const evidence = await browser.execute(() => window.__VZ_UI_TEST__ ?? null);
    writeFileSync(resolve(artifacts, 'frontend.json'), JSON.stringify(evidence, null, 2));
    if (!passed) {
      await browser.saveScreenshot(resolve(artifacts, 'failure.png'));
      writeFileSync(resolve(artifacts, 'page.html'), await browser.getPageSource());
    }
  },
};
