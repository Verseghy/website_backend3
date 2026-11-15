/**
 * The auth provider against a `fetch` stub: what each method asks the admin
 * API, and what it makes of every kind of answer (ADR 0006, ADR 0021 §4).
 */
import fc from "fast-check"
import { describe, expect, it, vi } from "vitest"

import type { ErrorEnvelope } from "@/bindings/ErrorEnvelope"
import type { MeResponse } from "@/bindings/MeResponse"

import {
  AuthProviderConfigError,
  createAuthProvider,
  parseLoginResponse,
  parseMeResponse,
  SessionCheckError,
  SignInError,
  signInFailureMessage,
} from "./auth-provider"

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
 * @param body - the body, serialised as JSON.
 * @returns a `Response`.
 */
function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/**
 * An error envelope as the server writes it.
 *
 * @param error - the code.
 * @param params - the placeholder values.
 * @returns the envelope.
 */
function envelope(
  error: string,
  params: Record<string, string | number> = {}
): ErrorEnvelope {
  return {
    error,
    description: `English text for ${error}.`,
    field: null,
    params,
  } as ErrorEnvelope
}

/**
 * A provider whose `fetch` answers every call with `answer`.
 *
 * @param answer - what the stub resolves with, or rejects with when it is an
 *   `Error`.
 * @returns the provider and the stub, to inspect the requests.
 */
function providerAnswering(answer: Response | Error) {
  const fetch = vi.fn<typeof globalThis.fetch>(async () => {
    if (answer instanceof Error) {
      throw answer
    }
    return answer.clone()
  })
  return {
    provider: createAuthProvider({ apiBaseUrl: "/api/admin", fetch }),
    fetch,
  }
}

/**
 * Awaits `promise` and returns what it rejected with.
 *
 * @param promise - a promise that must reject.
 * @returns the rejection.
 */
async function rejection(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise
  } catch (error) {
    return error
  }
  throw new Error("expected the promise to reject")
}

describe("createAuthProvider", () => {
  it.each(["", "api/admin", "/api/admin/", "https://example.org/api", "//x"])(
    "refuses the apiBaseUrl %j with the data provider's message",
    (apiBaseUrl) => {
      expect(() => createAuthProvider({ apiBaseUrl, fetch: vi.fn() })).toThrow(
        AuthProviderConfigError
      )
      expect(() => createAuthProvider({ apiBaseUrl, fetch: vi.fn() })).toThrow(
        /^apiBaseUrl must be a root-relative path such as "\/api\/admin"/
      )
    }
  )

  it("refuses a fetch that is not a function", () => {
    expect(() =>
      createAuthProvider({
        apiBaseUrl: "/api/admin",
        fetch: undefined as unknown as typeof globalThis.fetch,
      })
    ).toThrow(/fetch must be a function/)
  })
})

