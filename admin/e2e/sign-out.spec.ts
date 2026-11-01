/**
 * Signing out in a real browser against backend3 (ADR 0006 §2, parity
 * AUTH-04): „Kijelentkezés” in the user menu ends the session on the server,
 * so the token that the browser held stops working, the browser drops its
 * cookie, and a protected page asks for a sign-in again; and the same
 * account's session in another browser ends with it. Every test also passes
 * the CSP and console guard of `fixtures.ts`.
 *
 * Why its own Playwright project: logout ends every session of the account,
 * and the suite has one account, the seed account (ADR 0040), which every
 * other test signs in with, in parallel. `playwright.config.ts` runs this file
 * in the `sign-out` project, which starts after every other test has
 * finished, and the tests below run one after another.
 *
 * It needs the seed account: `LOCO_ENV=e2e cargo loco db seed` after the
 * server has migrated `backend3_e2e` (the E2E CI job does both).
 */
import { expect, signIn, test } from "./fixtures.ts"
import { SEED_ACCOUNT } from "./seed-account.ts"

/** The cookie that carries the admin session (ADR 0006 §1). */
const SESSION_COOKIE = "__Host-admin_session"
/** The signed-in account's endpoint, which a revoked token gets 401 from. */
const ME = "/api/admin/auth/me"
/** The sign-out endpoint. */
const LOGOUT = "/api/admin/auth/logout"

test.describe.configure({ mode: "serial" })

test.describe("signing out", () => {
  // After the sign-out, a protected page asks /auth/me, which answers 401.
  test.use({ expectedHttpErrors: [{ status: 401, path: ME }] })

  test("signs out through the user menu and ends the session on the server", async ({
    page,
    context,
    request,
  }) => {
    await signIn(page)
    const token = (await context.cookies()).find(
      (cookie) => cookie.name === SESSION_COOKIE
    )?.value
    expect(token, `${SESSION_COOKIE} after the sign-in`).toBeTruthy()

    await page.getByRole("button", { name: "Profil" }).click()
    await expect(page.getByRole("menu")).toContainText(SEED_ACCOUNT.name)
    await expect(page.getByRole("menu")).toContainText(SEED_ACCOUNT.email)
    const logout = page.waitForResponse(
      (response) => new URL(response.url()).pathname === LOGOUT
    )
    await page.getByRole("menuitem", { name: "Kijelentkezés" }).click()

    const answer = await logout
    expect(answer.status()).toBe(204)
    expect(answer.request().method()).toBe("POST")
    await expect(page.getByTestId("sign-in-page")).toBeVisible()
    expect(new URL(page.url()).pathname).toBe("/admin/login")
    // The answer's Set-Cookie deleted the cookie in the browser.
    const names = (await context.cookies()).map((cookie) => cookie.name)
    expect(names).not.toContain(SESSION_COOKIE)

    // The server revoked the token itself: sent again by hand, it opens
    // no session. A sign-out that only deleted the cookie would pass the
    // checks above and fail here.
    const replay = await request.get(ME, {
      headers: { Cookie: `${SESSION_COOKIE}=${token}` },
    })
    expect(replay.status()).toBe(401)

    // A protected page asks for a sign-in again.
    await page.goto("/admin/about")
    await expect(page.getByTestId("sign-in-page")).toBeVisible()
    await expect(page.getByTestId("about-page")).toHaveCount(0)
  })

  test("ends the account's session in another browser too", async ({
    page,
    browser,
    baseURL,
  }) => {
    // A second browser profile: its own cookie jar, its own session.
    const elsewhere = await browser.newContext({ baseURL })
    try {
      const otherPage = await elsewhere.newPage()
      await signIn(otherPage)
      await signIn(page)

      await page.getByRole("button", { name: "Profil" }).click()
      await page.getByRole("menuitem", { name: "Kijelentkezés" }).click()
      await expect(page.getByTestId("sign-in-page")).toBeVisible()

      // The other browser still holds its cookie, but the server no longer
      // accepts the session in it, so its next page asks for a sign-in.
      expect(
        (await elsewhere.cookies()).map((cookie) => cookie.name)
      ).toContain(SESSION_COOKIE)
      await otherPage.reload()
      await expect(otherPage.getByTestId("sign-in-page")).toBeVisible()
      await expect(otherPage.getByTestId("dashboard")).toHaveCount(0)
    } finally {
      await elsewhere.close()
    }
  })
})
