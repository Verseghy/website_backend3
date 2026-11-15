/**
 * The header's user menu with the real auth provider over a `fetch` stub
 * and the kit's notification: the account it shows, the items it is given
 * with the kit's `onClose` context, and „Kijelentkezés”, which posts
 * `/auth/logout` and shows the sign-in page only when the server confirms
 * the sign-out (ADR 0006 §2, ADR 0021).
 */
import type { ReactNode } from "react"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { CoreAdminContext, TestMemoryRouter, useUserMenu } from "ra-core"
import type { Location } from "react-router-dom"
import { describe, expect, it, vi } from "vitest"

import { createAuthProvider } from "@/auth/auth-provider"
import { createSessionExpiry } from "@/auth/session-expiry"
import type { MeResponse } from "@/bindings/MeResponse"
import { Notification } from "@/components/admin/notification"
import { DropdownMenuItem } from "@/components/ui/dropdown-menu"
import { createI18nProvider } from "@/i18n/provider"

import { AppUserMenu } from "./AppUserMenu"

/**
 * How long a test waits for what follows a click on „Kijelentkezés”: the
 * request, ra-core's navigation, which it defers with a `setTimeout`, or the
 * notification. On a loaded machine that takes longer than Testing
 * Library's default of one second.
 */
const AFTER_SIGN_OUT = { timeout: 5000 }

const ME: MeResponse = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  name: "Próba Szerkesztő",
  email: "proba@example.test",
  scope: "admin",
  roles: ["writer"],
  permissions: ["edit posts", "edit labels", "edit authors", "manage media"],
  is_admin: false,
  access: {},
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
 * Renders the menu at `/admin/about` inside ra-core's admin context, with
 * the real auth provider: `/auth/me` answers {@link ME}, and
 * `/auth/logout` answers what `logoutAnswer` returns.
 *
 * @param logoutAnswer - the answer of `POST /auth/logout`, or a rejection.
 * @param items - the menu's `children`, if any.
 * @returns the fetch stub and a getter of the router's current location.
 */
function renderMenu(logoutAnswer: () => Promise<Response>, items?: ReactNode) {
  let location: Location | undefined
  const fetch = vi.fn<typeof globalThis.fetch>(async (input, init) => {
    const url = String(input)
    if (url === "/api/admin/auth/me") {
      return json(200, ME)
    }
    if (url === "/api/admin/auth/logout" && init?.method === "POST") {
      return logoutAnswer()
    }
    return json(404, {})
  })
  render(
    <TestMemoryRouter
      initialEntries={["/admin/about"]}
      locationCallback={(current) => {
        location = current
      }}
    >
      <CoreAdminContext
        basename="/admin"
        authProvider={createAuthProvider({
          apiBaseUrl: "/api/admin",
          fetch,
          sessionExpiry: createSessionExpiry(),
        })}
        i18nProvider={createI18nProvider({ reportMissingKeys: true })}
      >
        <AppUserMenu>{items}</AppUserMenu>
        <Notification />
      </CoreAdminContext>
    </TestMemoryRouter>
  )
  return { fetch, location: () => location }
}

/**
 * Opens the menu and clicks „Kijelentkezés”.
 *
 * @returns the user-event instance, for further steps.
 */
async function signOutThroughTheMenu() {
  const user = userEvent.setup()
  await user.click(await screen.findByRole("button", { name: "Profil" }))
  await user.click(
    await screen.findByRole("menuitem", { name: "Kijelentkezés" })
  )
  return user
}

/**
 * A menu item written in the kit's pattern: it closes the menu through
 * ra-core's `useUserMenu().onClose`, and only through it, because Base UI's
 * own close on click is off (`closeOnClick={false}`).
 *
 * @param props - `onClick`, called after `onClose`.
 * @returns the item.
 */
function CloseThroughContextItem({ onClick }: { onClick: () => void }) {
  const userMenu = useUserMenu()
  return (
    <DropdownMenuItem
      closeOnClick={false}
      onClick={() => {
        userMenu?.onClose()
        onClick()
      }}
    >
      Próba menüpont
    </DropdownMenuItem>
  )
}

/** The calls of `fetch` to `/auth/logout`. */
function logoutCalls(fetch: ReturnType<typeof renderMenu>["fetch"]) {
  return fetch.mock.calls.filter(([url]) => url === "/api/admin/auth/logout")
}

