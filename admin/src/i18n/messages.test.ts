import { describe, expect, it } from "vitest"

import {
  lookupMessage,
  mergeMessages,
  messageKeys,
  placeholders,
  pluralFormCount,
} from "./messages"

const tree = {
  ra: {
    action: { save: "Mentés", delete: "Törlés" },
    boolean: { true: "Igen" },
  },
  app: { title: "Cím" },
}

describe("messageKeys", () => {
  it("lists every leaf as a dotted key", () => {
    expect(messageKeys(tree)).toEqual([
      "ra.action.save",
      "ra.action.delete",
      "ra.boolean.true",
      "app.title",
    ])
  })

  it("returns nothing for an empty tree", () => {
    expect(messageKeys({})).toEqual([])
  })
})

describe("lookupMessage", () => {
  it("finds a leaf", () => {
    expect(lookupMessage(tree, "ra.action.save")).toBe("Mentés")
  })

  it("returns undefined for a missing key", () => {
    expect(lookupMessage(tree, "ra.action.missing")).toBeUndefined()
    expect(lookupMessage(tree, "nope.at.all")).toBeUndefined()
  })

  it("returns undefined for an inner node, which is not a phrase", () => {
    expect(lookupMessage(tree, "ra.action")).toBeUndefined()
  })

  it("does not descend below a leaf", () => {
    expect(lookupMessage(tree, "app.title.length")).toBeUndefined()
  })

  it("ignores inherited object properties", () => {
    expect(lookupMessage(tree, "ra.toString")).toBeUndefined()
    expect(lookupMessage(tree, "constructor")).toBeUndefined()
  })
})

describe("mergeMessages", () => {
  it("lets overlay phrases win and keeps base-only keys", () => {
    const base = { ra: { action: { save: "Save", undo: "Undo" } } }
    const overlay = {
      ra: { action: { save: "Mentés" } },
      app: { title: "Cím" },
    }
    expect(mergeMessages(base, overlay)).toEqual({
      ra: { action: { save: "Mentés", undo: "Undo" } },
      app: { title: "Cím" },
    })
  })

  it("does not mutate either input", () => {
    const base = { ra: { action: { save: "Save" } } }
    const overlay = { ra: { action: { save: "Mentés" } } }
    mergeMessages(base, overlay)
    expect(base.ra.action.save).toBe("Save")
    expect(overlay.ra.action.save).toBe("Mentés")
  })

  it("refuses to replace a namespace with a phrase", () => {
    expect(() =>
      mergeMessages(
        { ra: { action: { save: "Save" } } },
        { ra: { action: "x" } }
      )
    ).toThrow(/disagree on "action"/)
  })

  it("refuses to replace a phrase with a namespace", () => {
    expect(() =>
      mergeMessages({ app: { title: "T" } }, { app: { title: { x: "y" } } })
    ).toThrow(/disagree on "title"/)
  })
})

describe("placeholders", () => {
  it("extracts sorted, distinct variable names across plural forms", () => {
    expect(
      placeholders("%{name} törlése |||| %{smart_count} %{name} törlése")
    ).toEqual(["name", "smart_count"])
  })

  it("returns nothing for a phrase without variables", () => {
    expect(placeholders("Mentés")).toEqual([])
  })
})

describe("pluralFormCount", () => {
  it("counts the forms", () => {
    expect(pluralFormCount("Mentés")).toBe(1)
    expect(pluralFormCount("Törölve |||| %{smart_count} elem törölve")).toBe(2)
  })
})
