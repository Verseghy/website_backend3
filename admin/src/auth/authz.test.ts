/**
 * The run-time role, permission and access-map readers of `/auth/me`
 * (ADR 0007 §1, §2, §7): they accept exactly the generated unions' values,
 * each once, and never find a name on the object prototype.
 */
import fc from "fast-check"
import { describe, expect, it } from "vitest"

import {
  accessActionsOf,
  isAccessAction,
  KNOWN_ACCESS_ACTIONS,
  KNOWN_PERMISSIONS,
  KNOWN_ROLES,
  readAccess,
  readPermissions,
  readRoles,
} from "./authz"

/** The nine actions of ADR 0007 §7, in the server's order. */
const ADR_ACTIONS = [
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

describe("the access actions", () => {
  it("are ra-core's five and backend3's four, in the server's order", () => {
    expect(Object.keys(KNOWN_ACCESS_ACTIONS)).toEqual(ADR_ACTIONS)
  })

  it("are known exactly by their own names", () => {
    for (const action of ADR_ACTIONS) {
      expect(isAccessAction(action)).toBe(true)
    }
    for (const other of [
      "List",
      "list ",
      "read",
      "export",
      "clone",
      "*",
      "",
      "toString",
      "__proto__",
      1,
      null,
      undefined,
      ["list"],
    ]) {
      expect(isAccessAction(other)).toBe(false)
    }
    fc.assert(
      fc.property(fc.string(), (name) => {
        expect(isAccessAction(name)).toBe(ADR_ACTIONS.includes(name))
      })
    )
  })
})

describe("readAccess", () => {
  it("reads a map of resources to known actions, each once, as sent", () => {
    const access = { labels: ["list", "show", "history"], posts: ["purge"] }
    expect(readAccess(access)).toBe(access)
    expect(readAccess({})).toEqual({})
    // A resource with no action is no defect: it simply allows nothing.
    expect(readAccess({ labels: [] })).toEqual({ labels: [] })
    expect(readAccess(Object.create(null))).toEqual({})
  })

  it.each([
    ["null", null],
    ["a list", [["list"]]],
    ["a string", "labels"],
    ["an unknown action", { labels: ["export"] }],
    ["an action twice", { labels: ["list", "list"] }],
    ["actions that are not a list", { labels: "list" }],
    ["a list of booleans", { labels: [true] }],
    ["an instance of a class", new Map([["labels", ["list"]]])],
  ])("refuses %s", (_case, value) => {
    expect(readAccess(value)).toBeUndefined()
  })

  it("never throws on any JSON value, and keeps only known actions", () => {
    fc.assert(
      fc.property(fc.jsonValue(), (value) => {
        const read = readAccess(value)
        if (read !== undefined) {
          for (const actions of Object.values(read)) {
            expect(actions.every((action) => isAccessAction(action))).toBe(true)
          }
        }
      })
    )
  })
})

describe("accessActionsOf", () => {
  it("lists a resource's own actions and nothing for any other name", () => {
    const access = { labels: ["list", "edit"] } as const
    expect(
      accessActionsOf({ ...access, labels: [...access.labels] }, "labels")
    ).toEqual(["list", "edit"])
    for (const name of [
      "posts",
      "",
      "Labels",
      "constructor",
      "toString",
      "__proto__",
      "hasOwnProperty",
    ]) {
      expect(accessActionsOf({ labels: ["list"] }, name)).toEqual([])
    }
  })

  it("finds a resource named __proto__ only as an own member", () => {
    const access = readAccess(JSON.parse('{"__proto__": ["list"]}'))
    expect(access).toBeDefined()
    expect(accessActionsOf(access ?? {}, "__proto__")).toEqual(["list"])
    expect(accessActionsOf({}, "__proto__")).toEqual([])
  })

  it("answers exactly the own members of any map", () => {
    fc.assert(
      fc.property(
        fc.dictionary(
          fc.string(),
          fc.uniqueArray(fc.constantFrom(...ADR_ACTIONS))
        ),
        fc.string(),
        (map, resource) => {
          const access = readAccess(map)
          expect(access).toBeDefined()
          expect(accessActionsOf(access ?? {}, resource)).toEqual(
            Object.hasOwn(map, resource) ? map[resource] : []
          )
        }
      )
    )
  })
})
