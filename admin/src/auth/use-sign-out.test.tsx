/**
 * The user's sign-out hook inside ra-core's admin context: it asks the auth
 * provider to end the session ({@link SIGN_OUT_PARAMS}), shows the sign-in
 * page once the provider confirms, and otherwise keeps the user where they
 * are and notifies why (ADR 0006 §2, ADR 0021 §4).
 */
import type { ReactNode } from "react"
import { act, render, screen, waitFor } from "@testing-library/react"
import fc from "fast-check"
import {
  CoreAdminContext,
  TestMemoryRouter,
  useNotificationContext,
  type AuthProvider,
} from "ra-core"
import type { Location } from "react-router-dom"
import { describe, expect, it, vi } from "vitest"

import { keyMessage } from "@/api/error-envelope"
import { createI18nProvider } from "@/i18n/provider"

import { SIGN_OUT_PARAMS, SignInError, SignOutError } from "./auth-provider"
import {
  signOutErrorMessage,
  useConfirmedSignOut,
  useSignOut,
} from "./use-sign-out"

/**
 * Shows the sign-out action as a button, and every queued notification as
 * its raw message and arguments, so a test reads what `notify` received.
 */
function Probe() {
  const signOut = useSignOut()
  const { notifications } = useNotificationContext()
  return (
    <>
      <button type="button" onClick={() => void signOut()}>
        sign-out
      </button>
      <ul>
        {notifications.map((notification, index) => (
          <li key={index} data-testid="notification">
            {JSON.stringify({
              message: notification.message,
              type: notification.type,
              args: notification.notificationOptions?.messageArgs,
            })}
          </li>
        ))}
      </ul>
    </>
  )
}

/**
 * Renders the probe at `/admin/about` with `logout` as the provider's
 * `logout`.
 *
 * @param logout - the stub of `authProvider.logout`.
 * @param probe - what to render in the admin context; the user menu's
 *   sign-out probe unless the test says otherwise.
 * @returns a getter of the router's current location.
 */
function renderProbe(
  logout: AuthProvider["logout"],
  probe: ReactNode = <Probe />
) {
  let location: Location | undefined
  const authProvider: AuthProvider = {
    login: vi.fn(),
    logout,
    checkAuth: vi.fn(async () => undefined),
    checkError: vi.fn(async () => undefined),
  }
  render(
    <TestMemoryRouter
      initialEntries={["/admin/about"]}
      locationCallback={(current) => {
        location = current
      }}
    >
      <CoreAdminContext
        basename="/admin"
        authProvider={authProvider}
        i18nProvider={createI18nProvider({ reportMissingKeys: true })}
      >
        {probe}
      </CoreAdminContext>
    </TestMemoryRouter>
  )
  return () => location
}

/**
 * Waits for what follows the sign-out: ra-core's navigation, which it
 * defers with a `setTimeout`, or the notification. On a loaded machine that
 * takes longer than Testing Library's default of one second.
 *
 * @param check - the assertion to retry until it passes.
 * @returns what `waitFor` returns.
 */
function waitForSignOut(check: () => void) {
  return waitFor(check, { timeout: 5000 })
}

/** Clicks the probe's sign-out and lets ra-core's deferred navigation run. */
async function clickSignOut() {
  await act(async () => {
    screen.getByRole("button", { name: "sign-out" }).click()
  })
}

/** The notifications the probe shows, parsed. */
function notifications(): unknown[] {
  return screen
    .queryAllByTestId("notification")
    .map((item) => JSON.parse(item.textContent ?? "null") as unknown)
}

