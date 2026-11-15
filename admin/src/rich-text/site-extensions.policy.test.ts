import type { Mark, Node as ProseMirrorNode, NodeType } from "@tiptap/pm/model"
import { describe, expect, it } from "vitest"

import { canonicalStyle } from "./css/canonical-style"
import { isAllowedHref } from "./link-href"
import { embedSrc, imageSrc } from "./media-src"
import type { AttributeValueRule } from "./policy"
import { CONTENT_V1_POLICY } from "./policy"
import {
  isCanonicalValue,
  isScalarGrammar,
  readCanonicalDecls,
} from "./style-decls"
import type { RenderedElement } from "@/test/site-schema"
import { siteSchema, specElements } from "@/test/site-schema"

/**
 * The schema's elements and attributes against the committed `policy.json`
 * (editor.md §3.2, §3.3): what the render rules can write is exactly what
 * content-v1 stores, every element and attribute of it, and never a value
 * the server would change or drop.
 */
const POLICY = CONTENT_V1_POLICY
const schema = siteSchema()

const alignments = ["left", "center", "right", "justify"]

/** Values of a cell border side: stored ones, and ones the server refuses. */
const borderSides = [
  null,
  "1px solid #000000",
  "none",
  "hidden",
  "medium dashed",
  "3px double red",
  "0px solid currentcolor",
  "21px solid #000000",
  "1px solid #000000; color: red",
]

/** The attributes of `tableCell` and `tableHeader`, which are the same. */
const CELL_SAMPLES: Readonly<Record<string, readonly unknown[]>> = {
  colspan: [1, 2, 1000, 1001, 0, "3", 2.5],
  rowspan: [1, 2, 65534, 65535, -1],
  colwidth: [null, [100], [100, 200]],
  backgroundColor: [
    null,
    "#d9d9d9",
    "#ffff0080",
    "transparent",
    "currentcolor",
    "red",
    "rgb(1, 2, 3)",
    "var(--x)",
  ],
  borderTop: borderSides,
  borderRight: borderSides,
  borderBottom: borderSides,
  borderLeft: borderSides,
  height: [null, "20px", "1px", "2000px", "2001px", "10%", "15pt"],
  width: [null, "128px", "128.25px", "50%", "0px", "128"],
  textAlign: [null, ...alignments, "start", "CENTER"],
  verticalAlign: [null, "top", "middle", "bottom", "baseline", "sub"],
  whiteSpace: [null, "nowrap", "normal", "pre"],
}

/**
 * The types whose attributes are sampled one at a time rather than in every
 * combination: a cell has thirteen attributes, and their product would be
 * millions of samples; an image has six, which would still be thousands. Each value is rendered alone, and once with every
 * attribute at its first stored value.
 */
const ONE_AT_A_TIME: ReadonlySet<string> = new Set([
  "tableCell",
  "tableHeader",
  "siteImage",
])

/**
 * Values for every attribute of every node and mark type: each value the
 * policy stores, plus defaults and values it refuses, which must render
 * nothing outside the policy.
 */
const SAMPLES: Readonly<
  Record<string, Readonly<Record<string, readonly unknown[]>>>
