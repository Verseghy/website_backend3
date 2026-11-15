/**
 * The access-denied and authentication-error pages (ADR 0007 §7): their
 * Hungarian texts and buttons, and the way ra-core reaches them from a
 * resource page that `canAccess` refuses or cannot answer.
 *
 * The redirects are ra-core's (`useRequireAccess`, which its list
 * controller calls); the test mounts the kit's `<Admin>` with these pages,
 * the real auth provider over a `fetch` stub and probe resources, so a
 * ra-core release that moves the pages' paths fails here. No screen of
 * backend3 registers the probes, so their `resources.*` labels are the only
 * missing Hungarian keys the test lets through.
 */
import { QueryClient } from "@tanstack/react-query"
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { ListBase, Resource, type AuthProvider } from "ra-core"
import { RouterProvider, createMemoryRouter } from "react-router-dom"
import { describe, expect, it, vi } from "vitest"

import type { MeResponse } from "@/bindings/MeResponse"
import { Admin } from "@/components/admin"
import { createDataProvider } from "@/data-provider/data-provider"
import { createI18nProvider } from "@/i18n/provider"
import { createAppLayout } from "@/layout/AppLayout"
import { ADMIN_BASENAME, adminRoutes } from "@/router"

import { AccessDeniedPage, AuthenticationErrorPage } from "./AccessPages"
import { createAuthProvider } from "./auth-provider"

/** The probe resources, none of them backend3's. */
const PROBES = ["probe-listed", "probe-refused"] as const

const ME: MeResponse = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  name: "Próba Szerkesztő",
  email: "proba@example.test",
  scope: "admin",
  roles: [],
  permissions: ["edit labels"],
  is_admin: false,
  access: { "probe-listed": ["list"], "probe-refused": ["edit"] },
}

/**
 * A list screen for a probe resource, built on ra-core's `ListBase` like
 * every real list: its controller asks `canAccess` for `list` and sends a
 * refused or unanswered question to the pages under test.
 */
function ProbeList() {
  return (
    <ListBase>
      <div data-testid="probe-list" />
    </ListBase>
  )
}

/** An empty list, for a probe that the map allows. */
const EMPTY_LIST = {
  async getList() {
    return { data: [], total: 0 }
  },
}

/**
 * Mounts the admin with the probe resources and backend3's two pages at
 * `entries`' last entry, signed in as {@link ME}.
 *
 * @param entries - the memory router's history; the last one is shown.
 * @param adapt - changes the auth provider before it is mounted, for a
 *   `canAccess` that fails.
 * @returns the memory router and the missing keys reported.
 */
