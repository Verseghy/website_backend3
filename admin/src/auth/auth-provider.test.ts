/**
 * The auth provider against a `fetch` stub: what each method asks the admin
 * API, and what it makes of every kind of answer (ADR 0006, ADR 0021 §4).
 */
import fc from "fast-check"
import type { AuthProvider } from "ra-core"
import { describe, expect, it, vi } from "vitest"

import { parseErrorEnvelope } from "@/api/error-envelope"
import type { ErrorEnvelope } from "@/bindings/ErrorEnvelope"
import type { MeResponse } from "@/bindings/MeResponse"
import { hangingFetch } from "@/test/hanging-fetch"

import { REFRESH_INTERVAL_MS } from "./activity-refresh"
import {
  AuthProviderConfigError,
  createAuthProvider,
  isSignOutRequest,
  parseLoginResponse,
  parseMeResponse,
  REFRESH_PATH,
  REFRESH_TIMEOUT_MS,
  refreshOutcome,
  RESUME_TIMEOUT_MS,
  sameAccess,
  SessionCheckError,
  SIGN_OUT_PARAMS,
  SignInError,
  signInFailureMessage,
  SignOutError,
  signOutFailureMessage,
} from "./auth-provider"
import { createSessionExpiry, type SessionExpiry } from "./session-expiry"

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
    provider: createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      sessionExpiry: createSessionExpiry(),
    }),
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
      expect(() =>
        createAuthProvider({
          apiBaseUrl,
          fetch: vi.fn(),
          sessionExpiry: createSessionExpiry(),
        })
      ).toThrow(AuthProviderConfigError)
      expect(() =>
        createAuthProvider({
          apiBaseUrl,
          fetch: vi.fn(),
          sessionExpiry: createSessionExpiry(),
        })
      ).toThrow(
        /^apiBaseUrl must be a root-relative path such as "\/api\/admin"/
      )
    }
  )

  it("refuses a fetch that is not a function", () => {
    expect(() =>
      createAuthProvider({
        apiBaseUrl: "/api/admin",
        fetch: undefined as unknown as typeof globalThis.fetch,
        sessionExpiry: createSessionExpiry(),
      })
    ).toThrow(/fetch must be a function/)
  })

  it.each([0, -1, 0.5, Number.NaN, Number.POSITIVE_INFINITY])(
    "refuses the refreshTimeoutMs %d, which would give up at once or never",
    (refreshTimeoutMs) => {
      expect(() =>
        createAuthProvider({
          apiBaseUrl: "/api/admin",
          fetch: vi.fn(),
          refreshTimeoutMs,
          sessionExpiry: createSessionExpiry(),
        })
      ).toThrow(
        /^refreshTimeoutMs must be a finite number of milliseconds, at least 1; got .*REFRESH_TIMEOUT_MS \(30 seconds\)/
      )
    }
  )

  it.each([0, -1, 0.5, Number.NaN, Number.POSITIVE_INFINITY])(
    "refuses the resumeTimeoutMs %d, which would give up at once or never",
    (resumeTimeoutMs) => {
      expect(() =>
        createAuthProvider({
          apiBaseUrl: "/api/admin",
          fetch: vi.fn(),
          resumeTimeoutMs,
          sessionExpiry: createSessionExpiry(),
        })
      ).toThrow(
        /^resumeTimeoutMs must be a finite number of milliseconds, at least 1; got .*RESUME_TIMEOUT_MS \(30 seconds\)/
      )
    }
  )

  it("accepts a resumeTimeoutMs of 1 millisecond or more", () => {
    for (const resumeTimeoutMs of [1, 1.5, RESUME_TIMEOUT_MS]) {
      expect(() =>
        createAuthProvider({
          apiBaseUrl: "/api/admin",
          fetch: vi.fn(),
          resumeTimeoutMs,
          sessionExpiry: createSessionExpiry(),
        })
      ).not.toThrow()
    }
  })

  it("accepts a refreshTimeoutMs of 1 millisecond or more", () => {
    for (const refreshTimeoutMs of [1, 1.5, REFRESH_TIMEOUT_MS]) {
      expect(() =>
        createAuthProvider({
          apiBaseUrl: "/api/admin",
          fetch: vi.fn(),
          refreshTimeoutMs,
          sessionExpiry: createSessionExpiry(),
        })
      ).not.toThrow()
    }
  })

  it("refuses an onAccessChange that is not a function", () => {
    expect(() =>
      createAuthProvider({
        apiBaseUrl: "/api/admin",
        fetch: vi.fn(),
        sessionExpiry: createSessionExpiry(),
        onAccessChange: "invalidate" as unknown as () => void,
      })
    ).toThrow(/onAccessChange must be a function or left out/)
  })
})

/** An account that may list and edit the probe resource `probes`. */
const EDITOR: MeResponse = {
  ...ME,
  access: { probes: ["list", "show", "edit"] },
}

/**
 * The admin API's `/auth/me` playing a sequence of answers: each call takes
 * the next one, and the last one repeats.
 *
 * @param answers - the answers, in order.
 * @returns the provider, the stub, the `onAccessChange` spy and the retry
 *   queue of the in-place sign-in, which no dialog has attached to.
 */
function providerRepeatingLast(...answers: Response[]) {
  let next = 0
  const fetch = vi.fn<typeof globalThis.fetch>(async () => {
    const answer = answers[Math.min(next, answers.length - 1)]
    next += 1
    return answer.clone()
  })
  const onAccessChange = vi.fn<() => void>()
  const sessionExpiry = createSessionExpiry()
  return {
    provider: createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      sessionExpiry,
      onAccessChange,
    }),
    fetch,
    onAccessChange,
    sessionExpiry,
  }
}

/**
 * The number of `/auth/me` requests a stub received.
 *
 * @param fetch - the stub.
 * @returns how many calls asked `/auth/me`.
 */
function meReads(fetch: ReturnType<typeof vi.fn<typeof globalThis.fetch>>) {
  return fetch.mock.calls.filter(([url]) => url === "/api/admin/auth/me").length
}

