// @vitest-environment node
import { describe, expect, it, vi } from "vitest"

import { createSessionExpiry } from "@/auth/session-expiry"
import { createDataProvider } from "@/data-provider/data-provider"

import {
  LABELS_PATH,
  LABELS_RESOURCE,
  createLabelHandlers,
  isLabelColour,
  parseLabel,
} from "./label-handlers"

describe("parseLabel", () => {
  it("reads a label with exactly the binding's three fields", () => {
    expect(
      parseLabel({
        id: 7,
        name: "Ünnepség",
        color: "#1E88E5",
        created_at: "2026-11-15T08:00:00Z",
      })
    ).toEqual({ id: 7, name: "Ünnepség", color: "#1E88E5" })
    expect(parseLabel({ id: 1, name: "", color: "#abcdef" })).toEqual({
      id: 1,
      name: "",
      color: "#abcdef",
    })
  })

  it.each([
    ["no object", "címke"],
    ["null", null],
    ["an array", [7, "Ünnepség", "#1E88E5"]],
    ["an id as text", { id: "7", name: "a", color: "#1E88E5" }],
    ["id 0", { id: 0, name: "a", color: "#1E88E5" }],
    ["an id above 2^53 − 1", { id: 2 ** 53, name: "a", color: "#1E88E5" }],
    ["no name", { id: 7, color: "#1E88E5" }],
    ["a name that is not text", { id: 7, name: 7, color: "#1E88E5" }],
    ["no colour", { id: 7, name: "a" }],
    ["a colour without #", { id: 7, name: "a", color: "1E88E5" }],
    ["a short colour", { id: 7, name: "a", color: "#1E8" }],
    ["a colour name", { id: 7, name: "a", color: "red" }],
    [
      "a colour that would break out of a style",
      { id: 7, name: "a", color: "#1E88E5; background: url(x)" },
    ],
  ])("refuses %s", (_what, value) => {
    expect(parseLabel(value)).toBeUndefined()
  })
})

describe("isLabelColour", () => {
  it.each(["#000000", "#FFFFFF", "#a1B2c3"])("accepts %s", (colour) => {
    expect(isLabelColour(colour)).toBe(true)
  })

  it.each(["", "#", "#12345", "#1234567", "#12345g", " #123456", "#123456 "])(
    "refuses %j",
    (colour) => {
      expect(isLabelColour(colour)).toBe(false)
    }
  )
})

describe("the labels' handlers", () => {
  it("read /api/admin/labels for the labels resource", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(
      async () =>
        new Response(
          JSON.stringify({
            data: [{ id: 2, name: "ábra", color: "#43A047" }],
            total: 1,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        )
    )
    const provider = createDataProvider({
      apiBaseUrl: "/api/admin",
      resources: { [LABELS_RESOURCE]: createLabelHandlers() },
      fetch,
      sessionExpiry: createSessionExpiry(),
    })
    await expect(
      provider.getList(LABELS_RESOURCE, {
        pagination: { page: 1, perPage: 25 },
        sort: { field: "name", order: "ASC" },
        filter: {},
      })
    ).resolves.toEqual({
      data: [{ id: 2, name: "ábra", color: "#43A047" }],
      total: 1,
    })
    expect(String(fetch.mock.calls[0]?.[0])).toBe(
      `/api/admin${LABELS_PATH}?page=1&perPage=25&sort=name&order=ASC`
    )
  })
})
