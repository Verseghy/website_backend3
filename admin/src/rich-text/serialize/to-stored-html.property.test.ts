import type { Mark, Node as ProseMirrorNode } from "@tiptap/pm/model"
import fc from "fast-check"
import { afterAll, describe, expect, it } from "vitest"

import { writeHtml } from "./write-html"
import { reparse, storedHtmlHarness } from "@/test/stored-html"
import { siteSchema } from "@/test/site-schema"

/**
 * editor.md §13.4, L2 fast-check: random documents with leading, trailing
 * and doubled spaces, tabs and hard breaks, and random HTML, through the
 * stored-HTML serialiser. Each run loads into a headless editor under
 * jsdom, which costs a few milliseconds, so the runs are fewer than in the
 * pure tests and the timeouts longer.
 */
const harness = storedHtmlHarness()
afterAll(() => harness.destroy())
const { J, load, save } = harness
const schema = siteSchema()
const { nodes, marks } = schema

// Random documents of the site schema.

const piece = fc.constantFrom(
  "a",
  "szó",
  "Ő",
  " ",
  "  ",
  "\t",
  "\n",
  "\u00a0",
  "&",
  "<",
  '"',
  "\u2003",
  // Neither survives an HTML parse in text (whitespace.ts): a plain-text
  // paste can still put them into the document.
  "\u0000",
  "\r"
)
const text = fc
  .array(piece, { minLength: 1, maxLength: 6 })
  .map((pieces) => pieces.join(""))

const markSet: fc.Arbitrary<readonly Mark[]> = fc
  .record({
    bold: fc.boolean(),
    italic: fc.boolean(),
    underline: fc.boolean(),
    strike: fc.boolean(),
    script: fc.constantFrom(null, "subscript", "superscript"),
    link: fc.option(
      fc.constantFrom("https://example.org/", "/x", "#top", "", null),
      {
        nil: undefined,
      }
    ),
    style: fc.option(
      fc.record({
        color: fc.constantFrom(null, "#c00000", "#0000ff"),
        backgroundColor: fc.constantFrom(null, "#ffff00", "transparent"),
        fontSize: fc.constantFrom(null, "0.85em", "2em"),
      }),
      { nil: undefined }
    ),
  })
  .map((choice) => {
    const chosen: Mark[] = []
    for (const name of ["bold", "italic", "underline", "strike"] as const) {
      if (choice[name]) {
        chosen.push(marks[name].create())
      }
    }
    if (choice.script !== null) {
      chosen.push(marks[choice.script].create())
    }
    if (choice.link !== undefined) {
      chosen.push(marks.link.create({ href: choice.link }))
    }
    if (choice.style !== undefined) {
      chosen.push(marks.textStyle.create(choice.style))
    }
    return chosen.reduce<readonly Mark[]>((set, mark) => mark.addToSet(set), [])
  })

const inline = fc.array(
  fc.oneof(
    {
      weight: 4,
      arbitrary: fc
        .tuple(text, markSet)
        .map(([value, set]) => schema.text(value, set)),
    },
    { weight: 1, arbitrary: fc.constant(nodes.hardBreak.create()) }
  ),
  { maxLength: 6 }
)
const alignment = fc.constantFrom(null, "left", "center", "right", "justify")

const { block } = fc.letrec<{ block: ProseMirrorNode; item: ProseMirrorNode }>(
  (tie) => ({
    block: fc.oneof(
      { depthSize: "small", withCrossShrink: true },
      fc
        .tuple(alignment, inline)
        .map(([textAlign, content]) =>
          nodes.paragraph.createChecked({ textAlign }, content)
        ),
      fc
        .tuple(fc.constantFrom(1, 2, 3, 4), alignment, inline)
        .map(([level, textAlign, content]) =>
          nodes.heading.createChecked({ level, textAlign }, content)
        ),
      fc.constant(nodes.horizontalRule.create()),
      fc
        .array(tie("block"), { minLength: 1, maxLength: 3 })
        .map((content) => nodes.blockquote.createChecked(null, content)),
      fc
        .tuple(
          fc.constantFrom(null, "circle", "square"),
          fc.array(tie("item"), { minLength: 1, maxLength: 3 })
        )
        .map(([listStyleType, content]) =>
          nodes.bulletList.createChecked({ listStyleType }, content)
        ),
      fc
        .tuple(
          fc.constantFrom(1, 3),
          fc.constantFrom(null, "a", "I"),
          fc.array(tie("item"), { minLength: 1, maxLength: 3 })
        )
        .map(([start, type, content]) =>
          nodes.orderedList.createChecked({ start, type }, content)
        )
    ),
    item: fc
      .tuple(
        fc.tuple(alignment, inline),
        fc.array(tie("block"), { maxLength: 2 })
      )
      .map(([[textAlign, content], rest]) =>
        nodes.listItem.createChecked(null, [
          nodes.paragraph.createChecked({ textAlign }, content),
          ...rest,
        ])
      ),
  })
)

