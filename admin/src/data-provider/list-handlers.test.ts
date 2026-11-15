// @vitest-environment node
/**
 * The generic read handlers over the shared admin list contract
 * (`./list-handlers.ts`), driven through backend3's data provider against a
 * `fetch` stub that answers as `GET /api/admin/<resource>` and
 * `GET /api/admin/<resource>/{id}` do. The handlers send through the
 * provider's `request`, so the tests also hold them to its rules
 * (ADR 0006, "Follow-up work"): a `401 unauthorized` waits for the in-place
 * sign-in and is sent again after it, and ra-core's `params.signal` ends a
 * request and its wait.
 */
import { HttpError } from "ra-core"
import { describe, expect, it, vi, type Mock } from "vitest"

import { createSessionExpiry, type SessionExpiry } from "@/auth/session-expiry"
import { createI18nProvider } from "@/i18n/provider"

import { DataProviderConfigError, createDataProvider } from "./data-provider"
import {
  UnreadableAnswerError,
  createListHandlers,
  parseListPage,
  type ListRow,
} from "./list-handlers"

/** The probe resource's row: an id and a title. */
interface ProbeRow extends ListRow {
  readonly title: string
}

/**
 * Reads a probe row: a positive integer id and a string title.
 *
 * @param value - one row of an answer.
 * @returns the row, or `undefined`.
 */
