/**
 * The in-place sign-in in a real browser against backend3 (ADR 0006,
 * "Follow-up work": the admin must not drop an open page on a `401`; it
 * shows a sign-in dialog in place, then retries).
 *
 * The session ends the way it ends for an editor who left the tab for two
 * hours: the browser no longer sends a valid session cookie (the test
 * deletes it from its own browser context, which ends no other test's
 * session), and the next request answers `401`. Coming back to the tab,
 * react-query repeats ra-core's session check; its `401` from the real
 * `/api/admin/auth/me` opens the dialog over the unchanged page instead of
 * the sign-in page, the real `/api/admin/auth/login` signs the seed account
 * in again, and the check that waited is sent again and passes. The page's
 * element stays the same throughout, so a form on it would keep what was
 * typed. The save of a typed value through the data provider is covered by
 * `src/data-provider/expired-session-edit.test.tsx` until an admin screen
 * with a form exists; the first such screen (the label forms) adds it here.
 *
 * „Kilépés” posts `/auth/logout`, but only after this context's cookie is
 * gone: the request carries no session, the server answers `401` and ends
 * none. So nothing here ends a session on the server, and the file shares
 * the seed account with the other tests instead of joining
 * `e2e/sign-out.spec.ts`. It needs the seed account: `LOCO_ENV=e2e cargo
 * loco db seed` after the server has migrated `backend3_e2e`.
 */
import type { Page } from "@playwright/test"

import { expect, signIn, test } from "./fixtures.ts"
import { SEED_ACCOUNT } from "./seed-account.ts"

/** The cookie that carries the admin session (ADR 0006 §1). */
const SESSION_COOKIE = "__Host-admin_session"

// The `401` answers that an ended session gets, and that Chromium logs: the
// session check and the identity on the way back to the tab, the activity
// tracker's refresh on a click before the new sign-in, a wrong password,
// and „Kilépés”'s sign-out without a session. A list of more than one
// answer goes in Playwright's `[value, options]` form: Playwright reads any
// array whose second item is an object as that form, and would take the
// first answer for the list.
test.use({
  expectedHttpErrors: [
    [
      { status: 401, path: "/api/admin/auth/me" },
      { status: 401, path: "/api/admin/auth/refresh" },
      { status: 401, path: "/api/admin/auth/login" },
      { status: 401, path: "/api/admin/auth/logout" },
    ],
    { scope: "test" },
  ],
})

/**
 * Signs in, opens the about page and marks its element, so that a test can
 * tell the same element from a new one after the session ended.
 *
 * @param page - the test's page.
 */
async function openMarkedAboutPage(page: Page): Promise<void> {
  await signIn(page)
  await page.goto("/admin/about")
  const about = page.getByTestId("about-page")
  await expect(about).toBeVisible()
  // React never touches an attribute it did not render; a remounted page
  // would come without it.
  await about.evaluate((element) => {
    element.setAttribute("data-e2e-mark", "before-the-session-ended")
  })
}

/**
 * Ends the session in this browser context and brings the tab back into
 * view, as a return to a tab left open for hours does.
 *
 * @param page - the test's page.
 */
async function endSessionAndReturn(page: Page): Promise<void> {
  await page.context().clearCookies({ name: SESSION_COOKIE })
  // What the browser fires on the document when the tab becomes visible
  // again; react-query refetches the session check on it.
  await page.evaluate(() => {
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }))
  })
}

test.describe("the in-place sign-in when the session ends", () => {
  test("asks for the password over the unchanged page and goes on after the sign-in", async ({
    page,
  }) => {
    await openMarkedAboutPage(page)
    await endSessionAndReturn(page)

    const dialog = page.getByRole("dialog", { name: "Lejárt a munkamenet" })
    await expect(dialog).toBeVisible()
    await expect(dialog.getByLabel("E-mail-cím")).toHaveValue(
      SEED_ACCOUNT.email
    )
    await expect(dialog.getByLabel("Jelszó")).toBeFocused()
    // The page behind it is the same element, still at its address.
    await expect(page.getByTestId("about-page")).toHaveAttribute(
      "data-e2e-mark",
      "before-the-session-ended"
    )
    expect(new URL(page.url()).pathname).toBe("/admin/about")

    // A wrong password keeps the dialog open with the one generic message.
    await dialog.getByLabel("Jelszó").fill("not the password")
    await dialog.getByRole("button", { name: "Bejelentkezés" }).click()
    await expect(dialog.getByRole("alert")).toHaveText(
      "Hibás e-mail-cím vagy jelszó."
    )

    const session = page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === "/api/admin/auth/login" &&
        response.status() === 200
    )
    const accounts: number[] = []
    page.on("response", (response) => {
      if (new URL(response.url()).pathname === "/api/admin/auth/me") {
        accounts.push(response.status())
      }
    })
    await dialog.getByLabel("Jelszó").fill(SEED_ACCOUNT.password)
    await dialog.getByRole("button", { name: "Bejelentkezés" }).click()
    expect(await (await session).headerValue("set-cookie")).toMatch(
      /^__Host-admin_session=[^;]+; HttpOnly; Secure; SameSite=Strict; Path=\/; Max-Age=7200$/
    )
    await expect(dialog).toBeHidden()
    // Two answers with the account: the dialog's own check of who signed
    // in, and the session check that waited, sent again.
    await expect
      .poll(() => accounts.filter((status) => status === 200).length)
      .toBeGreaterThanOrEqual(2)

    // Never the sign-in page: the same page element at the same address.
    await expect(page.getByTestId("about-page")).toHaveAttribute(
      "data-e2e-mark",
      "before-the-session-ended"
    )
    expect(new URL(page.url()).pathname).toBe("/admin/about")
    // The new session is real: the next page opens without another sign-in.
    await page.getByRole("link", { name: "Irányítópult" }).click()
    await expect(page.getByTestId("dashboard")).toBeVisible()
  })

  test("signs out with the server's word and leads to the sign-in page when the user leaves the dialog", async ({
    page,
  }) => {
    await openMarkedAboutPage(page)
    await endSessionAndReturn(page)

    const dialog = page.getByRole("dialog", { name: "Lejárt a munkamenet" })
    await expect(dialog).toBeVisible()
    const signOut = page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === "/api/admin/auth/logout" &&
        response.request().method() === "POST"
    )
    await dialog.getByRole("button", { name: "Kilépés" }).click()
    // The server's word that no session is left: the cookie is gone, so
    // there is none to end.
    expect((await signOut).status()).toBe(401)

    await expect(page.getByTestId("sign-in-page")).toBeVisible()
    expect(new URL(page.url()).pathname).toBe("/admin/login")
    await expect(dialog).toBeHidden()
  })
})
