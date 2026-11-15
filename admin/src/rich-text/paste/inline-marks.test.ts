import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { CONTENT_V1_POLICY } from "../policy"
import type { DeclarationReader, InlineMarks } from "./inline-marks"
import {
  FORMATTING_TAGS,
  ownMarks,
  replaceWithMarks,
  spanStyle,
} from "./inline-marks"

const policy = CONTENT_V1_POLICY
const parser = new DOMParser()

/** A reader over fixed declarations. */
const declarations =
  (values: Record<string, string> = {}): DeclarationReader =>
  (property) =>
    values[property] ?? ""

/** The reader of an element's CSSOM. */
const cssom =
  (element: Element): DeclarationReader =>
  (property) =>
    (element as HTMLElement).style.getPropertyValue(property)

const NONE: InlineMarks = {
  bold: false,
  italic: false,
  underline: false,
  strike: false,
  script: null,
}

describe("ownMarks", () => {
  it.each<[string, object]>([
    ["b", { bold: true }],
    ["strong", { bold: true }],
    ["i", { italic: true }],
    ["em", { italic: true }],
    ["u", { underline: true }],
    ["ins", { underline: true }],
    ["s", { strike: true }],
    ["strike", { strike: true }],
    ["del", { strike: true }],
    ["sub", { script: "sub" }],
    ["sup", { script: "sup" }],
    ["span", {}],
    ["font", {}],
  ])("reads the tag %s as its lowest-precedence declaration", (tag, marks) => {
    expect(ownMarks(tag, declarations())).toEqual({ ...NONE, ...marks })
  })

  it.each<[string, Record<string, string>, object]>([
    ["span", { "font-weight": "700" }, { bold: true }],
    ["span", { "font-weight": "bold" }, { bold: true }],
    ["span", { "font-weight": "600" }, { bold: true }],
    ["b", { "font-weight": "normal" }, {}],
    ["b", { "font-weight": "400" }, {}],
    ["strong", { "font-weight": "lighter" }, {}],
    ["span", { "font-style": "italic" }, { italic: true }],
    ["span", { "font-style": "oblique 10deg" }, { italic: true }],
    ["i", { "font-style": "normal" }, {}],
    [
      "span",
      { "text-decoration-line": "underline line-through" },
      { underline: true, strike: true },
    ],
    [
      "span",
      { "text-decoration": "underline solid rgb(0, 0, 0)" },
      { underline: true },
    ],
    ["u", { "text-decoration": "none" }, {}],
    ["s", { "text-decoration-line": "overline" }, {}],
    ["span", { "vertical-align": "super" }, { script: "sup" }],
    ["span", { "vertical-align": "sub" }, { script: "sub" }],
    ["sup", { "vertical-align": "baseline" }, {}],
    ["sub", { "vertical-align": "top" }, { script: "sub" }],
  ])("lets %s's own %j override its tag", (tag, values, marks) => {
    expect(ownMarks(tag, declarations(values))).toEqual({ ...NONE, ...marks })
  })

  it("reads Google Docs' document wrapper as no mark", () => {
    const wrapper = parser
      .parseFromString(
        '<b style="font-weight:normal;" id="docs-internal-guid-0a1b"><p>a</p></b>',
        "text/html"
      )
      .querySelector("b")!
    expect(ownMarks("b", cssom(wrapper))).toEqual(NONE)
  })

  it("never throws on any declared value", () => {
    fc.assert(
      fc.property(
        fc.constantFrom(...FORMATTING_TAGS),
        fc.dictionary(
          fc.constantFrom(
            "font-weight",
            "font-style",
            "text-decoration",
            "text-decoration-line",
            "vertical-align"
          ),
          fc.string({ maxLength: 12 })
        ),
        (tag, values) => {
          const marks = ownMarks(tag, declarations(values))
          expect([null, "sub", "sup"]).toContain(marks.script)
        }
      )
    )
  })
})

