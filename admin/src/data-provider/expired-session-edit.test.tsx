/**
 * An edit form whose session ends while the user types, run through ra-core
 * as a resource screen runs it (ADR 0006, "Follow-up work": the admin must
 * not drop an open form on a `401`; it shows a sign-in dialog in place,
 * then retries).
 *
 * Everything is real but the network: ra-core's edit controller and form,
 * the kit's inputs and save button, backend3's data provider with a
 * slice-style handler, the auth provider, the retry queue and the dialog,
 * over a `fetch` stub that plays the admin API with a session that can end.
 * The save meets a `401`; the dialog opens over the form, which stays
 * mounted with the typed value; after the sign-in the same save is sent
 * again and stores that value, and nobody is signed out.
 */
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import {
  CoreAdminContext,
  EditBase,
  TestMemoryRouter,
  useNotificationContext,
  useTranslate,
} from "ra-core"
import { render } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { refusalError } from "@/api/error-envelope"
import { createAuthProvider, SIGN_OUT_PARAMS } from "@/auth/auth-provider"
import {
  SessionExpiryDialog,
  type SessionExpiryOptions,
} from "@/auth/SessionExpiryDialog"
import { createSessionExpiry } from "@/auth/session-expiry"
import { useConfirmedSignOut } from "@/auth/use-sign-out"
import type { MeResponse } from "@/bindings/MeResponse"
import { SimpleForm } from "@/components/admin/simple-form"
import { TextInput } from "@/components/admin/text-input"
import { createI18nProvider } from "@/i18n/provider"

import { createDataProvider } from "./data-provider"

const ME: MeResponse = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  name: "Próba Szerkesztő",
  email: "proba@example.test",
  scope: "admin",
  roles: ["writer"],
  permissions: ["edit posts", "edit labels", "edit authors", "manage media"],
  is_admin: false,
  // The probe resource's row of the access map (ADR 0007 §7), which the
  // edit page asks canAccess about.
  access: { probes: ["list", "show", "edit"] },
}

/** What the user types into the form before the session ends. */
const TYPED = "Új cím, amelyet hosszan gépeltem"

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

/** The admin API's `401` envelope. */
const UNAUTHORIZED = {
  error: "unauthorized",
  description: "No valid session",
  field: null,
  params: {},
}

/**
 * The admin API with one probe record and a session that the test ends:
 * `/auth/me` and the probe routes answer `401` without it, `/auth/login`
 * starts it again with the right password.
 */
class FakeApi {
  /** Whether the browser's cookie holds a valid session. */
  signedIn = true
  /** The stored record. */
  record = { id: 1, title: "Régi cím" }
  /** Every `PUT` body that the server stored. */
  readonly saved: unknown[] = []
  /** The `fetch` stub. */
  readonly fetch = vi.fn<typeof globalThis.fetch>(async (input, init) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    if (url === "/api/admin/auth/login" && method === "POST") {
      const { password } = JSON.parse(String(init?.body)) as {
        password: string
      }
      if (password !== "helyes jelszó") {
        return json(401, { ...UNAUTHORIZED, error: "invalid_credentials" })
      }
      this.signedIn = true
      return json(200, { step: "done" })
    }
    if (!this.signedIn) {
      return json(401, UNAUTHORIZED)
    }
    if (url === "/api/admin/auth/me") {
      return json(200, ME)
    }
    if (url === "/api/admin/probes/1" && method === "GET") {
      return json(200, this.record)
    }
    if (url === "/api/admin/probes/1" && method === "PUT") {
      const body = JSON.parse(String(init?.body)) as { title: string }
      this.saved.push(body)
      this.record = { id: 1, title: body.title }
      return json(200, this.record)
    }
    return json(404, { ...UNAUTHORIZED, error: "not_found" })
  })

  /**
   * The requests sent to `url` with `method`.
   *
   * @param method - the HTTP method.
   * @param url - the full path.
   * @returns how many.
   */
  calls(method: string, url: string): number {
    return this.fetch.mock.calls.filter(
      ([input, init]) =>
        String(input) === url && (init?.method ?? "GET") === method
    ).length
  }
}

/**
 * The dialog as the layout mounts it: with ra-core's confirmed sign-out.
 *
 * @param props - the queue and the auth provider's two calls.
 */
function LayoutDialog(props: SessionExpiryOptions) {
  const signOut = useConfirmedSignOut()
  return <SessionExpiryDialog {...props} signOut={signOut} />
}

