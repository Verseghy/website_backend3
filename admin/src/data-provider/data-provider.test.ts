import { describe, expect, it, vi } from "vitest"

import { createSessionExpiry, type SessionExpiry } from "@/auth/session-expiry"
import { createI18nProvider } from "@/i18n/provider"

import {
  DATA_PROVIDER_METHODS,
  DataProviderConfigError,
  DataProviderNotImplementedError,
  NOT_AVAILABLE_MESSAGE_KEY,
  createDataProvider,
  type ApiRequestInit,
  type DataProviderContext,
  type ResourceHandlers,
} from "./data-provider"

/**
 * The two options that these tests do not exercise: a `fetch` that must
 * not be called and a retry queue with no dialog attached.
 *
 * @returns the options, fresh for each provider.
 */
function stubs() {
  return {
    fetch: vi.fn<typeof globalThis.fetch>(() => {
      throw new Error("this test sends no request")
    }),
    sessionExpiry: createSessionExpiry(),
  }
}

describe("createDataProvider", () => {
  it("declares supportAbortSignal, so ra-core passes its queries' signal", () => {
    const provider = createDataProvider({
      ...stubs(),
      apiBaseUrl: "/api/admin",
      resources: {},
    })
    expect(provider.supportAbortSignal).toBe(true)
  })

  it("implements every ra-core data provider method", () => {
    const provider = createDataProvider({
      ...stubs(),
      apiBaseUrl: "/api/admin",
      resources: {},
    })
    for (const method of DATA_PROVIDER_METHODS) {
      expect(typeof provider[method], method).toBe("function")
    }
  })

  it.each(DATA_PROVIDER_METHODS)(
    "rejects %s with a typed error until a slice adds a handler",
    async (method) => {
      const provider = createDataProvider({
        ...stubs(),
        apiBaseUrl: "/api/admin",
        resources: {},
      })
      const call = (
        provider[method] as (
          resource: string,
          params: unknown
        ) => Promise<unknown>
      )("posts", {})
      await expect(call).rejects.toBeInstanceOf(DataProviderNotImplementedError)
      await expect(call).rejects.toMatchObject({
        method,
        resource: "posts",
        message: NOT_AVAILABLE_MESSAGE_KEY,
      })
    }
  )

  it("rejects with a message that ra-core's notification shows in Hungarian", async () => {
    const provider = createDataProvider({
      ...stubs(),
      apiBaseUrl: "/api/admin",
      resources: {},
    })
    const error = await provider
      .getList("posts", {})
      .catch((reason: unknown) => reason)
    const i18n = createI18nProvider({ reportMissingKeys: true })
    // ra-core calls translate(error.message, { _: error.message }).
    const text = i18n.translate((error as Error).message, {
      _: (error as Error).message,
    })
    expect(text).toBe("Ez a funkció még nem érhető el.")
  })

  it("dispatches to a registered handler with the context", async () => {
    const getOne = vi.fn().mockResolvedValue({ data: { id: 1 } })
    const resources: Record<string, ResourceHandlers> = { posts: { getOne } }
    const provider = createDataProvider({
      ...stubs(),
      apiBaseUrl: "/api/admin",
      resources,
    })

    await expect(provider.getOne("posts", { id: 1 })).resolves.toEqual({
      data: { id: 1 },
    })
    expect(getOne).toHaveBeenCalledWith(
      { id: 1 },
      { apiBaseUrl: "/api/admin", request: expect.any(Function) }
    )
  })

  it("still rejects the methods a registered resource does not implement", async () => {
    const provider = createDataProvider({
      ...stubs(),
      apiBaseUrl: "/api/admin",
      resources: { posts: { getOne: vi.fn() } },
    })
    await expect(provider.delete("posts", { id: 1 })).rejects.toMatchObject({
      method: "delete",
    })
    await expect(provider.getOne("labels", { id: 1 })).rejects.toMatchObject({
      resource: "labels",
    })
  })

  it("does not treat inherited object properties as resources", async () => {
    const provider = createDataProvider({
      ...stubs(),
      apiBaseUrl: "/api/admin",
      resources: {},
    })
    await expect(provider.getList("constructor", {})).rejects.toBeInstanceOf(
      DataProviderNotImplementedError
    )
  })

  it.each([
    "",
    "/",
    "api/admin",
    "/api/admin/",
    "//evil.example/api",
    "https://example.org/api",
    "/\\evil.example",
    "/\\\\evil.example/api",
    "/api\\admin",
    "/api//admin",
    "/api/../admin",
    "/api/./admin",
    "/api/..",
    "/api admin",
    "/api?x=1",
    "/api#x",
    "/%2F%2Fevil.example",
  ])("refuses the apiBaseUrl %j with an actionable message", (apiBaseUrl) => {
    expect(() =>
      createDataProvider({ ...stubs(), apiBaseUrl, resources: {} })
    ).toThrow(DataProviderConfigError)
    expect(() =>
      createDataProvider({ ...stubs(), apiBaseUrl, resources: {} })
    ).toThrow(/root-relative path such as "\/api\/admin"/)
  })

  it("refuses a handler that is not an ra-core method", () => {
    const resources = { posts: { getAll: vi.fn() } } as unknown as Record<
      string,
      ResourceHandlers
    >
    expect(() =>
      createDataProvider({ ...stubs(), apiBaseUrl: "/api/admin", resources })
    ).toThrow(/resources\.posts has a handler "getAll"/)
  })

  it("refuses an empty resource name", () => {
    expect(() =>
      createDataProvider({
        ...stubs(),
        apiBaseUrl: "/api/admin",
        resources: { " ": {} },
      })
    ).toThrow(/empty name/)
  })
})

