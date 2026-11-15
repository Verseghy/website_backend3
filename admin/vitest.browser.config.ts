import { resolve } from "node:path"
import react from "@vitejs/plugin-react"
import { playwright } from "@vitest/browser-playwright"
import { defineConfig } from "vitest/config"

/**
 * The paste tests in real browsers (`npm run test:browser`, run by the
 * `Admin test` CI job after `npm test`; editor.md §13.5).
 *
 * The paste filter reads the styles of clipboard HTML through each engine's
 * CSSOM (`src/rich-text/paste/inline-styles.ts`), and prosemirror-view reads
 * pasted HTML with the engine's parser, so every test of the filter's steps,
 * the degrade-safely suite and the paste extension's tests also run in
 * Chromium, Firefox and WebKit, through Vitest's browser mode with
 * Playwright's browsers
 * (`npx playwright install --with-deps chromium firefox webkit` first). The
 * same files run under jsdom with the other unit tests
 * (`vitest.config.ts`). Kept apart from `vitest.config.ts`, which runs every
 * unit test under jsdom, so `npm test` needs no browser.
 *
 * Every engine builds the `<style>` sheets of a `DOMParser` document, and
 * jsdom does not; the filter reads a constructed sheet there instead, so
 * both runs exercise the same step 6.
 */
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": resolve(import.meta.dirname, "./src"),
    },
  },
  test: {
    include: [
      "src/rich-text/paste/*.test.ts",
      "src/rich-text/extensions/office-paste.test.ts",
    ],
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
    unstubGlobals: true,
    browser: {
      enabled: true,
      provider: playwright(),
      headless: true,
      // No screenshots: the suite asserts markup, not pixels.
      screenshotFailures: false,
      instances: [
        { browser: "chromium" },
        { browser: "firefox" },
        { browser: "webkit" },
      ],
    },
  },
})
