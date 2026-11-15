import { DOMSerializer } from "@tiptap/pm/model"
import { afterAll, describe, expect, it, vi } from "vitest"

import { CONTENT_V1_POLICY } from "../policy"
import prosemirrorModelPackage from "../../../node_modules/prosemirror-model/package.json"
import prosemirrorModelSource from "../../../node_modules/prosemirror-model/dist/index.js?raw"
import { inertDocument, toStoredHtml, verbatimDocument } from "./to-stored-html"
import { writeHtml } from "./write-html"
import {
  regressionFixtures,
  reparse,
  storedHtmlHarness,
} from "@/test/stored-html"
import { CANONICAL_BODIES, TABLE_SHAPES } from "@/test/editor-fixtures"
import { pasteEvent, siteSchema } from "@/test/site-schema"

/**
 * editor.md §13.4, L2: the editor's fixed points over the committed
 * fixtures, under jsdom, and the guard of §6 step 1.
 */
const harness = storedHtmlHarness()
afterAll(() => harness.destroy())
const { J, load, save } = harness
const FIXTURES = regressionFixtures()

/** The nodes the site schema has no node for yet (editor.md §4.3 rows 8-10). */
const PENDING = /<(?:img|iframe)\b/

/**
 * The top-level blocks of the canonical shapes of editor.md §3.7 that the
 * schema holds today, each as the server stores it.
 */
const SHAPES = reparse(
  FIXTURES.find(({ name }) => name === "canonical-shapes")?.output ?? ""
)
  .map((node) => writeHtml([node]))
  .filter((block) => !PENDING.test(block))

/**
 * Canonical bodies beyond the shapes, what the editor writes, and the
 * canonical tables (`src/test/editor-fixtures.ts`, shared with the golden
 * files of tests/fixtures/e1).
 */
const CANONICAL = [
  ...Object.values(CANONICAL_BODIES),
  ...Object.values(TABLE_SHAPES),
]

describe("toStoredHtml(load(z)) == z (P-fix)", () => {
  it("has the schema's shapes of editor.md §3.7 to check", () => {
    expect(SHAPES.length).toBeGreaterThanOrEqual(7)
    expect(SHAPES.some((shape) => shape.startsWith("<table"))).toBe(true)
  })

  it.each([...SHAPES, ...CANONICAL])("keeps %s", (body) => {
    expect(J(body)).toBe(body)
  })

  it("keeps all the shapes in one body", () => {
    const body = [...SHAPES, ...CANONICAL].join("")
    expect(J(body)).toBe(body)
  })
})

describe("J(J(x)) == J(x) over the regression fixtures", () => {
  it.each(FIXTURES.map((fixture) => [fixture.name, fixture]))(
    "%s",
    (_name, { input, output }) => {
      for (const body of [input, output]) {
        const once = J(body)
        expect(J(once)).toBe(once)
        expect(once).not.toMatch(/rgb\(|border-width:/)
      }
    }
  )
})

describe("toStoredHtml", () => {
  it("writes what the schema renders in the stored form", () => {
    expect(
      J(
        '<ul><li><p>pont</p></li></ul><p><a rel="x" href="/y" target="_self">l</a></p><p><span style="font-size: 1.5em; color: rgb(192, 0, 0)">a</span></p><blockquote><p>  idézet  </p></blockquote>'
      )
    ).toBe(
      '<ul><li>pont</li></ul><p><a href="/y" target="_blank" rel="noopener noreferrer">l</a></p><p><span style="color: #c00000; font-size: 1.5em">a</span></p><blockquote>idézet</blockquote>'
    )
  })

  it("keeps typed spaces as the whitespace rule writes them", () => {
    const schema = siteSchema()
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, schema.text(" a  b\tc ")),
    ])
    const stored = save(doc)
    expect(stored).toBe("<p>a&nbsp; b c</p>")
    expect(J(stored)).toBe(stored)
  })

  it("drops U+0000 from text pasted as plain text, as every HTML parse does", () => {
    // prosemirror-view's plain-text paste builds text nodes without an HTML
    // parse, so the document holds U+0000; the server and the next load
    // both drop it, so the stored text must not hold it either.
    const { editor } = harness
    load("<p>x</p>")
    editor.commands.focus("end")
    editor.view.pasteText("a\u0000b", pasteEvent({ "text/plain": "a\u0000b" }))
    expect(editor.state.doc.textContent).toBe("xa\u0000b")
    const stored = save(editor.state.doc)
    expect(stored).toBe("<p>xab</p>")
    expect(J(stored)).toBe(stored)
  })

  it("drops U+0000 before tightening, so a list item keeps its nested list", () => {
    // Dropped only by the writer, the paragraph would reach tightening with
    // content and be stored as `<li><ul>`, which reloads with the nested
    // list moved out of the item.
    const schema = siteSchema()
    const { nodes } = schema
    const paragraph = (value: string) =>
      nodes.paragraph.create(null, schema.text(value))
    const doc = nodes.doc.create(null, [
      nodes.bulletList.create(null, [
        nodes.listItem.create(null, [
          paragraph("\u0000"),
          nodes.bulletList.create(null, [
            nodes.listItem.create(null, [paragraph("b")]),
          ]),
        ]),
      ]),
    ])
    const stored = save(doc)
    expect(stored).toBe("<ul><li><p></p><ul><li>b</li></ul></li></ul>")
    expect(J(stored)).toBe(stored)
  })

  it("never writes the CSSOM's spelling, which the plain DOM path does", () => {
    const body =
      '<p style="text-align: center"><span style="background-color: #ffff0080; color: #c00000">a</span></p>'
    const doc = load(body)
    // The trap of editor.md §6 step 1: through the CSSOM, jsdom re-spells
    // the colours as rgb()...
    const container = document.createElement("div")
    container.append(
      DOMSerializer.fromSchema(doc.type.schema).serializeFragment(doc.content, {
        document,
      })
    )
    expect(container.innerHTML).toContain("rgb(")
    // ...which the stored HTML never holds.
    expect(save(doc)).toBe(body)
  })

  it.each<[string, () => unknown, string]>([
    [
      "a document that is not a ProseMirror node",
      () =>
        toStoredHtml(null as never, { policy: CONTENT_V1_POLICY, document }),
      "toStoredHtml: doc must be the editor's document",
    ],
    [
      "a node that is not the top node",
      () =>
        toStoredHtml(load("<p>a</p>").firstChild as never, {
          policy: CONTENT_V1_POLICY,
          document,
        }),
      "toStoredHtml: doc must be the editor's document",
    ],
    [
      "no policy",
      () => toStoredHtml(load("<p>a</p>"), { document } as never),
      "toStoredHtml: options.policy is required",
    ],
    [
      "no options",
      () => toStoredHtml(load("<p>a</p>"), undefined as never),
      "toStoredHtml: options.policy is required",
    ],
    [
      "no DOM document",
      () =>
        toStoredHtml(load("<p>a</p>"), {
          policy: CONTENT_V1_POLICY,
          document: {} as never,
        }),
      "toStoredHtml: options.document must be a DOM document",
    ],
    [
      "a document that cannot make an inert document",
      () =>
        toStoredHtml(load("<p>a</p>"), {
          policy: CONTENT_V1_POLICY,
          document: {
            createElement: () => document.createElement("p"),
          } as never,
        }),
      "toStoredHtml: options.document must be a DOM document",
    ],
  ])("refuses %s", (_name, call, message) => {
    expect(call).toThrow(TypeError)
    expect(call).toThrow(message)
  })
})

