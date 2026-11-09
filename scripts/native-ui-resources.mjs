// Audit emitted assets, where Vite has resolved @fontsource imports and URLs.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const localOrigin = 'https://bundled.invalid';

function audit(source, file) {
  // Normalize CSS escapes before inspecting declarations, including escaped
  // schemes; comments cannot conceal an external reference from this check.
  const text = source.replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/\\([\da-f]{1,6})\s?|\\([^\r\n])/gi,
      (_, hex, char) => hex ? String.fromCodePoint(parseInt(hex, 16)) : char);
  const references = [...text.matchAll(/\burl\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*?))\s*\)|@import\s+(?:"([^"]*)"|'([^']*)')/gi)]
    .map(match => match.slice(1).find(value => value !== undefined).trim());
  if (/\.html$/i.test(file)) {
    for (const match of text.matchAll(/<link\b[^>]*>/gi)) {
      if (/\brel\s*=\s*["'][^"']*\bstylesheet\b/i.test(match[0])) {
        const href = match[0].match(/\bhref\s*=\s*["']([^"']*)["']/i);
        if (href) references.push(href[1]);
      }
    }
  }
  for (const reference of references) {
    // Only paths within the packaged origin are allowed, including in imports.
    const target = new URL(reference, `${localOrigin}/${file}`);
    assert.ok(target.origin === localOrigin && !/^(?:[a-z][\w+.-]*:|\/\/)/i.test(reference),
      `${file}: non-local CSS/font reference: ${reference}`);
  }
}

function files(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? files(path) : /\.(css|html)$/i.test(path) ? [path] : [];
  });
}

// Negative controls use the same auditor as dist, without making a request.
for (const source of [
  '@font-face { src: url("https://external.invalid/font.woff2") }',
  '@import "https://external.invalid/style.css";',
  '@import url(//external.invalid/style.css);',
  '@font-face { src: url(https\\3a //external.invalid/font.woff2) }',
  '<link rel="stylesheet" href="https://external.invalid/style.css">',
]) {
  assert.throws(() => audit(source, 'negative-control.html'), /non-local CSS\/font reference/);
}
audit('@import "./local.css"; @font-face { src: url(/assets/inter.woff2) }', 'positive-control.css');
console.log('PASS: external CSS/font negative controls rejected');

const config = JSON.parse(readFileSync(join(root, 'app/src-tauri/tauri.conf.json'), 'utf8'));
const csp = config.app.security.csp;
for (const directive of ["default-src 'none'", "style-src 'self'", "font-src 'self'"]) {
  assert.ok(csp.split(';').map(value => value.trim()).includes(directive), `Missing exact CSP directive: ${directive}`);
}
console.log('PASS: CSP restricts styles and fonts to self');

const emitted = files(join(root, 'app/dist'));
assert.ok(emitted.some(file => file.endsWith('.css')), 'Expected built CSS');
assert.ok(emitted.some(file => file.endsWith('.html')), 'Expected built HTML');
for (const file of emitted) audit(readFileSync(file, 'utf8'), file);
console.log(`PASS: ${emitted.length} built CSS/HTML files contain only local references`);
