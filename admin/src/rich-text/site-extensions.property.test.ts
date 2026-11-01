import type { Mark, Node as ProseMirrorNode } from "@tiptap/pm/model"
import { DOMParser } from "@tiptap/pm/model"
import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { isAllowedHref } from "./link-href"
import { CONTENT_V1_POLICY } from "./policy"
import {
  isCanonicalValue,
  isScalarGrammar,
  readCanonicalDecls,
} from "./style-decls"
import type { RenderedElement } from "@/test/site-schema"
import {
  parseHtml,
  renderedElements,
  renderHtml,
  siteSchema,
} from "@/test/site-schema"

/**
 * Properties of the site schema over random input (editor.md §13.3, L1
 * fast-check): every document the schema can hold survives being rendered
 * and parsed again, and whatever HTML it is given, one parse already reaches
 * what it keeps and holds only content-v1. Text has no spaces at block or
 * mark edges here: whitespace is the stored-HTML serialiser's rule (§3.8)
 * and its own property tests.
 */
const schema = siteSchema()
const parser = DOMParser.fromSchema(schema)
const POLICY = CONTENT_V1_POLICY
const { nodes, marks } = schema

/** Parses a rendered container again. */
const reparse = (doc: ProseMirrorNode) => parser.parse(renderHtml(schema, doc))

/** Problems of rendered elements against the policy, as messages. */
function policyProblems(elements: readonly RenderedElement[]): string[] {
  const problems: string[] = []
  for (const { tag, attributes } of elements) {
    const element = POLICY.elements[tag]
    if (element === undefined) {
      problems.push(`${tag} is not a content-v1 element`)
      continue
    }
    for (const [name, value] of Object.entries(attributes)) {
      const rule = element.attributes.find(
        (attribute) => attribute.name === name
      )?.value
      if (rule === undefined) {
        problems.push(`${tag}[${name}] is not a content-v1 attribute`)
      } else if (rule.kind === "style") {
        const declarations = readCanonicalDecls(value, POLICY.css.alphabet)
        for (const [property, declared] of declarations ?? [["", value]]) {
          const styleRule = element.style?.find(
            (entry) => entry.property === property
          )
          if (
            styleRule?.role !== "stored" ||
            !isScalarGrammar(styleRule.grammar) ||
            !isCanonicalValue(styleRule.grammar, declared)
          ) {
            problems.push(`${tag}[style] holds "${property}: ${declared}"`)
          }
        }
      } else if (
        rule.kind === "link_href" &&
        !isAllowedHref(value, POLICY.urls)
      ) {
        problems.push(`${tag}[href="${value}"] is refused by the server`)
      } else if (
        (rule.kind === "forced" || rule.kind === "link_rel") &&
        value !== rule.value
      ) {
        problems.push(`${tag}[${name}="${value}"] is not ${rule.value}`)
      } else if (
        rule.kind === "enumerated" &&
        (!rule.values.includes(value) || value === rule.omitted)
      ) {
        problems.push(`${tag}[${name}="${value}"] is not a stored value`)
      } else if (
        rule.kind === "integer" &&
        !(
          /^[0-9]+$/.test(value) &&
          +value >= rule.min &&
          +value <= rule.max &&
          +value !== rule.omitted
        )
      ) {
        problems.push(`${tag}[${name}="${value}"] is not a stored integer`)
      }
    }
  }
  return problems
}

// Random documents of the schema.

const word = fc.stringMatching(/^[a-zA-Z0-9áéíóöőúüűÁÉÍÓÖŐÚÜŰ.,!?-]{1,8}$/)
const text = fc
  .array(word, { minLength: 1, maxLength: 4 })
  .map((words) => words.join(" "))

const alignment = fc.constantFrom(null, "left", "center", "right", "justify")
const colour = fc.constantFrom(null, "#c00000", "#0000ff", "#000000")
const highlight = fc.constantFrom(
  null,
  "#ffff00",
  "#000000b3",
  "transparent",
  "currentcolor"
)
const size = fc.constantFrom(
  null,
  "0.5em",
  "0.85em",
  "1.25em",
  "1.5em",
  "2em",
  "3em"
)
const href = fc.constantFrom(
  "https://example.org/",
  "http://example.org/a?b=1#c",
  "mailto:titkarsag@example.org",
  "tel:+3612345678",
  "/hirek/12",
  "#top",
  "../x"
)

/** One random mark set, built with `addToSet` so exclusions apply. */
const markSet: fc.Arbitrary<readonly Mark[]> = fc
  .record({
    bold: fc.boolean(),
    italic: fc.boolean(),
    underline: fc.boolean(),
    strike: fc.boolean(),
    script: fc.constantFrom(null, "subscript", "superscript"),
    // A link without href, or with an empty one, is kept by the server, so
    // the schema must keep it too.
    link: fc.option(fc.oneof(href, fc.constantFrom("", null)), {
      nil: undefined,
    }),
    style: fc
      .record({ color: colour, backgroundColor: highlight, fontSize: size })
      .filter((attrs) => Object.values(attrs).some((value) => value !== null)),
    styled: fc.boolean(),
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
    if (choice.styled) {
      chosen.push(marks.textStyle.create(choice.style))
    }
    return chosen.reduce<readonly Mark[]>((set, mark) => mark.addToSet(set), [])
  })