> = {
  paragraph: {
    textAlign: [null, ...alignments, "start", "CENTER", "center;color:red"],
  },
  heading: {
    level: [1, 2, 3, 4, 5],
    textAlign: [null, ...alignments, "start"],
  },
  orderedList: {
    start: [1, 0, 5, 9999, 10_000, -1, 2.5, "5"],
    type: [null, "a", "A", "i", "I", "1", "disc", "x"],
  },
  bulletList: {
    listStyleType: [null, "circle", "square", "disc", "decimal", "CIRCLE"],
  },
  blockquote: {},
  listItem: {},
  hardBreak: {},
  horizontalRule: {},
  link: {
    href: [
      null,
      "",
      "https://example.org/",
      "http://example.org",
      "mailto:a@b.example",
      "tel:+3612345",
      "/hirek/12",
      "#top",
      "javascript:alert(1)",
      "//evil.example/",
      "/storage/x.svg",
    ],
  },
  textStyle: {
    color: [
      null,
      "#c00000",
      "#c0000080",
      "transparent",
      "red",
      "var(--mt-accent)",
    ],
    backgroundColor: [
      null,
      "#ffff00",
      "#000000b3",
      "transparent",
      "currentcolor",
      "yellow",
    ],
    fontSize: [null, "0.5em", "1.25em", "3em", "3.5em", "12pt", "1.50em"],
  },
  bold: {},
  italic: {},
  underline: {},
  strike: {},
  subscript: {},
  superscript: {},
  table: {},
  tableRow: {},
  tableCell: CELL_SAMPLES,
  tableHeader: CELL_SAMPLES,
  siteImage: {
    src: [
      null,
      "/storage/uploads/posts/1/a.jpg",
      "https://verseghy-gimnazium.net/storage/uploads/b.PNG",
      "/storage/uploads/a.svg",
      "javascript:alert(1)",
      "blob:x",
    ],
    alt: [null, "", "  ", "kép"],
    displayWidth: [null, 175, 350.5, 700, 2000, 2001, 0, -1, "350"],
    ratio: [null, 1.4142857142857144, 0.5, 1e6, Number.NaN],
    uploadState: [null, "pending", "failed"],
    localKey: [null, "k1"],
  },
  youtubeEmbed: {
    videoId: [null, "dQw4w9WgXcQ", "videoseries", "abc", "AbCdEfGhIjKl"],
    start: [null, 0, 90, -1, 1.5],
    list: [null, "PLMOZDTW7ZvXM", "a.b"],
    title: [null, "Videó", "YouTube video player"],
  },
  padletEmbed: {
    key: [null, "KEY", "a b"],
    height: [608, 100, 2000, 99, 2001, "608", null],
    title: [null, "Padlet"],
  },
}

/** The node and mark types that render elements. */
const nodeTypes = Object.values(schema.nodes).filter(
  (type) => type !== schema.topNodeType && !type.isText
)
const markTypes = Object.values(schema.marks)

/**
 * Every sample of one type: its defaults, then every combination of its
 * attributes' sample values (so each heading level is also rendered
 * aligned, and each colour with each size).
 */
function samplesOf(name: string): Record<string, unknown>[] {
  if (ONE_AT_A_TIME.has(name)) {
    const entries = Object.entries(SAMPLES[name] ?? {})
    const alone = entries.flatMap(([attribute, values]) =>
      values.map((value) => ({ [attribute]: value }))
    )
    const all = Object.fromEntries(
      entries.map(([attribute, values]) => [
        attribute,
        values.find((value) => value !== null),
      ])
    )
    return [{}, ...alone, all]
  }
  const combinations = Object.entries(SAMPLES[name] ?? {}).reduce<
    Record<string, unknown>[]
  >(
    (partial, [attribute, values]) =>
      partial.flatMap((attrs) =>
        values.map((value) => ({ ...attrs, [attribute]: value }))
      ),
    [{}]
  )
  return [{}, ...combinations]
}

/** What every node and mark sample renders. */
function renderAll(): RenderedElement[] {
  const into: RenderedElement[] = []
  for (const type of nodeTypes) {
    for (const attrs of samplesOf(type.name)) {
      const node: ProseMirrorNode = (type as NodeType).create(attrs)
      const toDOM = type.spec.toDOM
      if (toDOM) {
        specElements(toDOM(node), type.name, into)
      }
    }
  }
  for (const type of markTypes) {
    for (const attrs of samplesOf(type.name)) {
      const mark: Mark = type.create(attrs)
      const toDOM = type.spec.toDOM
      if (toDOM) {
        specElements(toDOM(mark, true), type.name, into)
      }
    }
  }
  return into
}

const rendered = renderAll()

