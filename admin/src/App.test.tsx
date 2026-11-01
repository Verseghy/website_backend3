/**
 * The admin root mounted the way `main.tsx` mounts it, but on a memory router
 * built from the same route table (`adminRoutes`), with the real auth
 * provider over a `fetch` stub that plays the admin API's `/auth` routes.
 */
import type { ReactNode } from "react"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { Route, RouterProvider, createMemoryRouter } from "react-router-dom"
import { describe, expect, it, vi } from "vitest"

import { AdminApp } from "@/App"
import { createAuthProvider } from "@/auth/auth-provider"
import type { MeResponse } from "@/bindings/MeResponse"
import { createDataProvider } from "@/data-provider/data-provider"
import { createI18nProvider } from "@/i18n/provider"
import { adminRoutes } from "@/router"

const SHA = "fedcba9876543210fedcba9876543210fedcba98"

/**
 * How long a test waits for the page that follows a sign-out or the end of a
 * session. ra-core navigates after `logout` in a `setTimeout`, then clears its
 * caches, and React swaps the whole layout for the sign-in page; on a loaded
 * machine that takes longer than Testing Library's default of one second.
 */
const AFTER_SIGN_OUT = { timeout: 5000 }

const ME: MeResponse = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  name: "Próba Szerkesztő",
  email: "proba@example.test",
  scope: "admin",
}

/**
 * A JSON answer.
 *
 * @param status - the HTTP status.
 * @param body - the body.
 * @returns a `Response`.
 */
function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/**
 * A stand-in for the admin API's `/auth` routes: `/auth/me` answers the
 * account while `signedIn` is true and `401 unauthorized` otherwise,
 * `/auth/login` signs in with `password`, and `/auth/logout` ends the
 * session (`204`), or answers `401` when there is none.
 */
class FakeAuthApi {
  /** Whether the browser holds a session cookie. */
  signedIn: boolean
  /** The `fetch` stub to hand to the auth provider. */
  readonly fetch = vi.fn<typeof globalThis.fetch>(async (input, init) => {
    const url = String(input)
    if (url === "/api/admin/auth/me") {
      return this.signedIn
        ? json(200, ME)
        : json(401, {
            error: "unauthorized",
            description: "d",
            field: null,
            params: {},
          })
    }
    if (url === "/api/admin/auth/login" && init?.method === "POST") {
      const { password } = JSON.parse(String(init.body)) as {
        password: string
      }
      if (password !== "helyes jelszó") {
        return json(401, {
          error: "invalid_credentials",
          description: "d",
          field: null,
          params: {},
        })
      }
      this.signedIn = true
      return json(200, { step: "done" })
    }
    if (url === "/api/admin/auth/logout" && init?.method === "POST") {
      if (!this.signedIn) {
        return json(401, {
          error: "unauthorized",
          description: "d",
          field: null,
          params: {},
        })
      }
      this.signedIn = false
      return new Response(null, { status: 204 })
    }
    return json(404, {
      error: "not_found",
      description: "d",
      field: null,
      params: {},
    })
  })

  /**
   * @param signedIn - whether the browser starts with a session.
   */
  constructor(signedIn: boolean) {
    this.signedIn = signedIn
  }
}

/**
 * Mounts the admin at `entry`.
 *
 * @param entry - the initial URL, path plus query.
 * @param developmentRoutes - routes a development build adds, as `main.tsx`
 *   passes them.
 * @param api - the fake `/auth` routes; signed in unless the test says
 *   otherwise.
 * @returns the memory router, to inspect its location.
 */