describe("canAccess", () => {
  it("answers from the access map that checkAuth read, without a request of its own", async () => {
    const { provider, fetch } = providerRepeatingLast(json(200, EDITOR))
    await provider.checkAuth({})
    expect(meReads(fetch)).toBe(1)
    for (const [action, expected] of [
      ["list", true],
      ["show", true],
      ["edit", true],
      ["create", false],
      ["delete", false],
      ["purge", false],
      ["export", false],
    ] as const) {
      await expect(
        provider.canAccess?.({ action, resource: "probes" } as never)
      ).resolves.toBe(expected)
    }
    await expect(
      provider.canAccess?.({ action: "list", resource: "others" } as never)
    ).resolves.toBe(false)
    // A record changes nothing while no resource has a record-level rule.
    await expect(
      provider.canAccess?.({
        action: "edit",
        resource: "probes",
        record: { id: 1 },
      } as never)
    ).resolves.toBe(true)
    expect(meReads(fetch)).toBe(1)
  })

  it("reads /auth/me once for every question asked before the first answer", async () => {
    const { provider, fetch } = providerRepeatingLast(json(200, EDITOR))
    const answers = await Promise.all([
      provider.canAccess?.({ action: "list", resource: "probes" } as never),
      provider.canAccess?.({ action: "create", resource: "probes" } as never),
      provider.checkAuth({}),
    ])
    expect(answers.slice(0, 2)).toEqual([true, false])
    expect(meReads(fetch)).toBe(1)
  })

  it("rejects when it cannot read /auth/me, and never allows what it could not check", async () => {
    const { provider } = providerRepeatingLast(json(500, envelope("internal")))
    const error = await rejection(
      provider.canAccess?.({ action: "list", resource: "probes" } as never) ??
        Promise.resolve()
    )
    expect(error).toBeInstanceOf(SessionCheckError)
  })

  it("follows a changed account at the next read, and says so once", async () => {
    // The account loses the probe resource, for example after a role change
    // in another session (ADR 0007 §7).
    const { provider, onAccessChange } = providerRepeatingLast(
      json(200, EDITOR),
      json(200, ME)
    )
    await provider.checkAuth({})
    expect(onAccessChange).not.toHaveBeenCalled()
    await provider.checkError({ status: 403, message: "errors.forbidden" })
    expect(onAccessChange).toHaveBeenCalledTimes(1)
    await expect(
      provider.canAccess?.({ action: "list", resource: "probes" } as never)
    ).resolves.toBe(false)
    // The same answer again changes nothing.
    await provider.checkAuth({})
    expect(onAccessChange).toHaveBeenCalledTimes(1)
  })

  it("does not call onAccessChange for the first answer, or for one that changes nothing", async () => {
    const { provider, onAccessChange } = providerRepeatingLast(
      json(200, EDITOR),
      json(200, { ...EDITOR, name: "Új Név" })
    )
    await provider.checkAuth({})
    await provider.checkError({ status: 403, message: "errors.forbidden" })
    expect(onAccessChange).not.toHaveBeenCalled()
  })

  it("forgets the kept answer at a sign-in, a sign-out and a 401", async () => {
    for (const end of [
      (provider: AuthProvider) =>
        provider.login({ email: "a@example.test", password: "pw" }),
      (provider: AuthProvider) => provider.logout({}),
      (provider: AuthProvider) => provider.logout(SIGN_OUT_PARAMS),
      (provider: AuthProvider) => provider.checkAuth({}).catch(() => undefined),
    ]) {
      let reads = 0
      const fetch = vi.fn<typeof globalThis.fetch>(async (input, init) => {
        const url = String(input)
        if (url.endsWith("/auth/login")) {
          return json(200, { step: "done" })
        }
        if (url.endsWith("/auth/logout") && init?.method === "POST") {
          return new Response(null, { status: 204 })
        }
        // The first /auth/me answers the editor, every later one 401.
        reads += 1
        return reads === 1
          ? json(200, EDITOR)
          : json(401, envelope("unauthorized"))
      })
      const provider = createAuthProvider({
        apiBaseUrl: "/api/admin",
        fetch,
        sessionExpiry: createSessionExpiry(),
      })
      await provider.checkAuth({})
      await end(provider)
      // With nothing kept, the question reads /auth/me, which has no
      // session: no answer from the previous one.
      await expect(
        provider.canAccess?.({ action: "list", resource: "probes" } as never)
      ).rejects.toBeInstanceOf(SessionCheckError)
    }
  })

  it("keeps the answer while a 401 waits for the in-place sign-in, and drops it when the user gives up", async () => {
    const { provider, fetch, sessionExpiry } = providerRepeatingLast(
      json(200, EDITOR),
      json(401, envelope("unauthorized"))
    )
    sessionExpiry.attach()
    await provider.checkAuth({})
    // getIdentity never waits, and its 401 ends nothing.
    await expect(provider.getIdentity?.()).rejects.toBeInstanceOf(
      SessionCheckError
    )
    const check = rejection(provider.checkAuth({}))
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
    // A rejection here would send the open page to the authentication-error
    // page while the dialog may still renew the session.
    await expect(
      provider.canAccess?.({ action: "edit", resource: "probes" } as never)
    ).resolves.toBe(true)
    expect(meReads(fetch)).toBe(3)

    sessionExpiry.abandon()
    expect(((await check) as SessionCheckError).status).toBe(401)
    // ra-core signs out on that rejection: nothing is kept for the next one.
    await expect(
      provider.canAccess?.({ action: "list", resource: "probes" } as never)
    ).rejects.toBeInstanceOf(SessionCheckError)
  })

  it("keeps the answer when a sign-out was not confirmed", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async (input) =>
      String(input).endsWith("/auth/logout")
        ? json(500, envelope("internal"))
        : json(200, EDITOR)
    )
    const provider = createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      sessionExpiry: createSessionExpiry(),
    })
    await provider.checkAuth({})
    await expect(provider.logout(SIGN_OUT_PARAMS)).rejects.toBeInstanceOf(
      SignOutError
    )
    await expect(
      provider.canAccess?.({ action: "list", resource: "probes" } as never)
    ).resolves.toBe(true)
    expect(meReads(fetch)).toBe(1)
  })

  it("never stores an answer that was sent before a sign-out", async () => {
    let release: (answer: Response) => void = () => undefined
    let reads = 0
    const fetch = vi.fn<typeof globalThis.fetch>(
      () =>
        new Promise<Response>((resolve) => {
          reads += 1
          // Only /auth/me is asked here; the first read waits.
          if (reads === 1) {
            release = resolve
          } else {
            resolve(json(401, envelope("unauthorized")))
          }
        })
    )
    const provider = createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      sessionExpiry: createSessionExpiry(),
    })
    const slow = provider.checkAuth({})
    await provider.logout({})
    release(json(200, EDITOR))
    await slow
    await expect(
      provider.canAccess?.({ action: "list", resource: "probes" } as never)
    ).rejects.toBeInstanceOf(SessionCheckError)
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

  it("never signs out on a 403, and asks /auth/me again for it alone", async () => {
    const { provider, fetch } = providerAnswering(json(200, ME))
    await expect(
      provider.checkError({ status: 403, message: "errors.forbidden" })
    ).resolves.toBeUndefined()
    expect(fetch.mock.calls.map(([url]) => url)).toEqual(["/api/admin/auth/me"])
    expect(fetch.mock.calls[0][1]?.method).toBe("GET")
  })

  it.each([
    ["a server failure", json(500, envelope("internal"))],
    ["an answer that is no account", json(200, { pid: "" })],
    ["no answer at all", new TypeError("Failed to fetch")],
  ])(
    "keeps the session after a 403 when the refetch of /auth/me meets %s",
    async (_case, answer) => {
      const { provider } = providerAnswering(answer)
      await expect(
        provider.checkError({ status: 403, message: "errors.forbidden" })
      ).resolves.toBeUndefined()
    }
  )

  /**
   * A provider whose `/auth/me` answers are released by hand, in order.
   *
   * @returns the provider, the `onAccessChange` spy, the retry queue of the
   *   in-place sign-in (no dialog attached), the number of reads sent so
   *   far, and `answer`, which releases the oldest waiting read.
   */
  function providerAnsweringByHand() {
    const waiting: ((answer: Response) => void)[] = []
    let sent = 0
    const fetch = vi.fn<typeof globalThis.fetch>(
      () =>
        new Promise<Response>((resolve) => {
          sent += 1
          waiting.push(resolve)
        })
    )
    const onAccessChange = vi.fn<() => void>()
    const sessionExpiry = createSessionExpiry()
    const provider = createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      sessionExpiry,
      onAccessChange,
    })
    /** Lets queued promise callbacks run, so a chained read is sent. */
    const settle = () => new Promise((resolve) => setTimeout(resolve, 0))
    return {
      provider,
      onAccessChange,
      sessionExpiry,
      settle,
      sent: () => sent,
      answer: async (body: Response) => {
        const release = waiting.shift()
        expect(release, "a read is waiting").toBeDefined()
        release?.(body)
        await settle()
      },
    }
  }

  it("refetches with a read sent after the 403, not one already in flight", async () => {
    // The account loses the probe resource between a read that is in
    // flight and the refused call: only a read sent after the refusal can
    // say so.
    const { provider, onAccessChange, settle, sent, answer } =
      providerAnsweringByHand()
    const first = provider.checkAuth({})
    await answer(json(200, EDITOR))
    await first
    const inFlight = provider.checkAuth({})
    await settle()
    expect(sent()).toBe(2)
    const refused = provider.checkError({
      status: 403,
      message: "errors.forbidden",
    })
    await settle()
    // The refetch waits for the read in flight instead of joining it.
    expect(sent()).toBe(2)
    await answer(json(200, EDITOR))
    await inFlight
    expect(sent()).toBe(3)
    await answer(json(200, ME))
    await expect(refused).resolves.toBeUndefined()
    expect(onAccessChange).toHaveBeenCalledTimes(1)
    await expect(
      provider.canAccess?.({ action: "list", resource: "probes" } as never)
    ).resolves.toBe(false)
  })

  it("sends one read for a burst of 403s that arrive while a read is in flight", async () => {
    const { provider, settle, sent, answer } = providerAnsweringByHand()
    const inFlight = provider.checkAuth({})
    await settle()
    const refusals = [1, 2, 3].map(() =>
      provider.checkError({ status: 403, message: "errors.forbidden" })
    )
    await settle()
    expect(sent()).toBe(1)
    await answer(json(200, EDITOR))
    await inFlight
    // One more read for all three, sent after the first settled.
    expect(sent()).toBe(2)
    await answer(json(200, ME))
    await expect(Promise.all(refusals)).resolves.toEqual([
      undefined,
      undefined,
      undefined,
    ])
    expect(sent()).toBe(2)
  })

  it("sends no waiting refetch once the user signed out", async () => {
    const { provider, settle, sent, answer } = providerAnsweringByHand()
    const inFlight = provider.checkAuth({}).catch(() => undefined)
    await settle()
    const refused = provider.checkError({
      status: 403,
      message: "errors.forbidden",
    })
    await provider.logout({})
    await answer(json(200, EDITOR))
    await inFlight
    await expect(refused).resolves.toBeUndefined()
    // The read in flight, and nothing for the session that ended.
    expect(sent()).toBe(1)
  })

  it("signs out after a waiting refetch says the session has ended", async () => {
    const { provider, settle, answer } = providerAnsweringByHand()
    const inFlight = provider.checkAuth({})
    await settle()
    const refused = rejection(
      provider.checkError({ status: 403, message: "errors.forbidden" })
    )
    await answer(json(200, EDITOR))
    await inFlight
    await answer(json(401, envelope("unauthorized")))
    const error = (await refused) as SessionCheckError
    expect(error).toBeInstanceOf(SessionCheckError)
    expect(error.status).toBe(401)
    expect(error.message).toBe("errors.unauthorized")
  })

  it("signs out after a 403 when the refetch says the session has ended", async () => {
    const { provider } = providerAnswering(json(401, envelope("unauthorized")))
    const error = (await rejection(
      provider.checkError({ status: 403, message: "errors.forbidden" })
    )) as SessionCheckError
    expect(error).toBeInstanceOf(SessionCheckError)
    expect(error.status).toBe(401)
    expect(error.message).toBe("errors.unauthorized")
  })

  it("lets the refetch of a 403 wait for the in-place sign-in, and reads again after it", async () => {
    // The session ends between the refused call and the refetch, under an
    // open page: signing out would drop the page and its form (ADR 0006,
    // "Follow-up work").
    const { provider, onAccessChange, sessionExpiry, settle, sent, answer } =
      providerAnsweringByHand()
    sessionExpiry.attach()
    const first = provider.checkAuth({})
    await answer(json(200, EDITOR))
    await first
    let settled = false
    const refused = provider
      .checkError({ status: 403, message: "errors.forbidden" })
      .finally(() => {
        settled = true
      })
    await settle()
    await answer(json(401, envelope("unauthorized")))
    expect(sessionExpiry.isWaiting()).toBe(true)
    expect(settled).toBe(false)
    // The open page's questions are still answered, from the kept answer.
    await expect(
      provider.canAccess?.({ action: "edit", resource: "probes" } as never)
    ).resolves.toBe(true)
    expect(sent()).toBe(2)

    sessionExpiry.resume()
    await settle()
    expect(sent()).toBe(3)
    await answer(json(200, ME))
    await expect(refused).resolves.toBeUndefined()
    expect(onAccessChange).toHaveBeenCalledTimes(1)
    await expect(
      provider.canAccess?.({ action: "list", resource: "probes" } as never)
    ).resolves.toBe(false)
  })

  it("signs out after a 403 when the user gives the in-place sign-in up", async () => {
    const { provider, sessionExpiry, settle, answer } =
      providerAnsweringByHand()
    sessionExpiry.attach()
    const first = provider.checkAuth({})
    await answer(json(200, EDITOR))
    await first
    const refused = rejection(
      provider.checkError({ status: 403, message: "errors.forbidden" })
    )
    await settle()
    await answer(json(401, envelope("unauthorized")))
    expect(sessionExpiry.isWaiting()).toBe(true)
    sessionExpiry.abandon()
    const error = (await refused) as SessionCheckError
    expect(error).toBeInstanceOf(SessionCheckError)
    expect(error.status).toBe(401)
    expect(error.message).toBe("errors.unauthorized")
  })

  it("never signs out on any other error, and asks nothing for it", async () => {
    const { provider, fetch } = providerAnswering(json(200, ME))
    for (const error of [
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
  it.each([{}, undefined, null, { endSession: "true" }, { endSession: 1 }])(
    "asks the server nothing for ra-core's own call with %j and lets it show the sign-in page",
    async (params) => {
      // ra-core calls logout({}) after checkAuth or checkError found no
      // session: there is nothing to end on the server.
      const { provider, fetch } = providerAnswering(json(500, {}))
      await expect(provider.logout(params)).resolves.toBeUndefined()
      expect(fetch).not.toHaveBeenCalled()
    }
  )

  it("posts /auth/logout for the user's sign-out and resolves on 204", async () => {
    const { provider, fetch } = providerAnswering(
      new Response(null, { status: 204 })
    )
    await expect(provider.logout(SIGN_OUT_PARAMS)).resolves.toBeUndefined()
    expect(fetch).toHaveBeenCalledTimes(1)
    const [url, init] = fetch.mock.calls[0]
    expect(url).toBe("/api/admin/auth/logout")
    expect(init).toEqual({
      method: "POST",
      credentials: "same-origin",
      headers: { Accept: "application/json" },
    })
  })

  it("resolves on 401: the session had ended already", async () => {
    const { provider } = providerAnswering(json(401, envelope("unauthorized")))
    await expect(provider.logout(SIGN_OUT_PARAMS)).resolves.toBeUndefined()
  })

  it.each([
    ["a server error without an envelope", json(500, { error: "x" })],
    ["a proxy's page", new Response("<html>502</html>", { status: 502 })],
    ["a 200, which is not the server's confirmation", json(200, {})],
    ["a 403", json(403, {})],
  ])(
    "rejects %s with auth.sign_out.failed, keeping the user signed in",
    async (_case, answer) => {
      const { provider } = providerAnswering(answer)
      const error = await rejection(provider.logout(SIGN_OUT_PARAMS))
      expect(error).toBeInstanceOf(SignOutError)
      expect((error as SignOutError).status).toBe(answer.status)
      expect((error as SignOutError).display).toEqual({
        kind: "key",
        key: "auth.sign_out.failed",
        values: {},
      })
      expect((error as SignOutError).message).toBe("auth.sign_out.failed")
    }
  )

  it("rejects a refusal with its envelope's message", async () => {
    const known = providerAnswering(json(422, envelope("invalid_body")))
    expect(
      (
        (await rejection(
          known.provider.logout(SIGN_OUT_PARAMS)
        )) as SignOutError
      ).display
    ).toEqual({ kind: "key", key: "errors.invalid_body", values: {} })

    // A logout from a page the server does not take for the admin's own
    // (ADR 0006 §3, ADR 0044): the session holds, and the user is told why.
    const foreign = providerAnswering(json(403, envelope("origin_mismatch")))
    const refused = (await rejection(
      foreign.provider.logout(SIGN_OUT_PARAMS)
    )) as SignOutError
    expect(refused.status).toBe(403)
    expect(refused.display).toEqual({
      kind: "key",
      key: "errors.origin_mismatch",
      values: {},
    })

    // A code this build does not know shows the server's text (ADR 0021 §4).
    const unknown = providerAnswering(json(403, envelope("session_conflict")))
    const error = (await rejection(
      unknown.provider.logout(SIGN_OUT_PARAMS)
    )) as SignOutError
    expect(error.status).toBe(403)
    expect(error.display).toEqual({
      kind: "text",
      text: "English text for session_conflict.",
    })
    expect(error.message).toBe("English text for session_conflict.")
  })

  it("rejects with auth.sign_out.failed when the request never reaches the server", async () => {
    const { provider } = providerAnswering(new TypeError("Failed to fetch"))
    const error = await rejection(provider.logout(SIGN_OUT_PARAMS))
    expect(error).toBeInstanceOf(SignOutError)
    expect((error as SignOutError).status).toBeNull()
    expect((error as SignOutError).display).toEqual({
      kind: "key",
      key: "auth.sign_out.failed",
      values: {},
    })
  })

  it("ends the session exactly for 204 and 401", async () => {
    await fc.assert(
      fc.asyncProperty(fc.integer({ min: 200, max: 599 }), async (status) => {
        // A null-body status (204, 205, 304) cannot carry a body; the others
        // carry an empty JSON object.
        const answer = [204, 205, 304].includes(status)
          ? new Response(null, { status })
          : json(status, {})
        const { provider } = providerAnswering(answer)
        const outcome = await provider.logout(SIGN_OUT_PARAMS).then(
          () => "signed out",
          (error: unknown) => error
        )
        if (status === 204 || status === 401) {
          expect(outcome).toBe("signed out")
        } else {
          expect(outcome).toBeInstanceOf(SignOutError)
        }
      }),
      // The two that end it, and the other null-body statuses, every run.
      { examples: [[204], [401], [205], [304], [200]] }
    )
  })
})

describe("refreshSession", () => {
  it("posts /auth/refresh and reports a renewed session on 204 (ADR 0006 §1)", async () => {
    const { provider, fetch } = providerAnswering(
      new Response(null, { status: 204 })
    )
    await expect(provider.refreshSession()).resolves.toBe("refreshed")
    expect(REFRESH_PATH).toBe("/auth/refresh")
    expect(fetch).toHaveBeenCalledTimes(1)
    const [url, init] = fetch.mock.calls[0]
    expect(url).toBe("/api/admin/auth/refresh")
    expect(init).toEqual({
      method: "POST",
      credentials: "same-origin",
      headers: { Accept: "application/json" },
      signal: expect.any(AbortSignal),
    })
  })

  it("bounds the call by REFRESH_TIMEOUT_MS, well below the tracker's interval", async () => {
    expect(REFRESH_TIMEOUT_MS).toBe(30 * 1000)
    expect(REFRESH_TIMEOUT_MS).toBeLessThan(REFRESH_INTERVAL_MS)
    const timeout = vi.spyOn(AbortSignal, "timeout")
    try {
      const { provider, fetch } = providerAnswering(
        new Response(null, { status: 204 })
      )
      await provider.refreshSession()
      expect(timeout).toHaveBeenCalledExactlyOnceWith(REFRESH_TIMEOUT_MS)
      expect(fetch.mock.calls[0][1]?.signal).toBe(timeout.mock.results[0].value)
    } finally {
      timeout.mockRestore()
    }
  })

  it("gives up on a call that has no answer after the timeout, and reports a failure", async () => {
    const { fetch, signals } = hangingFetch()
    const provider = createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      refreshTimeoutMs: 20,
      sessionExpiry: createSessionExpiry(),
    })
    await expect(provider.refreshSession()).resolves.toBe("failed")
    expect(signals).toHaveLength(1)
    expect(signals[0].aborted).toBe(true)
    expect((signals[0].reason as DOMException).name).toBe("TimeoutError")
  })

  it("reports an ended session on 401 and never rejects or signs out", async () => {
    const { provider, fetch } = providerAnswering(
      json(401, envelope("unauthorized"))
    )
    await expect(provider.refreshSession()).resolves.toBe("ended")
    // Only the refresh was asked: no sign-out request follows.
    expect(fetch).toHaveBeenCalledTimes(1)
  })

  it("reports a failure for any other answer and for no answer", async () => {
    for (const answer of [
      json(500, envelope("internal")),
      json(403, envelope("forbidden")),
      json(200, {}),
      new TypeError("Failed to fetch"),
    ]) {
      const { provider } = providerAnswering(answer)
      await expect(provider.refreshSession()).resolves.toBe("failed")
    }
  })
})

