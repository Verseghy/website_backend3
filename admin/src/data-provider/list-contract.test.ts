// @vitest-environment node
import { describe, expect, it } from "vitest"

import { createI18nProvider } from "@/i18n/provider"

import {
  DEFAULT_PER_PAGE,
  INVALID_FILTER_KEY,
  ListFilterError,
  MAX_FILTER_IDS,
  PAGE_SIZES,
  isRecordId,
  listSearchParams,
  recordId,
} from "./list-contract"

describe("the list contract's constants", () => {
  it("offer the page sizes of SHR-02, 25 by default, and no 'all'", () => {
    // The Rust test tests/domain/list_contract.rs holds these lines to
    // domain::list_query; this test pins what the screens rely on.
    expect(PAGE_SIZES).toEqual([10, 25, 50, 100])
    expect(PAGE_SIZES).toContain(DEFAULT_PER_PAGE)
    expect(DEFAULT_PER_PAGE).toBe(25)
    // getMany asks for its ids on one page of the largest size.
    expect(MAX_FILTER_IDS).toBe(Math.max(...PAGE_SIZES))
  })
})

describe("listSearchParams", () => {
  it("writes ra-core's paging and sort as the server reads them", () => {
    expect(
      listSearchParams({
        pagination: { page: 2, perPage: 25 },
        sort: { field: "name", order: "DESC" },
        filter: {},
      }).toString()
    ).toBe("page=2&perPage=25&sort=name&order=DESC")
  })

  it("leaves out what ra-core did not give, so the server's defaults apply", () => {
    expect(listSearchParams({}).toString()).toBe("")
    expect(
      listSearchParams({ sort: { field: "", order: "ASC" } }).toString()
    ).toBe("")
    expect(
      listSearchParams({ pagination: { page: 1, perPage: 10 } }).toString()
    ).toBe("page=1&perPage=10")
  })

  it("repeats a filter once per value of an array, in order", () => {
    expect(listSearchParams({ filter: { id: [4, 2, 4] } }).toString()).toBe(
      "id=4&id=2&id=4"
    )
  })

  it("writes one value for a string, a number and a boolean, and skips a cleared filter", () => {
    const search = listSearchParams({
      filter: {
        q: "ünnep",
        page_id: 7,
        live: false,
        gone: undefined,
        none: null,
      },
    })
    expect([...search.entries()]).toEqual([
      ["q", "ünnep"],
      ["page_id", "7"],
      ["live", "false"],
    ])
  })

  it("encodes text the way the server's form decoder reads it back", () => {
    const query = listSearchParams({
      filter: { q: "Ő & ű=1 %_" },
    }).toString()
    expect(query).toBe("q=%C5%90+%26+%C5%B1%3D1+%25_")
    expect(new URLSearchParams(query).get("q")).toBe("Ő & ű=1 %_")
  })

  it("passes on values that only the server may refuse", () => {
    // A hand-edited URL can carry these; the server answers each with its
    // own 422 and the parameter's name (ADR 0021 §4).
    expect(
      listSearchParams({
        pagination: { page: Number.NaN, perPage: 1000 },
        sort: { field: "created_at", order: "ASC" },
        filter: { perPage: 5 },
      }).toString()
    ).toBe("page=NaN&perPage=1000&sort=created_at&order=ASC&perPage=5")
  })

  it.each([
    ["a nested object", { author: { id: 1 } }, "author"],
    ["an array inside an array", { id: [[1, 2]] }, "id"],
    ["a non-finite number", { page_id: Number.POSITIVE_INFINITY }, "page_id"],
    ["an object in an array", { id: [1, { id: 2 }] }, "id"],
  ])(
    "refuses %s, which no query parameter can carry, with the Hungarian filter message",
    (_what, filter, field) => {
      let caught: unknown
      try {
        listSearchParams({ filter })
      } catch (error) {
        caught = error
      }
      expect(caught).toBeInstanceOf(ListFilterError)
      expect(caught).toMatchObject({ field, message: INVALID_FILTER_KEY })
      const i18n = createI18nProvider({ reportMissingKeys: true })
      expect(i18n.translate((caught as Error).message)).toBe(
        "A szűrő értéke érvénytelen. Töröld a szűrőt, és próbáld újra!"
      )
    }
  )
})

describe("isRecordId", () => {
  it.each([1, 7, Number.MAX_SAFE_INTEGER])("accepts %d", (value) => {
    expect(isRecordId(value)).toBe(true)
  })

  it.each([
    0,
    -1,
    1.5,
    Number.NaN,
    Number.MAX_SAFE_INTEGER + 1,
    "1",
    null,
    undefined,
    BigInt(1),
  ])("refuses %s", (value) => {
    expect(isRecordId(value)).toBe(false)
  })
})

describe("recordId", () => {
  it.each([
    [7, 7],
    ["7", 7],
    ["007", 7],
    [String(Number.MAX_SAFE_INTEGER), Number.MAX_SAFE_INTEGER],
  ])("reads %j as %d", (id, expected) => {
    expect(recordId(id)).toBe(expected)
  })

  it.each([
    0,
    -3,
    2.5,
    Number.MAX_SAFE_INTEGER + 1,
    "",
    "0",
    "-3",
    "1e3",
    " 7",
    "7 ",
    "+7",
    "0x10",
    "../x",
    "..",
    "7/edit",
    "%37",
    "9007199254740992",
    "٣",
  ])("refuses %j, which names no record", (id) => {
    expect(recordId(id)).toBeUndefined()
  })
})