function parseProbe(value: unknown): ProbeRow | undefined {
  if (typeof value !== "object" || value === null) {
    return undefined
  }
  const { id, title } = value as Record<string, unknown>
  return typeof id === "number" &&
    Number.isSafeInteger(id) &&
    id > 0 &&
    typeof title === "string"
    ? { id, title }
    : undefined
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
 * An admin error envelope (ADR 0021 §4).
 *
 * @param error - the code.
 * @param field - the field it names, or `null`.
 * @returns the envelope.
 */
function envelope(error: string, field: string | null = null) {
  return { error, description: "English text", field, params: {} }
}

/**
 * The data provider with the probe resource's list handlers, over `fetch`
 * and a retry queue of the in-place sign-in.
 *
 * @param fetch - the stub that the provider's `request` calls.
 * @param sessionExpiry - the retry queue; by default one with no dialog
 *   attached, where a `401` comes back at once.
 * @returns the provider.
 */
function provider(
  fetch: typeof globalThis.fetch,
  sessionExpiry: SessionExpiry = createSessionExpiry()
) {
  return createDataProvider({
    apiBaseUrl: "/api/admin",
    resources: {
      probes: createListHandlers({ path: "/probes", parseRow: parseProbe }),
    },
    fetch,
    sessionExpiry,
  })
}

/**
 * The rejection of `call`.
 *
 * @param call - a promise that must reject.
 * @returns what it rejected with.
 */
async function rejection(call: Promise<unknown>): Promise<unknown> {
  return call.then(
    () => {
      throw new Error("expected a rejection")
    },
    (reason: unknown) => reason
  )
}

/**
 * The URLs a stub was called with, in order.
 *
 * @param fetch - the stub.
 * @returns the URLs.
 */
function urls(fetch: Mock<typeof globalThis.fetch>): string[] {
  return fetch.mock.calls.map(([input]) => String(input))
}

describe("createListHandlers", () => {
  it.each(["", "probes", "/probes/", "//evil.example", "/a/../b"])(
    "refuses the path %j with an actionable message",
    (path) => {
      expect(() => createListHandlers({ path, parseRow: parseProbe })).toThrow(
        DataProviderConfigError
      )
      expect(() => createListHandlers({ path, parseRow: parseProbe })).toThrow(
        /path must be a root-relative path/
      )
    }
  )

  it("refuses a missing row reader", () => {
    expect(() =>
      createListHandlers({
        path: "/probes",
        parseRow: undefined as unknown as typeof parseProbe,
      })
    ).toThrow(/parseRow of the handlers for \/probes must be a function/)
  })

  it("registers exactly the three reads", () => {
    expect(
      Object.keys(
        createListHandlers({ path: "/probes", parseRow: parseProbe })
      ).sort()
    ).toEqual(["getList", "getMany", "getOne"])
  })
})

describe("getList", () => {
  it("asks for one page with the session cookie and returns its rows and the total", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(200, {
        data: [
          { id: 3, title: "Harmadik" },
          { id: 1, title: "Első" },
        ],
        total: 27,
      })
    )
    await expect(
      provider(fetch).getList("probes", {
        pagination: { page: 2, perPage: 25 },
        sort: { field: "title", order: "ASC" },
        filter: {},
      })
    ).resolves.toEqual({
      data: [
        { id: 3, title: "Harmadik" },
        { id: 1, title: "Első" },
      ],
      total: 27,
    })
    expect(fetch).toHaveBeenCalledTimes(1)
    expect(fetch).toHaveBeenCalledWith(
      "/api/admin/probes?page=2&perPage=25&sort=title&order=ASC",
      {
        method: "GET",
        credentials: "same-origin",
        headers: { Accept: "application/json" },
      }
    )
  })

  it("keeps only the fields the binding declares", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(200, {
        data: [{ id: 1, title: "Első", password_hash: "x" }],
        total: 1,
      })
    )
    const { data } = await provider(fetch).getList("probes", {})
    expect(data).toEqual([{ id: 1, title: "Első" }])
    expect(urls(fetch)).toEqual(["/api/admin/probes"])
  })

  it("returns a page past the end as no rows with the total, so ra-core can go back", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(200, { data: [], total: 3 })
    )
    await expect(
      provider(fetch).getList("probes", {
        pagination: { page: 9, perPage: 10 },
      })
    ).resolves.toEqual({ data: [], total: 3 })
  })

  it.each([
    [403, "forbidden", null, "errors.forbidden"],
    [401, "unauthorized", null, "errors.unauthorized"],
    [422, "invalid_per_page", "perPage", "errors.invalid_per_page"],
    [422, "invalid_sort", "sort", "errors.invalid_sort"],
  ])(
    "rejects a %d %s with the envelope's Hungarian message and the status checkError reads",
    async (status, code, field, key) => {
      const fetch = vi.fn<typeof globalThis.fetch>(async () =>
        json(status, envelope(code, field))
      )
      const error = await rejection(provider(fetch).getList("probes", {}))
      expect(error).toBeInstanceOf(HttpError)
      expect(error).toMatchObject({
        status,
        message: key,
        body: { error: code, field },
      })
    }
  )

  it("shows a code this build does not know with the server's text", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(422, {
        ...envelope("search.term_too_long", "q"),
        description: "Too long",
      })
    )
    const error = await rejection(provider(fetch).getList("probes", {}))
    expect(error).toMatchObject({ status: 422, message: "Too long" })
  })

  it("rejects a refusal without an envelope with the generic Hungarian message", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(
      async () => new Response("<html>Bad gateway</html>", { status: 502 })
    )
    const error = await rejection(provider(fetch).getList("probes", {}))
    expect(error).toMatchObject({
      status: 502,
      message: "ra.notification.http_error",
    })
  })

  it("rejects a network failure with the generic Hungarian message and no status", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () => {
      throw new TypeError("Failed to fetch")
    })
    const error = await rejection(provider(fetch).getList("probes", {}))
    expect(error).toBeInstanceOf(Error)
    expect(error).not.toBeInstanceOf(HttpError)
    expect((error as Error).message).toBe("ra.notification.http_error")
    expect((error as { status?: unknown }).status).toBeUndefined()
    const i18n = createI18nProvider({ reportMissingKeys: true })
    expect(i18n.translate((error as Error).message)).toBe(
      "Hiba a szerverrel való kommunikációban"
    )
  })

  it.each([
    ["a body that is not JSON", "<html></html>"],
    ["an array", "[]"],
    ["no data", JSON.stringify({ total: 1 })],
    ["data that is not an array", JSON.stringify({ data: {}, total: 1 })],
    ["no total", JSON.stringify({ data: [] })],
    ["a negative total", JSON.stringify({ data: [], total: -1 })],
    ["a fractional total", JSON.stringify({ data: [], total: 1.5 })],
    ["a total as text", JSON.stringify({ data: [], total: "1" })],
    [
      "fewer in total than on the page",
      JSON.stringify({ data: [{ id: 1, title: "a" }], total: 0 }),
    ],
    [
      "a row the binding does not describe",
      JSON.stringify({ data: [{ id: "1", title: "a" }], total: 1 }),
    ],
  ])("rejects %s as an unreadable answer", async (_what, body) => {
    const fetch = vi.fn<typeof globalThis.fetch>(
      async () =>
        new Response(body, {
          status: 200,
          headers: { "Content-Type": "application/json" },
        })
    )
    const error = await rejection(provider(fetch).getList("probes", {}))
    expect(error).toBeInstanceOf(UnreadableAnswerError)
    expect(error).toMatchObject({
      message: "ra.notification.http_error",
      url: "/api/admin/probes",
      status: 200,
    })
  })

  it("sends nothing for a filter value no query parameter can carry", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>()
    const error = await rejection(
      provider(fetch).getList("probes", { filter: { id: { nested: 1 } } })
    )
    expect(error).toMatchObject({
      message: "errors.invalid_filter",
      field: "id",
    })
    expect(fetch).not.toHaveBeenCalled()
  })
})

