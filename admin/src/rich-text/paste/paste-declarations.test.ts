import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { canonicalStyle } from "../css/canonical-style"
import { padletRules } from "../padlet-input"
import { CONTENT_V1_POLICY } from "../policy"
import { youtubeRules } from "../youtube-input"
import type { PasteContext } from "./paste-context"
import { emptyReportDraft } from "./paste-context"
import {
  cssomDeclarations,
  listTypeFromStyle,
  pastedStyle,
  roundCellStyle,
  storedProperties,
  writeDeclarations,
} from "./paste-declarations"
import type { PasteSource } from "./paste-source"

const policy = CONTENT_V1_POLICY
const parser = new DOMParser()

/** The context of a paste from `source`. */
function context(source: PasteSource): PasteContext {
  return {
    policy,
    source,
    youtube: youtubeRules(policy),
    padlet: padletRules(policy),
    report: emptyReportDraft(),
  }
}

/** The first element matching `selector` in parsed HTML. */
function element(html: string, selector: string): Element {
  return parser.parseFromString(html, "text/html").querySelector(selector)!
}

const SIDES = (side: string) =>
  ["bottom", "left", "right", "top"]
    .map((name) => `border-${name}: ${side}`)
    .join("; ")

describe("storedProperties", () => {
  it("lists the properties an element stores, without input-only shorthands", () => {
    expect(storedProperties(policy, "td")).toEqual([
      "border-top",
      "border-right",
      "border-bottom",
      "border-left",
      "vertical-align",
      "white-space",
      "width",
      "height",
      "box-sizing",
      "text-align",
      "background-color",
    ])
    expect(storedProperties(policy, "span")).toEqual([
      "color",
      "background-color",
      "font-size",
    ])
    expect(storedProperties(policy, "li")).toEqual([])
    expect(storedProperties(policy, "div")).toEqual([])
  })
})

describe("cssomDeclarations", () => {
  it("reads the cascaded value of each property, leaving out the undeclared", () => {
    const p = element(
      "<p style='text-align: left; color: red; text-align: center'>a</p>",
      "p"
    )
    expect(cssomDeclarations(p, ["text-align", "width"])).toBe(
      "text-align: center"
    )
  })
})

describe("roundCellStyle (editor.md §5.2 step 8, paste-time rounding)", () => {
  it("rounds widths, heights and border widths to whole px, a border above 0 to at least 1 px", () => {
    expect(
      roundCellStyle(
        policy,
        "td",
        "border-bottom: 0.67px solid #000000; border-left: 0px solid currentcolor; border-right: none; border-top: 1.33px dashed #ff0000; box-sizing: content-box; height: 20.5px; width: 128.25px"
      )
    ).toBe(
      "border-bottom: 1px solid #000000; border-left: 0px solid currentcolor; border-right: none; border-top: 1px dashed #ff0000; box-sizing: content-box; height: 21px; width: 128px"
    )
  })

  it("keeps a style without lengths and gives none for none", () => {
    expect(roundCellStyle(policy, "td", "vertical-align: top")).toBe(
      "vertical-align: top"
    )
    expect(roundCellStyle(policy, "th", null)).toBeNull()
  })

  it("always gives a canonical style of whole px", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 1999 }),
        fc.integer({ min: 0, max: 99 }),
        fc.integer({ min: 0, max: 19 }),
        (width, hundredths, border) => {
          const input = canonicalStyle(
            policy,
            "td",
            `width: ${width}.${hundredths}px; border-top: ${border}.5px solid #000000`
          )
          const rounded = roundCellStyle(policy, "td", input)
          expect(rounded).not.toBeNull()
          expect(canonicalStyle(policy, "td", rounded!)).toBe(rounded)
          expect(rounded).not.toMatch(/\d\.\d+px/)
        }
      )
    )
  })
})

