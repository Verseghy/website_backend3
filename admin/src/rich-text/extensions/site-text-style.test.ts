import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { CONTENT_V1_POLICY } from "../policy"
import { MAX_DECLARATIONS } from "../style-decls"
import { readSpanDeclarations } from "./site-text-style"
import { parseHtml, renderHtml, siteSchema } from "@/test/site-schema"

/**
 * The innermost `span` of nested markup.
 *
 * @param html - markup whose last `span` (in document order) is the
 *   innermost one.
 */
function innermostSpan(html: string): Element {
  const container = document.createElement("div")
  container.innerHTML = html
  const spans = container.querySelectorAll("span")
  return spans[spans.length - 1]
}

/**
 * Spans nested in one another around `text`, one per style, outermost
 * first; `null` is a span without a style.
 */
function nested(styles: readonly (string | null)[], text = "x"): string {
  return styles.reduceRight<string>(
    (inner, style) =>
      style === null
        ? `<span>${inner}</span>`
        : `<span style="${style}">${inner}</span>`,
    text
  )
}

const read = (html: string) => [
  ...readSpanDeclarations(CONTENT_V1_POLICY, innermostSpan(html)),
]

describe("readSpanDeclarations", () => {
  it.each<[string, string, [string, string][]]>([
    [
      "a span's own declarations",
      nested(["color: #c00000; font-size: 2em"]),
      [
        ["color", "#c00000"],
        ["font-size", "2em"],
      ],
    ],
    [
      "the outer declarations, with the inner ones winning",
      nested(["color: #c00000; font-size: 2em", "color: #0000ff"]),
      [
        ["color", "#0000ff"],
        ["font-size", "2em"],
      ],
    ],
    [
      "an outer span across other inline elements",
      '<span style="color: #c00000"><strong><em><span style="font-size: 2em">x</span></em></strong></span>',
      [
        ["color", "#c00000"],
        ["font-size", "2em"],
      ],
    ],
    [
      "an unstyled span between, which adds nothing",
      nested(["color: #c00000", null, "font-size: 2em"]),
      [
        ["color", "#c00000"],
        ["font-size", "2em"],
      ],
    ],
    [
      "an inner declaration in another spelling, as the server writes it",
      nested(["color: #c00000; font-size: 2em", "color: red"]),
      [
        ["color", "#ff0000"],
        ["font-size", "2em"],
      ],
    ],
    [
      "the colour an inner background shorthand sets",
      nested(["background-color: #ffff00", "background: #00ff00"]),
      [["background-color", "#00ff00"]],
    ],
    [
      "the outer value where the server drops the inner declaration",
      nested(["color: #c00000", "color: transparent; font-size: 2em"]),
      [
        ["color", "#c00000"],
        ["font-size", "2em"],
      ],
    ],
    [
      "outer declarations in other spellings",
      nested(["COLOR: RGB(192, 0, 0) !important", "font-size: 1.50EM"]),
      [
        ["color", "#c00000"],
        ["font-size", "1.5em"],
      ],
    ],
    [
      "the outer values through an inner style outside the alphabet",
      nested(["color: #c00000", "font-family: 'Calibri'"]),
      [["color", "#c00000"]],
    ],
  ])("reads %s", (_name, html, expected) => {
    expect(read(html)).toEqual(expected)
  })

  it("reads each style attribute on its own, however deep the nesting", () => {
    // Stock's merge writes every outer declaration into the inner style, so
    // past MAX_DECLARATIONS the inner span's own colour was not read.
    const singles = [
      ...Array.from({ length: MAX_DECLARATIONS }, () => "color: #ff0000"),
      "color: #00ff00",
    ]
    expect(read(nested(singles))).toEqual([["color", "#00ff00"]])
    const triples = [
      ...Array.from(
        { length: 11 },
        () => "color: #ff0000; background-color: #ff0000; font-size: 3em"
      ),
      "color: #00ff00; background-color: #00ff00; font-size: 0.5em",
    ]
    // In canonicalStyle's order, which sorts by property.
    expect(read(nested(triples))).toEqual([
      ["background-color", "#00ff00"],
      ["color", "#00ff00"],
      ["font-size", "0.5em"],
    ])
  })

  it("stops below an element with a contenteditable attribute", () => {
    const container = document.createElement("div")
    container.innerHTML =
      '<span style="color: #c00000"><div contenteditable="true"><p><span style="font-size: 2em">x</span></p></div></span>'
    const inner = container.querySelector("p span")
    expect(inner).not.toBeNull()
    expect([
      ...readSpanDeclarations(CONTENT_V1_POLICY, inner as Element),
    ]).toEqual([["font-size", "2em"]])
  })

  it("leaves the parsed markup unchanged", () => {
    const schema = siteSchema()
    const container = document.createElement("div")
    const html = `<p>${nested(["color: #c00000", "font-size: 2em"])}</p>`
    container.innerHTML = html
    parseHtml(schema, html)
    expect(container.innerHTML).toBe(html)
  })

  it("gives the innermost run the innermost declaration of each property", () => {
    const schema = siteSchema()
    const colour = fc.constantFrom("#c00000", "#0000ff", "#00ff00")
    const size = fc.constantFrom("0.5em", "1.5em", "2em")
    const span = fc.record({
      color: fc.option(colour, { nil: null }),
      fontSize: fc.option(size, { nil: null }),
    })
    fc.assert(
      fc.property(fc.array(span, { minLength: 1, maxLength: 60 }), (chain) => {
        const styles = chain.map(({ color, fontSize }) => {
          const parts = [
            color === null ? null : `color: ${color}`,
            fontSize === null ? null : `font-size: ${fontSize}`,
          ].filter((part): part is string => part !== null)
          return parts.length === 0 ? null : parts.join("; ")
        })
        const doc = parseHtml(schema, `<p>${nested(styles)}</p>`)
        const marks = doc.firstChild?.firstChild?.marks ?? []
        const style = marks.find(({ type }) => type.name === "textStyle")
        const last = <K extends "color" | "fontSize">(key: K) =>
          chain
            .map((entry) => entry[key])
            .filter((value) => value !== null)
            .at(-1) ?? null
        expect(style?.attrs.color ?? null).toBe(last("color"))
        expect(style?.attrs.fontSize ?? null).toBe(last("fontSize"))
      }),
      { numRuns: 200 }
    )
  })

  it("parses a body nested to the server's depth limit", () => {
    const schema = siteSchema()
    const styles = [
      ...Array.from({ length: 255 }, () => "color: #ff0000; font-size: 2em"),
      "color: #00ff00",
    ]
    const doc = parseHtml(schema, `<p>${nested(styles)}</p>`)
    expect(renderHtml(schema, doc).innerHTML).toBe(
      '<p><span style="color: #00ff00; font-size: 2em">x</span></p>'
    )
  })
})
