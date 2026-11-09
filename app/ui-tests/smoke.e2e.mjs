import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// Inspect declarations even when CSP would block the resource. Used with the
// actual packaged stylesheets below, and a deliberately external negative control.
function externalReferences(references, base) {
  return references.filter(reference => {
    const target = new URL(reference, base);
    const local = new URL(base);
    return target.protocol !== local.protocol || target.host !== local.host;
  });
}

describe('native first launch (offline)', () => {
  it('renders the one-window shell and reaches first-run Models without frontend errors', async () => {
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 1,
      { timeoutMsg: 'Expected the main native window' });
    const heading = await $('h1');
    await heading.waitForDisplayed();
    assert.equal(await heading.getText(), 'Ask your library');
    await browser.waitUntil(async () => (await $('main').getText()).includes('Search model needed: download or import it first.'),
      { timeoutMsg: 'Fresh offline launch must need models' });
    for (const name of ['Library', 'AI apps', 'Settings', 'Ask']) {
      const link = await $(`nav a[href="#${name.toLowerCase().replace(' ', '-')}"]`);
      await link.click();
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

  it('loads bundled Inter and audits stylesheet/font resources, including a negative control', async () => {
    const fonts = await browser.execute(async () => {
      const loaded = await document.fonts.load('14px Inter', 'vectorAIz');
      await document.fonts.ready;
      const references = Array.from(document.querySelectorAll('link[rel="stylesheet"]'), link => link.href);
      const fontRules = [];
      function inspect(sheet) {
        for (const rule of sheet.cssRules) {
          if (rule.type === CSSRule.IMPORT_RULE) {
            references.push(rule.href);
            inspect(rule.styleSheet);
          }
          if (rule.type === CSSRule.FONT_FACE_RULE) {
            fontRules.push(rule.cssText);
            for (const match of rule.style.getPropertyValue('src').matchAll(/url\(["']?([^"')]+)["']?\)/g)) references.push(match[1]);
          }
        }
      }
      for (const sheet of document.styleSheets) inspect(sheet);
      const actualReferences = [...references];
      const actualFontRules = [...fontRules];
      const plantedLink = document.createElement('link');
      plantedLink.rel = 'stylesheet';
      plantedLink.href = 'https://external.invalid/planted.css';
      // Detached declarations cannot make a network request. Audit them through
      // the same rule walker, so a blocked/failed request cannot hide an origin.
      const plantedFont = new CSSStyleSheet();
      plantedFont.insertRule('@font-face { font-family: Planted; src: url("https://external.invalid/planted.woff2"); }');
      inspect(plantedFont);
      const plantedReferences = [plantedLink.href, ...references.slice(actualReferences.length)];
      const resources = performance.getEntriesByType('resource')
        .filter(entry => /\.(css|woff2?)(?:[?#]|$)/i.test(entry.name))
        .map(entry => entry.name);
      // Inspect the actual rendered element and compare glyph metrics against a
      // forced fallback, alongside loaded FontFace objects (not just a CSS name).
      const canvas = document.createElement('canvas');
      const context = canvas.getContext('2d');
      context.font = '14px Inter';
      const interWidth = context.measureText('vectorAIz iii WWW 123').width;
      context.font = '14px monospace';
      return {
        base:location.href, references:actualReferences, fontRules:actualFontRules, resources, plantedReferences,
        loaded:loaded.map(face => ({family:face.family,status:face.status})),
        checked:document.fonts.check('14px Inter', 'vectorAIz'),
        renderedFamily:getComputedStyle(document.querySelector('.view:not([hidden]) h1')).fontFamily,
        interWidth, fallbackWidth:context.measureText('vectorAIz iii WWW 123').width,
      };
    });
    assert.ok(fonts.checked && fonts.loaded.some(face => /Inter/.test(face.family) && face.status === 'loaded'));
    assert.match(fonts.renderedFamily, /Inter/);
    assert.notEqual(fonts.interWidth, fonts.fallbackWidth, 'Rendered glyphs must not use the forced fallback');
    assert.ok(fonts.fontRules.length, 'Inspect packaged font-face declarations');
    assert.ok(fonts.resources.some(url => /\.woff2?(?:[?#]|$)/i.test(url)), 'Capture WebView font loads through Resource Timing');
    assert.deepEqual(externalReferences([...fonts.references, ...fonts.resources], fonts.base), [], 'All CSS/font references and loads must be bundled');
    for (const reference of fonts.plantedReferences) {
      assert.deepEqual(externalReferences([...fonts.references, reference], fonts.base), [reference],
        'A planted external declaration must fail inspection even without a successful network load');
    }
    const config = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url)));
    const csp = config.app.security.csp;
    assert.match(csp, /default-src 'none'/);
    assert.match(csp, /(?:^|;\s*)style-src 'self'(?:;|$)/);
    assert.match(csp, /(?:^|;\s*)font-src 'self'(?:;|$)/);
  });
});