describe("refreshOutcome", () => {
  it("is refreshed for 204, ended for 401 and failed for every other status", () => {
    fc.assert(
      fc.property(fc.integer({ min: 100, max: 599 }), (status) => {
        const expected =
          status === 204 ? "refreshed" : status === 401 ? "ended" : "failed"
        expect(refreshOutcome(status)).toBe(expected)
      })
    )
  })
})

describe("isSignOutRequest", () => {
  it("is true for the user menu's params only", () => {
    expect(isSignOutRequest(SIGN_OUT_PARAMS)).toBe(true)
    expect(isSignOutRequest({ endSession: true, other: 1 })).toBe(true)
    fc.assert(
      fc.property(fc.anything(), (params) => {
        const expected =
          typeof params === "object" &&
          params !== null &&
          (params as { endSession?: unknown }).endSession === true
        expect(isSignOutRequest(params)).toBe(expected)
      })
    )
  })

  it("is frozen, so no caller can switch it off", () => {
    expect(Object.isFrozen(SIGN_OUT_PARAMS)).toBe(true)
  })
})

describe("signOutFailureMessage", () => {
  it("is the generic failure for any body without an envelope", () => {
    fc.assert(
      fc.property(
        fc.jsonValue().filter((body) => parseErrorEnvelope(body) === undefined),
        (body) => {
          expect(signOutFailureMessage(body)).toEqual({
            kind: "key",
            key: "auth.sign_out.failed",
            values: {},
          })
        }
      )
    )
    expect(signOutFailureMessage(undefined)).toEqual({
      kind: "key",
      key: "auth.sign_out.failed",
      values: {},
    })
  })

  it("is the envelope's message for an envelope", () => {
    expect(signOutFailureMessage(envelope("body_too_large"))).toEqual({
      kind: "key",
      key: "errors.body_too_large",
      values: {},
    })
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
    ["no roles", { ...ME, roles: undefined }],
    ["an unknown role", { ...ME, roles: ["editor"] }],
    ["a role twice", { ...ME, roles: ["writer", "writer"] }],
    ["roles that are not a list", { ...ME, roles: "writer" }],
    ["no permissions", { ...ME, permissions: undefined }],
    // Never created (ADR 0007 follow-up points 1 and 2).
    ["a retired permission", { ...ME, permissions: ["edit newsletter"] }],
    [
      "a permission in another spelling",
      { ...ME, permissions: ["edit_posts"] },
    ],
    ["permissions that are not a list", { ...ME, permissions: {} }],
    ["no admin flag", { ...ME, is_admin: undefined }],
    ["an admin flag that is not a boolean", { ...ME, is_admin: "true" }],
    ["no access map", { ...ME, access: undefined }],
    ["an access map that is a list", { ...ME, access: [] }],
    ["an unknown action", { ...ME, access: { probes: ["export"] } }],
    ["an action twice", { ...ME, access: { probes: ["list", "list"] } }],
    ["actions that are not a list", { ...ME, access: { probes: "list" } }],
  ])("refuses %s", (_case, body) => {
    expect(parseMeResponse(body)).toBeUndefined()
  })

  it("reads the roles, the permissions and the admin flag as sent", () => {
    const admin: MeResponse = {
      ...ME,
      roles: ["secretary", "admin"],
      permissions: ["edit posts", "view logs", "edit diaktoll"],
      is_admin: true,
    }
    expect(parseMeResponse(admin)).toEqual(admin)
    // An account without roles or grants is still an account.
    expect(
      parseMeResponse({ ...ME, roles: [], permissions: [], is_admin: false })
    ).toEqual({ ...ME, roles: [], permissions: [], is_admin: false })
  })

  it("reads the access map as sent", () => {
    expect(parseMeResponse(EDITOR)).toEqual(EDITOR)
    // A member named __proto__, as JSON.parse reads it from the network,
    // stays an own member and sets no prototype.
    const text = JSON.stringify({
      ...ME,
      access: { placeholder: ["list"] },
    }).replace('"placeholder"', '"__proto__"')
    const access = parseMeResponse(JSON.parse(text))?.access ?? {}
    expect(Object.getPrototypeOf(access)).toBe(Object.prototype)
    expect(Object.hasOwn(access, "__proto__")).toBe(true)
  })

  it("never throws on any access map, and accepts exactly lists of known actions", () => {
    const actions = [
      "list",
      "show",
      "create",
      "edit",
      "delete",
      "publish",
      "restore",
      "purge",
      "history",
    ]
    fc.assert(
      fc.property(
        fc.dictionary(
          fc.string(),
          fc.oneof(
            fc.uniqueArray(fc.constantFrom(...actions)),
            fc.array(fc.string()),
            fc.jsonValue()
          )
        ),
        (access) => {
          const known = Object.values(access).every(
            (value) =>
              Array.isArray(value) &&
              value.every((action) => actions.includes(action as string)) &&
              new Set(value).size === value.length
          )
          expect(parseMeResponse({ ...ME, access }) !== undefined).toBe(known)
        }
      )
    )
  })

  it("keeps no member beyond the MeResponse ones", () => {
    expect(parseMeResponse({ ...ME, token: "x" })).toEqual(ME)
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

describe("sameAccess", () => {
  it("compares what the account may do, not its name or address", () => {
    expect(sameAccess(ME, { ...ME })).toBe(true)
    expect(
      sameAccess(ME, { ...ME, name: "Más", email: "mas@example.test" })
    ).toBe(true)
    // The roles reach an access question only through the permissions and
    // the admin flag, which the server sends with them.
    expect(sameAccess(ME, { ...ME, roles: ["writer", "secretary"] })).toBe(true)
    const changes: MeResponse[] = [
      { ...ME, pid: "1d5e3f2a-0000-4000-8000-000000000000" },
      { ...ME, scope: "password_change" },
      { ...ME, is_admin: true },
      { ...ME, permissions: [] },
      EDITOR,
      { ...EDITOR, access: { probes: ["list"] } },
    ]
    for (const changed of changes) {
      expect(sameAccess(ME, changed)).toBe(false)
      expect(sameAccess(changed, ME)).toBe(false)
    }
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

describe("createAuthProvider sessionExpiry", () => {
  it.each([undefined, null, {}, { waitForSignIn: "no" }])(
    "refuses the sessionExpiry %j, which is not a retry queue",
    (sessionExpiry) => {
      expect(() =>
        createAuthProvider({
          apiBaseUrl: "/api/admin",
          fetch: vi.fn(),
          sessionExpiry: sessionExpiry as unknown as SessionExpiry,
        })
      ).toThrow(
        /^sessionExpiry must be the retry queue of createSessionExpiry\(\); main\.tsx creates one/
      )
    }
  )
})

/**
 * A provider whose `fetch` answers the calls in order, over a queue the
 * test controls (ADR 0006, "Follow-up work").
 *
 * @param answers - one answer per call, in order.
 * @returns the provider, the stub and the queue.
 */
function providerAnsweringInTurn(...answers: Response[]) {
  const fetch = vi.fn<typeof globalThis.fetch>(async () => {
    const answer = answers.shift()
    if (answer === undefined) {
      throw new Error("the test gave no answer for this call")
    }
    return answer
  })
  const sessionExpiry = createSessionExpiry()
  return {
    provider: createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      sessionExpiry,
    }),
    fetch,
    sessionExpiry,
  }
}

describe("checkAuth under an open page (the in-place sign-in)", () => {
  it("waits on a 401 while the dialog is mounted, and asks again after the sign-in", async () => {
    const { provider, fetch, sessionExpiry } = providerAnsweringInTurn(
      json(401, envelope("unauthorized")),
      json(200, ME)
    )
    sessionExpiry.attach()
    const check = provider.checkAuth({})
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
    expect(fetch).toHaveBeenCalledTimes(1)

    sessionExpiry.resume()
    await expect(check).resolves.toBeUndefined()
    expect(fetch).toHaveBeenCalledTimes(2)
    expect(fetch.mock.calls[1][0]).toBe("/api/admin/auth/me")
  })

  it("releases the body of every 401 it does not read, before it waits", async () => {
    // A cancelled body counts as used: nothing can read it any more, and its
    // connection is free for the read after the sign-in.
    const ended = json(401, envelope("unauthorized"))
    const other = json(401, envelope("invalid_credentials"))
    const { provider, sessionExpiry } = providerAnsweringInTurn(
      ended,
      json(200, ME),
      other
    )
    sessionExpiry.attach()
    const check = provider.checkAuth({})
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
    expect(ended.bodyUsed).toBe(true)
    sessionExpiry.resume()
    await expect(check).resolves.toBeUndefined()

    // A 401 with another code is not the ended session's: it rejects at
    // once, its body released too.
    await expect(provider.getIdentity?.()).rejects.toMatchObject({
      status: 401,
    })
    expect(other.bodyUsed).toBe(true)
  })

  it("rejects with ra-core's sign-in prompt when the user gives up", async () => {
    const { provider, sessionExpiry } = providerAnsweringInTurn(
      json(401, envelope("unauthorized"))
    )
    sessionExpiry.attach()
    const check = rejection(provider.checkAuth({}))
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
    sessionExpiry.abandon()
    const error = (await check) as SessionCheckError
    expect(error).toBeInstanceOf(SessionCheckError)
    expect(error.status).toBe(401)
    expect(error.message).toBe("ra.auth.auth_check_error")
  })

  it("stops waiting when ra-core cancels the check", async () => {
    const { provider, sessionExpiry } = providerAnsweringInTurn(
      json(401, envelope("unauthorized"))
    )
    sessionExpiry.attach()
    const controller = new AbortController()
    const check = rejection(provider.checkAuth({ signal: controller.signal }))
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
    controller.abort()
    const error = (await check) as SessionCheckError
    expect(error.status).toBeNull()
    expect(error.message).toBe("ra.notification.http_error")
    expect(sessionExpiry.isWaiting()).toBe(false)
  })

  it("never makes getIdentity wait: its 401 rejects at once", async () => {
    const { provider, sessionExpiry } = providerAnsweringInTurn(
      json(401, envelope("unauthorized"))
    )
    sessionExpiry.attach()
    await expect(provider.getIdentity?.()).rejects.toMatchObject({
      status: 401,
      message: "ra.auth.auth_check_error",
    })
    expect(sessionExpiry.isWaiting()).toBe(false)
  })
})

describe("sessionAccount", () => {
  it("is unknown before the first /auth/me answer", () => {
    const { provider } = providerAnsweringInTurn()
    expect(provider.sessionAccount()).toBeUndefined()
  })

  it("is the account of the last /auth/me answer, from checkAuth or getIdentity", async () => {
    const other = { ...ME, pid: "1c8d5b63-8f4e-4d66-8b80-5c4d3e2f1a0b" }
    const { provider } = providerAnsweringInTurn(
      json(200, ME),
      json(200, other)
    )
    await provider.checkAuth({})
    expect(provider.sessionAccount()).toEqual({
      pid: ME.pid,
      name: ME.name,
      email: ME.email,
    })
    await provider.getIdentity?.()
    expect(provider.sessionAccount()?.pid).toBe(other.pid)
  })

  it.each([
    ["ra-core's own logout", {}, []],
    [
      "a confirmed sign-out",
      SIGN_OUT_PARAMS,
      [new Response(null, { status: 204 })],
    ],
    [
      "a sign-out of an ended session",
      SIGN_OUT_PARAMS,
      [json(401, envelope("unauthorized"))],
    ],
  ])("is forgotten after %s", async (_case, params, answers) => {
    const { provider } = providerAnsweringInTurn(json(200, ME), ...answers)
    await provider.checkAuth({})
    await provider.logout(params)
    expect(provider.sessionAccount()).toBeUndefined()
  })

  it.each([
    ["ra-core's own logout", {}, []],
    [
      "a confirmed sign-out",
      SIGN_OUT_PARAMS,
      [new Response(null, { status: 204 })],
    ],
  ])(
    "gives the in-place sign-in up after %s, so no later 401 waits",
    async (_case, params, answers) => {
      const { provider, sessionExpiry } = providerAnsweringInTurn(
        json(200, ME),
        json(401, envelope("unauthorized")),
        ...answers,
        json(401, envelope("unauthorized"))
      )
      sessionExpiry.attach()
      await provider.checkAuth({})
      // A check is waiting when ra-core signs out.
      const waiting = rejection(provider.checkAuth({}))
      await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
      await provider.logout(params)
      expect(((await waiting) as SessionCheckError).status).toBe(401)
      // ra-core's last check before it leaves: answered at once.
      const error = (await rejection(
        provider.checkAuth({})
      )) as SessionCheckError
      expect(error.status).toBe(401)
      expect(sessionExpiry.isWaiting()).toBe(false)
    }
  )

  it("is kept when the server did not confirm the sign-out", async () => {
    const { provider } = providerAnsweringInTurn(
      json(200, ME),
      json(500, envelope("internal"))
    )
    await provider.checkAuth({})
    await rejection(provider.logout(SIGN_OUT_PARAMS))
    expect(provider.sessionAccount()?.pid).toBe(ME.pid)
  })
})

describe("resumeSession", () => {
  it("signs the same account in again and answers resumed", async () => {
    const { provider, fetch } = providerAnsweringInTurn(
      json(200, ME),
      json(200, { step: "done" }),
      json(200, ME)
    )
    await provider.checkAuth({})
    await expect(provider.resumeSession("helyes jelszó")).resolves.toBe(
      "resumed"
    )
    const [loginUrl, loginInit] = fetch.mock.calls[1]
    expect(loginUrl).toBe("/api/admin/auth/login")
    expect(loginInit?.method).toBe("POST")
    expect(JSON.parse(String(loginInit?.body))).toEqual({
      email: ME.email,
      password: "helyes jelszó",
    })
    expect(fetch.mock.calls[2][0]).toBe("/api/admin/auth/me")
  })

  it.each([
    [
      "another account holds the new session",
      { ...ME, pid: "1c8d5b63-8f4e-4d66-8b80-5c4d3e2f1a0b" },
    ],
    [
      "the new session is not a full session",
      { ...ME, scope: "password_change" },
    ],
  ])("answers restart when %s", async (_case, newMe) => {
    const { provider } = providerAnsweringInTurn(
      json(200, ME),
      json(200, { step: "done" }),
      json(200, newMe)
    )
    await provider.checkAuth({})
    await expect(provider.resumeSession("helyes jelszó")).resolves.toBe(
      "restart"
    )
    // Still the account whose session ended: the dialog keeps showing it,
    // and a second try signs that account in, not the cookie's.
    expect(provider.sessionAccount()).toEqual({
      pid: ME.pid,
      name: ME.name,
      email: ME.email,
    })
  })

  it("remembers the account of a resumed session", async () => {
    const renamed = { ...ME, name: "Átnevezett Szerkesztő" }
    const { provider } = providerAnsweringInTurn(
      json(200, ME),
      json(200, { step: "done" }),
      json(200, renamed)
    )
    await provider.checkAuth({})
    await provider.resumeSession("helyes jelszó")
    expect(provider.sessionAccount()?.name).toBe(renamed.name)
  })

  it.each(["change_password", "totp"])(
    "rejects a %s step with auth.sign_in.failed, as long as login knows only done",
    async (step) => {
      // ADR 0006 §4's two later steps: until their slices teach the
      // provider about them, the in-place sign-in treats them as a failed
      // sign-in and keeps the dialog open; it never asks /auth/me.
      const { provider, fetch } = providerAnsweringInTurn(
        json(200, ME),
        json(200, { step })
      )
      await provider.checkAuth({})
      const error = (await rejection(
        provider.resumeSession("helyes jelszó")
      )) as SignInError
      expect(error).toBeInstanceOf(SignInError)
      expect(error.message).toBe("auth.sign_in.failed")
      expect(fetch).toHaveBeenCalledTimes(2)
    }
  )

  it("bounds both requests by one RESUME_TIMEOUT_MS signal", async () => {
    expect(RESUME_TIMEOUT_MS).toBe(30 * 1000)
    const timeout = vi.spyOn(AbortSignal, "timeout")
    try {
      const { provider, fetch } = providerAnsweringInTurn(
        json(200, ME),
        json(200, { step: "done" }),
        json(200, ME)
      )
      await provider.checkAuth({})
      await provider.resumeSession("helyes jelszó")
      expect(timeout).toHaveBeenCalledExactlyOnceWith(RESUME_TIMEOUT_MS)
      const signal = timeout.mock.results[0].value as AbortSignal
      expect(fetch.mock.calls[1][1]?.signal).toBe(signal)
      expect(fetch.mock.calls[2][1]?.signal).toBe(signal)
    } finally {
      timeout.mockRestore()
    }
  })

  it.each([
    ["the sign-in", 1],
    ["the check of /auth/me after it", 2],
  ])(
    "gives up with auth.sign_in.failed when %s has no answer after the timeout",
    async (_case, stalledCall) => {
      const { fetch: hanging, signals } = hangingFetch()
      const answers = [json(200, ME), json(200, { step: "done" })]
      let calls = 0
      const fetch = vi.fn<typeof globalThis.fetch>(async (input, init) => {
        calls += 1
        return calls - 1 === stalledCall
          ? hanging(input, init)
          : (answers.shift() as Response)
      })
      const sessionExpiry = createSessionExpiry()
      sessionExpiry.attach()
      const provider = createAuthProvider({
        apiBaseUrl: "/api/admin",
        fetch,
        resumeTimeoutMs: 20,
        sessionExpiry,
      })
      await provider.checkAuth({})
      const error = (await rejection(
        provider.resumeSession("helyes jelszó")
      )) as SignInError
      expect(error).toBeInstanceOf(SignInError)
      expect(error.message).toBe("auth.sign_in.failed")
      expect(signals).toHaveLength(1)
      expect((signals[0].reason as DOMException).name).toBe("TimeoutError")
      // The dialog's own check never waits on the dialog.
      expect(sessionExpiry.isWaiting()).toBe(false)
      expect(provider.sessionAccount()?.pid).toBe(ME.pid)
    }
  )

  it("rejects a wrong password with the sign-in screen's generic message", async () => {
    const { provider, fetch } = providerAnsweringInTurn(
      json(200, ME),
      json(401, envelope("invalid_credentials"))
    )
    await provider.checkAuth({})
    const error = (await rejection(
      provider.resumeSession("rossz jelszó")
    )) as SignInError
    expect(error).toBeInstanceOf(SignInError)
    expect(error.status).toBe(401)
    expect(error.display).toEqual({
      kind: "key",
      key: "errors.invalid_credentials",
      values: {},
    })
    expect(fetch).toHaveBeenCalledTimes(2)
  })

  it("rejects with auth.sign_in.failed, sending nothing, before any account is known", async () => {
    const { provider, fetch } = providerAnsweringInTurn()
    const error = (await rejection(
      provider.resumeSession("helyes jelszó")
    )) as SignInError
    expect(error).toBeInstanceOf(SignInError)
    expect(error.message).toBe("auth.sign_in.failed")
    expect(fetch).not.toHaveBeenCalled()
  })

  it.each([
    ["answers 401", json(401, envelope("unauthorized"))],
    ["fails", json(500, envelope("internal"))],
  ])(
    "rejects with auth.sign_in.failed, without waiting, when /auth/me %s after the sign-in",
    async (_case, meAnswer) => {
      const { provider, sessionExpiry } = providerAnsweringInTurn(
        json(200, ME),
        json(200, { step: "done" }),
        meAnswer
      )
      // A mounted dialog must not make the dialog's own check wait on itself.
      sessionExpiry.attach()
      await provider.checkAuth({})
      const error = (await rejection(
        provider.resumeSession("helyes jelszó")
      )) as SignInError
      expect(error.message).toBe("auth.sign_in.failed")
      expect(sessionExpiry.isWaiting()).toBe(false)
    }
  )

  it("refuses a password that is not a string before any request", async () => {
    const { provider, fetch } = providerAnsweringInTurn(json(200, ME))
    await provider.checkAuth({})
    await expect(
      provider.resumeSession(undefined as unknown as string)
    ).rejects.toThrow(TypeError)
    expect(fetch).toHaveBeenCalledTimes(1)
  })

  it("keeps the access answer of a resumed session, and has ra-core re-ask when it changed", async () => {
    // The account lost the probe resource while its session had ended
    // (ADR 0007 §7): the page goes on with the access it has now.
    const { provider, fetch, onAccessChange } = providerRepeatingLast(
      json(200, EDITOR),
      json(200, { step: "done" }),
      json(200, ME)
    )
    await provider.checkAuth({})
    await expect(provider.resumeSession("helyes jelszó")).resolves.toBe(
      "resumed"
    )
    expect(onAccessChange).toHaveBeenCalledTimes(1)
    await expect(
      provider.canAccess?.({ action: "list", resource: "probes" } as never)
    ).resolves.toBe(false)
    expect(meReads(fetch)).toBe(2)
  })

  it("keeps the ended session's access answer after a restart", async () => {
    const { provider, fetch, onAccessChange } = providerRepeatingLast(
      json(200, EDITOR),
      json(200, { step: "done" }),
      json(200, { ...ME, pid: "1c8d5b63-8f4e-4d66-8b80-5c4d3e2f1a0b" })
    )
    await provider.checkAuth({})
    await expect(provider.resumeSession("helyes jelszó")).resolves.toBe(
      "restart"
    )
    // The dialog now ends the other account's session; until then the
    // page answers for the account whose session ended, never the other.
    expect(onAccessChange).not.toHaveBeenCalled()
    await expect(
      provider.canAccess?.({ action: "list", resource: "probes" } as never)
    ).resolves.toBe(true)
    expect(meReads(fetch)).toBe(2)
  })
})
