import { defineConfig, devices } from "@playwright/test"

/**
 * End-to-end tests (`npm run e2e`, the `E2E` CI job, ADR 0022 §3).
 *
 * They run against the built admin served by a running backend3, never the
 * Vite dev server (ADR 0004 "What must be true"): CI builds `admin/dist`,
 * starts backend3 with `LOCO_ENV=e2e` on port 5150 and waits for `/_ping`
 * first. There is deliberately no `webServer` entry; starting backend3 is the
 * caller's job.
 *
 * `baseURL` is the admin's root. The tests navigate with root-absolute paths
 * (`/admin/about`): a relative `about` would resolve against a base without a
 * trailing slash to `/about`, outside the admin.
 *
 * The admin runs in Chromium, in two projects. Every admin test signs in with
 * the one seed account (ADR 0040), in parallel, and a sign-out ends every
 * session of that account (ADR 0006 §2). A sign-out among the other tests
 * would therefore end their sessions mid-test. So the files that sign out
 * (`SIGN_OUT_SPECS`) run in the `sign-out` project, which Playwright starts
 * only after every test of the `chromium` project has passed, and each of them
 * runs its tests one after another (`mode: "serial"`).
 *
 * `e2e/storage.spec.ts`, the files under `/storage` (editor.md §13 P9), runs
 * in all three engines instead, in its own three projects, each with its PDF
 * viewer on, as in the browsers people use: Playwright's default Chromium is
 * the headless shell, which has no PDF viewer, so the full Chromium build runs
 * it (`channel: "chromium"`, new headless mode); and Playwright's Firefox
 * turns pdf.js off by default (`pdfjs.disabled`), so the project turns it back
 * on. Playwright's WebKit for Linux has no PDF viewer at all. CI installs all
 * three (`npx playwright install --with-deps`). It signs in to nothing, so a
 * sign-out cannot touch it, and it runs beside the admin projects.
 */

/** The spec files whose tests end the seed account's sessions. */
const SIGN_OUT_SPECS = /[/\\]sign-out\.spec\.ts$/

/** The spec that runs in all three engines, and only in them. */
const STORAGE_SPEC = /storage\.spec\.ts$/

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  // No retries: a flaky routing or CSP check is a defect to fix, not to hide.
  retries: 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: "http://localhost:5150/admin",
    locale: "hu-HU",
    timezoneId: "Europe/Budapest",
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
      testIgnore: [SIGN_OUT_SPECS, STORAGE_SPEC],
    },
    {
      name: "sign-out",
      use: { ...devices["Desktop Chrome"] },
      testMatch: SIGN_OUT_SPECS,
      // After every other admin test: see the documentation above.
      dependencies: ["chromium"],
    },
    {
      name: "storage-chromium",
      testMatch: STORAGE_SPEC,
      use: { ...devices["Desktop Chrome"], channel: "chromium" },
    },
    {
      name: "storage-firefox",
      testMatch: STORAGE_SPEC,
      use: {
        ...devices["Desktop Firefox"],
        launchOptions: { firefoxUserPrefs: { "pdfjs.disabled": false } },
      },
    },
    {
      name: "storage-webkit",
      testMatch: STORAGE_SPEC,
      use: { ...devices["Desktop Safari"] },
    },
  ],
})
