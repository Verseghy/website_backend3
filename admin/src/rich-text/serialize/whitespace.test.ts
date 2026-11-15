import type { Mark, Node as ProseMirrorNode } from "@tiptap/pm/model"
import { DOMParser } from "@tiptap/pm/model"
import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { normalizeInlineWhitespace, normalizeWhitespace } from "./whitespace"
import { renderHtml, siteSchema } from "@/test/site-schema"

const schema = siteSchema()
const { nodes, marks } = schema
const NBSP = "\u00a0"

/** The text of a textblock, with each hard break as `|` and marks as `*…*`. */
function show(content: readonly ProseMirrorNode[]): string {
  return content
    .map((node) =>
      node.isText
        ? node.marks.length > 0
          ? `*${node.text}*`
          : (node.text ?? "")
        : node.type === nodes.hardBreak
          ? "|"
          : "#"
    )
    .join("")
}

/**
 * Builds inline content from a pattern: `|` is a hard break, text between
 * `*` is bold, everything else plain text.
 */
function inline(pattern: string): ProseMirrorNode[] {
  const content: ProseMirrorNode[] = []
  let bold = false
  for (const piece of pattern.split(/(\||\*)/)) {
    if (piece === "|") {
      content.push(nodes.hardBreak.create())
    } else if (piece === "*") {
      bold = !bold
    } else if (piece !== "") {
      content.push(schema.text(piece, bold ? [marks.bold.create()] : []))
    }
  }
  return content
}

const isBreak = (node: ProseMirrorNode) => node.type === nodes.hardBreak
const rule = (pattern: string) =>
  show(normalizeInlineWhitespace(inline(pattern), isBreak))

describe("normalizeInlineWhitespace", () => {
  it.each<[string, string, string]>([
    ["nothing to do", "a b", "a b"],
    ["leading whitespace", "  \t\na", "a"],
    ["trailing whitespace", "a \n\t", "a"],
    ["whitespace only", " \t ", ""],
    ["whitespace after a hard break", "a| b", "a|b"],
    ["whitespace before a hard break, an inner run", "a |b", "a |b"],
    ["two spaces", "a  b", `a${NBSP} b`],
    ["three spaces", "a   b", `a${NBSP}${NBSP} b`],
    ["a tab", "a\tb", "a b"],
    ["a newline and a space", "a\n b", `a${NBSP} b`],
    ["CR LF", "a\r\nb", `a${NBSP} b`],
    ["a form feed", "a\fb", "a b"],
    [
      "no-break spaces, which are not collapsible",
      `${NBSP}a${NBSP}`,
      `${NBSP}a${NBSP}`,
    ],
    ["a run beside a no-break space", `a ${NBSP} b`, `a ${NBSP} b`],
    [
      "other Unicode spaces, which are kept",
      "a\u2003\u200bb",
      "a\u2003\u200bb",
    ],
    [
      "a run across marks, each character keeping its mark",
      "a *  b*",
      `a${NBSP}*${NBSP} b*`,
    ],
    ["leading whitespace inside a mark", "* a*", "*a*"],
    ["a mark of whitespace only at the end", "a* *", "a"],
    ["whitespace after a break inside a mark", "*a| b*", "*a*|*b*"],
    ["two breaks with whitespace between", "a| |b", "a||b"],
    ["a break at the end", "a |", "a |"],
    // No HTML parse keeps U+0000 in text, so it is dropped before the runs
    // are counted, as the reload counts them.
    ["U+0000, dropped", "a\u0000b", "ab"],
    ["U+0000 only", "\u0000\u0000", ""],
    ["U+0000 inside a run, which the reload joins", "a \u0000 b", `a${NBSP} b`],
    ["U+0000 before leading whitespace", "\u0000 a", "a"],
    ["U+0000 after trailing whitespace", "a \u0000", "a"],
    ["U+0000 between a break and a space", "a|\u0000 b", "a|b"],
    ["U+0000 in a mark of its own", "a*\u0000*b", "ab"],
  ])("handles %s", (_name, pattern, expected) => {
    expect(rule(pattern)).toBe(expected)
  })

  it("joins neighbouring text nodes with the same marks", () => {
    const content = [schema.text("a "), schema.text(" b")]
    const result = normalizeInlineWhitespace(content, isBreak)
    expect(result).toHaveLength(1)
    expect(result[0].text).toBe(`a${NBSP} b`)
  })

  it("keeps the marks of each character", () => {
    const strong = [marks.bold.create()]
    const result = normalizeInlineWhitespace(
      [schema.text("a ", strong), schema.text(" b")],
      isBreak
    )
    expect(result.map((node) => [node.text, node.marks.length])).toEqual([
      [`a${NBSP}`, 1],
      [" b", 0],
    ])
  })
})

