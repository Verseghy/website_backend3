/**
 * `invalidateAccessChecks` against ra-core's real access hooks: a mounted
 * `useCanAccess` and `useCanAccessResources` show a cached answer until the
 * auth provider's `onAccessChange` invalidates it, and then follow the new
 * `/auth/me` answer at once (ADR 0007 §7). The test pins the query key that
 * `./access-checks.ts` relies on: a ra-core release that renames it fails
 * here.
 */
import { QueryClient } from "@tanstack/react-query"
import { act, render, screen, waitFor } from "@testing-library/react"
import {
  CoreAdminContext,
  TestMemoryRouter,
  useCanAccess,
  useCanAccessResources,
  type AuthProvider,
} from "ra-core"
import { describe, expect, it, vi } from "vitest"

import type { MeResponse } from "@/bindings/MeResponse"
import { createDataProvider } from "@/data-provider/data-provider"
import { createI18nProvider } from "@/i18n/provider"

import { CAN_ACCESS_QUERY_KEY, invalidateAccessChecks } from "./access-checks"
import { createAuthProvider } from "./auth-provider"

const ME: MeResponse = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  name: "Próba Szerkesztő",
  email: "proba@example.test",
  scope: "admin",
  roles: [],
  permissions: ["edit labels"],
  is_admin: false,
  access: { probes: ["list", "edit"] },
}

/** Shows one `useCanAccess` answer. */
function ProbeAccess() {
  const { canAccess, isPending } = useCanAccess({
    resource: "probes",
    action: "edit",
  })
  return (
    <output
      data-testid="edit"
      data-state={isPending ? "pending" : String(canAccess)}
    />
  )
}

/** Shows one `useCanAccessResources` answer. */
function ProbeResources() {
  const { canAccess, isPending } = useCanAccessResources({
    resources: ["probes"],
    action: "list",
  })
  return (
    <output
      data-testid="list"
      data-state={isPending ? "pending" : String(canAccess?.probes)}
    />
  )
}

/**
 * Mounts both probes under ra-core with `authProvider` and `queryClient`.
 *
 * @param authProvider - the provider ra-core asks.
 * @param queryClient - the client ra-core caches the answers in.
 */
function mount(authProvider: AuthProvider, queryClient: QueryClient) {
  render(
    <TestMemoryRouter>
      <CoreAdminContext
        authProvider={authProvider}
        dataProvider={createDataProvider({
          apiBaseUrl: "/api/admin",
          resources: {},
        })}
        i18nProvider={createI18nProvider({ reportMissingKeys: true })}
        queryClient={queryClient}
      >
        <ProbeAccess />
        <ProbeResources />
      </CoreAdminContext>
    </TestMemoryRouter>
  )
}

describe("invalidateAccessChecks", () => {
  it("names the prefix of ra-core's canAccess queries", () => {
    expect(CAN_ACCESS_QUERY_KEY).toEqual(["auth", "canAccess"])
  })

  it("makes the mounted access hooks follow a changed /auth/me answer", async () => {
    let me: MeResponse = ME
    const fetch = vi.fn<typeof globalThis.fetch>(
      async () =>
        new Response(JSON.stringify(me), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        })
    )
    const queryClient = new QueryClient()
    const authProvider = createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      onAccessChange: () => {
        void invalidateAccessChecks(queryClient)
      },
    })
    await authProvider.checkAuth({})
    mount(authProvider, queryClient)
    await waitFor(() => {
      expect(screen.getByTestId("edit")).toHaveAttribute("data-state", "true")
      expect(screen.getByTestId("list")).toHaveAttribute("data-state", "true")
    })

    // The account loses the probes, for example in another session; a 403
    // makes the provider ask /auth/me again.
    me = { ...ME, permissions: [], access: {} }
    await act(async () => {
      await authProvider.checkError({ status: 403 })
    })
    await waitFor(() => {
      expect(screen.getByTestId("edit")).toHaveAttribute("data-state", "false")
      expect(screen.getByTestId("list")).toHaveAttribute("data-state", "false")
    })
    // Two /auth/me reads: checkAuth's and the 403's; the hooks asked none.
    expect(fetch).toHaveBeenCalledTimes(2)
  })

  it("leaves the cached answers alone without it", async () => {
    // The control: without onAccessChange the mounted hooks keep what they
    // were told, which is why main.tsx passes it.
    let me: MeResponse = ME
    const fetch = vi.fn<typeof globalThis.fetch>(
      async () => new Response(JSON.stringify(me), { status: 200 })
    )
    const queryClient = new QueryClient()
    const authProvider = createAuthProvider({ apiBaseUrl: "/api/admin", fetch })
    await authProvider.checkAuth({})
    mount(authProvider, queryClient)
    await waitFor(() => {
      expect(screen.getByTestId("edit")).toHaveAttribute("data-state", "true")
    })
    me = { ...ME, permissions: [], access: {} }
    await act(async () => {
      await authProvider.checkError({ status: 403 })
    })
    expect(screen.getByTestId("edit")).toHaveAttribute("data-state", "true")
    // Invalidating by hand shows the new answer.
    await act(async () => {
      await invalidateAccessChecks(queryClient)
    })
    await waitFor(() => {
      expect(screen.getByTestId("edit")).toHaveAttribute("data-state", "false")
    })
  })
})