describe("login", () => {
  it("posts the form's values as JSON to /auth/login and resolves on step done", async () => {
    const { provider, fetch } = providerAnswering(json(200, { step: "done" }))

    await expect(
      provider.login({ email: "proba@example.test", password: "titkos jelszó" })
    ).resolves.toBeUndefined()

    expect(fetch).toHaveBeenCalledTimes(1)
    const [url, init] = fetch.mock.calls[0]
    expect(url).toBe("/api/admin/auth/login")
    expect(init?.method).toBe("POST")
    expect(init?.credentials).toBe("same-origin")
    expect(init?.headers).toEqual({
      Accept: "application/json",
      "Content-Type": "application/json",
    })
    expect(JSON.parse(String(init?.body))).toEqual({
      email: "proba@example.test",
      password: "titkos jelszó",
    })
  })

  it("sends only the two members, whatever else the form holds", async () => {
    const { provider, fetch } = providerAnswering(json(200, { step: "done" }))
    await provider.login({
      email: "a@example.test",
      password: "p",
      remember: true,
    })
    expect(JSON.parse(String(fetch.mock.calls[0][1]?.body))).toEqual({
      email: "a@example.test",
      password: "p",
    })
  })

  it("shows the generic invalid_credentials for every 401, whatever its body", async () => {
    for (const body of [
      envelope("invalid_credentials"),
      envelope("unauthorized"),
      "x",
    ]) {
      const { provider } = providerAnswering(json(401, body))
      const error = await rejection(
        provider.login({ email: "a@example.test", password: "rossz" })
      )
      expect(error).toBeInstanceOf(SignInError)
      expect((error as SignInError).status).toBe(401)
      expect((error as SignInError).display).toEqual({
        kind: "key",
        key: "errors.invalid_credentials",
        values: {},
      })
      expect((error as Error).message).toBe("errors.invalid_credentials")
    }
  })

  it.each([
    [422, "invalid_body"],
    [413, "body_too_large"],
  ])("shows the envelope's code for a %i", async (status, code) => {
    const { provider } = providerAnswering(json(status, envelope(code)))
    const error = (await rejection(
      provider.login({ email: "a@example.test", password: "p" })
    )) as SignInError
    expect(error.status).toBe(status)
    expect(error.display).toEqual({
      kind: "key",
      key: `errors.${code}`,
      values: {},
    })
  })

  it("shows the server's description for a code this build does not know", async () => {
    const { provider } = providerAnswering(
      json(429, envelope("too_many_attempts", { seconds: 60 }))
    )
    const error = (await rejection(
      provider.login({ email: "a@example.test", password: "p" })
    )) as SignInError
    expect(error.display).toEqual({
      kind: "text",
      text: "English text for too_many_attempts.",
    })
  })

  it.each([
    [503, "service_unavailable"],
    [500, "internal_server_error"],
  ])(
    "shows the envelope of a %i that carries one, not the generic failure",
    async (status, code) => {
      // A server error with a well-formed envelope is shown like any other
      // refusal (ADR 0021 §4): here the server's text, because this build
      // has no `errors.<code>` for it. Loco's own 500 body has no `params`,
      // so it is no envelope and gets auth.sign_in.failed (below).
      const { provider } = providerAnswering(json(status, envelope(code)))
      const error = (await rejection(
        provider.login({ email: "a@example.test", password: "p" })
      )) as SignInError
      expect(error.status).toBe(status)
      expect(error.display).toEqual({
        kind: "text",
        text: `English text for ${code}.`,
      })
    }
  )

  it.each([
    [
      "Loco's own 500 body, which has no params",
      json(500, {
        error: "internal_server_error",
        description: "Internal Server Error",
      }),
    ],
    ["a server error without an envelope", json(500, "Internal Server Error")],
    ["a proxy's HTML page", new Response("<html></html>", { status: 502 })],
    ["an empty 503", new Response(null, { status: 503 })],
    ["a 200 without a step", json(200, {})],
    ["a 200 with a step this build does not know", json(200, { step: "totp" })],
    ["a 200 that is not JSON", new Response("ok", { status: 200 })],
  ])("shows auth.sign_in.failed for %s", async (_case, answer) => {
    const { provider } = providerAnswering(answer)
    const error = (await rejection(
      provider.login({ email: "a@example.test", password: "p" })
    )) as SignInError
    expect(error).toBeInstanceOf(SignInError)
    expect(error.display).toEqual({
      kind: "key",
      key: "auth.sign_in.failed",
      values: {},
    })
  })

  it("shows auth.sign_in.failed when the request never reaches the server", async () => {
    const { provider } = providerAnswering(new TypeError("Failed to fetch"))
    const error = (await rejection(
      provider.login({ email: "a@example.test", password: "p" })
    )) as SignInError
    expect(error.status).toBeNull()
    expect(error.display).toEqual({
      kind: "key",
      key: "auth.sign_in.failed",
      values: {},
    })
  })

  it.each([
    undefined,
    null,
    {},
    { email: "a@example.test" },
    { email: 1, password: "p" },
  ])("refuses the parameters %j before any request", async (params) => {
    const { provider, fetch } = providerAnswering(json(200, { step: "done" }))
    await expect(provider.login(params)).rejects.toThrow(
      /expects \{ email: string, password: string \}/
    )
    expect(fetch).not.toHaveBeenCalled()
  })
})

describe("checkAuth", () => {
  it("asks /auth/me with a GET and resolves for a signed-in account", async () => {
    const { provider, fetch } = providerAnswering(json(200, ME))
    await expect(provider.checkAuth({})).resolves.toBeUndefined()
    const [url, init] = fetch.mock.calls[0]
    expect(url).toBe("/api/admin/auth/me")
    expect(init?.method).toBe("GET")
    expect(init?.credentials).toBe("same-origin")
  })

  it("resolves for a password_change session too", async () => {
    const { provider } = providerAnswering(
      json(200, { ...ME, scope: "password_change" })
    )
    await expect(provider.checkAuth({})).resolves.toBeUndefined()
  })

  it("rejects a 401 with ra-core's sign-in prompt", async () => {
    const { provider } = providerAnswering(json(401, envelope("unauthorized")))
    const error = (await rejection(provider.checkAuth({}))) as SessionCheckError
    expect(error).toBeInstanceOf(SessionCheckError)
    expect(error.status).toBe(401)
    expect(error.message).toBe("ra.auth.auth_check_error")
  })

  it.each([
    ["a 500", json(500, envelope("internal"))],
    ["a 200 that is not an account", json(200, { pid: "" })],
    ["a 200 that is not JSON", new Response("ok", { status: 200 })],
  ])("rejects %s with the server-error notice", async (_case, answer) => {
    const { provider } = providerAnswering(answer)
    const error = (await rejection(provider.checkAuth({}))) as SessionCheckError
    expect(error.message).toBe("ra.notification.http_error")
  })

  it("rejects when the server cannot be reached", async () => {
    const { provider } = providerAnswering(new TypeError("Failed to fetch"))
    const error = (await rejection(provider.checkAuth({}))) as SessionCheckError
    expect(error.status).toBeNull()
    expect(error.message).toBe("ra.notification.http_error")
  })
})

