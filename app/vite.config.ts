import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  test: { exclude: ["ui-tests/**", "node_modules/**"], environment: "jsdom", setupFiles: ["./src/test-setup.ts"] },
});
