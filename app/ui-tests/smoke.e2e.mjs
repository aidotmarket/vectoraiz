import assert from 'node:assert/strict';

describe('native first launch (offline)', () => {
  it('renders the one-window shell and reaches first-run Models without frontend errors', async () => {
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 1,
      { timeoutMsg: 'Expected the main native window' });
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

  it('loads bundled Inter in the packaged WebView', async () => {
    const fonts = await browser.execute(async () => {
      const loaded = await document.fonts.load('14px Inter', 'vectorAIz');
      await document.fonts.ready;
      // Inspect the actual rendered element and compare glyph metrics against a
      // forced fallback, alongside loaded FontFace objects (not just a CSS name).
      const canvas = document.createElement('canvas');
      const context = canvas.getContext('2d');
      context.font = '14px Inter';
      const interWidth = context.measureText('vectorAIz iii WWW 123').width;
      context.font = '14px monospace';
      return {
        loaded:loaded.map(face => ({family:face.family,status:face.status})),
        checked:document.fonts.check('14px Inter', 'vectorAIz'),
        renderedFamily:getComputedStyle(document.querySelector('.view:not([hidden]) h1')).fontFamily,
        interWidth, fallbackWidth:context.measureText('vectorAIz iii WWW 123').width,
      };
    });
    assert.ok(fonts.checked && fonts.loaded.some(face => /Inter/.test(face.family) && face.status === 'loaded'));
    assert.match(fonts.renderedFamily, /Inter/);
    assert.notEqual(fonts.interWidth, fonts.fallbackWidth, 'Rendered glyphs must not use the forced fallback');
  });
});