describe("AppUserMenu", () => {
  it("shows the account's first letter, then its name, address and the Hungarian sign-out", async () => {
    renderMenu(async () => new Response(null, { status: 204 }))
    const trigger = await screen.findByRole("button", { name: "Profil" })
    await waitFor(() => expect(trigger).toHaveTextContent("P"))

    await userEvent.setup().click(trigger)

    const menu = await screen.findByRole("menu")
    expect(within(menu).getByText(ME.name)).toBeInTheDocument()
    expect(within(menu).getByText(ME.email)).toBeInTheDocument()
    expect(
      within(menu).getByRole("menuitem", { name: "Kijelentkezés" })
    ).toBeInTheDocument()
  })

  it("shows given items between the account and the sign-out, as the kit's menu does", async () => {
    const user = userEvent.setup()
    renderMenu(
      async () => new Response(null, { status: 204 }),
      <CloseThroughContextItem onClick={() => {}} />
    )
    await user.click(await screen.findByRole("button", { name: "Profil" }))

    const menu = await screen.findByRole("menu")
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((item) => item.textContent)
    ).toEqual(["Próba menüpont", "Kijelentkezés"])
    // One separator after the account, and one after the given items.
    expect(within(menu).getAllByRole("separator")).toHaveLength(2)
  })

  it("has one separator only when it is given no items", async () => {
    const user = userEvent.setup()
    renderMenu(async () => new Response(null, { status: 204 }))
    await user.click(await screen.findByRole("button", { name: "Profil" }))

    const menu = await screen.findByRole("menu")
    expect(within(menu).getAllByRole("menuitem")).toHaveLength(1)
    expect(within(menu).getAllByRole("separator")).toHaveLength(1)
  })

  it("lets an item close the menu through useUserMenu().onClose, as in the kit's pattern", async () => {
    const user = userEvent.setup()
    const clicked = vi.fn()
    const { fetch } = renderMenu(
      async () => new Response(null, { status: 204 }),
      <CloseThroughContextItem onClick={clicked} />
    )
    await user.click(await screen.findByRole("button", { name: "Profil" }))

    await user.click(
      await screen.findByRole("menuitem", { name: "Próba menüpont" })
    )

    expect(clicked).toHaveBeenCalledOnce()
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
    // Closing the menu signs nobody out.
    expect(logoutCalls(fetch)).toHaveLength(0)
  })

  it("posts /auth/logout and shows the sign-in page once the server confirms", async () => {
    const { fetch, location } = renderMenu(
      async () => new Response(null, { status: 204 })
    )

    await signOutThroughTheMenu()

    await waitFor(
      () => expect(location()?.pathname).toBe("/admin/login"),
      AFTER_SIGN_OUT
    )
    const calls = logoutCalls(fetch)
    expect(calls).toHaveLength(1)
    expect(calls[0][1]).toMatchObject({
      method: "POST",
      credentials: "same-origin",
    })
  })

  it("shows the sign-in page when the session had ended already (401)", async () => {
    const { location } = renderMenu(async () =>
      json(401, {
        error: "unauthorized",
        description: "d",
        field: null,
        params: {},
      })
    )
    await signOutThroughTheMenu()
    await waitFor(
      () => expect(location()?.pathname).toBe("/admin/login"),
      AFTER_SIGN_OUT
    )
  })

  it.each([
    ["a server error", async () => json(500, { error: "x" })],
    [
      "a network failure",
      async (): Promise<Response> => {
        throw new TypeError("Failed to fetch")
      },
    ],
  ])(
    "keeps the user signed in on the page after %s, and says so in Hungarian",
    async (_case, answer) => {
      const { fetch, location } = renderMenu(answer)

      await signOutThroughTheMenu()

      expect(
        await screen.findByText(
          "A kijelentkezés nem sikerült, továbbra is be vagy jelentkezve. Próbáld újra!",
          {},
          AFTER_SIGN_OUT
        )
      ).toBeInTheDocument()
      expect(location()?.pathname).toBe("/admin/about")
      expect(logoutCalls(fetch)).toHaveLength(1)
      // The menu is still there to try again.
      expect(screen.getByRole("button", { name: "Profil" })).toBeInTheDocument()
    }
  )
})
