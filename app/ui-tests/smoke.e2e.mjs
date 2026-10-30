import assert from 'node:assert/strict';

describe('native first launch (offline)', () => {
  it('renders vectorAIz and the first-run models screen without frontend errors', async () => {
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 1,
      { timeoutMsg: 'Expected the main native window' });
    const heading = await $('h1');
    await heading.waitForDisplayed();
    assert.equal(await heading.getText(), 'DELIBERATE FAILURE: vectorAIz');
    const models = await $('aside[aria-label="Models"]');
    await models.waitForDisplayed();
    await browser.waitUntil(async () => /Checking models…|Models/.test(await models.getText()),
      { timeoutMsg: 'Expected model checking or first-run models screen' });
    // Wait for the real status IPC to finish, so startup errors cannot pass unnoticed.
    await browser.waitUntil(async () => (await $('main').getText()).includes('Models needed: download or import them first.'),
      { timeoutMsg: 'Fresh offline launch must need models' });
    const evidence = await browser.execute(() => window.__VZ_UI_TEST__);
    assert.ok(evidence, 'Startup error observer must be installed before frontend scripts');
    assert.deepEqual(evidence.errors, [], 'No uncaught errors, rejected promises or console errors');
    assert.deepEqual(evidence.network, [], 'No external frontend network requests');
  });
});
