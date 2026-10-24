/**
 * The admin served by backend3: loading, browser-history deep links and the
 * `index.html` fallback (ADR 0004 §3), under the admin CSP (ADR 0023 §6).
 * Every test also passes the CSP and console guard of `fixtures.ts`.
 */
import { expect, test } from "./fixtures.ts"

test.describe("the admin served by backend3", () => {
  test("loads /admin with the Hungarian dashboard and the admin CSP", async ({
    page,
  }) => {
    const response = await page.goto("/admin")
    expect(response?.status()).toBe(200)
    // The Vite dev server sends no CSP: this proves backend3 served the page.
    expect(response?.headers()["content-security-policy"]).toContain(
      "default-src 'self'"
    )

    await expect(page.getByTestId("dashboard")).toContainText(
      "Üdv a Verseghy-adminban!"
    )
    await expect(page.locator("html")).toHaveAttribute("lang", "hu")
    await expect(page).toHaveTitle("Verseghy-admin")
    // Browser-history routing: ra-core's hash router would have added "#/".
    expect(new URL(page.url()).hash).toBe("")
  })

  test("reopens a deep client route after a reload", async ({ page }) => {
    await page.goto("/admin/about")
    await expect(page.getByTestId("about-page")).toBeVisible()

    await page.reload()

    await expect(page.getByTestId("about-page")).toBeVisible()
    expect(new URL(page.url()).pathname).toBe("/admin/about")
  })

  test("shows the same not-found screen for an unknown record route after a reload", async ({
    page,
  }) => {
    const response = await page.goto("/admin/posts/12/show")
    // The fallback serves index.html; the SPA, not the server, says "not found".
    expect(response?.status()).toBe(200)
    await expect(page.getByText("Nem található")).toBeVisible()

    await page.reload()

    await expect(page.getByText("Nem található")).toBeVisible()
    expect(new URL(page.url()).pathname).toBe("/admin/posts/12/show")
  })

  test("renders the SPA for a media-library deep link and keeps its query", async ({
    page,
  }) => {
    const response = await page.goto("/admin/media?path=uploads/posts")
    expect(response?.status()).toBe(200)
    await expect(page.getByRole("contentinfo")).toBeVisible()

    await page.reload()

    await expect(page.getByRole("contentinfo")).toBeVisible()
    const url = new URL(page.url())
    expect(url.pathname).toBe("/admin/media")
    expect(url.searchParams.get("path")).toBe("uploads/posts")
  })

  test("navigates client-side under the /admin basename", async ({ page }) => {
    await page.goto("/admin")
    await page
      .getByRole("contentinfo")
      .getByRole("link", { name: "Névjegy" })
      .click()

    await expect(page.getByTestId("about-page")).toBeVisible()
    expect(new URL(page.url()).pathname).toBe("/admin/about")

    await page.getByRole("link", { name: "Irányítópult" }).click()
    await expect(page.getByTestId("dashboard")).toBeVisible()
    expect(new URL(page.url()).pathname).toBe("/admin/")
  })

  test("links the footer to the source of the running build", async ({
    page,
  }) => {
    await page.goto("/admin")
    const link = page
      .getByRole("contentinfo")
      .getByRole("link", { name: "Forráskód" })
    await expect(link).toHaveAttribute(
      "href",
      /^https:\/\/github\.com\/Verseghy\/website_backend3\/tree\/([0-9a-f]{40}|master)$/
    )
  })
})
