import { resolve } from "node:path"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vitest/config"

/**
 * Unit tests (`npm run test`, the `Admin test` CI job, ADR 0022 §3).
 *
 * Kept apart from `vite.config.ts` so tests never depend on build-only
 * settings (base path, dev proxy, source-commit validation). Playwright's
 * end-to-end tests in `e2e/` are not unit tests and are excluded.
 */
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": resolve(import.meta.dirname, "./src"),
    },
  },
  server: {
    fs: {
      // The stored-HTML serialiser's tests read the server's sanitiser
      // regression fixtures (editor.md §13.4, "Over every committed
      // fixture"; src/test/stored-html.ts). Vite serves nothing outside
      // admin/ unless it is listed, so that one directory is added here.
      allow: [
        import.meta.dirname,
        resolve(import.meta.dirname, "../tests/fixtures/regressions"),
      ],
    },
  },
  test: {
    environment: "jsdom",
    include: [
      "src/**/*.test.{ts,tsx}",
      "scripts/**/*.test.mjs",
      "eslint-rules/**/*.test.mjs",
    ],
    exclude: ["node_modules/**", "e2e/**", "dist/**"],
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
    unstubGlobals: true,
  },
})