function mountAdmin(
  entry: string,
  developmentRoutes?: ReactNode,
  api: FakeAuthApi = new FakeAuthApi(true)
) {
  const router = createMemoryRouter(
    adminRoutes(
      <AdminApp
        dataProvider={createDataProvider({
          apiBaseUrl: "/api/admin",
          resources: {},
        })}
        authProvider={createAuthProvider({
          apiBaseUrl: "/api/admin",
          fetch: api.fetch,
        })}
        i18nProvider={createI18nProvider({ reportMissingKeys: true })}
        sourceCommit={SHA}
        developmentRoutes={developmentRoutes}
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

  it("renders a development route inside the layout when main.tsx passes one", async () => {
    mountAdmin(
      "/admin/clipboard-recorder",
      <Route
        path="/clipboard-recorder"
        element={<div data-testid="development-route" />}
      />
    )
    expect(await screen.findByTestId("development-route")).toBeInTheDocument()
    expect(screen.getByRole("contentinfo")).toBeInTheDocument()
  })
})

describe("AdminApp without a session", () => {
  it("sends a visitor at /admin to the Hungarian sign-in page", async () => {
    const router = mountAdmin("/admin", undefined, new FakeAuthApi(false))
    expect(
      await screen.findByRole("heading", { level: 1, name: "Bejelentkezés" })
    ).toBeInTheDocument()
    expect(router.state.location.pathname).toBe("/admin/login")
    expect(screen.queryByTestId("dashboard")).not.toBeInTheDocument()
  })

  it.each([
    "/admin/about",
    "/admin/posts/12/show",
    "/admin/media?path=uploads/posts",
  ])("renders no layout page at %s before the sign-in", async (entry) => {
    const router = mountAdmin(entry, undefined, new FakeAuthApi(false))
    await screen.findByTestId("sign-in-page")
    expect(router.state.location.pathname).toBe("/admin/login")
    expect(screen.queryByTestId("about-page")).not.toBeInTheDocument()
    expect(screen.queryByText("Nem található")).not.toBeInTheDocument()
    // The only footer is the sign-in page's own, with the source link only.
    expect(
      within(screen.getByRole("contentinfo")).queryByRole("link", {
        name: "Névjegy",
      })
    ).not.toBeInTheDocument()
  })

  it("keeps a development route behind the sign-in too", async () => {
    const router = mountAdmin(
      "/admin/clipboard-recorder",
      <Route
        path="/clipboard-recorder"
        element={<div data-testid="development-route" />}
      />,
      new FakeAuthApi(false)
    )
    await screen.findByTestId("sign-in-page")
    expect(router.state.location.pathname).toBe("/admin/login")
    expect(screen.queryByTestId("development-route")).not.toBeInTheDocument()
  })

  it("returns to the page the visitor asked for after the sign-in", async () => {
    const api = new FakeAuthApi(false)
    const router = mountAdmin("/admin/about", undefined, api)
    await screen.findByTestId("sign-in-page")

    const user = userEvent.setup()
    await user.type(screen.getByLabelText(/E-mail-cím/), ME.email)
    await user.type(screen.getByLabelText(/Jelszó/), "helyes jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))

    expect(await screen.findByTestId("about-page")).toBeInTheDocument()
    expect(router.state.location.pathname).toBe("/admin/about")
    expect(api.signedIn).toBe(true)
  })

  it("lands on the dashboard after a sign-in from /admin", async () => {
    const router = mountAdmin("/admin", undefined, new FakeAuthApi(false))
    await screen.findByTestId("sign-in-page")

    const user = userEvent.setup()
    await user.type(screen.getByLabelText(/E-mail-cím/), ME.email)
    await user.type(screen.getByLabelText(/Jelszó/), "helyes jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))

    expect(await screen.findByTestId("dashboard")).toBeInTheDocument()
    expect(router.state.location.pathname).toBe("/admin")
  })

  it("stays on the sign-in page with the generic message after a wrong password", async () => {
    const router = mountAdmin("/admin", undefined, new FakeAuthApi(false))
    await screen.findByTestId("sign-in-page")

    const user = userEvent.setup()
    await user.type(screen.getByLabelText(/E-mail-cím/), ME.email)
    await user.type(screen.getByLabelText(/Jelszó/), "rossz jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Hibás e-mail-cím vagy jelszó."
    )
    expect(router.state.location.pathname).toBe("/admin/login")
  })
})

describe("AdminApp with a session", () => {
  it("asks /auth/me before it renders a page", async () => {
    const api = new FakeAuthApi(true)
    mountAdmin("/admin", undefined, api)
    await screen.findByTestId("dashboard")
    expect(api.fetch).toHaveBeenCalledWith(
      "/api/admin/auth/me",
      expect.objectContaining({ method: "GET" })
    )
  })

  it("shows backend3's user menu in the header", async () => {
    mountAdmin("/admin")
    await screen.findByTestId("dashboard")
    const trigger = screen.getByRole("button", { name: "Profil" })
    expect(trigger.closest("header")).not.toBeNull()
    await userEvent.setup().click(trigger)
    expect(await screen.findByText(ME.name)).toBeInTheDocument()
    expect(
      screen.getByRole("menuitem", { name: "Kijelentkezés" })
    ).toBeInTheDocument()
  })

  it("signs out through the user menu: the server ends the session, and every page then asks for a sign-in", async () => {
    const api = new FakeAuthApi(true)
    const router = mountAdmin("/admin/about", undefined, api)
    await screen.findByTestId("about-page")

    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Profil" }))
    await user.click(
      await screen.findByRole("menuitem", { name: "Kijelentkezés" })
    )

    expect(
      await screen.findByTestId("sign-in-page", {}, AFTER_SIGN_OUT)
    ).toBeInTheDocument()
    expect(router.state.location.pathname).toBe("/admin/login")
    expect(api.signedIn).toBe(false)
    expect(
      api.fetch.mock.calls.filter(
        ([url, init]) =>
          url === "/api/admin/auth/logout" && init?.method === "POST"
      )
    ).toHaveLength(1)

    // The session is gone on the server, so the dashboard asks again.
    await router.navigate("/admin/")
    await waitFor(
      () => expect(router.state.location.pathname).toBe("/admin/login"),
      AFTER_SIGN_OUT
    )
    expect(screen.queryByTestId("dashboard")).not.toBeInTheDocument()
  })

  it("lands on the dashboard after the next sign-in, not on the page signed out from", async () => {
    const api = new FakeAuthApi(true)
    const router = mountAdmin("/admin/about", undefined, api)
    await screen.findByTestId("about-page")
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Profil" }))
    await user.click(
      await screen.findByRole("menuitem", { name: "Kijelentkezés" })
    )
    await screen.findByTestId("sign-in-page", {}, AFTER_SIGN_OUT)

    await user.type(screen.getByLabelText(/E-mail-cím/), ME.email)
    await user.type(screen.getByLabelText(/Jelszó/), "helyes jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))

    expect(
      await screen.findByTestId("dashboard", {}, AFTER_SIGN_OUT)
    ).toBeInTheDocument()
    // ra-core's default `afterLoginUrl` ("/") under the basename.
    expect(router.state.location.pathname).toBe("/admin/")
  })

  it("lets ra-core send the user to the sign-in page when the session ends", async () => {
    const api = new FakeAuthApi(true)
    const router = mountAdmin("/admin/about", undefined, api)
    await screen.findByTestId("about-page")

    // The session ends (expiry, deactivation); the next page checks it.
    api.signedIn = false
    await router.navigate("/admin/")
    await waitFor(
      () => expect(router.state.location.pathname).toBe("/admin/login"),
      AFTER_SIGN_OUT
    )
    expect(
      await screen.findByTestId("sign-in-page", {}, AFTER_SIGN_OUT)
    ).toBeInTheDocument()
  })
})
