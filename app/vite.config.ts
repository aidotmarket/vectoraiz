import { configDefaults, defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { readFileSync } from "node:fs";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  define: { __REPOSITORY_LICENCE__: JSON.stringify(readFileSync(new URL("../LICENSE", import.meta.url), "utf8")) },
  test: { exclude: [...configDefaults.exclude, "ui-tests/**"], environment: "jsdom", setupFiles: ["./src/test-setup.ts"] },
});
