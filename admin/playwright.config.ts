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
 */
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
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
})