/** Why a rendered attribute value breaks its rule, or `null` when it does not. */
function valueProblem(
  tag: string,
  rule: AttributeValueRule,
  value: string
): string | null {
  switch (rule.kind) {
    case "style": {
      const declarations = readCanonicalDecls(value, POLICY.css.alphabet)
      if (declarations === null) {
        return "is outside the canonical alphabet"
      }
      for (const [property, declared] of declarations) {
        const styleRule = POLICY.elements[tag].style?.find(
          (entry) => entry.property === property
        )
        if (styleRule?.role !== "stored") {
          return `declares ${property}, which ${tag} does not store`
        }
        // A cell border side is a triple, `none` or `hidden`; the whole
        // style is compared with the server's spelling below.
        if (
          isScalarGrammar(styleRule.grammar) &&
          !isCanonicalValue(styleRule.grammar, declared)
        ) {
          return `declares ${property}: ${declared}, outside its grammar or spelling`
        }
      }
      if (declarations.size === 0) {
        return "is empty"
      }
      // The order is the serialiser's post-pass job (editor.md §6 step
      // 2.1), so only the declarations are compared.
      const stored = canonicalStyle(POLICY, tag, value)
      const sorted = (style: string | null) =>
        JSON.stringify(
          [
            ...(readCanonicalDecls(style ?? "", POLICY.css.alphabet) ?? []),
          ].sort()
        )
      return sorted(stored) === sorted(value)
        ? null
        : `is stored as "${stored}"`
    }
    case "marker":
      return value === "" ? null : "is not empty"
    case "text":
      return null
    case "integer": {
      const number = Number(value)
      return /^[0-9]+$/.test(value) &&
        number >= rule.min &&
        number <= rule.max &&
        number !== rule.omitted
        ? null
        : "is not a stored integer"
    }
    case "enumerated":
      return rule.values.includes(value) && value !== rule.omitted
        ? null
        : "is not a stored value"
    case "forced":
    case "link_rel":
      return value === rule.value ? null : `is not ${rule.value}`
    case "link_href":
      return isAllowedHref(value, POLICY.urls)
        ? null
        : "is a link the server refuses"
    case "image_src":
      return imageSrc(value, POLICY.urls) === value
        ? null
        : "is an image path the server refuses or rewrites"
    case "embed_src":
      return embedSrc(value, POLICY.urls) === value
        ? null
        : "is an embed address the server refuses or rewrites"
    case "integer_or_full": {
      const number = Number(value)
      return value === rule.full ||
        (/^[0-9]+$/.test(value) && number >= rule.min && number <= rule.max)
        ? null
        : "is neither a stored integer nor the full width"
    }
    default: {
      // Every kind is checked; a new one fails `npm run typecheck` here.
      const unchecked: never = rule
      return `has the rule ${JSON.stringify(unchecked)}, which this test does not check yet`
    }
  }
}

describe("the site schema against policy.json", () => {
  it("has samples for every node and mark type and every attribute", () => {
    const missing: string[] = []
    for (const type of [...nodeTypes, ...markTypes]) {
      const samples = SAMPLES[type.name]
      if (samples === undefined) {
        missing.push(type.name)
        continue
      }
      for (const attribute of Object.keys(type.spec.attrs ?? {})) {
        if (!samples[attribute]?.length) {
          missing.push(`${type.name}.${attribute}`)
        }
      }
    }
    expect(missing, "add sample values for these to SAMPLES").toEqual([])
  })

  it("renders exactly the policy's elements", () => {
    const tags = new Set(rendered.map(({ tag }) => tag))
    expect([...tags].sort()).toEqual([...POLICY.tags].sort())
  })

  it.each(POLICY.tags)(
    "renders exactly the policy's attributes on %s",
    (tag) => {
      const names = new Set(
        rendered
          .filter((element) => element.tag === tag)
          .flatMap(({ attributes }) => Object.keys(attributes))
      )
      const expected = POLICY.elements[tag].attributes.map(({ name }) => name)
      expect([...names].sort()).toEqual([...expected].sort())
    }
  )

  it.each(POLICY.tags)(
    "can store every CSS property the policy stores on %s",
    (tag) => {
      const declared = new Set(
        rendered
          .filter((element) => element.tag === tag && element.attributes.style)
          .flatMap(({ attributes }) => [
            ...(readCanonicalDecls(
              attributes.style,
              POLICY.css.alphabet
            )?.keys() ?? []),
          ])
      )
      const stored = (POLICY.elements[tag].style ?? [])
        .filter(({ role }) => role === "stored")
        .map(({ property }) => property)
      expect([...declared].sort()).toEqual([...stored].sort())
    }
  )

  it("renders no attribute value the server would change or drop", () => {
    const problems: string[] = []
    for (const { tag, attributes, writtenBy } of rendered) {
      for (const [name, value] of Object.entries(attributes)) {
        const rule = POLICY.elements[tag]?.attributes.find(
          (attribute) => attribute.name === name
        )?.value
        const problem =
          rule === undefined
            ? "is not a content-v1 attribute"
            : valueProblem(tag, rule, value)
        if (problem !== null) {
          problems.push(`${writtenBy}: ${tag}[${name}="${value}"] ${problem}`)
        }
      }
    }
    expect(problems).toEqual([])
  })

  it("never renders a DOM node of its own", () => {
    expect(rendered.filter(({ tag }) => tag === "#dom-node")).toEqual([])
  })
})
