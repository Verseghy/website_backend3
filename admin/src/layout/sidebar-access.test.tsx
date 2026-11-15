/**
 * The sidebar shows only the resources whose list the signed-in account may
 * open (ADR 0007 §7, NAV-01), and follows a changed access map after a
 * `403` without signing the user out.
 *
 * The kit's `app-sidebar.tsx` asks `canAccess` for `list` for every
 * registered resource, and the auth provider answers from the access map of
 * `/auth/me`. The test mounts the kit's `<Admin>` with backend3's layout, the
 * real auth provider over a `fetch` stub and three probe resources: one the
 * account may list, one it may only edit, and one the map leaves out. No
 * screen of backend3 registers them, so their `resources.*` labels are the
 * only missing Hungarian keys the test lets through.
 */
import { QueryClient } from "@tanstack/react-query"
import { act, render, screen, waitFor, within } from "@testing-library/react"
import { Resource } from "ra-core"
import { RouterProvider, createMemoryRouter } from "react-router-dom"
import { describe, expect, it, vi } from "vitest"

import { invalidateAccessChecks } from "@/auth/access-checks"
import { createAuthProvider } from "@/auth/auth-provider"
import type { MeResponse } from "@/bindings/MeResponse"
import { Admin } from "@/components/admin"
import { createDataProvider } from "@/data-provider/data-provider"
import { createI18nProvider } from "@/i18n/provider"
import { ADMIN_BASENAME, adminRoutes } from "@/router"

import { createAppLayout } from "./AppLayout"

/** The probe resources, none of them backend3's. */
const PROBES = ["probe-listed", "probe-edit-only", "probe-absent"] as const

const ME: MeResponse = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  name: "Próba Szerkesztő",
  email: "proba@example.test",
  scope: "admin",
  roles: [],
  permissions: ["edit labels"],
  is_admin: false,
  access: {
    "probe-listed": ["list", "show"],
    "probe-edit-only": ["edit"],
  },
}

/** A list screen for a probe resource; never opened here. */
function ProbeList() {
  return <div data-testid="probe-list" />
}

/**
 * Mounts the admin with the probe resources at `/admin`, signed in as an
 * account whose `/auth/me` answer is `me()` at each read.
 *
 * @param me - returns the current `/auth/me` answer.
 * @returns the auth provider, its watched `canAccess`, the stub and the
 *   missing keys reported.
 */
function mount(me: () => MeResponse) {
  const fetch = vi.fn<typeof globalThis.fetch>(
    async () =>
      new Response(JSON.stringify(me()), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
  )
  const missingKeys: string[] = []
  const queryClient = new QueryClient()
  const authProvider = createAuthProvider({
    apiBaseUrl: "/api/admin",
    fetch,
    onAccessChange: () => {
      void invalidateAccessChecks(queryClient)
    },
  })
  // Watched before the first render, so a test can wait until every
  // question of the sidebar has been answered.
  const canAccess = vi.spyOn(authProvider, "canAccess")
  // The session's activity tracker (ADR 0006 §1) listens on a target that
  // nothing dispatches on, so it never calls the refresh.
  const AppLayout = createAppLayout(undefined, {
    target: new EventTarget(),
    now: () => 0,
    refresh: () => Promise.resolve(),
  })
  const router = createMemoryRouter(
    adminRoutes(
      <Admin
        basename={ADMIN_BASENAME}
        authProvider={authProvider}
        dataProvider={createDataProvider({
          apiBaseUrl: "/api/admin",
          resources: {},
        })}
        i18nProvider={createI18nProvider({
          reportMissingKeys: true,
          onMissingKey: (key) => {
            missingKeys.push(key)
          },
        })}
        queryClient={queryClient}
        layout={AppLayout}
        requireAuth
        dashboard={() => <div data-testid="dashboard" />}
        disableTelemetry
      >
        {PROBES.map((name) => (
          <Resource key={name} name={name} list={ProbeList} />
        ))}
      </Admin>
    ),
    { initialEntries: [ADMIN_BASENAME] }
  )
  render(<RouterProvider router={router} />)
  return { authProvider, canAccess, fetch, missingKeys }
}

/**
 * The links of the sidebar's menu, by their target path.
 *
 * @returns the `href` of every link in the sidebar's navigation.
 */
function sidebarTargets(): string[] {
  const sidebar = document.querySelector('[data-sidebar="content"]')
  expect(sidebar).not.toBeNull()
  return within(sidebar as HTMLElement)
    .queryAllByRole("link")
    .map((link) => link.getAttribute("href") ?? "")
}

describe("the sidebar", () => {
  it("lists only the resources whose list the access map allows", async () => {
    const { fetch, missingKeys } = mount(() => ME)
    await screen.findByTestId("dashboard")
    await waitFor(() => {
      expect(sidebarTargets()).toEqual([
        `${ADMIN_BASENAME}/`,
        `${ADMIN_BASENAME}/probe-listed`,
      ])
    })
    // The questions were answered from checkAuth's /auth/me, with no
    // request of their own; a later read is ra-core's own checkAuth.
    expect(
      fetch.mock.calls.every(([url]) => url === "/api/admin/auth/me")
    ).toBe(true)
    expect(
      missingKeys.filter(
        (key) => !PROBES.some((name) => key.startsWith(`resources.${name}.`))
      )
    ).toEqual([])
  })

  it("lists nothing but the dashboard for an account with no access", async () => {
    const { canAccess } = mount(() => ({ ...ME, permissions: [], access: {} }))
    await screen.findByTestId("dashboard")
    // Wait until the sidebar asked about each probe and got its answer, and
    // no skeleton is left: an empty sidebar is then the answer, not a
    // sidebar that has not asked yet.
    await waitFor(() => {
      expect(
        canAccess.mock.calls.filter(([params]) => params.action === "list")
          .length
      ).toBeGreaterThanOrEqual(PROBES.length)
    })
    await expect(
      Promise.all(canAccess.mock.results.map((result) => result.value))
    ).resolves.toEqual(canAccess.mock.results.map(() => false))
    await waitFor(() => {
      expect(
        document.querySelector(
          '[data-sidebar="content"] [data-slot="skeleton"]'
        )
      ).toBeNull()
    })
    expect(sidebarTargets()).toEqual([`${ADMIN_BASENAME}/`])
  })

  it("drops an entry after a 403 shows the access changed, and keeps the session", async () => {
    let me = ME
    const { authProvider } = mount(() => me)
    await waitFor(() => {
      expect(sidebarTargets()).toContain(`${ADMIN_BASENAME}/probe-listed`)
    })
    // The account loses the resource in another session; a call to it is
    // refused with 403, and ra-core hands that to checkError.
    me = { ...ME, permissions: [], access: {} }
    await act(async () => {
      await expect(
        authProvider.checkError({ status: 403, message: "errors.forbidden" })
      ).resolves.toBeUndefined()
    })
    await waitFor(() => {
      expect(sidebarTargets()).toEqual([`${ADMIN_BASENAME}/`])
    })
    // Still signed in: the dashboard stays.
    expect(screen.getByTestId("dashboard")).toBeInTheDocument()
  })
})