describe("the guard on prosemirror-model's style branch (editor.md §6 step 1)", () => {
  it("runs on the pinned prosemirror-model", () => {
    expect(prosemirrorModelPackage.version).toBe("1.25.12")
  })

  it("finds renderSpec's style branch unchanged in the installed code", () => {
    expect(prosemirrorModelSource).toMatch(
      /else if \(name == "style" && dom\.style\)\s+dom\.style\.cssText = attrs\[name\];\s+else\s+dom\.setAttribute\(name, attrs\[name\]\);/
    )
  })

  it("assigns cssText to an element with a style property, and setAttribute without one", () => {
    const schema = siteSchema()
    const paragraph = schema.nodes.paragraph.create(
      { textAlign: "center" },
      schema.text("a")
    )
    const assigned: string[] = []
    const withStyle = {
      createElement: (tag: string) => {
        const created = document.createElement(tag)
        Object.defineProperty(created, "style", {
          value: {
            set cssText(value: string) {
              assigned.push(value)
            },
          },
        })
        return created
      },
      createTextNode: (data: string) => document.createTextNode(data),
      createDocumentFragment: () => document.createDocumentFragment(),
    } as unknown as Document
    const serializer = DOMSerializer.fromSchema(schema)
    const throughCssom = serializer.serializeNode(paragraph, {
      document: withStyle,
    }) as Element
    expect(assigned).toEqual(["text-align: center"])
    expect(throughCssom.hasAttribute("style")).toBe(false)

    const verbatim = serializer.serializeNode(paragraph, {
      document: verbatimDocument(document),
    }) as Element
    expect(verbatim.getAttribute("style")).toBe("text-align: center")
  })
})

describe("inertDocument", () => {
  it("makes a document without a browsing context, once per document", () => {
    const inert = inertDocument(document)
    expect(inert).not.toBe(document)
    // No browsing context: nothing created in it loads a resource.
    expect(inert.defaultView).toBeNull()
    expect(inertDocument(document)).toBe(inert)
    const other = document.implementation.createHTMLDocument("")
    expect(inertDocument(other)).not.toBe(inert)
    expect(inertDocument(other)).toBe(inertDocument(other))
  })
})

describe("the document toStoredHtml renders in (editor.md §6 step 1)", () => {
  const body =
    '<p><strong>a</strong> <a href="/x" target="_blank" rel="noopener noreferrer">b</a></p><ul><li>c</li></ul>'

  it("creates nothing in the document it is given, only in its inert one", () => {
    // In a browser an `img` of the page's document fetches as soon as its
    // `src` is set, so a save must never create elements there.
    const doc = load(body)
    const live = vi.spyOn(document, "createElement")
    const liveNS = vi.spyOn(document, "createElementNS")
    const inert = vi.spyOn(inertDocument(document), "createElement")
    expect(save(doc)).toBe(body)
    expect(live).not.toHaveBeenCalled()
    expect(liveNS).not.toHaveBeenCalled()
    expect(inert).toHaveBeenCalled()
  })

  it("writes the same bytes when given an inert document", () => {
    const doc = load(body)
    expect(
      toStoredHtml(doc, {
        policy: CONTENT_V1_POLICY,
        document: document.implementation.createHTMLDocument(""),
      })
    ).toBe(save(doc))
  })
})

describe("verbatimDocument", () => {
  it("creates elements without a style property, whose style is written as given", () => {
    const stand = verbatimDocument(document)
    const created = stand.createElement("span")
    expect(created.style).toBeUndefined()
    created.setAttribute("style", "COLOR: RED")
    expect(created.getAttribute("style")).toBe("COLOR: RED")
    const namespaced = stand.createElementNS(
      "http://www.w3.org/1999/xhtml",
      "p"
    ) as HTMLElement
    expect(namespaced.style).toBeUndefined()
    expect(stand.createTextNode("a").data).toBe("a")
    expect(stand.createDocumentFragment().childNodes).toHaveLength(0)
  })
})
