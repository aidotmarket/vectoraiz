import assert from 'node:assert/strict';
import { assertLayout, assertDialogActions } from './layout-checks.mjs';

describe('native first launch (offline)', () => {
  it('renders the one-window shell and reaches first-run Models without frontend errors', async () => {
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 1,
      { timeoutMsg: 'Expected the main native window' });
    // Explicitly select this one-window app. The service's automatic focus
    // discovery requires the separate wdio IPC plugin, which we do not use.
    const [mainWindow] = await browser.getWindowHandles();
    await browser.switchToWindow(mainWindow);
    const heading = await $('main > .view[aria-label="Ask your library"] h1');
    await heading.waitForDisplayed();
    assert.equal(await heading.getText(), 'Ask your library');
    await browser.waitUntil(async () => (await $('main').getText()).includes('Search model needed: download or import it first.'),
      { timeoutMsg: 'Fresh offline launch must need models' });
    for (const name of ['Library', 'AI apps', 'Settings', 'Ask']) {
      const link = await $(`nav a[href="#${name.toLowerCase().replace(' ', '-')}"]`);
      await link.click();
      await $(`main > .view[aria-label="${name === 'Ask' ? 'Ask your library' : name}"]:not([hidden])`).waitForDisplayed();
      assert.equal(await link.getAttribute('aria-current'), 'page');
    }
    await $('nav a[href="#settings"]').click();
    await $('section#models').waitForDisplayed();
    assert.match(await $('section#models').getText(), /Models/);
    // A7 macOS regression: every action row has the same explicit 12px flex gap.
    const gaps = await browser.execute(() => Array.from(document.querySelectorAll('.actions'))
      .map(row => ({display:getComputedStyle(row).display,gap:getComputedStyle(row).gap})));
    assert.ok(gaps.length);
    assert.ok(gaps.every(row => row.display === 'flex' && row.gap === '12px'), JSON.stringify(gaps));
    const evidence = await browser.execute(() => window.__VZ_UI_TEST__);
    assert.ok(evidence, 'Startup error observer must be installed before frontend scripts');
    assert.deepEqual(evidence.errors, [], 'No uncaught errors, rejected promises or console errors');
    assert.deepEqual(evidence.network, [], 'No external frontend network requests');
  });

  it('A8 keeps navigation in the viewport while long views scroll and action groups wrap only as needed', async () => {
    await assertLayout(browser, view => $(`nav a[href="#${view}"]`).click());
  });

  it('A8 generic Continue/Cancel and confirmation/Cancel share a row and wrap at narrow widths', async () => {
    for (const width of [1280, 360]) {
      await browser.setWindowSize(width, 800);
      await $('nav a[href="#ai-apps"]').click();
      await $('#connect-other').click();
      await assertDialogActions(browser);
      await $('[role="dialog"] input').setValue('Layout test');
      await $('[role="dialog"] button').click();
      await $('#app-confirm').waitForDisplayed();
      await assertDialogActions(browser);
      await $('#app-done').click();
    }
    await browser.setWindowSize(1280, 800);
  });

  it('loads bundled Inter in the packaged WebView', async () => {
    const fonts = await browser.execute(async () => {
      const loaded = await document.fonts.load('14px Inter', 'vectorAIz');
      await document.fonts.ready;
      // Use the heading's real family, size and weight. Remove only Inter to
      // measure the same fallback stack the app would use if Inter failed.
      const rendered = getComputedStyle(document.querySelector('.view:not([hidden]) h1'));
      const families = rendered.fontFamily.split(',').map(family => family.trim());
      const isInter = family => family.replace(/["']/g, '') === 'Inter';
      const fallbackFamily = families.filter(family => !isInter(family)).join(', ');
      const canvas = document.createElement('canvas');
      const context = canvas.getContext('2d');
      const measure = family => {
        context.font = `${rendered.fontWeight} ${rendered.fontSize} ${family}`;
        return context.measureText('vectorAIz iii WWW 123').width;
      };
      const renderedWidth = measure(rendered.fontFamily);
      context.font = `${rendered.fontWeight} ${rendered.fontSize} Inter`;
      const interWidth = context.measureText('vectorAIz iii WWW 123').width;
      return {
        loaded:loaded.map(face => ({family:face.family,status:face.status})),
        checked:document.fonts.check('14px Inter', 'vectorAIz'),
        renderedFamily:families[0].replace(/["']/g, ''), fallbackFamily,
        renderedWidth, interWidth, fallbackWidth:measure(fallbackFamily),
      };
    });
    assert.ok(fonts.checked && fonts.loaded.some(face => /Inter/.test(face.family) && face.status === 'loaded'));
    assert.equal(fonts.renderedFamily, 'Inter');
    assert.ok(fonts.fallbackFamily, 'The app must retain its real fallback family stack');
    assert.equal(fonts.renderedWidth, fonts.interWidth, 'Rendered glyphs must resolve to Inter');
    assert.notEqual(fonts.renderedWidth, fonts.fallbackWidth, 'Rendered glyphs must not use the app fallback');
  });
});