describe("getOne", () => {
  it("asks for the record by its id and returns it", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(200, { id: 12, title: "Tizenkettedik" })
    )
    await expect(provider(fetch).getOne("probes", { id: 12 })).resolves.toEqual(
      { data: { id: 12, title: "Tizenkettedik" } }
    )
    // ra-core passes the id of a URL as text.
    await expect(
      provider(fetch).getOne("probes", { id: "12" })
    ).resolves.toEqual({ data: { id: 12, title: "Tizenkettedik" } })
    expect(urls(fetch)).toEqual([
      "/api/admin/probes/12",
      "/api/admin/probes/12",
    ])
  })

  it.each(["../x", "..", "0", "-1", "1e3", "12/edit", ""])(
    "answers the id %j, which names no record, with not_found and sends nothing",
    async (id) => {
      const fetch = vi.fn<typeof globalThis.fetch>()
      const error = await rejection(provider(fetch).getOne("probes", { id }))
      expect(error).toBeInstanceOf(HttpError)
      expect(error).toMatchObject({ status: 404, message: "errors.not_found" })
      expect(fetch).not.toHaveBeenCalled()
    }
  )

  it("rejects the server's 404 with its Hungarian message", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(404, envelope("not_found"))
    )
    const error = await rejection(provider(fetch).getOne("probes", { id: 5 }))
    expect(error).toMatchObject({ status: 404, message: "errors.not_found" })
  })

  it("refuses an answer that is another record or no record", async () => {
    for (const body of [{ id: 6, title: "Hatodik" }, { id: 5 }, null]) {
      const fetch = vi.fn<typeof globalThis.fetch>(async () => json(200, body))
      const error = await rejection(provider(fetch).getOne("probes", { id: 5 }))
      expect(error, JSON.stringify(body)).toBeInstanceOf(UnreadableAnswerError)
    }
  })
})

