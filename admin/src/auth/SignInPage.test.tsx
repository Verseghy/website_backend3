/**
 * The sign-in screen with the real auth provider over a `fetch` stub: its
 * Hungarian strings, its required fields, and the message it shows for each
 * kind of failed sign-in (ADR 0006 §4, ADR 0021 §4).
 */
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { CoreAdminContext, TestMemoryRouter } from "ra-core"
import { describe, expect, it, vi } from "vitest"

import { createI18nProvider } from "@/i18n/provider"

import { createAuthProvider, SignInError } from "./auth-provider"
import { createSignInPage, signInErrorMessage } from "./SignInPage"

const SHA = "0123456789abcdef0123456789abcdef01234567"

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
 * Renders the sign-in page inside ra-core's admin context, with the real auth
 * provider calling `fetch`.
 *
 * @param fetch - the stub that answers the sign-in.
 */
function renderSignInPage(fetch: typeof globalThis.fetch) {
  const SignInPage = createSignInPage(SHA)
  render(
    <TestMemoryRouter initialEntries={["/admin/login"]}>
      <CoreAdminContext
        basename="/admin"
        authProvider={createAuthProvider({ apiBaseUrl: "/api/admin", fetch })}
        i18nProvider={createI18nProvider({ reportMissingKeys: true })}
      >
        <SignInPage />
      </CoreAdminContext>
    </TestMemoryRouter>
  )
}

/**
 * Fills both fields and submits the form.
 *
 * @param email - the address to type.
 * @param password - the password to type.
 */
async function signIn(email: string, password: string) {
  const user = userEvent.setup()
  await user.type(screen.getByLabelText(/E-mail-cím/), email)
  await user.type(screen.getByLabelText(/Jelszó/), password)
  await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))
}

describe("SignInPage", () => {
  it("renders the Hungarian heading, fields, button and source link", () => {
    renderSignInPage(vi.fn())
    expect(
      screen.getByRole("heading", { level: 1, name: "Bejelentkezés" })
    ).toBeInTheDocument()
    expect(screen.getByText("Verseghy-admin")).toBeInTheDocument()
    expect(
      screen.getByText("Jelentkezz be a szerkesztőfelület használatához.")
    ).toBeInTheDocument()
    const email = screen.getByLabelText(/E-mail-cím/)
    expect(email).toHaveAttribute("type", "email")
    expect(email).toHaveAttribute("autocomplete", "username")
    const password = screen.getByLabelText(/Jelszó/)
    expect(password).toHaveAttribute("type", "password")
    expect(password).toHaveAttribute("autocomplete", "current-password")
    expect(screen.getByRole("button", { name: "Bejelentkezés" })).toBeEnabled()
    // The browser's own validation would speak the browser's language.
    expect(email.closest("form")).toHaveAttribute("novalidate")
    // AGPL-3.0 §13: the source offer is on the one page without a session.
    const footer = screen.getByRole("contentinfo")
    expect(
      within(footer).getByRole("link", { name: "Forráskód" })
    ).toHaveAttribute(
      "href",
      `https://github.com/Verseghy/website_backend3/tree/${SHA}`
    )
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("requires both fields, in Hungarian, before it sends anything", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>()
    renderSignInPage(fetch)
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Bejelentkezés" }))
    expect(await screen.findAllByText("Kötelező")).toHaveLength(2)
    expect(fetch).not.toHaveBeenCalled()
  })

  it("posts the typed address and password to the sign-in endpoint", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(200, { step: "done" })
    )
    renderSignInPage(fetch)
    await signIn("proba@example.test", "titkos jelszó")
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1))
    const [url, init] = fetch.mock.calls[0]
    expect(url).toBe("/api/admin/auth/login")
    expect(JSON.parse(String(init?.body))).toEqual({
      email: "proba@example.test",
      password: "titkos jelszó",
    })
  })

  it("shows the one generic message for a wrong address or password", async () => {
    renderSignInPage(async () =>
      json(401, {
        error: "invalid_credentials",
        description: "The e-mail address or the password is wrong.",
        field: null,
        params: {},
      })
    )
    await signIn("proba@example.test", "rossz jelszó")
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Hibás e-mail-cím vagy jelszó."
    )
    // The form keeps what was typed, so the user can correct it.
    expect(screen.getByLabelText(/E-mail-cím/)).toHaveValue(
      "proba@example.test"
    )
    expect(screen.getByRole("button", { name: "Bejelentkezés" })).toBeEnabled()
  })

  it.each([
    [422, "invalid_body", "A kérés formátuma hibás."],
    [413, "body_too_large", "Az elküldött adat túl nagy."],
  ])(
    "shows the envelope's message for a %i %s",
    async (status, error, text) => {
      renderSignInPage(async () =>
        json(status, { error, description: "d", field: null, params: {} })
      )
      await signIn("proba@example.test", "jelszó")
      expect(await screen.findByRole("alert")).toHaveTextContent(text)
    }
  )

  it("shows the server's text for a code this build does not know", async () => {
    renderSignInPage(async () =>
      json(429, {
        error: "too_many_attempts",
        description: "Too many attempts; try again later.",
        field: null,
        params: {},
      })
    )
    await signIn("proba@example.test", "jelszó")
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Too many attempts; try again later."
    )
  })

  it("shows the generic failure when the server cannot be reached", async () => {
    renderSignInPage(async () => {
      throw new TypeError("Failed to fetch")
    })
    await signIn("proba@example.test", "jelszó")
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "A bejelentkezés most nem sikerült. Próbáld újra később!"
    )
  })

  it("disables the button while the sign-in is under way and clears an old failure", async () => {
    let answer: (response: Response) => void = () => {}
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(
        json(401, {
          error: "invalid_credentials",
          description: "d",
          field: null,
          params: {},
        })
      )
      .mockImplementationOnce(
        () =>
          new Promise<Response>((resolve) => {
            answer = resolve
          })
      )
    renderSignInPage(fetch)
    await signIn("proba@example.test", "rossz")
    await screen.findByRole("alert")

    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Bejelentkezés" }))

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Bejelentkezés" })
      ).toBeDisabled()
    )
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    answer(
      json(401, {
        error: "invalid_credentials",
        description: "d",
        field: null,
        params: {},
      })
    )
    expect(await screen.findByRole("alert")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Bejelentkezés" })).toBeEnabled()
  })
})

describe("signInErrorMessage", () => {
  it("uses the provider's message for a SignInError", () => {
    const display = { kind: "text", text: "x" } as const
    expect(signInErrorMessage(new SignInError(429, display))).toBe(display)
  })

  it.each([new Error("bug"), "text", undefined])(
    "falls back to the generic failure for %j",
    (error) => {
      expect(signInErrorMessage(error)).toEqual({
        kind: "key",
        key: "auth.sign_in.failed",
        values: {},
      })
    }
  )
})
