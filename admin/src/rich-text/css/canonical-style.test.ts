import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { CONTENT_V1_POLICY, PolicyError } from "../policy"
import {
  isCanonicalValue,
  isScalarGrammar,
  readCanonicalDecls,
} from "../style-decls"
import {
  BORDER_SIDES,
  canonicalStyle,
  sanitizeStyle,
  SHORTHAND_TARGETS,
} from "./canonical-style"
import type { Json } from "@/test/rich-text-policy"
import { policyWith, readVectors } from "@/test/rich-text-policy"

const POLICY = CONTENT_V1_POLICY
const IMAGE = "display: block; height: auto; margin: 0 auto; max-width: 100%"

/** The elements with a `style` grammar. */
const STYLED = Object.entries(POLICY.elements)
  .filter(([, element]) => element.style !== null)
  .map(([tag]) => tag)

/** A side as the server writes it: `none`, `hidden` or a full triple. */
const SIDE =
  /^(?:none|hidden|(?:0|[1-9][0-9]*)(?:\.[0-9]?[1-9])?px [a-z]+ (?:#[0-9a-f]{6}(?:[0-9a-f]{2})?|transparent|currentcolor))$/

/** Problems of a canonical string against the element's grammar. */
function problems(tag: string, css: string): string[] {
  const found: string[] = []
  if ([...css].some((c) => !POLICY.css.alphabet.includes(c))) {
    found.push("outside the alphabet")
  }
  const declarations = readCanonicalDecls(css, POLICY.css.alphabet)
  if (declarations === null) {
    return [...found, "unreadable"]
  }
  const names = [...declarations.keys()]
  if (names.join() !== [...names].sort().join()) {
    found.push("not sorted")
  }
  if (`${[...declarations].map(([p, v]) => `${p}: ${v}`).join("; ")}` !== css) {
    found.push("not written prop: value; ...")
  }
  for (const [property, value] of declarations) {
    const rule = POLICY.elements[tag].style?.find(
      (entry) => entry.property === property
    )
    if (rule?.role !== "stored") {
      found.push(`${property} is not stored on ${tag}`)
    } else if (isScalarGrammar(rule.grammar)) {
      if (!isCanonicalValue(rule.grammar, value)) {
        found.push(`${property}: ${value} is not canonical`)
      }
    } else if (!SIDE.test(value)) {
      found.push(`${property}: ${value} is not a side`)
    }
  }
  return found
}

describe("canonicalStyle", () => {
  // editor.md §3.1: both canonicalisers must write every line's output.
  it.each(readVectors().map((vector, line) => [line + 1, vector]))(
    "writes the server's output for css-vectors.jsonl line %i",
    (_line, { element, input, output }) => {
      expect(canonicalStyle(POLICY, element, input)).toBe(output)
    }
  )

  it("covers the whole vector file", () => {
    expect(readVectors().length).toBeGreaterThan(250)
  })

  // Reader cases beyond css-vectors.jsonl. Their outputs were checked against
  // the server's grammar (`sanitize_style`) when they were written; the
  // vector file is what keeps the two sides together from then on.
  it.each<[string, string, string, string | null]>([
    [
      "the shapes of editor.md §3.7 unchanged",
      "span",
      "color: #c00000; font-size: 1.25em",
      "color: #c00000; font-size: 1.25em",
    ],
    [
      "Chrome's CSSOM spelling",
      "span",
      "color: rgb(192, 0, 0); background-color: rgba(255, 255, 0, 0.5)",
      "background-color: #ffff0080; color: #c00000",
    ],
    [
      "grouped border widths",
      "td",
      "border-width: 1px 2px 3px 4px; border-style: solid; border-color: black",
      "border-bottom: 3px solid #000000; border-left: 4px solid #000000; border-right: 2px solid #000000; border-top: 1px solid #000000",
    ],
    [
      "Word's points and windowtext",
      "td",
      "border: solid windowtext .5pt; width: 72pt",
      "border-bottom: 0.67px solid #000000; border-left: 0.67px solid #000000; border-right: 0.67px solid #000000; border-top: 0.67px solid #000000; box-sizing: content-box; width: 96px",
    ],
    [
      "a locked value in another spelling",
      "img",
      "margin: 0PX AUTO; width: 350px",
      `${IMAGE}; width: 350px`,
    ],
    [
      "an unclosed rgb() at the end",
      "span",
      "color: rgb(255, 0, 0",
      "color: #ff0000",
    ],
    ["an escaped identifier", "span", "color: r\\65 d", "color: #ff0000"],
    [
      "a semicolon inside a string",
      "span",
      'font-family: "a;color: blue"; color: red',
      "color: #ff0000",
    ],
    [
      "a semicolon inside a URL",
      "span",
      "background: url(a;color:blue); color: red",
      "color: #ff0000",
    ],
    [
      "a semicolon inside a bad URL",
      "span",
      "background: url(a b;c); color: red",
      "color: #ff0000",
    ],
    [
      "a semicolon inside a block",
      "span",
      "x: [a; color: blue]; color: red",
      "color: #ff0000",
    ],
    [
      "a semicolon inside rgb()",
      "span",
      "color: rgb(1;2); background-color: red",
      "background-color: #ff0000",
    ],
    [
      "a bad string ending at its newline",
      "span",
      'x: "a\n; color: red',
      "color: #ff0000",
    ],
    [
      "a declaration after an unclosed comment",
      "span",
      "color: red; /* x; color: blue",
      "color: #ff0000",
    ],
    ["nothing after an unclosed block", "span", "x: (; color: red", null],
    ["a NUL inside a keyword", "p", "text-align: cen\u0000ter", null],
    ["an empty string", "span", "", null],
    [
      "cssparser's reading of `-\\` before a newline",
      "span",
      "x: -\\\n; color: red",
      "color: #ff0000",
    ],
    [
      "an escaped hash digit",
      "span",
      'color: #\\31 23; background-color: url( "x" ; color: blue)',
      "color: #112233",
    ],
    [
      "a word after !important",
      "span",
      "color: red; background-color: blue !important x; font-size: 1.5em",
      "color: #ff0000; font-size: 1.5em",
    ],
  ])("reads %s", (_name, tag, style, expected) => {
    expect(canonicalStyle(POLICY, tag, style)).toBe(expected)
  })

  it("follows the policy it is given, not a copy of its own", () => {
    const narrower = policyWith((json) => {
      const ul = (json.elements as { [key: string]: Json }).ul as {
        [key: string]: Json
      }
      ;(ul.style as { grammar: { values: string[] } }[])[0].grammar.values = [
        "square",
      ]
    })
    expect(canonicalStyle(POLICY, "ul", "list-style-type: circle")).toBe(
      "list-style-type: circle"
    )
    expect(canonicalStyle(narrower, "ul", "list-style-type: circle")).toBeNull()
    expect(canonicalStyle(narrower, "ul", "list-style-type: square")).toBe(
      "list-style-type: square"
    )
  })

  it("reads the border keyword widths and the initial and unset parts from the policy", () => {
    const thicker = policyWith((json) => {
      const border = (json.css as { [key: string]: Json }).border as {
        [key: string]: Json
      }
      ;(border.width_keywords as { [key: string]: number }).thin = 2
      ;(border.initial as { width: number }).width = 4
    })
    expect(canonicalStyle(thicker, "td", "border-top: thin solid")).toBe(
      "border-bottom: none; border-left: none; border-right: none; border-top: 2px solid currentcolor"
    )
    expect(canonicalStyle(thicker, "td", "border-top: solid")).toBe(
      "border-bottom: none; border-left: none; border-right: none; border-top: 4px solid currentcolor"
    )
  })

  it("refuses a policy with a scalar shorthand it cannot expand", () => {
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
    expect(() => canonicalStyle(policy, "p", "text-align: left")).toThrow(
      PolicyError
    )
    expect(() => canonicalStyle(policy, "p", "text-align: left")).toThrow(
      "span font is an input-only shorthand whose stored property canonicalStyle does not know"
    )
  })

  it("refuses a policy whose border parts are not a width and a colour", () => {
    const policy = policyWith((json) => {
      const border = (json.css as { [key: string]: Json }).border as {
        [key: string]: Json
      }
      ;(border.unset as { colour: string }).colour = "var(--x)"
    })
    expect(() => canonicalStyle(policy, "td", "border: none")).toThrow(
      "css.border.unset must be a width in px and a colour keyword"
    )
  })

  it("maps every scalar input-only shorthand of the policy to a stored property of the same grammar", () => {
    let checked = 0
    for (const [tag, element] of Object.entries(POLICY.elements)) {
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

  it("names the cell sides the policy stores, in CSS's order", () => {
    for (const tag of ["td", "th"]) {
      const stored = POLICY.elements[tag].style
        ?.filter(
          (rule) =>
            rule.grammar.kind === "border_side" && rule.role === "stored"
        )
        .map((rule) => rule.property)
      expect(stored).toEqual([...BORDER_SIDES])
    }
  })
})

describe("sanitizeStyle", () => {
  it.each<[string, string, string, string | null, number]>([
    [
      "declarations beside rejected ones",
      "span",
      "font-family: Calibri; COLOR: Red !important; mso-ansi-language: HU; font-size: 1.5EM",
      "color: #ff0000; font-size: 1.5em",
      2,
    ],
    [
      "a comment and a cell size",
      "td",
      "width: 96pt; color: red; /* x */; vertical-align: bottom",
      "box-sizing: content-box; vertical-align: bottom; width: 128px",
      2,
    ],
    [
      "an invalid later occurrence",
      "span",
      "color: red; color: hsl(0, 100%, 50%)",
      "color: #ff0000",
      1,
    ],
    [
      "a default, which is not counted",
      "ul",
      "list-style-type: circle; list-style-type: disc",
      null,
      0,
    ],
    [
      "an expanded shorthand, which is not counted",
      "td",
      "border: 1px solid",
      "border-bottom: 1px solid currentcolor; border-left: 1px solid currentcolor; border-right: 1px solid currentcolor; border-top: 1px solid currentcolor",
      0,
    ],
    [
      "a box-sizing without a size, which is not counted",
      "td",
      "box-sizing: content-box",
      null,
      0,
    ],
    [
      "every declaration on an element without a grammar",
      "li",
      "text-align: center; x",
      null,
      2,
    ],
    [
      "the declarations past the thirty-second",
      "p",
      `${"x: 1; ".repeat(32)}text-align: center`,
      null,
      33,
    ],
  ])("counts the removals of %s", (_name, tag, style, css, rejected) => {
    expect(sanitizeStyle(POLICY, tag, style)).toEqual({
      css,
      rejected,
      outsideAlphabet: false,
    })
  })

  it("drops a string outside the policy's alphabet and says so", () => {
    const narrow = policyWith((json) => {
      const css = json.css as { [key: string]: Json }
      css.alphabet = (css.alphabet as string).replace("#", "")
    })
    expect(sanitizeStyle(narrow, "span", "color: red")).toEqual({
      css: null,
      rejected: 0,
      outsideAlphabet: true,
    })
    expect(sanitizeStyle(narrow, "p", "text-align: center").css).toBe(
      "text-align: center"
    )
  })
})

describe("canonicalStyle over random styles", () => {
  const property = fc.constantFrom(
    ...new Set(
      Object.values(POLICY.elements).flatMap(
        (element) => element.style?.map((rule) => rule.property) ?? []
      )
    ),
    "font-family",
    "padding",
    "COLOR",
    "Border-Top"
  )
  const value = fc.oneof(
    fc.constantFrom(
      "red",
      "#C00",
      "#c0000080",
      "rgb(192, 0, 0)",
      "rgba(0 0 0 / 50%)",
      "transparent",
      "currentcolor",
      "windowtext",
      "1px",
      "0",
      ".5pt",
      "1.0cm",
      "20px",
      "21px",
      "100%",
      "1.25em",
      "0.4em",
      "auto",
      "center",
      "justify",
      "middle",
      "nowrap",
      "circle",
      "disc",
      "solid",
      "dashed",
      "none",
      "hidden",
      "thin",
      "medium",
      "thick",
      "1px solid #000",
      "solid windowtext 1.0pt",
      "1px 2px",
      "solid dashed dotted double",
      "red green blue",
      "block",
      "0 auto",
      "fit-content",
      "collapse",
      "table-row-group",
      "content-box",
      "16 / 9",
      "url(x)",
      "var(--x)"
    ),
    fc.string({ maxLength: 12 })
  )
  const declaration = fc
    .tuple(property, value, fc.boolean())
    .map(([p, v, important]) => `${p}: ${v}${important ? " !important" : ""}`)
  const style = fc.oneof(
    fc.array(declaration, { maxLength: 8 }).map((list) => list.join("; ")),
    fc.string({ maxLength: 60 })
  )
  const tag = fc.constantFrom(...STYLED, "li", "div")

  it("never throws, and writes only what the element's grammar stores", () => {
    fc.assert(
      fc.property(tag, style, (element, input) => {
        const css = canonicalStyle(POLICY, element, input)
        if (css !== null) {
          expect(problems(element, css), `${element} { ${input} }`).toEqual([])
        }
      }),
      { numRuns: 2000 }
    )
  })

  it("is a fixed point on its own output", () => {
    fc.assert(
      fc.property(tag, style, (element, input) => {
        const css = canonicalStyle(POLICY, element, input)
        if (css !== null) {
          expect(sanitizeStyle(POLICY, element, css)).toEqual({
            css,
            rejected: 0,
            outsideAlphabet: false,
          })
        }
      }),
      { numRuns: 2000 }
    )
  })

  it("never writes the CSSOM's rgb() or a grouped border-width", () => {
    fc.assert(
      fc.property(tag, style, (element, input) => {
        const css = canonicalStyle(POLICY, element, input) ?? ""
        expect(css).not.toContain("rgb(")
        expect(css).not.toContain("border-width:")
      })
    )
  })
})
