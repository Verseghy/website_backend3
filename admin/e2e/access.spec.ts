/**
 * Permission-gated navigation in a real browser against backend3 (ADR 0007
 * §7, NAV-01): `/auth/me` carries the seed account's access map, the
 * sidebar links only the resources that the map lets it list, the pages
 * that are no resource (the dashboard, the about page) stay open, and the
 * two pages ra-core sends a refused resource page to are backend3's
 * Hungarian ones. Every test also passes the CSP and console guard of
 * `fixtures.ts`.
 *
 * `authProvider.canAccess` answers from the access map without asking the
 * server, so the sidebar's questions add no request: the test counts the
 * admin API requests of a page load. The server's access table
 * (`RESOURCE_ACCESS` in `src/domain/authz.rs`) has the labels row, so the
 * seed account's map is `{labels: ["list", "show"]}` and the check of the
 * map's lists runs on it. The admin registers no resource yet, so the
 * sidebar holds only the dashboard and the check of its resource links
 * starts to bite when the labels screen registers its resource. The
 * redirect to the access-denied page needs a resource and an account that
 * may not list it, which the seed does not create, so the test opens the
 * pages directly; `src/auth/AccessPages.test.tsx` drives the redirects
 * through ra-core. The
 * 403 path (the Hungarian message, the refetch of `/auth/me`, the user kept
 * signed in) needs such an account too; the unit tests
 * (`src/data-provider/refusal.test.tsx`, `src/layout/sidebar-access.test.tsx`)
 * drive it through ra-core.
 *
 * It needs the seed account: `LOCO_ENV=e2e cargo loco db seed` after the
 * server has migrated `backend3_e2e` (the E2E CI job does both).
 */
import { expect, signIn, test } from "./fixtures.ts"

/** The actions an access map may list (ADR 0007 §7), in the server's order. */
const ACTIONS = [
  "list",
  "show",
  "create",
  "edit",
  "delete",
  "publish",
  "restore",
  "purge",
  "history",
]

test.describe("the access map", () => {
  test("decides the sidebar's resources, which add no request", async ({
    page,
  }) => {
    await signIn(page)
    // A fresh page load, with every admin API request it makes.
    const apiPaths: string[] = []
    page.on("request", (request) => {
      const path = new URL(request.url()).pathname
      if (path.startsWith("/api/admin/")) {
        apiPaths.push(path)
      }
    })
    const me = page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === "/api/admin/auth/me" &&
        response.status() === 200
    )
    await page.goto("/admin")
    const body = (await (await me).json()) as { access?: unknown }
    await expect(page.getByTestId("dashboard")).toBeVisible()

    // The seed account holds the admin role, so the map holds every
    // action of every row of the server's access table, each list in the
    // server's order.
    expect(typeof body.access).toBe("object")
    const access = body.access as Record<string, string[]>
    for (const [resource, actions] of Object.entries(access)) {
      expect(actions.length, resource).toBeGreaterThan(0)
      expect(
        [...actions].sort((a, b) => ACTIONS.indexOf(a) - ACTIONS.indexOf(b)),
        resource
      ).toEqual(actions)
    }

    // Every resource link of the sidebar is one the map lets the account
    // list; the dashboard link is always there.
    const sidebar = page.locator('[data-sidebar="content"]')
    await expect(
      sidebar.getByRole("link", { name: "Irányítópult" })
    ).toHaveAttribute("href", "/admin/")
    await expect(sidebar.locator('[data-slot="skeleton"]')).toHaveCount(0)
    for (const href of await sidebar
      .getByRole("link")
      .evaluateAll((links) => links.map((link) => link.getAttribute("href")))) {
      if (href === "/admin/") {
        continue
      }
      const resource = (href ?? "").replace(/^\/admin\//, "").split("/")[0]
      expect(access[resource] ?? [], `sidebar link ${href}`).toContain("list")
    }

    // The page load asked /auth/me and nothing else: the sidebar's access
    // questions were answered from it.
    expect(new Set(apiPaths)).toEqual(new Set(["/api/admin/auth/me"]))
  })

  test("leaves the pages that are no resource open", async ({ page }) => {
    await signIn(page)
    await page.goto("/admin/about")
    await expect(page.getByTestId("about-page")).toBeVisible()
    await page
      .locator('[data-sidebar="content"]')
      .getByRole("link", { name: "Irányítópult" })
      .click()
    await expect(page.getByTestId("dashboard")).toBeVisible()
    expect(new URL(page.url()).pathname).toBe("/admin/")
  })

  test("shows the Hungarian access-denied page, which leads to the dashboard", async ({
    page,
  }) => {
    await signIn(page)
    await page.goto("/admin/access-denied")
    const denied = page.getByTestId("access-denied-page")
    await expect(
      denied.getByRole("heading", { name: "Hozzáférés megtagadva" })
    ).toBeVisible()
    await expect(denied).toContainText(
      "Nincs jogosultságod az oldal megtekintéséhez."
    )
    await denied.getByRole("link", { name: "Irányítópult" }).click()
    await expect(page.getByTestId("dashboard")).toBeVisible()
    expect(new URL(page.url()).pathname).toBe("/admin/")
  })

  test("shows the Hungarian authentication-error page, whose back button returns", async ({
    page,
  }) => {
    await signIn(page)
    await page.goto("/admin/about")
    await expect(page.getByTestId("about-page")).toBeVisible()
    await page.goto("/admin/authentication-error")
    const failed = page.getByTestId("authentication-error-page")
    await expect(
      failed.getByRole("heading", { name: "Hitelesítési hiba" })
    ).toBeVisible()
    await failed.getByRole("button", { name: "Vissza" }).click()
    await expect(page.getByTestId("about-page")).toBeVisible()
    expect(new URL(page.url()).pathname).toBe("/admin/about")
  })
})