describe("checkError", () => {
  it("signs out on a 401", async () => {
    const { provider } = providerAnswering(json(200, ME))
    const error = (await rejection(
      provider.checkError({ status: 401, message: "Unauthorized" })
    )) as SessionCheckError
    expect(error).toBeInstanceOf(SessionCheckError)
    expect(error.message).toBe("errors.unauthorized")
  })

  it("never signs out on a 403, or on any other error", async () => {
    const { provider, fetch } = providerAnswering(json(200, ME))
    for (const error of [
      { status: 403, message: "Forbidden" },
      { status: 404 },
      { status: 409 },
      { status: 422 },
      { status: 500 },
      { status: "401" },
      new Error("network"),
      undefined,
      null,
    ]) {
      await expect(provider.checkError(error)).resolves.toBeUndefined()
    }
    expect(fetch).not.toHaveBeenCalled()
  })

  it("signs out exactly when the status is 401", async () => {
    const { provider } = providerAnswering(json(200, ME))
    await fc.assert(
      fc.asyncProperty(fc.integer({ min: 100, max: 599 }), async (status) => {
        const outcome = await provider.checkError({ status }).then(
          () => "kept",
          () => "signed out"
        )
        expect(outcome).toBe(status === 401 ? "signed out" : "kept")
      })
    )
  })
})

describe("getIdentity", () => {
  it("answers the account of /auth/me, its pid as the id", async () => {
    const { provider } = providerAnswering(json(200, ME))
    await expect(provider.getIdentity?.()).resolves.toEqual({
      id: ME.pid,
      fullName: ME.name,
      email: ME.email,
      scope: "admin",
    })
  })

  it("rejects without a session", async () => {
    const { provider } = providerAnswering(json(401, envelope("unauthorized")))
    await expect(provider.getIdentity?.()).rejects.toBeInstanceOf(
      SessionCheckError
    )
  })
})

describe("logout", () => {
  it("asks the server nothing yet and lets ra-core show the sign-in page", async () => {
    const { provider, fetch } = providerAnswering(json(200, ME))
    await expect(provider.logout({})).resolves.toBeUndefined()
    expect(fetch).not.toHaveBeenCalled()
  })
})

describe("parseLoginResponse", () => {
  it("reads the done step and keeps only the step", () => {
    expect(parseLoginResponse({ step: "done" })).toEqual({ step: "done" })
    expect(parseLoginResponse({ step: "done", token: "x" })).toEqual({
      step: "done",
    })
  })

  it.each([
    ["undefined (not JSON)", undefined],
    ["null", null],
    ["an array", [{ step: "done" }]],
    ["a string", "done"],
    ["no step", {}],
    ["a numeric step", { step: 1 }],
    ["the totp step, which this build does not handle yet", { step: "totp" }],
    ["the change_password step, not handled yet", { step: "change_password" }],
    ["a step only the prototype has", { step: "toString" }],
    ["a step in other letters", { step: "Done" }],
  ])("refuses %s", (_case, body) => {
    expect(parseLoginResponse(body)).toBeUndefined()
  })

  it("accepts exactly the bodies whose step is done", () => {
    fc.assert(
      fc.property(fc.jsonValue(), (body) => {
        const isDone =
          typeof body === "object" &&
          body !== null &&
          !Array.isArray(body) &&
          (body as Record<string, unknown>).step === "done"
        expect(parseLoginResponse(body)).toEqual(
          isDone ? { step: "done" } : undefined
        )
      })
    )
    fc.assert(
      fc.property(fc.string(), (step) => {
        expect(parseLoginResponse({ step })).toEqual(
          step === "done" ? { step } : undefined
        )
      })
    )
  })
})

describe("parseMeResponse", () => {
  it("reads both scopes", () => {
    expect(parseMeResponse(ME)).toEqual(ME)
    expect(parseMeResponse({ ...ME, scope: "password_change" })).toEqual({
      ...ME,
      scope: "password_change",
    })
  })

  it.each([
    ["null", null],
    ["an array", [ME]],
    ["no pid", { ...ME, pid: undefined }],
    ["an empty pid", { ...ME, pid: "" }],
    ["a numeric name", { ...ME, name: 1 }],
    ["no email", { ...ME, email: null }],
    ["an mfa scope", { ...ME, scope: "mfa" }],
  ])("refuses %s", (_case, body) => {
    expect(parseMeResponse(body)).toBeUndefined()
  })

  it("never throws on any JSON value", () => {
    fc.assert(
      fc.property(fc.jsonValue(), (body) => {
        const me = parseMeResponse(body)
        if (me !== undefined) {
          expect(["admin", "password_change"]).toContain(me.scope)
        }
      })
    )
  })
})

describe("signInFailureMessage", () => {
  it("is invalid_credentials for every 401 body", () => {
    fc.assert(
      fc.property(fc.jsonValue(), (body) => {
        expect(signInFailureMessage(401, body)).toEqual({
          kind: "key",
          key: "errors.invalid_credentials",
          values: {},
        })
      })
    )
  })

  it("is the generic failure for any other status without an envelope", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 400, max: 599 }).filter((status) => status !== 401),
        (status) => {
          expect(signInFailureMessage(status, undefined)).toEqual({
            kind: "key",
            key: "auth.sign_in.failed",
            values: {},
          })
        }
      )
    )
  })
})
