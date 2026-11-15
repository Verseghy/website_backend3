import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { basePathProblem, isSameOriginBasePath } from "./base-path"

const ORIGIN = "https://backend3.example"

describe("isSameOriginBasePath", () => {
  it.each(["/api", "/api/admin", "/api/v1.2", "/a-b_c~d/.well-known"])(
    "accepts %j",
    (path) => {
      expect(isSameOriginBasePath(path)).toBe(true)
    }
  )

  it.each([
    "",
    "/",
    "api/admin",
    "/api/admin/",
    "//evil.example/api",
    "https://example.org/api",
    "/\\evil.example",
    "/api//admin",
    "/api/../admin",
    "/api/./admin",
    "/api/..",
    "/api admin",
    "/api?x=1",
    "/api#x",
    "/%2F%2Fevil.example",
  ])("refuses %j", (path) => {
    expect(isSameOriginBasePath(path)).toBe(false)
  })

  it.each([undefined, null, 42, ["/api"], { path: "/api" }])(
    "refuses the non-string %j",
    (value) => {
      expect(isSameOriginBasePath(value)).toBe(false)
    }
  )

  it("never accepts a path that a browser resolves to another origin or path", () => {
    // Pieces that matter to a URL parser, so that both outcomes are common.
    const piece = fc.constantFrom(
      "/",
      "\\",
      ".",
      "..",
      "api",
      "x",
      "-",
      "~",
      ":",
      "@",
      "?",
      "#",
      "%2F",
      " "
    )
    const candidate = fc
      .array(piece, { maxLength: 10 })
      .map((pieces) => pieces.join(""))
    fc.assert(
      fc.property(candidate, (path) => {
        if (!isSameOriginBasePath(path)) {
          return
        }
        const resolved = new URL(`${path}/auth/me`, `${ORIGIN}/admin/`)
        expect(resolved.origin).toBe(ORIGIN)
        expect(resolved.pathname).toBe(`${path}/auth/me`)
      })
    )
  })

  it("accepts every path built from unreserved segments", () => {
    const segment = fc
      .stringMatching(/^[A-Za-z0-9._~-]{1,8}$/)
      .filter((part) => part !== "." && part !== "..")
    fc.assert(
      fc.property(
        fc.array(segment, { minLength: 1, maxLength: 4 }),
        (parts) => {
          expect(isSameOriginBasePath(`/${parts.join("/")}`)).toBe(true)
        }
      )
    )
  })
})

describe("basePathProblem", () => {
  it("is undefined for a good path", () => {
    expect(basePathProblem("apiBaseUrl", "/api/admin")).toBeUndefined()
  })

  it("names the option, the expected shape and the value", () => {
    const problem = basePathProblem("apiBaseUrl", "https://example.org/api")
    expect(problem).toMatch(/^apiBaseUrl must be a root-relative path/)
    expect(problem).toContain('such as "/api/admin"')
    expect(problem).toContain('but it is "https://example.org/api".')
  })

  it("shows a non-string value as JSON", () => {
    expect(basePathProblem("apiBaseUrl", 42)).toContain("but it is 42.")
  })
})