/** Shows ra-core's notifications, translated as the kit's toaster does. */
function Notifications() {
  const { notifications } = useNotificationContext()
  const translate = useTranslate()
  return (
    <ul>
      {notifications.map((notification, index) => (
        <li key={index} data-testid="notification">
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

/**
 * Mounts an edit screen of the probe resource with the dialog beside it,
 * as the layout places it.
 *
 * @param api - the fake admin API.
 * @returns the auth provider's spied `logout` and `checkError`.
 */
function mountEditScreen(api: FakeApi) {
  // The probe resource has no `resources.probes.*` messages, as no screen
  // registers it; every other missing key still fails the test.
  const missingKeys: string[] = []
  const i18nProvider = createI18nProvider({
    reportMissingKeys: true,
    onMissingKey: (key) => {
      missingKeys.push(key)
    },
  })
  const sessionExpiry = createSessionExpiry()
  const authProvider = createAuthProvider({
    apiBaseUrl: "/api/admin",
    fetch: api.fetch,
    sessionExpiry,
  })
  const logout = vi.spyOn(authProvider, "logout")
  const checkError = vi.spyOn(authProvider, "checkError")
  const dataProvider = createDataProvider({
    apiBaseUrl: "/api/admin",
    fetch: api.fetch,
    sessionExpiry,
    resources: {
      // What a slice's handlers do: send through `request`, and reject a
      // refused answer with `refusalError`.
      probes: {
        async getOne(params, context) {
          const response = await context.request(
            `/probes/${String(params.id)}`,
            { method: "GET", signal: params.signal }
          )
          const body: unknown = await response.json()
          if (!response.ok) {
            throw refusalError(response.status, body)
          }
          return { data: body as { id: number; title: string } }
        },
        async update(params, context) {
          const response = await context.request(
            `/probes/${String(params.id)}`,
            {
              method: "PUT",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ title: params.data.title }),
            }
          )
          const body: unknown = await response.json()
          if (!response.ok) {
            throw refusalError(response.status, body)
          }
          return { data: body as { id: number; title: string } }
        },
      },
    },
  })
  render(
    <TestMemoryRouter initialEntries={["/probes/1"]}>
      <CoreAdminContext
        dataProvider={dataProvider}
        authProvider={authProvider}
        i18nProvider={i18nProvider}
      >
        <EditBase
          resource="probes"
          id={1}
          // The form stays on screen while the save is on its way.
          mutationMode="pessimistic"
          redirect={false}
        >
          <SimpleForm>
            <TextInput source="title" />
          </SimpleForm>
        </EditBase>
        <LayoutDialog
          expiry={sessionExpiry}
          account={() => authProvider.sessionAccount()}
          resume={(password) => authProvider.resumeSession(password)}
        />
        <Notifications />
      </CoreAdminContext>
    </TestMemoryRouter>
  )
  return {
    logout,
    checkError,
    missingKeys: () =>
      missingKeys.filter((key) => !key.startsWith("resources.probes.")),
  }
}

describe("an edit form whose session ends before the save", () => {
  it("keeps the typed value in the open form and saves it after the in-place sign-in", async () => {
    const api = new FakeApi()
    const { logout, checkError, missingKeys } = mountEditScreen(api)
    const user = userEvent.setup()

    const input = await screen.findByDisplayValue("Régi cím")
    await user.clear(input)
    await user.type(input, TYPED)

    // The session ends while the user types (expiry, revocation).
    api.signedIn = false
    await user.click(screen.getByRole("button", { name: "Mentés" }))

    const dialog = await screen.findByRole("dialog", {
      name: "Lejárt a munkamenet",
    })
    // The save met its 401 and waits; the form is the same, with the value.
    expect(api.calls("PUT", "/api/admin/probes/1")).toBe(1)
    expect(api.saved).toEqual([])
    expect(screen.getByDisplayValue(TYPED)).toBe(input)
    expect(input).toBeInTheDocument()

    await user.type(within(dialog).getByLabelText("Jelszó"), "helyes jelszó")
    await user.click(
      within(dialog).getByRole("button", { name: "Bejelentkezés" })
    )

    // The same save, sent again after the sign-in, stored the typed value.
    await waitFor(() => expect(api.saved).toEqual([{ title: TYPED }]))
    expect(api.calls("PUT", "/api/admin/probes/1")).toBe(2)
    await waitFor(() => expect(dialog).not.toBeInTheDocument())
    expect(await screen.findByTestId("notification")).toHaveTextContent(
      "Módosítva"
    )
    expect(screen.getByDisplayValue(TYPED)).toBe(input)
    // Nobody was signed out, and ra-core never saw the 401.
    expect(logout).not.toHaveBeenCalled()
    expect(checkError).not.toHaveBeenCalledWith(
      expect.objectContaining({ status: 401 })
    )
    expect(missingKeys()).toEqual([])
  })

  it("signs out with the server's word when the user leaves the dialog, and sends the save no more", async () => {
    const api = new FakeApi()
    const { logout, checkError } = mountEditScreen(api)
    const user = userEvent.setup()

    const input = await screen.findByDisplayValue("Régi cím")
    await user.clear(input)
    await user.type(input, TYPED)
    api.signedIn = false
    await user.click(screen.getByRole("button", { name: "Mentés" }))

    const dialog = await screen.findByRole("dialog", {
      name: "Lejárt a munkamenet",
    })
    // ra-core logs every rejected data-provider call outside production
    // (`useDataProvider`): the save, and the record's own refetch on the
    // way to the sign-in page. This test expects exactly those 401s, so it
    // owns the console spy.
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined)
    await user.click(within(dialog).getByRole("button", { name: "Kilépés" }))

    // „Kilépés” asked the server first: its 401 says that no session is
    // left (the session had ended).
    expect(logout).toHaveBeenNthCalledWith(1, SIGN_OUT_PARAMS)
    await waitFor(() =>
      expect(api.calls("POST", "/api/admin/auth/logout")).toBe(1)
    )
    // Then the save got its 401 back: checkError signed out, as before.
    await waitFor(() =>
      expect(checkError).toHaveBeenCalledWith(
        expect.objectContaining({ status: 401, message: "errors.unauthorized" })
      )
    )
    expect(api.saved).toEqual([])
    expect(api.calls("PUT", "/api/admin/probes/1")).toBe(1)
    // ra-core's own sign-outs after it found no session post nothing more.
    expect(api.calls("POST", "/api/admin/auth/logout")).toBe(1)
    expect(consoleError.mock.calls.length).toBeGreaterThan(0)
    for (const call of consoleError.mock.calls) {
      expect(call).toEqual([
        expect.objectContaining({
          status: 401,
          message: "errors.unauthorized",
        }),
      ])
    }
  })
})
