/**
 * Signing out in a real browser against backend3 (ADR 0006 §2, parity
 * AUTH-04): „Kijelentkezés” in the user menu ends the session on the server,
 * so the token that the browser held stops working, the browser drops its
 * cookie, and a protected page asks for a sign-in again; and the same
 * account's session in another browser ends with it. A page on another
 * origin of the same site cannot sign the admin out: the server refuses its
 * cookie-authenticated write (ADR 0006 §3, ADR 0044), while the admin's own
 * sign-out carries the admin origin. Every test also passes the CSP and
 * console guard of `fixtures.ts`.
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
import { createServer } from "node:http"

import { expect, signIn, test } from "./fixtures.ts"
import { SEED_ACCOUNT } from "./seed-account.ts"

/** The cookie that carries the admin session (ADR 0006 §1). */
const SESSION_COOKIE = "__Host-admin_session"
/** The signed-in account's endpoint, which a revoked token gets 401 from. */
const ME = "/api/admin/auth/me"
/** The sign-out endpoint. */
const LOGOUT = "/api/admin/auth/logout"
/** The origin the E2E server serves the admin from (`config/e2e.yaml`). */
const ADMIN_ORIGIN = "http://localhost:5150"
/** A page served on another origin of the admin's site, and how to stop it. */
interface SiblingPage {
  /** The page's origin: `http://localhost:<port>`. */
  readonly origin: string
  /** Stops its server. */
  readonly close: () => Promise<void>
}

/**
 * Serves, on another origin of the same site (`localhost` on a free port),
 * a page whose script posts the logout to the admin: a no-cors `POST`, a
 * write that needs no CORS permission, which the browser sends with the
 * cookie and never lets the page read the answer of. Browsers send the
 * `SameSite=Strict` session cookie with it, as they do with the requests
 * of a sibling host such as the public site (ADR 0014).
 *
 * A real server on the loopback interface rather than a Playwright route:
 * Chromium's Local Network Access treats a page that a route fulfils as
 * coming from outside, and refuses its request to the loopback address of
 * the admin before it is sent. Port 0: no fixed port to share.
 *
 * @returns the page's origin and how to stop its server.
 */
async function serveSiblingPage(): Promise<SiblingPage> {
  const page = `<!doctype html><title>x</title><script>
    fetch("${ADMIN_ORIGIN}${LOGOUT}", {
      method: "POST",
      mode: "no-cors",
      credentials: "include",
    })
  </script>`
  const server = createServer((_request, response) => {
    response.setHeader("content-type", "text/html; charset=utf-8")
    response.end(page)
  })
  await new Promise<void>((resolve) =>
    server.listen(0, "127.0.0.1", () => resolve())
  )
  const address = server.address()
  if (address === null || typeof address === "string") {
    throw new Error("the sibling page's server has no TCP address")
  }
  return {
    origin: `http://localhost:${address.port}`,
    close: () =>
      new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve()))
      ),
  }
}

test.describe.configure({ mode: "serial" })

test.describe("signing out", () => {
  // After the sign-out, a protected page asks /auth/me, which answers 401.
  test.use({ expectedHttpErrors: [{ status: 401, path: ME }] })

  test("a page on another origin of the same site cannot sign the admin out", async ({
    page,
    context,
  }) => {
    // ADR 0006 §3 as amended by ADR 0044: the browser attaches the session
    // cookie to a same-site page's write, so only the `Origin` check stops
    // it. Here, in a real browser: the page's logout carries the cookie and
    // its own origin, the server answers 403 and ends nothing. In this
    // project, because a check that let it through would end every session
    // of the seed account.
    await signIn(page)
    const sibling = await serveSiblingPage()
    const attacker = await context.newPage()
    try {
      const refused = attacker.waitForResponse(
        (response) => new URL(response.url()).pathname === LOGOUT
      )
      await attacker.goto(`${sibling.origin}/`)
      const answer = await refused
      // 403 is the `Origin` check: without the cookie the logout answers
      // 401, and the seed account meets the route's requirement, so the
      // browser did send the cookie with the sibling page's write and the
      // server refused it for its Origin. (Playwright shows neither the
      // request headers nor the body of a no-cors request; the server logs
      // the Origin it saw at WARN.)
      expect(answer.status()).toBe(403)
    } finally {
      await attacker.close()
      await sibling.close()
    }

    // The session holds: the admin still opens a protected page.
    await page.goto("/admin/about")
    await expect(page.getByTestId("about-page")).toBeVisible()
    await expect(page.getByTestId("sign-in-page")).toHaveCount(0)
  })

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
    // The browser sends the admin's own origin with the write, which the
    // server's `Origin` check of cookie-authenticated writes takes
    // (ADR 0006 §3, ADR 0044).
    expect(await answer.request().headerValue("origin")).toBe(ADMIN_ORIGIN)
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
