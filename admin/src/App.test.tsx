/**
 * The admin root mounted the way `main.tsx` mounts it, but on a memory router
 * built from the same route table (`adminRoutes`).
 */
import { render, screen, within } from "@testing-library/react"
import { RouterProvider, createMemoryRouter } from "react-router-dom"
import { describe, expect, it } from "vitest"

import { AdminApp } from "@/App"
import { createDataProvider } from "@/data-provider/data-provider"
import { createI18nProvider } from "@/i18n/provider"
import { adminRoutes } from "@/router"

const SHA = "fedcba9876543210fedcba9876543210fedcba98"

/**
 * Mounts the admin at `entry`.
 *
 * @param entry - the initial URL, path plus query.
 * @returns the memory router, to inspect its location.
 */
function mountAdmin(entry: string) {
  const router = createMemoryRouter(
    adminRoutes(
      <AdminApp
        dataProvider={createDataProvider({
          apiBaseUrl: "/api/admin",
          resources: {},
        })}
        i18nProvider={createI18nProvider({ reportMissingKeys: true })}
        sourceCommit={SHA}
      />
    ),
    { initialEntries: [entry] }
  )
  render(<RouterProvider router={router} />)
  return router
}

describe("AdminApp", () => {
  it("renders the Hungarian dashboard at /admin", async () => {
    mountAdmin("/admin")
    expect(await screen.findByTestId("dashboard")).toHaveTextContent(
      "Üdv a Verseghy-adminban!"
    )
    expect(screen.getAllByText("Verseghy-admin").length).toBeGreaterThan(0)
  })

  it("prefixes ra-core's links with /admin exactly once (basename on the admin root only)", async () => {
    mountAdmin("/admin")
    await screen.findByTestId("dashboard")
    const footer = screen.getByRole("contentinfo")
    expect(
      within(footer).getByRole("link", { name: "Névjegy" })
    ).toHaveAttribute("href", "/admin/about")
  })

  it("keeps the kit sidebar's logo and dashboard links under /admin", async () => {
    mountAdmin("/admin/about")
    await screen.findByTestId("about-page")
    expect(screen.getByRole("link", { name: "Irányítópult" })).toHaveAttribute(
      "href",
      "/admin/"
    )
    expect(
      screen.getByRole("link", { name: "Verseghy-admin" })
    ).toHaveAttribute("href", "/admin/")
  })

  it("marks the dashboard menu item active on the dashboard only", async () => {
    mountAdmin("/admin")
    await screen.findByTestId("dashboard")
    expect(screen.getByRole("link", { name: "Irányítópult" })).toHaveAttribute(
      "data-active"
    )
  })

  it("does not mark the dashboard menu item active elsewhere", async () => {
    mountAdmin("/admin/about")
    await screen.findByTestId("about-page")
    expect(
      screen.getByRole("link", { name: "Irányítópult" })
    ).not.toHaveAttribute("data-active")
  })

  it("renders the footer as the page's contentinfo landmark, outside <main>", async () => {
    mountAdmin("/admin")
    const footer = await screen.findByRole("contentinfo")
    // jsdom's role lookup (aria-query) reports every <footer> as contentinfo.
    // Browsers follow HTML-AAM instead: a footer inside main, article, aside,
    // nav or section is a generic element, which is not a landmark, and the
    // E2E suite's getByRole("contentinfo") would not find it.
    expect(footer.tagName).toBe("FOOTER")
    expect(footer.closest("main, article, aside, nav, section")).toBeNull()
  })

  it("renders the footer source link to the running commit", async () => {
    mountAdmin("/admin")
    const footer = await screen.findByRole("contentinfo")
    expect(
      within(footer).getByRole("link", { name: "Forráskód" })
    ).toHaveAttribute(
      "href",
      `https://github.com/Verseghy/website_backend3/tree/${SHA}`
    )
  })

  it("opens a deep client route directly, with browser-style (not hash) locations", async () => {
    const router = mountAdmin("/admin/about")
    expect(await screen.findByTestId("about-page")).toHaveTextContent(
      `Verzió (commit): ${SHA}`
    )
    expect(router.state.location.pathname).toBe("/admin/about")
    expect(router.state.location.hash).toBe("")
  })

  it("renders the Hungarian not-found screen for an unknown record route", async () => {
    mountAdmin("/admin/posts/12/show")
    expect(await screen.findByText("Nem található")).toBeInTheDocument()
  })

  it("keeps a media-library deep link's query intact (ADR 0004 §3)", async () => {
    const router = mountAdmin("/admin/media?path=uploads/posts")
    await screen.findByRole("contentinfo")
    expect(router.state.location.pathname).toBe("/admin/media")
    expect(router.state.location.search).toBe("?path=uploads/posts")
  })
})
