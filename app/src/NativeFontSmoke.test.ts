import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { expect, it } from "vitest";

// Exercise the packaged smoke's actual assertion with controlled glyph metrics.
// A loaded FontFace and an Inter CSS name alone must not accept fallback glyphs.
async function runFontSmoke(interRenders: boolean, family = "Inter, system-ui, sans-serif") {
  const tests = new Map<string, () => Promise<void>>();
  const measured: string[] = [];
  const context = {
    font: "",
    measureText() {
      measured.push(this.font);
      return {width:this.font.includes("Inter") && interRenders ? 133 : this.font.includes("monospace") ? 156 : 121};
    },
  };
  const style = {fontFamily:family,fontSize:"26px",fontWeight:"600"};
  const sandbox = {
    assert,
    describe: (_name: string, work: () => void) => work(),
    it: (name: string, work: () => Promise<void>) => tests.set(name, work),
    browser: {execute: (work: () => Promise<unknown>) => work()},
    document: {
      fonts: {load: async () => [{family:"Inter",status:"loaded"}],ready:Promise.resolve(),check: () => true},
      querySelector: () => ({}),
      createElement: () => ({getContext: () => context}),
    },
    getComputedStyle: () => style,
  };
  const source = readFileSync("ui-tests/smoke.e2e.mjs", "utf8").replace(/^import assert[^\n]+\n/, "");
  vm.runInNewContext(source, sandbox);
  await tests.get("loads bundled Inter in the packaged WebView")!();
  return measured;
}

it("A7 packaged font smoke measures the real fallback stack at the rendered size and weight", async () => {
  const measured = await runFontSmoke(true);
  expect(measured).toContain("600 26px system-ui, sans-serif");
});
it("A7 packaged font smoke rejects fallback glyphs even with a loaded Inter face", async () => {
  await expect(runFontSmoke(false)).rejects.toThrow();
});
it("A7 packaged font smoke requires Inter to be the rendered first-choice family", async () => {
  await expect(runFontSmoke(true, "system-ui, Inter, sans-serif")).rejects.toThrow();
});