describe("getMany", () => {
  it("reads the records through the list's id filter, each id once, on one page of 100", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(200, {
        data: [
          { id: 3, title: "Harmadik" },
          { id: 1, title: "Első" },
        ],
        total: 2,
      })
    )
    await expect(
      provider(fetch).getMany("probes", { ids: [3, "1", 3] })
    ).resolves.toEqual({
      data: [
        { id: 3, title: "Harmadik" },
        { id: 1, title: "Első" },
      ],
    })
    expect(urls(fetch)).toEqual(["/api/admin/probes?perPage=100&id=3&id=1"])
  })

  it("asks nothing for no ids, or for ids that name no record", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>()
    await expect(
      provider(fetch).getMany("probes", { ids: [] })
    ).resolves.toEqual({ data: [] })
    await expect(
      provider(fetch).getMany("probes", { ids: [0, "x", "../1"] })
    ).resolves.toEqual({ data: [] })
    expect(fetch).not.toHaveBeenCalled()
  })

  it("splits more than 100 ids into requests of at most 100, and joins the answers", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async (input) => {
      const ids = new URL(
        String(input),
        "https://backend3.example"
      ).searchParams
        .getAll("id")
        .map(Number)
      return json(200, {
        data: ids.map((id) => ({ id, title: `#${id}` })),
        total: ids.length,
      })
    })
    const ids = Array.from({ length: 150 }, (_, index) => index + 1)
    const { data } = await provider(fetch).getMany("probes", { ids })
    expect(data.map((row) => row.id)).toEqual(ids)
    const asked = urls(fetch).map((url) =>
      new URL(url, "https://backend3.example").searchParams.getAll("id")
    )
    expect(asked.map((group) => group.length)).toEqual([100, 50])
    for (const url of urls(fetch)) {
      expect(
        new URL(url, "https://backend3.example").searchParams.get("perPage")
      ).toBe("100")
    }
  })

  it("rejects when the server refuses one of the requests", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(403, envelope("forbidden"))
    )
    const error = await rejection(
      provider(fetch).getMany("probes", { ids: [1] })
    )
    expect(error).toMatchObject({ status: 403, message: "errors.forbidden" })
  })
})

