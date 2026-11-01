import fc from "fast-check"
import { describe, expect, it } from "vitest"

import type { StyleGrammar } from "./policy"
import { CONTENT_V1_POLICY } from "./policy"
import type { ScalarGrammar } from "./style-decls"
import {
  isCanonicalValue,
  isScalarGrammar,
  keptDeclarations,
  MAX_DECLARATIONS,
  readCanonicalDecls,
  readStoredDeclarations,
  readStyleDecls,
  resolveStoredDeclarations,
  SHORTHAND_TARGETS,
} from "./style-decls"
import type { Json } from "@/test/rich-text-policy"
import {
  policyWith,
  readVectors,
  writeDeclarations,
} from "@/test/rich-text-policy"

const ALPHABET = CONTENT_V1_POLICY.css.alphabet

/** The elements whose styles the schema reads (tables and media come later). */
const SCHEMA_STYLED = ["p", "h1", "h2", "h3", "h4", "span", "ul"]

describe("readCanonicalDecls", () => {
  it.each<[string, string, [string, string][] | null]>([
    ["one declaration", "text-align: center", [["text-align", "center"]]],
    [
      "several, in first-occurrence order",
      "color: #c00000; font-size: 1.25em",
      [
        ["color", "#c00000"],
        ["font-size", "1.25em"],
      ],
    ],
    ["a trailing semicolon", "text-align: center;", [["text-align", "center"]]],
    [
      "no space after the colon",
      "text-align:center",
      [["text-align", "center"]],
    ],
    [
      "the last occurrence winning",
      "color: #000000; color: #ff0000",
      [["color", "#ff0000"]],
    ],
    [
      "TipTap's merged nested style with null",
      "null;font-size: 1.5em",
      [["font-size", "1.5em"]],
    ],
    ["an empty style", "", []],
    ["empty pieces and values", " ; ; color: ;: x", []],
    [
      "the aspect-ratio slash",
      "aspect-ratio: 16 / 9",
      [["aspect-ratio", "16 / 9"]],
    ],
    ["upper case", "TEXT-ALIGN: CENTER", null],
    ["a function", "color: rgb(192, 0, 0)", null],
    ["!important", "color: #ff0000 !important", null],
    ["a comment", "color: #ff0000 /* x */", null],
    ["a string", 'font-family: "x;color: #ff0000"', null],
    ["an escape", "color: #ff0000; background: u\\72l(x)", null],
    ["a non-ASCII character", "color: #ff0000; content: é", null],
  ])("reads %s", (_name, style, expected) => {
    const read = readCanonicalDecls(style, ALPHABET)
    expect(read === null ? null : [...read]).toEqual(expected)
  })

  it("reads the first MAX_DECLARATIONS non-empty declarations only", () => {
    const filler = Array.from({ length: MAX_DECLARATIONS - 2 }, () => "x: 1")
    const style = [
      "text-align: left",
      " ",
      "null",
      ...filler,
      "text-align: right",
    ].join(";")
    expect(readCanonicalDecls(style, ALPHABET)?.get("text-align")).toBe("left")
    expect(
      readCanonicalDecls(
        [...filler, "text-align: right"].join(";"),
        ALPHABET
      )?.get("text-align")
    ).toBe("right")
  })

  it("reads nothing from a string with a character outside the alphabet", () => {
    fc.assert(
      fc.property(fc.string({ unit: "binary" }), (style) => {
        const outside = [...style].some(
          (character) => !ALPHABET.includes(character)
        )
        expect(readCanonicalDecls(style, ALPHABET) === null).toBe(outside)
      })
    )
  })

  it("reads back every declaration list written in canonical form", () => {
    const name = fc.stringMatching(/^[a-z][a-z-]{0,12}$/)
    const value = fc.stringMatching(
      /^[a-z0-9#.%/-][a-z0-9#.%/ -]{0,10}[a-z0-9#.%/-]$/
    )
    fc.assert(
      fc.property(
        fc.uniqueArray(fc.tuple(name, value), { selector: ([n]) => n }),
        (pairs) => {
          const style = pairs.map(([n, v]) => `${n}: ${v}`).join("; ")
          expect([...(readCanonicalDecls(style, ALPHABET) ?? [])]).toEqual(
            pairs
          )
        }
      )
    )
  })
})

describe("readStyleDecls", () => {
  it("reads the raw style attribute, not the CSSOM", () => {
    const element = document.createElement("span")
    element.setAttribute("style", "color: #c00000; font-size: 1.25em")
    // jsdom's CSSOM re-spells the colour as rgb(192, 0, 0).
    expect(element.style.color).not.toBe("#c00000")
    expect([...readStyleDecls(element, ALPHABET)]).toEqual([
      ["color", "#c00000"],
      ["font-size", "1.25em"],
    ])
  })

  it.each([
    ["no style attribute", null],
    ["a style outside the alphabet", "COLOR: RED"],
  ])("reads nothing from an element with %s", (_name, style) => {
    const element = document.createElement("p")
    if (style !== null) {
      element.setAttribute("style", style)
    }
    expect(readStyleDecls(element, ALPHABET).size).toBe(0)
  })
})

describe("isCanonicalValue", () => {
  const keyword: ScalarGrammar = {
    kind: "keyword",
    values: ["circle", "square", "disc"],
    omitted: "disc",
  }
  const px: ScalarGrammar = { kind: "px", min: 1, max: 2000 }
  const autoOrPx: ScalarGrammar = { kind: "auto_or_px", min: 100, max: 2000 }
  const em: ScalarGrammar = { kind: "em", min: 0.5, max: 3 }
  const colour: ScalarGrammar = { kind: "colour", opaque_only: false }
  const opaque: ScalarGrammar = { kind: "colour", opaque_only: true }
  const locked: ScalarGrammar = { kind: "locked", value: "0 auto" }

  it.each<[string, ScalarGrammar, string, boolean]>([
    ["a keyword", keyword, "circle", true],
    ["the omitted default keyword", keyword, "disc", false],
    ["an unknown keyword", keyword, "decimal", false],
    ["a keyword in upper case", keyword, "CIRCLE", false],
    ["whole px", px, "20px", true],
    ["two-decimal px", px, "1.33px", true],
    ["px at the minimum", px, "1px", true],
    ["px at the maximum", px, "2000px", true],
    ["px below the minimum", px, "0.99px", false],
    ["px above the maximum", px, "2000.01px", false],
    ["px with a trailing zero", px, "1.50px", false],
    ["px with three decimals", px, "1.333px", false],
    ["px with a leading zero", px, "020px", false],
    ["px without a leading zero", px, ".5px", false],
    ["px with a sign", px, "+20px", false],
    ["another unit", px, "20pt", false],
    ["a bare number", px, "20", false],
    ["auto", autoOrPx, "auto", true],
    ["px of auto_or_px", autoOrPx, "608px", true],
    ["px below auto_or_px's minimum", autoOrPx, "99px", false],
    ["em", em, "1.25em", true],
    ["em at both ends", em, "0.5em", true],
    ["em at 3", em, "3em", true],
    ["em below 0.5", em, "0.49em", false],
    ["em above 3", em, "3.01em", false],
    ["em in upper case", em, "3EM", false],
    ["a hex colour", colour, "#c00000", true],
    ["a hex colour with alpha", colour, "#000000b3", true],
    ["a hex colour with a full alpha", colour, "#c00000ff", false],
    ["transparent", colour, "transparent", true],
    ["currentcolor", colour, "currentcolor", true],
    ["an upper-case hex colour", colour, "#C00000", false],
    ["a short hex colour", colour, "#c00", false],
    ["a named colour", colour, "red", false],
    ["an opaque colour", opaque, "#c00000", true],
    ["an alpha in an opaque-only colour", opaque, "#c0000080", false],
    ["transparent in an opaque-only colour", opaque, "transparent", false],
    ["currentcolor in an opaque-only colour", opaque, "currentcolor", false],
    ["the locked value", locked, "0 auto", true],
    ["another spelling of the locked value", locked, "0px auto", false],
  ])("judges %s", (_name, grammar, value, expected) => {
    expect(isCanonicalValue(grammar, value)).toBe(expected)
  })

  it("accepts a number in its one spelling exactly when it is in range", () => {
    fc.assert(
      fc.property(fc.integer({ min: 0, max: 100_000 }), (hundredths) => {
        const whole = Math.floor(hundredths / 100)
        const fraction = String(hundredths % 100)
          .padStart(2, "0")
          .replace(/0+$/, "")
        const text = fraction === "" ? `${whole}` : `${whole}.${fraction}`
        expect(isCanonicalValue(em, `${text}em`)).toBe(
          hundredths >= 50 && hundredths <= 300
        )
        expect(isCanonicalValue(px, `${text}px`)).toBe(
          hundredths >= 100 && hundredths <= 200_000
        )
        // Any other spelling of the same number is refused.
        expect(
          isCanonicalValue(
            em,
            `${whole}.${String(hundredths % 100).padStart(2, "0")}0em`
          )
        ).toBe(false)
      })
    )
  })
})

describe("isScalarGrammar", () => {
  it.each<[StyleGrammar, boolean]>([
    [{ kind: "keyword", values: [], omitted: null }, true],
    [{ kind: "locked", value: "x" }, true],
    [{ kind: "border_side" }, false],
    [{ kind: "border_widths" }, false],
    [{ kind: "border_styles" }, false],
    [{ kind: "border_colours" }, false],
  ])("answers %o with %s", (grammar, expected) => {
    expect(isScalarGrammar(grammar)).toBe(expected)
  })
})

describe("readStoredDeclarations", () => {
  it.each<[string, string, string | null, [string, string][]]>([
    [
      "a paragraph alignment",
      "p",
      "text-align: justify",
      [["text-align", "justify"]],
    ],
    [
      "a heading alignment",
      "h3",
      "text-align: center",
      [["text-align", "center"]],
    ],
    [
      "the span declarations",
      "span",
      "background-color: #ffff00; color: #c00000; font-size: 1.82em",
      [
        ["background-color", "#ffff00"],
        ["color", "#c00000"],
        ["font-size", "1.82em"],
      ],
    ],
    [
      "a bullet style",
      "ul",
      "list-style-type: square",
      [["list-style-type", "square"]],
    ],
    ["the omitted default bullet", "ul", "list-style-type: disc", []],
    ["a property the element does not store", "p", "color: #c00000", []],
    ["an input-only shorthand", "span", "background: #ffff00", []],
    ["a value outside the grammar", "span", "font-size: 4em", []],
    ["a value in another spelling", "span", "color: #C00000", []],
    ["transparent text", "span", "color: transparent", []],
    ["an element without a style", "strong", "color: #c00000", []],
    ["an element outside content-v1", "div", "text-align: center", []],
    ["no style at all", "p", null, []],
    [
      "a later canonical occurrence",
      "span",
      "color: #000000; color: #c00000",
      [["color", "#c00000"]],
    ],
    [
      "nothing of a property whose last occurrence the server would rewrite",
      "span",
      "color: #c00000; color: red",
      [],
    ],
    [
      "a style outside the alphabet, which keeps nothing",
      "span",
      "color: #c00000; font-family: Calibri",
      [],
    ],
    [
      "nothing of a stored property that a later input-only shorthand sets",
      "span",
      "background-color: #00ff00; background: #ff0000",
      [],
    ],
    [
      "a stored property set after the shorthand",
      "span",
      "background: #ff0000; background-color: #00ff00",
      [["background-color", "#00ff00"]],
    ],
    [
      "the other declarations next to a shorthand",
      "span",
      "color: #c00000; background: #ff0000; font-size: 2em",
      [
        ["color", "#c00000"],
        ["font-size", "2em"],
      ],
    ],
  ])("keeps %s", (_name, tag, style, expected) => {
    expect([...readStoredDeclarations(CONTENT_V1_POLICY, tag, style)]).toEqual(
      expected
    )
  })

  it("refuses to read a cell border side, which needs its own reader", () => {
    expect(() =>
      readStoredDeclarations(CONTENT_V1_POLICY, "td", "border-top: none")
    ).toThrow("td border-top has the grammar border_side")
  })

  it("skips the cell border shorthands, which set no property it returns", () => {
    expect([
      ...readStoredDeclarations(
        CONTENT_V1_POLICY,
        "td",
        "border: 1px solid #000000; text-align: center; border-width: 2px"
      ),
    ]).toEqual([["text-align", "center"]])
  })

  it("refuses a scalar input-only shorthand whose stored property it does not know", () => {
    const policy = policyWith((json) => {
      const span = (json.elements as { [key: string]: Json }).span as {
        [key: string]: Json
      }
      ;(span.style as Json[]).push({
        property: "font",
        grammar: { kind: "em", min: 0.5, max: 3 },
        role: "input_only",
        presence: "optional",
        provisional: null,
      })
    })
    expect(() => readStoredDeclarations(policy, "span", "font: 2em")).toThrow(
      "span font is an input-only shorthand whose stored property is unknown"
    )
    expect([
      ...readStoredDeclarations(policy, "span", "color: #c00000"),
    ]).toEqual([["color", "#c00000"]])
  })

  it("agrees with every vector the server writes for the elements the schema reads", () => {
    let checked = 0
    for (const { element, input, output } of readVectors()) {
      if (!SCHEMA_STYLED.includes(element)) {
        continue
      }
      // The server's canonical output reads back unchanged.
      if (output !== null) {
        expect(
          writeDeclarations(
            readStoredDeclarations(CONTENT_V1_POLICY, element, output)
          ),
          `${element} ${output}`
        ).toBe(output)
      }
      // Every declaration this reader keeps of an input is in the server's
      // output with the same value: it may refuse more (other spellings),
      // never keep something else.
      const server = readCanonicalDecls(output ?? "", ALPHABET)
      for (const [property, value] of readStoredDeclarations(
        CONTENT_V1_POLICY,
        element,
        input
      )) {
        expect(server?.get(property), `${element} ${input}`).toBe(value)
      }
      checked += 1
    }
    expect(checked).toBeGreaterThan(90)
  })

  it("keeps only stored properties with canonical values, whatever the style", () => {
    const pieces = fc.constantFrom(
      "color",
      "background-color",
      "background",
      "font-size",
      "text-align",
      "list-style-type",
      "#c00000",
      "#c0000080",
      "transparent",
      "1.5em",
      "0.4em",
      "center",
      "circle",
      "disc",
      ":",
      ";",
      " ",
      "red",
      "(",
      "!"
    )
    fc.assert(
      fc.property(
        fc.constantFrom(...SCHEMA_STYLED),
        fc.array(pieces, { maxLength: 20 }).map((parts) => parts.join("")),
        (tag, style) => {
          for (const [property, value] of readStoredDeclarations(
            CONTENT_V1_POLICY,
            tag,
            style
          )) {
            const rule = CONTENT_V1_POLICY.elements[tag].style?.find(
              (entry) => entry.property === property
            )
            expect(rule?.role).toBe("stored")
            expect(
              rule &&
                isScalarGrammar(rule.grammar) &&
                isCanonicalValue(rule.grammar, value)
            ).toBe(true)
          }
        }
      )
    )
  })
})

describe("resolveStoredDeclarations", () => {
  it.each<[string, string, string | null, [string, string | null][]]>([
    [
      "a refused value as null",
      "span",
      "color: #c00000; color: red",
      [["color", null]],
    ],
    [
      "the shorthand's stored property as null",
      "span",
      "background-color: #00ff00; background: #ff0000",
      [["background-color", null]],
    ],
    [
      "a stored property after the shorthand",
      "span",
      "background: #ff0000; background-color: #00ff00",
      [["background-color", "#00ff00"]],
    ],
    [
      "the omitted default as null, which the server leaves out",
      "ul",
      "list-style-type: disc",
      [["list-style-type", null]],
    ],
    [
      "nothing for properties the element does not store",
      "span",
      "line-height: 2; text-align: center",
      [],
    ],
    ["nothing for a style outside the alphabet", "span", "color: Red", []],
    ["nothing for no style", "span", null, []],
  ])("resolves %s", (_name, tag, style, expected) => {
    expect([
      ...resolveStoredDeclarations(CONTENT_V1_POLICY, tag, style),
    ]).toEqual(expected)
  })

  it("maps every scalar input-only shorthand of the policy to a stored property of the same kind", () => {
    let checked = 0
    for (const [tag, element] of Object.entries(CONTENT_V1_POLICY.elements)) {
      for (const rule of element.style ?? []) {
        if (rule.role !== "input_only" || !isScalarGrammar(rule.grammar)) {
          continue
        }
        const target = SHORTHAND_TARGETS[rule.property]
        const stored = element.style?.find((entry) => entry.property === target)
        expect(stored?.role, `${tag} ${rule.property}`).toBe("stored")
        expect(stored?.grammar, `${tag} ${rule.property}`).toEqual(rule.grammar)
        checked += 1
      }
    }
    expect(checked).toBeGreaterThan(0)
  })

  it("keeps a declaration only when it is the last one that sets its property", () => {
    const declaration = fc
      .tuple(
        fc.constantFrom(
          "color",
          "background-color",
          "background",
          "font-size",
          "line-height"
        ),
        fc.constantFrom("#c00000", "#00ff00", "transparent", "2em", "red")
      )
      .map(([property, value]) => `${property}: ${value}`)
    fc.assert(
      fc.property(
        fc.array(declaration, { maxLength: 40 }).map((list) => list.join("; ")),
        (style) => {
          // The pairs the server reads, in source order: the first
          // MAX_DECLARATIONS, each naming the stored property it sets.
          const setters = style
            .split(";")
            .slice(0, MAX_DECLARATIONS)
            .map((piece) => piece.split(":").map((part) => part.trim()))
            .map(([property, value]) => ({
              target: SHORTHAND_TARGETS[property] ?? property,
              property,
              value,
            }))
          for (const [property, value] of readStoredDeclarations(
            CONTENT_V1_POLICY,
            "span",
            style
          )) {
            const last = setters
              .filter(({ target }) => target === property)
              .at(-1)
            expect(last, style).toEqual({ target: property, property, value })
          }
        }
      )
    )
  })
})

describe("keptDeclarations", () => {
  it("drops the refused properties and keeps the order", () => {
    expect([
      ...keptDeclarations(
        new Map<string, string | null>([
          ["font-size", "2em"],
          ["color", null],
          ["background-color", "#ffff00"],
        ])
      ),
    ]).toEqual([
      ["font-size", "2em"],
      ["background-color", "#ffff00"],
    ])
    expect(keptDeclarations(new Map()).size).toBe(0)
  })
})
