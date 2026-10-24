import { describe, expect, it, vi } from "vitest"

import { createI18nProvider } from "@/i18n/provider"

import {
  DATA_PROVIDER_METHODS,
  DataProviderConfigError,
  DataProviderNotImplementedError,
  NOT_AVAILABLE_MESSAGE_KEY,
  createDataProvider,
  type ResourceHandlers,
} from "./data-provider"

describe("createDataProvider", () => {
  it("implements every ra-core data provider method", () => {
    const provider = createDataProvider({
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
    const provider = createDataProvider({ apiBaseUrl: "/api/admin", resources })

    await expect(provider.getOne("posts", { id: 1 })).resolves.toEqual({
      data: { id: 1 },
    })
    expect(getOne).toHaveBeenCalledWith({ id: 1 }, { apiBaseUrl: "/api/admin" })
  })

  it("still rejects the methods a registered resource does not implement", async () => {
    const provider = createDataProvider({
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
    expect(() => createDataProvider({ apiBaseUrl, resources: {} })).toThrow(
      DataProviderConfigError
    )
    expect(() => createDataProvider({ apiBaseUrl, resources: {} })).toThrow(
      /root-relative path such as "\/api\/admin"/
    )
  })

  it("refuses a handler that is not an ra-core method", () => {
    const resources = { posts: { getAll: vi.fn() } } as unknown as Record<
      string,
      ResourceHandlers
    >
    expect(() =>
      createDataProvider({ apiBaseUrl: "/api/admin", resources })
    ).toThrow(/resources\.posts has a handler "getAll"/)
  })

  it("refuses an empty resource name", () => {
    expect(() =>
      createDataProvider({ apiBaseUrl: "/api/admin", resources: { " ": {} } })
    ).toThrow(/empty name/)
  })
})

describe("createDataProvider apiBaseUrl", () => {
  it.each(["/api", "/api/admin", "/api/v1.2", "/a-b_c~d/.well-known"])(
    "accepts %j, which a browser resolves on the admin's own origin",
    (apiBaseUrl) => {
      expect(() =>
        createDataProvider({ apiBaseUrl, resources: {} })
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
      createDataProvider({ apiBaseUrl: "/\\evil.example", resources: {} })
    ).toThrow(DataProviderConfigError)
  })
})