/** Inline content: marked text runs and unmarked hard breaks. */
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
  { maxLength: 5 }
)

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
          fc.constantFrom(1, 0, 2, 5, 9999),
          fc.constantFrom(null, "a", "A", "i", "I"),
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
  .array(block, { minLength: 1, maxLength: 5 })
  .map((content) => nodes.doc.createChecked(null, content))

describe("random documents of the site schema", () => {
  // Rendering to jsdom and parsing it again costs about 30 ms a document,
  // so this property runs fewer, and has a longer timeout than the default.
  it("survive being rendered and parsed again", () => {
    fc.assert(
      fc.property(documents, (doc) => {
        const again = reparse(doc)
        if (!again.eq(doc)) {
          expect(again.toJSON()).toEqual(doc.toJSON())
        }
      }),
      { numRuns: 150 }
    )
  }, 30_000)

  it("render only content-v1", () => {
    fc.assert(
      fc.property(documents, (doc) => {
        expect(policyProblems(renderedElements(doc))).toEqual([])
      }),
      { numRuns: 300 }
    )
  })
})

// Random HTML, policy and otherwise.

const declaration = fc
  .tuple(
    fc.constantFrom(
      "color",
      "background-color",
      "background",
      "font-size",
      "font-family",
      "text-align",
      "list-style-type",
      "vertical-align",
      "font-weight",
      "text-decoration",
      "line-height",
      "COLOR"
    ),
    fc.constantFrom(
      "#c00000",
      "#C00000",
      "#c0000080",
      "red",
      "transparent",
      "currentcolor",
      "rgb(192, 0, 0)",
      "1.5em",
      "0.4em",
      "12pt",
      "center",
      "start",
      "circle",
      "disc",
      "lower-alpha",
      "super",
      "sub",
      "bold",
      "underline",
      "line-through",
      "Calibri",
      "var(--x)",
      "url(x)"
    )
  )
  .map(([property, value]) => `${property}: ${value}`)

const attribute = fc.oneof(
  fc
    .array(declaration, { minLength: 1, maxLength: 4 })
    .map((list) => ["style", list.join("; ")] as const),
  fc
    .constantFrom(
      "https://example.org/",
      "javascript:alert(1)",
      "//evil.example/",
      "/storage/x.svg",
      "mailto:a@b.example",
      "#top",
      "ftp://x.example/",
      ""
    )
    .map((value) => ["href", value] as const),
  fc
    .constantFrom("0", "1", "5", "007", "10000", "-1", "5x")
    .map((value) => ["start", value] as const),
  fc
    .constantFrom("a", "A", "i", "I", "1", "disc")
    .map((value) => ["type", value] as const),
  fc
    .tuple(
      fc.constantFrom(
        "class",
        "id",
        "title",
        "target",
        "rel",
        "align",
        "lang",
        "data-x",
        "name"
      ),
      fc.constantFrom("x", "_self", "nofollow", "center", "hu")
    )
    .map(([name, value]) => [name, value] as const)
)

const tag = fc.constantFrom(
  "p",
  "h1",
  "h2",
  "h4",
  "h5",
  "div",
  "span",
  "strong",
  "b",
  "em",
  "i",
  "u",
  "s",
  "del",
  "sub",
  "sup",
  "a",
  "blockquote",
  "ul",
  "ol",
  "li",
  "code",
  "pre",
  "font",
  "center",
  "section"
)
const voidTag = fc.constantFrom("br", "hr", "img", "wbr")

/** Escapes an attribute value for a double-quoted attribute. */
const quoted = (value: string) =>
  value.replace(/&/g, "&amp;").replace(/"/g, "&quot;")

const { html } = fc.letrec<{ html: string }>((tie) => ({
  html: fc.oneof(
    { depthSize: "small", withCrossShrink: true },
    word,
    voidTag.map((name) => `<${name}>`),
    fc
      .tuple(
        tag,
        fc.uniqueArray(attribute, { selector: ([name]) => name, maxLength: 3 }),
        fc.array(tie("html"), { maxLength: 4 })
      )
      .map(
        ([name, attributes, children]) =>
          `<${name}${attributes.map(([key, value]) => ` ${key}="${quoted(value)}"`).join("")}>${children.join("")}</${name}>`
      )
  ),
}))

const bodies = fc
  .array(html, { minLength: 1, maxLength: 5 })
  .map((parts) => parts.join(""))

describe("random HTML parsed by the site schema", () => {
  it("keeps only content-v1", () => {
    fc.assert(
      fc.property(bodies, (body) => {
        expect(
          policyProblems(renderedElements(parseHtml(schema, body))),
          body
        ).toEqual([])
      }),
      { numRuns: 500 }
    )
  })

  it("reaches what it keeps in one parse", () => {
    fc.assert(
      fc.property(bodies, (body) => {
        const once = parseHtml(schema, body)
        const twice = reparse(once)
        if (!twice.eq(once)) {
          expect(twice.toJSON(), body).toEqual(once.toJSON())
        }
      }),
      { numRuns: 500 }
    )
  })
})