function mount(
  entries: string[],
  adapt: (provider: AuthProvider) => AuthProvider = (provider) => provider
) {
  const fetch = vi.fn<typeof globalThis.fetch>(
    async () =>
      new Response(JSON.stringify(ME), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
  )
  const missingKeys: string[] = []
  const router = createMemoryRouter(
    adminRoutes(
      <Admin
        basename={ADMIN_BASENAME}
        authProvider={adapt(
          createAuthProvider({ apiBaseUrl: "/api/admin", fetch })
        )}
        dataProvider={createDataProvider({
          apiBaseUrl: "/api/admin",
          resources: {
            "probe-listed": EMPTY_LIST,
            "probe-refused": EMPTY_LIST,
          },
        })}
        i18nProvider={createI18nProvider({
          reportMissingKeys: true,
          onMissingKey: (key) => {
            missingKeys.push(key)
          },
        })}
        // No retries: react-query asks a failed question three more times,
        // with a backoff of about seven seconds in all, before ra-core sees
        // the failure. The admin keeps those retries (main.tsx).
        queryClient={
          new QueryClient({ defaultOptions: { queries: { retry: false } } })
        }
        // The session's activity tracker (ADR 0006 §1) listens on a target
        // that nothing dispatches on, so it never calls the refresh.
        layout={createAppLayout(undefined, {
          target: new EventTarget(),
          now: () => 0,
          refresh: () => Promise.resolve(),
        })}
        requireAuth
        accessDenied={AccessDeniedPage}
        authenticationError={AuthenticationErrorPage}
        dashboard={() => <div data-testid="dashboard" />}
        disableTelemetry
      >
        {PROBES.map((name) => (
          <Resource key={name} name={name} list={ProbeList} />
        ))}
      </Admin>
    ),
    { initialEntries: entries }
  )
  render(<RouterProvider router={router} />)
  return { router, missingKeys }
}

/**
 * The missing keys that are not the probe resources' own labels.
 *
 * @param missingKeys - what the i18n provider reported.
 * @returns the keys that a real screen would lack.
 */
function realMissingKeys(missingKeys: string[]): string[] {
  return missingKeys.filter(
    (key) => !PROBES.some((name) => key.startsWith(`resources.${name}.`))
  )
}

describe("AccessDeniedPage", () => {
  it("is where ra-core sends a page whose list the access map refuses", async () => {
    const { router, missingKeys } = mount([`${ADMIN_BASENAME}/probe-refused`])
    const page = await screen.findByTestId("access-denied-page")
    expect(router.state.location.pathname).toBe(
      `${ADMIN_BASENAME}/access-denied`
    )
    expect(screen.queryByTestId("probe-list")).toBeNull()
    expect(within(page).getByRole("heading", { level: 1 })).toHaveTextContent(
      "Hozzáférés megtagadva"
    )
    expect(page).toHaveTextContent(
      "Nincs jogosultságod az oldal megtekintéséhez."
    )
    // Only the dashboard: back would return to the refused page.
    expect(within(page).getAllByRole("link")).toHaveLength(1)
    expect(
      within(page).getByRole("link", { name: "Irányítópult" })
    ).toHaveAttribute("href", `${ADMIN_BASENAME}/`)
    expect(within(page).queryByRole("button")).toBeNull()
    expect(realMissingKeys(missingKeys)).toEqual([])
  })

  it("leads to the dashboard", async () => {
    const { router } = mount([`${ADMIN_BASENAME}/probe-refused`])
    const page = await screen.findByTestId("access-denied-page")
    await userEvent
      .setup()
      .click(within(page).getByRole("link", { name: "Irányítópult" }))
    expect(await screen.findByTestId("dashboard")).toBeInTheDocument()
    expect(router.state.location.pathname).toBe(`${ADMIN_BASENAME}/`)
  })

  it("is never shown for a page the map allows", async () => {
    const { router } = mount([`${ADMIN_BASENAME}/probe-listed`])
    expect(await screen.findByTestId("probe-list")).toBeInTheDocument()
    expect(router.state.location.pathname).toBe(
      `${ADMIN_BASENAME}/probe-listed`
    )
    expect(screen.queryByTestId("access-denied-page")).toBeNull()
  })
})

describe("AuthenticationErrorPage", () => {
  /**
   * The provider with a `canAccess` that cannot answer, as when it could
   * not read `/auth/me`.
   *
   * @param provider - the real provider.
   * @returns the same provider, its `canAccess` rejecting.
   */
  const unanswerable = (provider: AuthProvider): AuthProvider => ({
    ...provider,
    canAccess: () => Promise.reject(new Error("no answer")),
  })

  it("is where ra-core sends a page whose question could not be answered", async () => {
    const { router, missingKeys } = mount(
      [`${ADMIN_BASENAME}/probe-listed`],
      unanswerable
    )
    const page = await screen.findByTestId("authentication-error-page")
    expect(router.state.location.pathname).toBe(
      `${ADMIN_BASENAME}/authentication-error`
    )
    expect(screen.queryByTestId("probe-list")).toBeNull()
    expect(within(page).getByRole("heading", { level: 1 })).toHaveTextContent(
      "Hitelesítési hiba"
    )
    expect(page).toHaveTextContent(
      "A hitelesítési szerver hibát jelzett, ezért a belépési adataidat nem sikerült ellenőrizni."
    )
    expect(
      within(page).getByRole("button", { name: "Vissza" })
    ).toBeInTheDocument()
    expect(
      within(page).getByRole("link", { name: "Irányítópult" })
    ).toHaveAttribute("href", `${ADMIN_BASENAME}/`)
    expect(realMissingKeys(missingKeys)).toEqual([])
  })

  it("goes back to the page it came from, which asks again", async () => {
    // Opened directly after the dashboard, as if ra-core had pushed it.
    const { router } = mount([
      `${ADMIN_BASENAME}/`,
      `${ADMIN_BASENAME}/authentication-error`,
    ])
    const page = await screen.findByTestId("authentication-error-page")
    await userEvent
      .setup()
      .click(within(page).getByRole("button", { name: "Vissza" }))
    expect(await screen.findByTestId("dashboard")).toBeInTheDocument()
    expect(router.state.location.pathname).toBe(`${ADMIN_BASENAME}/`)
  })
})