describe("createDataProvider apiBaseUrl", () => {
  it.each(["/api", "/api/admin", "/api/v1.2", "/a-b_c~d/.well-known"])(
    "accepts %j, which a browser resolves on the admin's own origin",
    (apiBaseUrl) => {
      expect(() =>
        createDataProvider({ ...stubs(), apiBaseUrl, resources: {} })
      ).not.toThrow()
      const resolved = new URL(
        `${apiBaseUrl}/posts`,
        "https://backend3.example/admin/"
      )
      expect(resolved.origin).toBe("https://backend3.example")
      expect(resolved.pathname).toBe(`${apiBaseUrl}/posts`)
    }
  )

  it("refuses a backslash path that a URL parser would send to another host", () => {
    // WHATWG URL parsing treats "\" like "/", so "/\evil.example" is the
    // protocol-relative URL "//evil.example".
    expect(
      new URL("/\\evil.example/api", "https://backend3.example/admin/").origin
    ).toBe("https://evil.example")
    expect(() =>
      createDataProvider({
        ...stubs(),
        apiBaseUrl: "/\\evil.example",
        resources: {},
      })
    ).toThrow(DataProviderConfigError)
  })
})

describe("createDataProvider fetch and sessionExpiry", () => {
  it("refuses a fetch that is not a function", () => {
    expect(() =>
      createDataProvider({
        ...stubs(),
        apiBaseUrl: "/api/admin",
        resources: {},
        fetch: undefined as unknown as typeof globalThis.fetch,
      })
    ).toThrow(/^fetch must be a function; main\.tsx passes window\.fetch/)
  })

  it.each([undefined, null, {}, { waitForSignIn: true }])(
    "refuses the sessionExpiry %j, which is not a retry queue",
    (sessionExpiry) => {
      expect(() =>
        createDataProvider({
          ...stubs(),
          apiBaseUrl: "/api/admin",
          resources: {},
          sessionExpiry: sessionExpiry as unknown as SessionExpiry,
        })
      ).toThrow(
        /^sessionExpiry must be the retry queue of createSessionExpiry\(\)/
      )
    }
  )
})

/**
 * A provider with one `getOne` handler that hands its context out, so a
 * test can call `request` as a slice's handler does.
 *
 * @param answers - what the `fetch` stub answers, one per call, in order.
 * @returns the context, the stub and the retry queue.
 */
async function requestContext(...answers: Response[]) {
  const fetch = vi.fn<typeof globalThis.fetch>(async () => {
    const answer = answers.shift()
    if (answer === undefined) {
      throw new Error("the test gave no answer for this call")
    }
    return answer
  })
  const sessionExpiry = createSessionExpiry()
  let captured: DataProviderContext | undefined
  const provider = createDataProvider({
    apiBaseUrl: "/api/admin",
    resources: {
      labels: {
        async getOne(_params, context) {
          captured = context
          return { data: { id: 1 } }
        },
      },
    },
    fetch,
    sessionExpiry,
  })
  await provider.getOne("labels", { id: 1 })
  if (captured === undefined) {
    throw new Error("the handler did not run")
  }
  return { request: captured.request, fetch, sessionExpiry }
}

/**
 * An answer with only a status.
 *
 * @param status - the HTTP status.
 * @returns a `Response` with a short text body.
 */
function answer(status: number): Response {
  return new Response(status === 204 ? null : `status ${String(status)}`, {
    status,
  })
}

