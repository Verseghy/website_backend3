/**
 * The sliding session refresh in the browser (ADR 0006 §1, "Sliding
 * refresh, explicit"): a signed-in page calls `POST /api/admin/auth/refresh`
 * on typing or clicking, at most every 10 minutes, and the server answers
 * with a renewed two-hour session cookie.
 *
 * The page's clock is Playwright's (`page.clock`), so ten minutes pass in a
 * moment; the server keeps its own clock, so each refresh is real. Nothing
 * here signs out, so the test shares the seed account with the others.
 */
import type { Page, Request } from "@playwright/test"

import { expect, signIn, test } from "./fixtures.ts"

/** The path of the explicit refresh. */
const REFRESH = "/api/admin/auth/refresh"

/**
 * Whether `request` is a refresh call.
 *
 * @param request - a request of the page.
 * @returns `true` for a `POST` to {@link REFRESH}.
 */
function isRefresh(request: Request): boolean {
  return (
    request.method() === "POST" && new URL(request.url()).pathname === REFRESH
  )
}

/**
 * Runs `action` and returns the refresh call's answer that it caused.
 *
 * @param page - the test's page.
 * @param action - the user activity.
 * @returns the answer's status and its `Set-Cookie`.
 */
async function refreshAfter(
  page: Page,
  action: () => Promise<void>
): Promise<{ status: number; setCookie: string | null }> {
  const answer = page.waitForResponse((response) =>
    isRefresh(response.request())
  )
  await action()
  const response = await answer
  return {
    status: response.status(),
    setCookie: await response.headerValue("set-cookie"),
  }
}

test("a signed-in page refreshes its session on activity, at most every 10 minutes", async ({
  page,
}) => {
  await page.clock.install()
  await signIn(page)
  const calls: string[] = []
  page.on("request", (request) => {
    if (isRefresh(request)) {
      calls.push(request.url())
    }
  })
  const dashboard = page.getByTestId("dashboard")

  // The first activity after the page loads refreshes at once.
  const first = await refreshAfter(page, () => dashboard.click())
  expect(first.status).toBe(204)
  expect(first.setCookie).toMatch(
    /^__Host-admin_session=[^;]+; HttpOnly; Secure; SameSite=Strict; Path=\/; Max-Age=7200$/
  )
  expect(calls).toHaveLength(1)

  // Nine minutes later, clicks and keys call nothing.
  await page.clock.fastForward("09:00")
  await dashboard.click()
  await page.keyboard.press("a")
  // A request would leave with the event; give the page a moment to show
  // that none does.
  await page.waitForTimeout(300)
  expect(calls).toHaveLength(1)

  // Ten minutes after the first call, a key press refreshes again.
  await page.clock.fastForward("01:00")
  const second = await refreshAfter(page, () => page.keyboard.press("b"))
  expect(second.status).toBe(204)
  expect(second.setCookie).toContain("Max-Age=7200")
  expect(calls).toHaveLength(2)

  // The renewed cookie is the session: the next page still opens.
  await page.goto("/admin/about")
  await expect(page.getByTestId("about-page")).toBeVisible()
})
