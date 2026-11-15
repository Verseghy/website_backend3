/**
 * The run-time role and permission readers of `/auth/me` (ADR 0007 §1, §2,
 * §7): they accept exactly the generated unions' values, each once.
 */
import fc from "fast-check"
import { describe, expect, it } from "vitest"

import {
  KNOWN_PERMISSIONS,
  KNOWN_ROLES,
  readPermissions,
  readRoles,
} from "./authz"

/** The sixteen permission identifiers of ADR 0007 §1, in its order. */
const ADR_PERMISSIONS = [
  "edit posts",
  "publish posts",
  "edit labels",
  "edit authors",
  "edit events",
  "edit canteens",
  "edit colleagues",
  "edit users",
  "edit pages",
  "edit menu",
  "manage media",
  "view logs",
  "edit notices",
  "edit banners",
  "edit site settings",
  "edit diaktoll",
]

describe("the known values", () => {
  it("are the four roles in id order", () => {
    expect(Object.keys(KNOWN_ROLES)).toEqual([
      "writer",
      "secretary",
      "supervisor",
      "admin",
    ])
  })

  it("are the sixteen permissions of ADR 0007 in its order", () => {
    expect(Object.keys(KNOWN_PERMISSIONS)).toEqual(ADR_PERMISSIONS)
    // Never created (ADR 0007 follow-up points 1 and 2).
    expect(Object.keys(KNOWN_PERMISSIONS)).not.toContain("edit newsletter")
    expect(Object.keys(KNOWN_PERMISSIONS)).not.toContain("manage backups")
  })
})

describe("readRoles", () => {
  it("reads a list of known roles, each once, in its order", () => {
    expect(readRoles([])).toEqual([])
    expect(readRoles(["admin", "writer"])).toEqual(["admin", "writer"])
    expect(readRoles(Object.keys(KNOWN_ROLES))).toEqual(
      Object.keys(KNOWN_ROLES)
    )
  })

  it.each([
    ["not a list", "admin"],
    ["null", null],
    ["an object", { admin: true }],
    ["an unknown role", ["editor"]],
    ["another spelling", ["Admin"]],
    ["a role twice", ["admin", "admin"]],
    ["a number", [4]],
    ["a name from the prototype", ["toString"]],
  ])("refuses %s", (_case, value) => {
    expect(readRoles(value)).toBeUndefined()
  })
})

describe("readPermissions", () => {
  it("reads a list of known permissions, each once, in its order", () => {
    expect(readPermissions(ADR_PERMISSIONS)).toEqual(ADR_PERMISSIONS)
    expect(readPermissions(["view logs", "edit posts"])).toEqual([
      "view logs",
      "edit posts",
    ])
  })

  it.each([
    ["not a list", "edit posts"],
    ["a retired name", ["edit newsletter"]],
    ["another spelling", ["edit_posts"]],
    ["trailing space", ["edit posts "]],
    ["a permission twice", ["edit posts", "edit posts"]],
    ["a name from the prototype", ["hasOwnProperty"]],
  ])("refuses %s", (_case, value) => {
    expect(readPermissions(value)).toBeUndefined()
  })

  it("accepts a list exactly when every entry is a known name, once", () => {
    const name = fc.oneof(
      fc.constantFrom(...ADR_PERMISSIONS),
      fc.string({ maxLength: 20 })
    )
    fc.assert(
      fc.property(fc.array(name, { maxLength: 20 }), (list) => {
        const valid =
          list.every((entry) => ADR_PERMISSIONS.includes(entry)) &&
          new Set(list).size === list.length
        expect(readPermissions(list)).toEqual(valid ? list : undefined)
      })
    )
  })

  it("never throws on any JSON value", () => {
    fc.assert(
      fc.property(fc.jsonValue(), (value) => {
        const read = readPermissions(value)
        if (read !== undefined) {
          expect(read.every((entry) => ADR_PERMISSIONS.includes(entry))).toBe(
            true
          )
        }
        readRoles(value)
      })
    )
  })
})
