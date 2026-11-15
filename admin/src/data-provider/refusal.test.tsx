/**
 * A list that the admin API refuses with `403 forbidden` (ADR 0007 §5), run
 * through ra-core as a resource screen runs it: a data-provider handler that
 * rejects with `refusalError`, ra-core's list controller, its call to the
 * auth provider's `checkError`, and its notification, translated by the
 * admin's Hungarian i18n provider.
 *
 * The screen shows the Hungarian 403 message, never an empty list, and the
 * user stays signed in: `checkError` signs out on `401` only (ADR 0006). The
 * 403 makes the auth provider ask `/auth/me` again, since the account's
 * access may have changed since its last answer (ADR 0007 §7).
 */
import { render, screen, waitFor } from "@testing-library/react"
import {
  CoreAdminContext,
  ListBase,
  TestMemoryRouter,
  useListContext,
  useNotificationContext,
  useTranslate,
} from "ra-core"
import { describe, expect, it, vi } from "vitest"

import { refusalError } from "@/api/error-envelope"
import { createAuthProvider } from "@/auth/auth-provider"
import type { MeResponse } from "@/bindings/MeResponse"
import { createI18nProvider } from "@/i18n/provider"

import { createDataProvider } from "./data-provider"

const ME: MeResponse = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  name: "Próba Titkár",
  email: "titkar@example.test",
  scope: "admin",
  roles: ["secretary"],
  permissions: ["edit events", "edit canteens", "edit colleagues"],
  is_admin: false,
  // The access map still lists the probes, as it did before a role change
  // in another session took them away: the admin asks, the server refuses.
  access: { probes: ["list"] },
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
 * The admin API as a secretary's session sees it: `/auth/me` answers the
 * account, and the probe list, which needs a permission the account lacks,
 * answers `403 forbidden` as the route table's guard does.
 */
const fetchStub = vi.fn<typeof globalThis.fetch>(async (input) => {
  const url = String(input)
  if (url === "/api/admin/auth/me") {
    return json(200, ME)
  }
  if (url === "/api/admin/probes") {
    return json(403, {
      error: "forbidden",
      description: "The signed-in account may not perform this operation",
      field: null,
      params: {},
    })
  }
  return json(404, {
    error: "not_found",
    description: "d",
    field: null,
    params: {},
  })
})

/** Shows ra-core's notifications, translated as the kit's toaster does. */
function Notifications() {
  const { notifications } = useNotificationContext()
  const translate = useTranslate()
  return (
    <ul>
      {notifications.map((notification, index) => (
        <li key={index} role="alert">
          {typeof notification.message === "string"
            ? translate(
                notification.message,
                notification.notificationOptions?.messageArgs
              )
            : null}
        </li>
      ))}
    </ul>
  )
}

/** Exposes what the list holds: no rows at all, or how many. */
function Rows() {
  const { data, error } = useListContext()
  return (
    <output
      data-testid="rows"
      data-count={data === undefined ? "none" : String(data.length)}
      data-failed={String(error != null)}
    />
  )
}

describe("a list the admin API refuses with 403 forbidden", () => {
  it("shows the Hungarian message instead of rows, and keeps the session", async () => {
    // ra-core logs every rejected data-provider call outside production
    // (ra-core 5.15.5 `useDataProvider`); this test expects exactly that log
    // and nothing else, so it owns the spy that `src/test/setup.ts` installs.
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined)
    // The probe resource has no `resources.probes.*` messages, as no screen
    // registers it; every other missing key still fails the test.
    const missingKeys: string[] = []
    const i18nProvider = createI18nProvider({
      reportMissingKeys: true,
      onMissingKey: (key) => {
        missingKeys.push(key)
      },
    })
    const authProvider = createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch: fetchStub,
    })
    const logout = vi.spyOn(authProvider, "logout")
    const checkError = vi.spyOn(authProvider, "checkError")
    const dataProvider = createDataProvider({
      apiBaseUrl: "/api/admin",
      resources: {
        // What a slice's list handler does with a refused answer.
        probes: {
          async getList(_params, context) {
            const response = await fetchStub(`${context.apiBaseUrl}/probes`)
            const body: unknown = await response.json()
            if (!response.ok) {
              throw refusalError(response.status, body)
            }
            return { data: body as { id: number }[], total: 0 }
          },
        },
      },
    })
    render(
      <TestMemoryRouter>
        <CoreAdminContext
          dataProvider={dataProvider}
          authProvider={authProvider}
          i18nProvider={i18nProvider}
        >
          <ListBase resource="probes">
            <Rows />
          </ListBase>
          <Notifications />
        </CoreAdminContext>
      </TestMemoryRouter>
    )

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nincs jogosultságod ehhez a művelethez."
    )
    const rows = screen.getByTestId("rows")
    expect(rows).toHaveAttribute("data-count", "none")
    expect(rows).toHaveAttribute("data-failed", "true")
    // ra-core asked the auth provider about the 403, which kept the session.
    await waitFor(() => {
      expect(checkError).toHaveBeenCalledWith(
        expect.objectContaining({ status: 403, message: "errors.forbidden" })
      )
    })
    expect(logout).not.toHaveBeenCalled()
    // checkAuth read /auth/me once before the list; the 403 read it again.
    await waitFor(() => {
      expect(
        fetchStub.mock.calls.filter(([url]) => String(url).endsWith("/auth/me"))
          .length
      ).toBeGreaterThanOrEqual(2)
    })
    expect(
      fetchStub.mock.calls.some(([url]) => String(url).endsWith("/auth/logout"))
    ).toBe(false)
    expect(
      missingKeys.filter((key) => !key.startsWith("resources.probes."))
    ).toEqual([])
    expect(consoleError.mock.calls).toEqual([
      [expect.objectContaining({ status: 403, message: "errors.forbidden" })],
    ])
  })
})