describe("pastedStyle (editor.md §5.2 step 8)", () => {
  const OFFICE_CELL =
    "<table><tr><td style='border: .5pt solid windowtext; width: 48pt; height: 15.0pt; vertical-align: bottom; padding: 1px; font-family: Calibri; mso-number-format: General'>a</td></tr></table>"

  it("maps a generic cell onto content-v1, rounded, with white-space: normal", () => {
    expect(
      pastedStyle(context("generic"), element(OFFICE_CELL, "td"), "td")
    ).toBe(
      `${SIDES("1px solid #000000")}; box-sizing: content-box; height: 20px; vertical-align: bottom; white-space: normal; width: 64px`
    )
  })

  it("leaves white-space to an Excel desktop cell", () => {
    expect(
      pastedStyle(context("excel"), element(OFFICE_CELL, "td"), "td")
    ).toBe(
      `${SIDES("1px solid #000000")}; box-sizing: content-box; height: 20px; vertical-align: bottom; width: 64px`
    )
  })

  it("keeps a cell's own white-space", () => {
    expect(
      pastedStyle(
        context("generic"),
        element(
          "<table><tr><td style='white-space: nowrap'>a</td></tr></table>",
          "td"
        ),
        "td"
      )
    ).toBe("white-space: nowrap")
  })

  it("keeps the editor's own cell as written, without rounding", () => {
    expect(
      pastedStyle(
        context("internal"),
        element(
          "<table><tr><td style='border-top: 1.33px solid rgb(0, 0, 0); width: 128.25px'>a</td></tr></table>",
          "td"
        ),
        "td"
      )
    ).toBe(
      "border-bottom: none; border-left: none; border-right: none; border-top: 1.33px solid #000000; box-sizing: content-box; width: 128.25px"
    )
  })

  it("reads a cell's hints under its own style", () => {
    expect(
      pastedStyle(
        context("generic"),
        element(
          "<table><tr><td width=120 bgcolor=yellow align=middle valign=top style='vertical-align: bottom'>a</td></tr></table>",
          "td"
        ),
        "td"
      )
    ).toBe(
      "background-color: #ffff00; box-sizing: content-box; text-align: center; vertical-align: bottom; white-space: normal; width: 120px"
    )
  })

  it("reads a paragraph's align under its own text-align", () => {
    expect(
      pastedStyle(
        context("generic"),
        element("<p align=RIGHT style='text-align: center'>a</p>", "p"),
        "p"
      )
    ).toBe("text-align: center")
    expect(
      pastedStyle(
        context("generic"),
        element("<p align=justify>a</p>", "p"),
        "p"
      )
    ).toBe("text-align: justify")
    expect(
      pastedStyle(
        context("generic"),
        element("<p align=middle>a</p>", "p"),
        "p"
      )
    ).toBeNull()
  })

  it("drops what content-v1 does not store", () => {
    expect(
      pastedStyle(
        context("generic"),
        element(
          "<p style='margin: 0cm; line-height: 107%; font-family: Calibri; text-indent: -18pt'>a</p>",
          "p"
        ),
        "p"
      )
    ).toBeNull()
    expect(
      pastedStyle(
        context("generic"),
        element("<ul><li style='color: red'>a</li></ul>", "li"),
        "li"
      )
    ).toBeNull()
  })
})

describe("writeDeclarations", () => {
  it("writes declarations as the canonical string", () => {
    expect(
      writeDeclarations(
        policy,
        "td",
        new Map([
          ["width", "10px"],
          ["text-align", "left"],
        ])
      )
    ).toBe("box-sizing: content-box; text-align: left; width: 10px")
    expect(writeDeclarations(policy, "p", new Map())).toBeNull()
  })
})

describe("listTypeFromStyle", () => {
  it.each([
    ["lower-alpha", "a"],
    ["lower-latin", "a"],
    ["upper-alpha", "A"],
    ["upper-latin", "A"],
    ["lower-roman", "i"],
    ["upper-roman", "I"],
    ["decimal", null],
    ["disc", null],
  ])("maps list-style-type %s to %s", (type, expected) => {
    expect(
      listTypeFromStyle(
        element(`<ol style="list-style-type: ${type}"><li>a</li></ol>`, "ol")
      )
    ).toBe(expected)
  })

  it("gives none for a list without a style", () => {
    expect(listTypeFromStyle(element("<ol><li>a</li></ol>", "ol"))).toBeNull()
  })
})
