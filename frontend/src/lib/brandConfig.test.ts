import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AIM_DATA_BRAND, getActiveBrand } from "./brandConfig";

const legacyProductTerms = [
  ["aim", " channel"].join(""),
  ["aim", "-channel"].join(""),
  ["aim", "_channel"].join(""),
  ["aim", " federate"].join(""),
  ["aim", "-federate"].join(""),
  ["feder", "ated"].join(""),
];

function expectNoLegacyProductTerms(value: string) {
  const normalizedValue = value.toLowerCase();

  for (const term of legacyProductTerms) {
    expect(normalizedValue).not.toContain(term);
  }
}

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("AIM Data brand configuration", () => {
  it("uses AIM Data and its logo paths by default", () => {
    vi.stubEnv("VITE_BRAND", "");

    expect(getActiveBrand()).toBe(AIM_DATA_BRAND);
    expect(getActiveBrand()).toMatchObject({
      name: "AIM Data",
      productName: "AIM Data",
      logoPath: "/aim-data-logo.jpg",
      logoSmPath: "/aim-data-logo-sm.png",
    });
  });

  it.each(["aim-data", "aim_data", "aim"])(
    "uses AIM Data for the %s runtime brand",
    (runtimeBrand) => {
      vi.stubEnv("VITE_BRAND", runtimeBrand);

      expect(getActiveBrand()).toBe(AIM_DATA_BRAND);
      expect([getActiveBrand().logoPath, getActiveBrand().logoSmPath]).toEqual([
        "/aim-data-logo.jpg",
        "/aim-data-logo-sm.png",
      ]);
    },
  );

  it("contains no legacy or retired product term", () => {
    const source = readFileSync(resolve("src/lib/brandConfig.ts"), "utf8");

    expectNoLegacyProductTerms(source);
    expectNoLegacyProductTerms(Object.values(AIM_DATA_BRAND).join(" "));
  });
});
