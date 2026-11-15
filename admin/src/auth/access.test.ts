/**
 * `decideAccess`, the answer of `authProvider.canAccess` (ADR 0007 §7): the
 * access map decides, a record-level rule may refuse a record on top of it,
 * and anything the map does not list is refused.
 */
import fc from "fast-check"
import { describe, expect, it, vi } from "vitest"

import type { AccessAction } from "@/bindings/AccessAction"

import {
  decideAccess,
  RECORD_RULES,
  type AccessSubject,
  type RecordRule,
} from "./access"
import { KNOWN_ACCESS_ACTIONS } from "./authz"

/** Every action, in the server's order. */
const ACTIONS = Object.keys(KNOWN_ACCESS_ACTIONS) as AccessAction[]

/** An invented writer who may work on the probe resource `probes`. */
const WRITER: AccessSubject = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  permissions: ["edit posts", "edit labels", "edit authors", "manage media"],
  is_admin: false,
  access: { probes: ["list", "show", "edit", "history"], others: ["list"] },
}

describe("decideAccess", () => {
  it("allows exactly the actions the access map lists for the resource", () => {
    for (const action of ACTIONS) {
      expect(decideAccess(WRITER, { action, resource: "probes" })).toBe(
        ["list", "show", "edit", "history"].includes(action)
      )
    }
    expect(decideAccess(WRITER, { action: "list", resource: "others" })).toBe(
      true
    )
    expect(decideAccess(WRITER, { action: "edit", resource: "others" })).toBe(
      false
    )
  })

  it.each([
    ["a resource the map leaves out", "list", "posts"],
    ["an action backend3 never offers", "export", "probes"],
    ["ra-core's clone", "clone", "probes"],
    ["another spelling of an action", "List", "probes"],
    ["another spelling of a resource", "list", "Probes"],
    ["an empty resource", "list", ""],
    ["a prototype name as the resource", "list", "constructor"],
    ["__proto__ as the resource", "list", "__proto__"],
    ["a prototype name as the action", "toString", "probes"],
  ])("refuses %s", (_case, action, resource) => {
    expect(decideAccess(WRITER, { action, resource })).toBe(false)
  })

  it("refuses everything to an account whose map is empty", () => {
    const nobody: AccessSubject = { ...WRITER, permissions: [], access: {} }
    for (const action of ACTIONS) {
      expect(decideAccess(nobody, { action, resource: "probes" })).toBe(false)
    }
  })

  it("asks the resource's record rule, with the record and the account, only for a record", () => {
    const rule = vi.fn<RecordRule>(
      (_action, record) => record.published !== true
    )
    const rules = { probes: rule }
    expect(
      decideAccess(
        WRITER,
        {
          action: "edit",
          resource: "probes",
          record: { id: 1, published: false },
        },
        rules
      )
    ).toBe(true)
    expect(
      decideAccess(
        WRITER,
        {
          action: "edit",
          resource: "probes",
          record: { id: 2, published: true },
        },
        rules
      )
    ).toBe(false)
    expect(rule).toHaveBeenCalledWith(
      "edit",
      { id: 2, published: true },
      WRITER
    )
    rule.mockClear()
    // Without a record, or with one that is no plain object, only the map
    // answers.
    for (const record of [undefined, null, 5, "x", [{ published: true }]]) {
      expect(
        decideAccess(
          WRITER,
          { action: "edit", resource: "probes", record },
          rules
        )
      ).toBe(true)
    }
    expect(rule).not.toHaveBeenCalled()
  })

  it("never asks a rule for an action the map refused", () => {
    const rule = vi.fn<RecordRule>(() => true)
    expect(
      decideAccess(
        WRITER,
        { action: "delete", resource: "probes", record: { id: 1 } },
        { probes: rule }
      )
    ).toBe(false)
    expect(rule).not.toHaveBeenCalled()
  })

  it("finds no rule on the prototype of the rules table", () => {
    const subject: AccessSubject = {
      ...WRITER,
      access: { toString: ["edit" as const] },
    }
    expect(
      decideAccess(subject, {
        action: "edit",
        resource: "toString",
        record: { id: 1 },
      })
    ).toBe(true)
  })

  it("has no record rule yet: the posts and users slices add theirs", () => {
    expect(Object.keys(RECORD_RULES)).toEqual([])
  })

  it("never allows what the map does not list, whatever the rule says", () => {
    const allowAll: RecordRule = () => true
    fc.assert(
      fc.property(
        fc.dictionary(
          fc.string({ maxLength: 8 }),
          fc.uniqueArray(fc.constantFrom(...ACTIONS))
        ),
        fc.string({ maxLength: 8 }),
        fc.oneof(fc.constantFrom(...ACTIONS), fc.string({ maxLength: 8 })),
        fc.option(fc.object(), { nil: undefined }),
        (access, resource, action, record) => {
          const subject: AccessSubject = { ...WRITER, access }
          const listed =
            Object.hasOwn(access, resource) &&
            (access[resource] as string[]).includes(action)
          expect(
            decideAccess(
              subject,
              { action, resource, record },
              { [resource]: allowAll }
            )
          ).toBe(listed)
          // A refusing rule refuses a record, and only a record.
          expect(
            decideAccess(
              subject,
              { action, resource, record },
              { [resource]: () => false }
            )
          ).toBe(listed && record === undefined)
        }
      )
    )
  })
})
