import fc from "fast-check"
import { describe, expect, it } from "vitest"

import type { Component, Declaration } from "./declarations"
import {
  asciiLowerCase,
  MAX_COMPONENTS,
  MAX_DECLARATIONS,
  readDeclarations,
  readValue,
} from "./declarations"

/**
 * The reader's cases follow the server's own tests of
 * `src/domain/rich_text/css/declarations.rs`, so a difference between the
 * two readers shows up here as well as in `css-vectors.jsonl`.
 */
const names = (style: string) =>
  readDeclarations(style).read.map((declaration) => declaration.name)

/** The value of a style that holds exactly one readable declaration. */
function one(style: string): readonly Component[] | null {
  const { read, unreadable } = readDeclarations(style)
  return read.length === 1 && unreadable === 0 ? read[0].value : null
}

const ident = (value: string): Component => ({ kind: "ident", value })
const px = (value: number): Component => ({
  kind: "dimension",
  value,
  unit: "px",
})

describe("readDeclarations", () => {
  it("splits declarations at top-level semicolons", () => {
    expect(
      readDeclarations("color: red;  ; WIDTH : 10PX;;background-color:#C00000;")
    ).toEqual({
      read: [
        { name: "color", value: [ident("red")] },
        { name: "width", value: [px(10)] },
        {
          name: "background-color",
          value: [{ kind: "hash", value: "C00000" }],
        },
      ],
      unreadable: 0,
    })
    expect(readDeclarations("")).toEqual({ read: [], unreadable: 0 })
    expect(readDeclarations(" ;; ")).toEqual({ read: [], unreadable: 0 })
  })

  it("skips an unreadable declaration up to the next top-level semicolon", () => {
    const declarations = readDeclarations(
      "color: url(x); width: 1px; height: (2px; 3px); text-align: left"
    )
    expect(names("color: url(x); width: 1px")).toEqual(["width"])
    expect(declarations.unreadable).toBe(2)
    // The `;` inside the parenthesised block does not end the declaration.
    expect(declarations.read.map(({ name }) => name)).toEqual([
      "width",
      "text-align",
    ])
    // An unclosed block swallows the rest of the attribute.
    expect(names("width: [1px; height: 2px")).toEqual([])
  })

  it.each([
    "background: url(https://evil.example/x.png)",
    "background: url( 'x' )",
    "background: URL(x)",
    "background: u\\72 l(x)",
    "width: expression(alert(1))",
    "color: var(--x)",
    "width: calc(1px + 2px)",
    "content: attr(title)",
    "width: env(safe-area-inset-left)",
    "color: hsl(0, 100%, 50%)",
    'font-family: "Calibri"',
    "font-family: 'Calibri",
    "color: red}",
    "color: red)",
    "color: {red}",
    "@import x",
    "color: @x",
    "font-family: a, b",
    "color: red; x",
    "width: 1px + 2px",
    "color:: red",
    "color red",
    ": red",
    "1px: red",
    "color: <!-- red",
    "color: red -->",
    "color: #",
    "width: 1px.",
    "color: rgb(255, 0, 0, 0, 0, 0, 0, 0)",
    "color: rgb(x)",
    "color: rgb(var(--r), 0, 0)",
    "border: 1px 1px 1px 1px 1px 1px 1px 1px 1px",
  ])("makes %s unreadable", (style) => {
    const declarations = readDeclarations(style)
    expect(declarations.unreadable).toBeGreaterThanOrEqual(1)
    expect(
      declarations.read.every(
        ({ name }) =>
          !["background", "width", "content", "font-family"].includes(name)
      )
    ).toBe(true)
  })

  it("makes a declaration with a comment unreadable", () => {
    expect(readDeclarations("color: red /* x */").unreadable).toBe(1)
    expect(readDeclarations("col/**/or: red").unreadable).toBe(1)
    expect(readDeclarations("width: 1px/**/; color: red").read).toHaveLength(1)
    // A trailing comment counts as a declaration.
    expect(readDeclarations("/* x */").unreadable).toBe(1)
    expect(readDeclarations("color: red !/**/important").unreadable).toBe(1)
  })

  it("strips !important only at the end", () => {
    expect(one("color: red !important")).toEqual([ident("red")])
    expect(one("color:red!IMPORTANT  ")).toEqual([ident("red")])
    expect(one("color: red ! important")).toEqual([ident("red")])
    expect(one("color: !important")).toEqual([])
    for (const style of [
      "color: red !important x",
      "color: red !imp",
      "color: red !",
      "color: ! red",
      "color: red !important !important",
    ]) {
      expect(readDeclarations(style).unreadable, style).toBe(1)
    }
  })

  it("keeps each component's kind", () => {
    expect(one("border: thin SOLID #ABC")).toEqual([
      ident("thin"),
      ident("solid"),
      { kind: "hash", value: "ABC" },
    ])
    expect(one("aspect-ratio: 16/9")).toEqual([
      { kind: "number", value: 16 },
      { kind: "slash" },
      { kind: "number", value: 9 },
    ])
    expect(one("x: 12.5PT 100% #1a2 r\\65 d")).toEqual([
      { kind: "dimension", value: 12.5, unit: "pt" },
      { kind: "percentage", value: 100 },
      { kind: "hash", value: "1a2" },
      ident("red"),
    ])
    expect(one("color: RGBA( 255 , 0,0 ,50% )")).toEqual([
      {
        kind: "rgb",
        arguments: [
          { kind: "number", value: 255 },
          { kind: "comma" },
          { kind: "number", value: 0 },
          { kind: "comma" },
          { kind: "number", value: 0 },
          { kind: "comma" },
          { kind: "percentage", value: 50 },
        ],
      },
    ])
    expect(one("color: rgb(1 2 3 / .5)")).toEqual([
      {
        kind: "rgb",
        arguments: [
          { kind: "number", value: 1 },
          { kind: "number", value: 2 },
          { kind: "number", value: 3 },
          { kind: "slash" },
          { kind: "number", value: 0.5 },
        ],
      },
    ])
  })

  it("reads numbers from their source text", () => {
    expect(one("x: .5pt")).toEqual([
      { kind: "dimension", value: 0.5, unit: "pt" },
    ])
    expect(one("x: +3")).toEqual([{ kind: "number", value: 3 }])
    expect(one("x: -0.25em")).toEqual([
      { kind: "dimension", value: -0.25, unit: "em" },
    ])
    expect(one("x: 1e3px")).toEqual([px(1000)])
    expect(one("x: 2E-2PX")).toEqual([px(0.02)])
    expect(one("x: 1em")).toEqual([{ kind: "dimension", value: 1, unit: "em" }])
    expect(one("x: 1e+1em")).toEqual([
      { kind: "dimension", value: 10, unit: "em" },
    ])
    expect(one("x: 0.67px")).toEqual([px(0.67)])
    expect(one("x: 0.67%")).toEqual([{ kind: "percentage", value: 0.67 }])
    // Too large for a double: the declaration cannot be read.
    expect(readDeclarations("x: 1e400px").unreadable).toBe(1)
  })

  it("decodes escapes and NUL in names as cssparser does", () => {
    expect(one("x: \\72 ed")).toEqual([ident("red")])
    expect(one("x: \\000072 ed")).toEqual([ident("red")])
    expect(one("x: a\\0 b")).toEqual([ident("a�b")])
    expect(one("x: a\\d800 b")).toEqual([ident("a�b")])
    expect(one("x: a\\110000 b")).toEqual([ident("a�b")])
    expect(one("x: a\u0000b")).toEqual([ident("a�b")])
    expect(one("x: \\\\a")).toEqual([ident("\\a")])
    expect(one("x: a\\")).toEqual([ident("a�")])
    expect(one("x: é")).toEqual([ident("é")])
    expect(one("C\\4f LOR: red")?.length).toBe(1)
    expect(names("C\\4f LOR: red")).toEqual(["color"])
  })

  it("lower-cases ASCII letters only", () => {
    expect(one("x: İSTANBUL")).toEqual([ident("İstanbul")])
    expect(asciiLowerCase("ÁRVÍZTŰRŐ TÜKÖRFÚRÓGÉP")).toBe(
      "ÁrvÍztŰrŐ tÜkÖrfÚrÓgÉp"
    )
  })

  it("reads at most thirty-two declarations", () => {
    const style = Array.from({ length: 40 }, (_, i) => `p${i}: ${i}`).join("; ")
    const declarations = readDeclarations(style)
    expect(declarations.read).toHaveLength(MAX_DECLARATIONS)
    expect(declarations.read.at(-1)?.name).toBe("p31")
    expect(declarations.unreadable).toBe(8)
    // Unreadable declarations count towards the limit, empty ones do not.
    const counted = readDeclarations(`${"x; ".repeat(32)};;;; color: red`)
    expect([counted.read.length, counted.unreadable]).toEqual([0, 33])
  })

  it("reads at most eight components in a value", () => {
    const value = (count: number) => Array(count).fill("1px").join(" ")
    expect(one(`border-width: ${value(MAX_COMPONENTS)}`)).toEqual(
      Array(MAX_COMPONENTS).fill(px(1))
    )
    const longer = readDeclarations(
      `border-width: ${value(MAX_COMPONENTS + 1)}; color: red`
    )
    expect(longer.unreadable).toBe(1)
    expect(longer.read).toEqual([{ name: "color", value: [ident("red")] }])
    // `!important` is not a component, and an rgb() call is one.
    expect(one(`x: ${value(MAX_COMPONENTS)} !important`)).toHaveLength(
      MAX_COMPONENTS
    )
    expect(one(`x: ${value(MAX_COMPONENTS - 1)} rgb(1, 2, 3)`)).toHaveLength(
      MAX_COMPONENTS
    )
    expect(readValue(value(MAX_COMPONENTS + 1))).toBeNull()
  })

  it("never throws, and keeps its limits, on any string", () => {
    fc.assert(
      fc.property(fc.string({ unit: "binary", maxLength: 200 }), (style) => {
        const declarations = readDeclarations(style)
        expect(declarations.read.length).toBeLessThanOrEqual(MAX_DECLARATIONS)
        for (const declaration of declarations.read) {
          expect(declaration.value.length).toBeLessThanOrEqual(MAX_COMPONENTS)
          expect(asciiLowerCase(declaration.name)).toBe(declaration.name)
        }
      }),
      { numRuns: 1000 }
    )
  })

  it("never throws on CSS-shaped input", () => {
    const part = fc.constantFrom(
      "color",
      ":",
      ";",
      " ",
      "red",
      "#fff",
      "1px",
      ".5pt",
      "(",
      ")",
      "[",
      "]",
      "{",
      "}",
      "/*",
      "*/",
      "!",
      "important",
      "rgb(",
      "url(",
      '"',
      "'",
      "\\",
      "/",
      ",",
      "%",
      "e3",
      "-",
      "+",
      "\u0000",
      "é",
      "@media",
      "<!--",
      "-->",
      "\n",
      "\r\n",
      "\f"
    )
    fc.assert(
      fc.property(fc.array(part, { maxLength: 40 }), (parts) => {
        readDeclarations(parts.join(""))
        readValue(parts.join(""))
      }),
      { numRuns: 2000 }
    )
  })

  it("reads written declarations back in order", () => {
    fc.assert(
      fc.property(
        fc.array(
          fc.tuple(
            fc.stringMatching(/^[a-z][a-z-]{0,12}$/),
            fc.stringMatching(/^[a-z]{1,8}$/),
            fc.integer({ min: 0, max: 99_999 })
          ),
          { maxLength: 40 }
        ),
        (list) => {
          const style = list
            .map(([name, keyword, number]) => `${name}: ${keyword} ${number}px`)
            .join("; ")
          const read = readDeclarations(style)
          const expected = Math.min(list.length, MAX_DECLARATIONS)
          expect(read.read).toHaveLength(expected)
          expect(read.unreadable).toBe(list.length - expected)
          read.read.forEach((declaration: Declaration, index) => {
            const [name, keyword, number] = list[index]
            expect(declaration).toEqual({
              name,
              value: [ident(keyword), px(number)],
            })
          })
        }
      )
    )
  })

  it("parses numbers like Number()", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 0, max: 999_999 }),
        fc.integer({ min: 0, max: 999 }),
        fc.integer({ min: -3, max: 3 }),
        (whole, fraction, exponent) => {
          const text = `${whole}.${String(fraction).padStart(3, "0")}e${exponent}`
          expect(readValue(`${text}pt`)).toEqual([
            { kind: "dimension", value: Number(text), unit: "pt" },
          ])
          expect(readValue(`${text}%`)).toEqual([
            { kind: "percentage", value: Number(text) },
          ])
        }
      )
    )
  })
})

describe("readValue", () => {
  it("reads a bare value", () => {
    expect(readValue("0 auto")).toEqual([
      { kind: "number", value: 0 },
      ident("auto"),
    ])
    expect(readValue("")).toEqual([])
    expect(readValue("16 / 9")).toEqual([
      { kind: "number", value: 16 },
      { kind: "slash" },
      { kind: "number", value: 9 },
    ])
    expect(readValue("url(x)")).toBeNull()
    expect(readValue("a; b")).toBeNull()
  })
})