const documents = fc
  .array(block, { minLength: 1, maxLength: 4 })
  .map((content) => nodes.doc.createChecked(null, content))

describe("toStoredHtml over random documents", () => {
  it("is a fixed point: toStoredHtml(load(toStoredHtml(d))) == toStoredHtml(d)", () => {
    fc.assert(
      fc.property(documents, (doc) => {
        const stored = save(doc)
        expect(save(load(stored)), JSON.stringify(doc.toJSON())).toBe(stored)
      }),
      { numRuns: 300 }
    )
  }, 60_000)

  it("writes HTML that an HTML parser reads back to the same bytes", () => {
    fc.assert(
      fc.property(documents, (doc) => {
        const stored = save(doc)
        expect(writeHtml(reparse(stored))).toBe(stored)
      }),
      { numRuns: 300 }
    )
  }, 60_000)

  it("never writes the CSSOM's rgb() or a grouped border-width", () => {
    fc.assert(
      fc.property(documents, (doc) => {
        expect(save(doc)).not.toMatch(/rgb\(|border-width:/)
      }),
      { numRuns: 200 }
    )
  }, 60_000)
})

// Random HTML, in content-v1 and outside it.

const declaration = fc
  .tuple(
    fc.constantFrom(
      "color",
      "COLOR",
      "background-color",
      "background",
      "font-size",
      "text-align",
      "list-style-type",
      "font-family"
    ),
    fc.constantFrom(
      "#c00000",
      "#C00",
      "red",
      "rgb(192, 0, 0)",
      "rgba(0, 0, 0, 0.5)",
      "transparent",
      "1.5em",
      "12pt",
      "center",
      "CENTER",
      "start",
      "square",
      "Calibri"
    )
  )
  .map(([property, value]) => `${property}: ${value}`)
const attribute = fc.oneof(
  fc
    .array(declaration, { minLength: 1, maxLength: 3 })
    .map((list) => ` style="${list.join("; ")}"`),
  fc.constantFrom(
    ' href="https://example.org/"',
    ' href="javascript:alert(1)"',
    ' href=""',
    ' start="5"',
    ' type="i"',
    ' class="x"',
    ' target="_self"'
  )
)
const tag = fc.constantFrom(
  "p",
  "h2",
  "h5",
  "div",
  "span",
  "strong",
  "b",
  "em",
  "u",
  "s",
  "sub",
  "sup",
  "a",
  "blockquote",
  "ul",
  "ol",
  "li"
)
const textPiece = fc.constantFrom(
  "a",
  "b c",
  " ",
  "  ",
  "\n  ",
  "\t",
  "&nbsp;",
  "&amp;"
)
const { html } = fc.letrec<{ html: string }>((tie) => ({
  html: fc.oneof(
    { depthSize: "small", withCrossShrink: true },
    textPiece,
    fc.constantFrom("<br>", "<hr>"),
    fc
      .tuple(
        tag,
        fc.array(attribute, { maxLength: 2 }),
        fc.array(tie("html"), { maxLength: 4 })
      )
      .map(
        ([name, attributes, children]) =>
          `<${name}${attributes.join("")}>${children.join("")}</${name}>`
      )
  ),
}))
const bodies = fc
  .array(html, { minLength: 1, maxLength: 4 })
  .map((parts) => parts.join(""))

describe("J over random HTML", () => {
  it("is idempotent: J(J(x)) == J(x)", () => {
    fc.assert(
      fc.property(bodies, (body) => {
        const once = J(body)
        expect(J(once), body).toBe(once)
      }),
      { numRuns: 400 }
    )
  }, 60_000)
})