describe("normalizeWhitespace", () => {
  const doc = (...content: ProseMirrorNode[]) => nodes.doc.create(null, content)
  const paragraph = (value: string) =>
    nodes.paragraph.create(null, value === "" ? [] : schema.text(value))

  it("returns the same document when nothing changes", () => {
    const unchanged = doc(paragraph("a b"), nodes.horizontalRule.create())
    expect(normalizeWhitespace(unchanged)).toBe(unchanged)
  })

  it("rewrites every textblock, however deep", () => {
    const deep = doc(
      nodes.blockquote.create(null, [
        nodes.bulletList.create(null, [
          nodes.listItem.create(null, [paragraph(" a  b ")]),
        ]),
      ]),
      nodes.heading.create({ level: 2 }, schema.text("c ")),
      paragraph("d")
    )
    const result = normalizeWhitespace(deep)
    expect(result.textContent).toBe(`a${NBSP} bcd`)
    // The untouched paragraph is shared, not copied.
    expect(result.lastChild).toBe(deep.lastChild)
  })

  it("empties a paragraph that holds only U+0000, as the reload does", () => {
    // The post-pass must see this paragraph as empty: a list item's first
    // paragraph is tightened only when it has content (post-pass.ts).
    const item = nodes.listItem.create(null, [
      paragraph("\u0000"),
      nodes.bulletList.create(null, [
        nodes.listItem.create(null, [paragraph("b")]),
      ]),
    ])
    const result = normalizeWhitespace(
      doc(nodes.bulletList.create(null, [item]))
    )
    const first = result.firstChild?.firstChild?.firstChild
    expect(first?.type).toBe(nodes.paragraph)
    expect(first?.childCount).toBe(0)
  })
})

// Random textblocks with every kind of whitespace.
const piece = fc.constantFrom(
  "a",
  "szó",
  " ",
  "  ",
  "\t",
  "\n",
  "\r\n",
  NBSP,
  `${NBSP} `,
  "\f",
  "\u0000"
)
const run: fc.Arbitrary<ProseMirrorNode | null> = fc
  .tuple(
    fc.array(piece, { minLength: 1, maxLength: 4 }),
    fc.constantFrom(null, "bold", "italic", "link")
  )
  .map(([pieces, mark]) => {
    const value = pieces.join("")
    const set: Mark[] =
      mark === null
        ? []
        : [
            mark === "link"
              ? marks.link.create({ href: "/x" })
              : marks[mark].create(),
          ]
    return value === "" ? null : schema.text(value, set)
  })
const content = fc
  .array(
    fc.oneof(
      { weight: 4, arbitrary: run },
      { weight: 1, arbitrary: fc.constant(nodes.hardBreak.create()) }
    ),
    { maxLength: 8 }
  )
  .map((list) => list.filter((node): node is ProseMirrorNode => node !== null))

describe("the whitespace rule over random textblocks", () => {
  const parser = DOMParser.fromSchema(schema)

  it("writes text that prosemirror-model loads back unchanged", () => {
    fc.assert(
      fc.property(content, (inlineContent) => {
        const block = nodes.paragraph.create(
          null,
          normalizeInlineWhitespace(inlineContent, isBreak)
        )
        const written = nodes.doc.create(null, [block])
        const loaded = parser.parse(renderHtml(schema, written))
        expect(loaded.toJSON()).toEqual(written.toJSON())
      }),
      { numRuns: 500 }
    )
  })

  it("is idempotent", () => {
    fc.assert(
      fc.property(content, (inlineContent) => {
        const once = normalizeInlineWhitespace(inlineContent, isBreak)
        expect(show(normalizeInlineWhitespace(once, isBreak))).toBe(show(once))
      }),
      { numRuns: 500 }
    )
  })

  it("leaves only single spaces after no-break spaces, none at the edges and no U+0000", () => {
    fc.assert(
      fc.property(content, (inlineContent) => {
        const result = show(normalizeInlineWhitespace(inlineContent, isBreak))
        for (const segment of result.replace(/\*/g, "").split("|")) {
          expect(segment).not.toMatch(/[\t\n\r\f\0]| {2}|^ /)
        }
        expect(result.replace(/\*/g, "")).not.toMatch(/ $/)
      }),
      { numRuns: 500 }
    )
  })
})