describe("useSignOut", () => {
  it("asks the provider to end the session and shows the sign-in page", async () => {
    const logout = vi.fn<AuthProvider["logout"]>(async () => undefined)
    const location = renderProbe(logout)

    await clickSignOut()

    expect(logout).toHaveBeenCalledTimes(1)
    expect(logout).toHaveBeenCalledWith(SIGN_OUT_PARAMS)
    await waitForSignOut(() =>
      expect(location()?.pathname).toBe("/admin/login")
    )
    // No return to the page this user left after the next sign-in.
    expect(location()?.state).toBeNull()
    expect(notifications()).toEqual([])
  })

  it("keeps the user on the page and notifies the provider's message when it rejects", async () => {
    const logout = vi.fn<AuthProvider["logout"]>(async () => {
      throw new SignOutError(500, keyMessage("auth.sign_out.failed"))
    })
    const location = renderProbe(logout)

    await clickSignOut()

    await waitForSignOut(() =>
      expect(notifications()).toEqual([
        { message: "auth.sign_out.failed", type: "error", args: {} },
      ])
    )
    expect(location()?.pathname).toBe("/admin/about")
  })

  it("notifies the envelope's message with its values", async () => {
    const logout = vi.fn<AuthProvider["logout"]>(async () => {
      throw new SignOutError(
        413,
        keyMessage("errors.body_too_large", { limit: 5 })
      )
    })
    renderProbe(logout)
    await clickSignOut()
    await waitForSignOut(() =>
      expect(notifications()).toEqual([
        { message: "errors.body_too_large", type: "error", args: { limit: 5 } },
      ])
    )
  })

  it("notifies the server's own text for a code this build does not know", async () => {
    const text = "Cross-origin write refused"
    const logout = vi.fn<AuthProvider["logout"]>(async () => {
      throw new SignOutError(403, { kind: "text", text })
    })
    renderProbe(logout)
    await clickSignOut()
    await waitForSignOut(() =>
      expect(notifications()).toEqual([
        { message: text, type: "error", args: { _: text } },
      ])
    )
  })

  it("notifies the generic failure for any other rejection, and never rejects itself", async () => {
    const logout = vi.fn<AuthProvider["logout"]>(async () => {
      throw new TypeError("a programming error")
    })
    const location = renderProbe(logout)
    await clickSignOut()
    await waitForSignOut(() =>
      expect(notifications()).toEqual([
        { message: "auth.sign_out.failed", type: "error", args: {} },
      ])
    )
    expect(location()?.pathname).toBe("/admin/about")
  })
})

/**
 * The in-place sign-in's sign-out as a button; what it resolved or rejected
 * with is shown as text, beside the queued notifications, so a test reads
 * that the hook itself notifies nothing.
 */
function ConfirmedProbe() {
  const signOut = useConfirmedSignOut()
  const { notifications: queued } = useNotificationContext()
  return (
    <>
      <button
        type="button"
        onClick={() => {
          signOut().then(
            () => {
              document.body.dataset.outcome = "resolved"
            },
            (error: unknown) => {
              document.body.dataset.outcome =
                error instanceof Error ? error.name : "other"
            }
          )
        }}
      >
        sign-out
      </button>
      <p data-testid="queued">{queued.length}</p>
    </>
  )
}

describe("useConfirmedSignOut", () => {
  it("asks the provider to end the session, resolves, and returns to the page after the next sign-in", async () => {
    delete document.body.dataset.outcome
    const logout = vi.fn<AuthProvider["logout"]>(async () => undefined)
    const location = renderProbe(logout, <ConfirmedProbe />)

    await clickSignOut()

    expect(logout).toHaveBeenCalledExactlyOnceWith(SIGN_OUT_PARAMS)
    await waitForSignOut(() =>
      expect(document.body.dataset.outcome).toBe("resolved")
    )
    await waitForSignOut(() =>
      expect(location()?.pathname).toBe("/admin/login")
    )
    // As ra-core's own sign-out after an ended session, which runs at the
    // same time, has it: back to this page after the next sign-in.
    expect(location()?.state).toEqual({
      nextPathname: "/admin/about",
      nextSearch: "",
    })
  })

  it("rejects with the provider's error, keeps the user on the page and notifies nothing", async () => {
    delete document.body.dataset.outcome
    const logout = vi.fn<AuthProvider["logout"]>(async () => {
      throw new SignOutError(500, keyMessage("auth.sign_out.failed"))
    })
    const location = renderProbe(logout, <ConfirmedProbe />)

    await clickSignOut()

    await waitForSignOut(() =>
      expect(document.body.dataset.outcome).toBe("SignOutError")
    )
    expect(location()?.pathname).toBe("/admin/about")
    expect(screen.getByTestId("queued")).toHaveTextContent("0")
  })
})

describe("signOutErrorMessage", () => {
  it("is the display of a SignOutError", () => {
    const display = keyMessage("errors.invalid_body")
    expect(signOutErrorMessage(new SignOutError(422, display))).toBe(display)
  })

  it("is auth.sign_out.failed for anything that is not a SignOutError", () => {
    const generic = keyMessage("auth.sign_out.failed")
    expect(
      signOutErrorMessage(
        new SignInError(500, keyMessage("auth.sign_in.failed"))
      )
    ).toEqual(generic)
    fc.assert(
      fc.property(fc.anything(), (error) => {
        expect(signOutErrorMessage(error)).toEqual(generic)
      })
    )
  })
})
