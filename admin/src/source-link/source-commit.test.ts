import { describe, expect, it } from "vitest"

import {
  InvalidSourceCommitError,
  parseSourceCommit,
  sourceTreeUrl,
} from "./source-commit"

const SHA = "0123456789abcdef0123456789abcdef01234567"

describe("parseSourceCommit", () => {
  it("accepts a full lowercase commit SHA", () => {
    expect(parseSourceCommit(SHA)).toBe(SHA)
  })

  it("trims surrounding whitespace from the build argument", () => {
    expect(parseSourceCommit(` ${SHA}\n`)).toBe(SHA)
  })

  it.each([undefined, "", "   "])(
    "treats %j as a local build without a commit",
    (value) => {
      expect(parseSourceCommit(value)).toBeNull()
    }
  )

  it.each([
    ["an abbreviated SHA", "0123456"],
    ["an uppercase SHA", SHA.toUpperCase()],
    ["a branch name", "master"],
    ["a 41-character value", `${SHA}0`],
    ["a URL fragment", `${SHA.slice(0, 39)}#`],
  ])("rejects %s with an actionable error", (_label, value) => {
    expect(() => parseSourceCommit(value)).toThrow(InvalidSourceCommitError)
    expect(() => parseSourceCommit(value)).toThrow(
      /VITE_SOURCE_COMMIT must be empty or a full 40-character/
    )
  })
})

describe("sourceTreeUrl", () => {
  it("links the running commit (ADR 0012 §4)", () => {
    expect(sourceTreeUrl(SHA)).toBe(
      `https://github.com/Verseghy/website_backend3/tree/${SHA}`
    )
  })

  it("links master when the build carries no commit", () => {
    expect(sourceTreeUrl("")).toBe(
      "https://github.com/Verseghy/website_backend3/tree/master"
    )
    expect(sourceTreeUrl(undefined)).toBe(
      "https://github.com/Verseghy/website_backend3/tree/master"
    )
  })
})