describe("the in-place sign-in and ra-core's signal (ADR 0006, D158)", () => {
  /** The admin API's answer for a request without a valid session. */
  const sessionEnded = () => json(401, envelope("unauthorized"))

  /** One page of one probe row. */
  const onePage = () =>
    json(200, { data: [{ id: 1, title: "Első" }], total: 1 })

  it("holds a 401 unauthorized for the sign-in and sends the same request again after it", async () => {
    const sessionExpiry = createSessionExpiry()
    sessionExpiry.attach()
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(sessionEnded())
      .mockResolvedValueOnce(onePage())
    const answer = provider(fetch, sessionExpiry).getList("probes", {
      pagination: { page: 1, perPage: 25 },
    })
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))
    expect(fetch).toHaveBeenCalledTimes(1)

    sessionExpiry.resume()

    await expect(answer).resolves.toEqual({
      data: [{ id: 1, title: "Első" }],
      total: 1,
    })
    expect(urls(fetch)).toEqual([
      "/api/admin/probes?page=1&perPage=25",
      "/api/admin/probes?page=1&perPage=25",
    ])
  })

  it("holds the record and the id filter reads the same way", async () => {
    const sessionExpiry = createSessionExpiry()
    sessionExpiry.attach()
    // Each URL's first request meets the ended session; the requests
    // sent after the sign-in get their answers.
    const asked = new Set<string>()
    const fetch = vi.fn<typeof globalThis.fetch>(async (input) => {
      const url = String(input)
      if (!asked.has(url)) {
        asked.add(url)
        return sessionEnded()
      }
      return url.endsWith("/probes/1")
        ? json(200, { id: 1, title: "Első" })
        : onePage()
    })
    const dataProvider = provider(fetch, sessionExpiry)
    const record = dataProvider.getOne("probes", { id: 1 })
    const many = dataProvider.getMany("probes", { ids: [1] })
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(2))
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))

    // One sign-in releases both.
    sessionExpiry.resume()

    await expect(record).resolves.toEqual({ data: { id: 1, title: "Első" } })
    await expect(many).resolves.toEqual({ data: [{ id: 1, title: "Első" }] })
    expect(fetch).toHaveBeenCalledTimes(4)
  })

  it("rejects the 401 with the Hungarian message and its status when the user gives up, so checkError signs out", async () => {
    const sessionExpiry = createSessionExpiry()
    sessionExpiry.attach()
    const fetch = vi.fn<typeof globalThis.fetch>(async () => sessionEnded())
    const answer = rejection(
      provider(fetch, sessionExpiry).getList("probes", {})
    )
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))

    sessionExpiry.abandon()

    const error = await answer
    expect(error).toBeInstanceOf(HttpError)
    expect(error).toMatchObject({ status: 401, message: "errors.unauthorized" })
    expect(fetch).toHaveBeenCalledTimes(1)
  })

  it("hands a 401 with another code back at once, without a wait", async () => {
    const sessionExpiry = createSessionExpiry()
    sessionExpiry.attach()
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      json(401, envelope("invalid_credentials"))
    )
    const error = await rejection(
      provider(fetch, sessionExpiry).getList("probes", {})
    )
    expect(error).toMatchObject({
      status: 401,
      message: "errors.invalid_credentials",
    })
    expect(sessionExpiry.isWaiting()).toBe(false)
    expect(sessionExpiry.round()).toBe(0)
  })

  it("passes ra-core's signal of getList, getOne and getMany on to fetch", async () => {
    const signal = new AbortController().signal
    const fetch = vi.fn<typeof globalThis.fetch>(async (input) =>
      String(input).endsWith("/probes/1")
        ? json(200, { id: 1, title: "Első" })
        : onePage()
    )
    const dataProvider = provider(fetch)
    await dataProvider.getList("probes", { signal })
    await dataProvider.getOne("probes", { id: 1, signal })
    await dataProvider.getMany("probes", { ids: [1], signal })
    expect(fetch).toHaveBeenCalledTimes(3)
    for (const [, init] of fetch.mock.calls) {
      expect(init?.signal).toBe(signal)
    }
  })

  it("rejects with the signal's reason, not a server problem, when the screen leaves during the wait", async () => {
    const sessionExpiry = createSessionExpiry()
    sessionExpiry.attach()
    const fetch = vi.fn<typeof globalThis.fetch>(async () => sessionEnded())
    const controller = new AbortController()
    const answer = rejection(
      provider(fetch, sessionExpiry).getList("probes", {
        signal: controller.signal,
      })
    )
    await vi.waitFor(() => expect(sessionExpiry.isWaiting()).toBe(true))

    controller.abort()

    const error = await answer
    expect(error).toBe(controller.signal.reason)
    expect(error).toMatchObject({ name: "AbortError" })
    expect(sessionExpiry.isWaiting()).toBe(false)
    // Leaving is neither a sign-in nor a give-up, and nothing was resent.
    expect(sessionExpiry.round()).toBe(0)
    expect(fetch).toHaveBeenCalledTimes(1)
  })

  it("rejects a request aborted on its way unchanged", async () => {
    const controller = new AbortController()
    const fetch = vi.fn<typeof globalThis.fetch>(async (_input, init) => {
      controller.abort()
      throw init?.signal?.reason
    })
    const error = await rejection(
      provider(fetch).getOne("probes", { id: 1, signal: controller.signal })
    )
    expect(error).toBe(controller.signal.reason)
    expect(error).not.toMatchObject({ message: "ra.notification.http_error" })
  })

  it("rejects with the signal's reason, not an unreadable answer, when the screen leaves while the body is read", async () => {
    const controller = new AbortController()
    // A 200 whose body ends with the abort, as the browser ends a body
    // whose request's signal fires.
    const fetch = vi.fn<typeof globalThis.fetch>(
      async () =>
        new Response(
          new ReadableStream<Uint8Array>({
            pull(stream) {
              controller.abort()
              stream.error(controller.signal.reason)
            },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        )
    )
    const error = await rejection(
      provider(fetch).getList("probes", { signal: controller.signal })
    )
    expect(error).toBe(controller.signal.reason)
    expect(error).not.toBeInstanceOf(UnreadableAnswerError)
  })
})

describe("parseListPage", () => {
  it("reads a page whose every row the reader accepts", () => {
    expect(
      parseListPage({ data: [{ id: 1, title: "a" }], total: 4 }, parseProbe)
    ).toEqual({ data: [{ id: 1, title: "a" }], total: 4 })
    expect(parseListPage({ data: [], total: 0 }, parseProbe)).toEqual({
      data: [],
      total: 0,
    })
  })

  it("refuses the page when one row is unreadable", () => {
    expect(
      parseListPage(
        { data: [{ id: 1, title: "a" }, { id: 2 }], total: 2 },
        parseProbe
      )
    ).toBeUndefined()
  })
})
