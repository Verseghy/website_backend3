/**
 * The Hungarian sign-in screen in a real browser against backend3 (ADR 0006
 * §1, §4): the seed account signs in and reaches the dashboard, a protected
 * route sends a visitor without a session to the sign-in screen and back,
 * and a wrong password shows the one generic message. The session cookie is
 * checked as the browser stores it. Every test also passes the CSP and
 * console guard of `fixtures.ts`; the signed-out tests list the `401`
 * answers they expect, and nothing else.
 *
 * It needs the seed account: `LOCO_ENV=e2e cargo loco db seed` after the
 * server has migrated `backend3_e2e` (the E2E CI job does both).
 */
import { expect, signIn, test } from "./fixtures.ts"
import { SEED_ACCOUNT } from "./seed-account.ts"

/** The cookie that carries the admin session (ADR 0006 §1). */
const SESSION_COOKIE = "__Host-admin_session"

test.describe("the sign-in screen", () => {
  test("signs the seed account in and opens the dashboard", async ({
    page,
  }) => {
    const response = await page.goto("/admin/login")
    expect(response?.status()).toBe(200)
    expect(response?.headers()["content-security-policy"]).toContain(
      "default-src 'self'"
    )
    await expect(
      page.getByRole("heading", { level: 1, name: "Bejelentkezés" })
    ).toBeVisible()
    await expect(page.locator("html")).toHaveAttribute("lang", "hu")

    await page.getByLabel("E-mail-cím").fill(SEED_ACCOUNT.email)
    await page.getByLabel("Jelszó").fill(SEED_ACCOUNT.password)
    await page.getByRole("button", { name: "Bejelentkezés" }).click()

    await expect(page.getByTestId("dashboard")).toContainText(
      "Üdv a Verseghy-adminban!"
    )
    expect(new URL(page.url()).pathname).toBe("/admin/")

    // The cookie holds the session across a reload: checkAuth asks
    // /auth/me again and gets the account.
    await page.reload()
    await expect(page.getByTestId("dashboard")).toBeVisible()
  })

  test("stores the session in a cookie that page scripts cannot read", async ({
    page,
    context,
  }) => {
    await signIn(page)

    const cookie = (await context.cookies()).find(
      (candidate) => candidate.name === SESSION_COOKIE
    )
    expect(cookie, `${SESSION_COOKIE} after the sign-in`).toBeDefined()
    expect(cookie).toMatchObject({
      httpOnly: true,
      secure: true,
      sameSite: "Strict",
      path: "/",
    })
    // A __Host- cookie has no Domain: the browser binds it to this host.
    expect(cookie?.domain).toBe("localhost")
    expect(await page.evaluate(() => document.cookie)).not.toContain(
      SESSION_COOKIE
    )
  })

  test("offers the source of the running build to visitors without a session", async ({
    page,
  }) => {
    await page.goto("/admin/login")
    const link = page
      .getByRole("contentinfo")
      .getByRole("link", { name: "Forráskód" })
    await expect(link).toHaveAttribute(
      "href",
      /^https:\/\/github\.com\/Verseghy\/website_backend3\/tree\/([0-9a-f]{40}|master)$/
    )
  })

  test.describe("without a session", () => {
    // checkAuth asks /auth/me, which answers 401 until the sign-in.
    test.use({
      expectedHttpErrors: [{ status: 401, path: "/api/admin/auth/me" }],
    })

    test("sends a protected route to the sign-in screen and back after the sign-in", async ({
      page,
    }) => {
      await page.goto("/admin/about")

      await expect(
        page.getByRole("heading", { level: 1, name: "Bejelentkezés" })
      ).toBeVisible()
      expect(new URL(page.url()).pathname).toBe("/admin/login")
      await expect(page.getByTestId("about-page")).toHaveCount(0)

      await page.getByLabel("E-mail-cím").fill(SEED_ACCOUNT.email)
      await page.getByLabel("Jelszó").fill(SEED_ACCOUNT.password)
      await page.getByRole("button", { name: "Bejelentkezés" }).click()

      await expect(page.getByTestId("about-page")).toBeVisible()
      expect(new URL(page.url()).pathname).toBe("/admin/about")
    })

    test("sends the dashboard to the sign-in screen", async ({ page }) => {
      await page.goto("/admin")
      await expect(page.getByTestId("sign-in-page")).toBeVisible()
      expect(new URL(page.url()).pathname).toBe("/admin/login")
      await expect(page.getByTestId("dashboard")).toHaveCount(0)
    })
  })

  test.describe("with a wrong password", () => {
    // The sign-in answers 401 invalid_credentials.
    test.use({
      expectedHttpErrors: [{ status: 401, path: "/api/admin/auth/login" }],
    })

    test("shows the one generic Hungarian message and sets no session", async ({
      page,
      context,
    }) => {
      await page.goto("/admin/login")
      await page.getByLabel("E-mail-cím").fill(SEED_ACCOUNT.email)
      await page.getByLabel("Jelszó").fill("egy rossz jelszó")
      await page.getByRole("button", { name: "Bejelentkezés" }).click()

      await expect(page.getByRole("alert")).toHaveText(
        "Hibás e-mail-cím vagy jelszó."
      )
      expect(new URL(page.url()).pathname).toBe("/admin/login")
      const cookies = await context.cookies()
      expect(cookies.map((cookie) => cookie.name)).not.toContain(SESSION_COOKIE)
    })

    test("answers an unknown address with the same message", async ({
      page,
    }) => {
      await page.goto("/admin/login")
      await page.getByLabel("E-mail-cím").fill("senki@example.org")
      await page.getByLabel("Jelszó").fill(SEED_ACCOUNT.password)
      await page.getByRole("button", { name: "Bejelentkezés" }).click()

      await expect(page.getByRole("alert")).toHaveText(
        "Hibás e-mail-cím vagy jelszó."
      )
    })
  })
})
