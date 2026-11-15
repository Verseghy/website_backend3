import { EditorState } from "@tiptap/pm/state"
import { EditorView } from "@tiptap/pm/view"
import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { CONTENT_V1_POLICY, requireAttributeRule } from "../policy"
import {
  LIST_STYLE_OF_TYPE,
  LIST_TYPE_ATTRIBUTE,
  listTypeDecorations,
  readIntegerAttribute,
} from "./site-lists"
import { parseHtml, siteSchema } from "@/test/site-schema"

const START = requireAttributeRule(CONTENT_V1_POLICY, "ol", "start", "integer")

describe("readIntegerAttribute", () => {
  it.each<[string, string | null, number | null]>([
    ["a number", "5", 5],
    ["the omitted default, which is a number like any other", "1", 1],
    ["zero, the minimum", "0", 0],
    ["the maximum", "9999", 9999],
    ["leading zeros, which the server rewrites", "005", 5],
    ["surrounding ASCII whitespace", " \t5\n", 5],
    ["a number above the maximum", "10000", null],
    ["a negative number", "-3", null],
    ["a sign", "+5", null],
    ["trailing text, which parseInt would accept", "5x", null],
    ["a decimal", "5.0", null],
    ["more than ten digits", "00000000005", null],
    ["a non-breaking space, which is not ASCII whitespace", " 5", null],
    ["an empty value", "", null],
    ["no attribute", null, null],
  ])("reads %s", (_name, value, expected) => {
    expect(readIntegerAttribute(value, START)).toBe(expected)
  })

  it("answers a number in range or null, never anything else", () => {
    fc.assert(
      fc.property(fc.string({ unit: "binary" }), (value) => {
        const read = readIntegerAttribute(value, START)
        if (read !== null) {
          expect(Number.isInteger(read)).toBe(true)
          expect(read).toBeGreaterThanOrEqual(START.min)
          expect(read).toBeLessThanOrEqual(START.max)
        }
      })
    )
  })

  it("reads every number in range from its decimal spelling", () => {
    fc.assert(
      fc.property(fc.integer({ min: 0, max: 20_000 }), (number) => {
        expect(readIntegerAttribute(String(number), START)).toBe(
          number <= START.max ? number : null
        )
      })
    )
  })
})

describe("listTypeDecorations", () => {
  const schema = siteSchema()

  /**
   * The attribute each `ol` gets in a view that shows the decorations, in
   * document order (`null` for an undecorated list).
   */
  function decorated(html: string): (string | null)[] {
    const view = new EditorView(document.createElement("div"), {
      state: EditorState.create({ doc: parseHtml(schema, html) }),
      decorations: (state) => listTypeDecorations(state.doc, "orderedList"),
    })
    const attributes = [...view.dom.querySelectorAll("ol")].map((list) =>
      list.getAttribute(LIST_TYPE_ATTRIBUTE)
    )
    view.destroy()
    return attributes
  }

  it("names the list style of every typed list, nested ones included", () => {
    expect(
      decorated(
        '<ol type="a"><li><p>x</p><ol type="I"><li>y</li></ol></li></ol><ol type="i"><li>z</li></ol><ol type="A"><li>w</li></ol>'
      )
    ).toEqual(["lower-alpha", "upper-roman", "lower-roman", "upper-alpha"])
  })

  it("leaves decimal lists and bullet lists alone", () => {
    expect(
      decorated(
        '<ol><li>x</li></ol><ol type="1"><li>y</li></ol><ul><li>z</li></ul>'
      )
    ).toEqual([null, null])
  })

  it("covers exactly the policy's list types", () => {
    const typeRule = requireAttributeRule(
      CONTENT_V1_POLICY,
      "ol",
      "type",
      "enumerated"
    )
    expect(Object.keys(LIST_STYLE_OF_TYPE).sort()).toEqual(
      typeRule.values.filter((value) => value !== typeRule.omitted).sort()
    )
  })
})