/**
 * A `401` with the admin API's error envelope.
 *
 * @param code - the error code; `unauthorized`, the answer for an ended
 *   session, by default.
 * @returns the answer.
 */
function refused401(code = "unauthorized"): Response {
  return new Response(
    JSON.stringify({ error: code, description: "d", field: null, params: {} }),
    { status: 401, headers: { "Content-Type": "application/json" } }
  )
}

describe("DataProviderContext.request", () => {
  it("sends below the base path with the session cookie and the handler's options", async () => {
    const { request, fetch } = await requestContext(answer(200))
    const response = await request("/labels/1?x=y", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: '{"name":"Hírek"}',
    })
    expect(response.status).toBe(200)
    expect(fetch).toHaveBeenCalledWith("/api/admin/labels/1?x=y", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: '{"name":"Hírek"}',
      credentials: "same-origin",
    })
  })

  it("always sends the cookie the same-origin way, whatever the handler asks", async () => {
    const { request, fetch } = await requestContext(answer(200))
    await request("/labels", {
      credentials: "include",
    } as unknown as ApiRequestInit)
    expect(fetch.mock.calls[0][1]?.credentials).toBe("same-origin")
  })

  it.each(["labels", "", "?x=1", 42, undefined])(
    "refuses the path %j, which does not start with /",
    async (path) => {
      const { request, fetch } = await requestContext()
      expect(() => request(path as string)).toThrow(
        /^request expects a path below apiBaseUrl that starts with "\/"/
      )
      expect(fetch).not.toHaveBeenCalled()
    }
  )

  it("refuses a stream body, which could not be sent again after a sign-in", async () => {
    const { request, fetch } = await requestContext()
    expect(() =>
      request("/media", {
        method: "POST",
        body: new ReadableStream() as unknown as string,
      })
    ).toThrow(/cannot send a stream body/)
    expect(fetch).not.toHaveBeenCalled()
  })

  it("hands a 401 back at once while no sign-in dialog is mounted", async () => {
    const { request, fetch, sessionExpiry } = await requestContext(refused401())
    expect((await request("/labels")).status).toBe(401)
    expect(fetch).toHaveBeenCalledTimes(1)
    expect(sessionExpiry.isWaiting()).toBe(false)
  })

  it("waits on a 401 for the in-place sign-in, then sends the same request again", async () => {
    const { request, fetch, sessionExpiry } = await requestContext(
      refused401(),
      answer(200)
    )
    sessionExpiry.attach()
    const body = new FormData()
    body.set("file", new Blob(["kép"]), "kep.png")
    const pending = request("/labels/1/image", { method: "POST", body })
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
    expect(fetch).toHaveBeenCalledTimes(1)

    sessionExpiry.resume()
    expect((await pending).status).toBe(200)
    expect(fetch).toHaveBeenCalledTimes(2)
    // The second request is the first one, unchanged.
    expect(fetch.mock.calls[1]).toEqual(fetch.mock.calls[0])
    expect(fetch.mock.calls[1][1]?.body).toBe(body)
  })

  it("hands a 401 with another code back at once, with a sign-in dialog mounted", async () => {
    // For example a wrong current password on an own-account change: the
    // session still holds, so no dialog may open over it.
    const { request, fetch, sessionExpiry } = await requestContext(
      refused401("invalid_credentials")
    )
    sessionExpiry.attach()
    const response = await request("/account", { method: "PUT", body: "{}" })
    expect(response.status).toBe(401)
    await expect(response.json()).resolves.toMatchObject({
      error: "invalid_credentials",
    })
    expect(fetch).toHaveBeenCalledTimes(1)
    expect(sessionExpiry.isWaiting()).toBe(false)
  })

  it("hands the 401 back when the user gives up", async () => {
    const { request, fetch, sessionExpiry } = await requestContext(refused401())
    sessionExpiry.attach()
    const pending = request("/labels", { method: "GET" })
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
    sessionExpiry.abandon()
    expect((await pending).status).toBe(401)
    expect(fetch).toHaveBeenCalledTimes(1)
  })

  it("stops waiting when the request's signal fires", async () => {
    const { request, sessionExpiry } = await requestContext(refused401())
    sessionExpiry.attach()
    const controller = new AbortController()
    const pending = request("/labels", { signal: controller.signal })
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
    controller.abort(new DOMException("left the page", "AbortError"))
    await expect(pending).rejects.toMatchObject({
      name: "AbortError",
      message: "left the page",
    })
    expect(sessionExpiry.isWaiting()).toBe(false)
  })
})