describe("spanStyle", () => {
  /** The span style of the first element of `html`, pasted from `source`. */
  const styleOf = (
    html: string,
    source: "generic" | "internal" = "generic"
  ) => {
    const element = parser
      .parseFromString(html, "text/html")
      .body.querySelector("*")!
    return spanStyle(policy, source, element, cssom(element))
  }

  it("keeps colour, highlight and an em size, canonically", () => {
    expect(
      styleOf(
        '<span style="color: rgb(192, 0, 0); background-color: yellow; font-size: 1.25em; font-family: Arial">a</span>'
      )
    ).toBe("background-color: #ffff00; color: #c00000; font-size: 1.25em")
  })

  it("drops black text, white and transparent highlights and absolute sizes", () => {
    expect(
      styleOf(
        '<span style="color: #000000; background-color: transparent; font-size: 11pt">a</span>'
      )
    ).toBeNull()
    expect(
      styleOf(
        '<span style="color: windowtext; background-color: #FFFFFF">a</span>'
      )
    ).toBeNull()
  })

  it("reads font[color] under the element's own colour", () => {
    expect(styleOf('<font color="#c9211e">a</font>')).toBe("color: #c9211e")
    expect(styleOf('<font color="red" style="color: blue">a</font>')).toBe(
      "color: #0000ff"
    )
  })

  it("keeps the editor's own span style as written, black included", () => {
    expect(
      styleOf('<span style="color: rgb(0, 0, 0)">a</span>', "internal")
    ).toBe("color: #000000")
  })
})

describe("replaceWithMarks", () => {
  /** Replaces the first formatting element of a paragraph and returns the paragraph's HTML. */
  const replaced = (
    html: string,
    marks: Partial<InlineMarks>,
    style: string | null
  ) => {
    const paragraph = parser
      .parseFromString(`<p>${html}</p>`, "text/html")
      .querySelector("p")!
    replaceWithMarks(paragraph.firstElementChild!, { ...NONE, ...marks }, style)
    return paragraph.innerHTML
  }

  it("wraps the children in the span and every mark", () => {
    expect(
      replaced(
        "<b>x<i>y</i></b>",
        {
          bold: true,
          italic: true,
          underline: true,
          strike: true,
          script: "sup",
        },
        "color: #c00000"
      )
    ).toBe(
      '<span style="color: #c00000"><strong><em><u><s><sup>x<i>y</i></sup></s></u></em></strong></span>'
    )
  })

  it("unwraps an element without marks or style", () => {
    expect(replaced("<span>a<em>b</em></span>c", {}, null)).toBe("a<em>b</em>c")
  })

  it.each<[string, Partial<InlineMarks>, string | null, string]>([
    [
      '<span class="x" style="color: red; font-family: Arial">a<em>b</em></span>',
      {},
      "color: #ff0000",
      '<span style="color: #ff0000">a<em>b</em></span>',
    ],
    ['<strong id="y">a</strong>', { bold: true }, null, "<strong>a</strong>"],
    ['<sub lang="hu">2</sub>', { script: "sub" }, null, "<sub>2</sub>"],
  ])(
    "rewrites %s in place when it already is the one wrapper it needs",
    (html, marks, style, expected) => {
      const paragraph = parser
        .parseFromString(`<p>${html}</p>`, "text/html")
        .querySelector("p")!
      const element = paragraph.firstElementChild!
      const children = [...element.childNodes]
      replaceWithMarks(element, { ...NONE, ...marks }, style)
      expect(paragraph.innerHTML).toBe(expected)
      expect(paragraph.firstElementChild).toBe(element)
      expect([...element.childNodes]).toEqual(children)
    }
  )

  it("replaces an element that needs another tag or more than one wrapper", () => {
    expect(replaced('<b style="color: red">a</b>', { bold: true }, null)).toBe(
      "<strong>a</strong>"
    )
    expect(replaced("<span>a</span>", { bold: true }, "color: #ff0000")).toBe(
      '<span style="color: #ff0000"><strong>a</strong></span>'
    )
  })
})
