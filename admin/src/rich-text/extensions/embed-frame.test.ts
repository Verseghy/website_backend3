import { Editor } from "@tiptap/core"
import type { Node as ProseMirrorNode } from "@tiptap/pm/model"
import { AllSelection, NodeSelection, TextSelection } from "@tiptap/pm/state"
import { CellSelection } from "@tiptap/pm/tables"
import { Transform } from "@tiptap/pm/transform"
import fc from "fast-check"
import { afterAll, describe, expect, it } from "vitest"

import { loadDocument } from "../load-document"
import { CONTENT_V1_POLICY, PolicyError } from "../policy"
import { siteExtensions } from "../site-extensions"
import {
  canPlaceMedia,
  forcedFrameAttributes,
  isMediaPlace,
  loneElementChild,
  requireStoredStyle,
  wrapperIframe,
} from "./embed-frame"
import { SITE_IMAGE_NODE } from "./site-image"
import { policyWith } from "@/test/rich-text-policy"
import { parseHtml, siteSchema } from "@/test/site-schema"

/** An element parsed from HTML. */
function element(html: string): Element {
  const container = document.createElement("div")
  container.innerHTML = html
  return container.firstElementChild!
}

describe("forcedFrameAttributes", () => {
  it("reads the three forced iframe values of policy.json (editor.md §3.2)", () => {
    expect(forcedFrameAttributes(CONTENT_V1_POLICY)).toEqual({
      allow:
        "accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share",
      allowfullscreen: "",
      referrerpolicy: "strict-origin-when-cross-origin",
    })
  })

  it("refuses a policy where one is not forced", () => {
    const policy = policyWith((json) => {
      const elements = json.elements as {
        iframe: { attributes: { name: string; value: unknown }[] }
      }
      const allow = elements.iframe.attributes.find((a) => a.name === "allow")!
      allow.value = { kind: "text" }
    })
    expect(() => forcedFrameAttributes(policy)).toThrow(PolicyError)
  })
})

describe("requireStoredStyle", () => {
  it("returns a style the server stores as it is", () => {
    expect(
      requireStoredStyle(
        CONTENT_V1_POLICY,
        "iframe",
        "height: 608px; width: 100%"
      )
    ).toBe("height: 608px; width: 100%")
  })

  it.each([
    ["iframe", "width: 100%; height: 608px"],
    ["iframe", "min-height: 199px"],
    ["img", "width: 350px"],
  ])("refuses %s { %s }, which the server rewrites", (tag, style) => {
    expect(() => requireStoredStyle(CONTENT_V1_POLICY, tag, style)).toThrow(
      PolicyError
    )
  })
})

describe("loneElementChild", () => {
  it.each([
    ["<p><img src=x></p>", "img"],
    ["<p>\n  <iframe></iframe>\n</p>", "iframe"],
    ["<p><a><img src=x></a></p>", "a"],
  ])("finds the one element of %j", (html, tag) => {
    expect(loneElementChild(element(html))?.localName).toBe(tag)
  })

  it.each([
    "<p></p>",
    "<p>szöveg<img src=x></p>",
    "<p><img src=x><img src=y></p>",
    "<p><img src=x><!-- c --></p>",
    "<p> <img src=x></p>",
  ])("finds nothing in %j", (html) => {
    expect(loneElementChild(element(html))).toBeNull()
  })
})

describe("wrapperIframe", () => {
  it("finds the wrapper's first iframe child", () => {
    expect(
      wrapperIframe(
        element('<div data-youtube-video="">x<iframe src="a"></iframe></div>')
      )?.getAttribute("src")
    ).toBe("a")
    expect(wrapperIframe(element("<p><span></span></p>"))).toBeNull()
  })
})

const TABLE =
  '<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td>cella</td><td>másik</td></tr></tbody></table>'

/** One block of each kind that can hold a cursor, top level and nested. */
const BODY = `<p>bekezdés</p><h2>címsor</h2><ul><li>pont</li></ul><blockquote>idézet</blockquote>${TABLE}<p>vége</p>`

/**
 * The position after the first character of `text` in a document.
 *
 * @param doc - the document.
 * @param text - text that occurs in it.
 */
function inside(doc: ProseMirrorNode, text: string): number {
  let found = -1
  doc.descendants((node, pos) => {
    if (found < 0 && node.isText && node.text!.includes(text)) {
      found = pos + node.text!.indexOf(text) + 1
    }
  })
  if (found < 0) {
    throw new Error(`no text "${text}" in the document`)
  }
  return found
}

/** The position of the first node of a type in a document. */
function nodeAt(doc: ProseMirrorNode, type: string): number {
  let found = -1
  doc.descendants((node, pos) => {
    if (found < 0 && node.type.name === type) {
      found = pos
    }
  })
  return found
}

describe("isMediaPlace and canPlaceMedia (editor.md §4.2)", () => {
  const schema = siteSchema()
  const doc = parseHtml(schema, BODY)

  it.each([
    ["the start of the document", () => 0, true],
    ["between two top-level blocks", () => doc.child(0).nodeSize, true],
    ["the end of the document", () => doc.content.size, true],
    ["a top-level paragraph", () => inside(doc, "bekezdés"), true],
    ["a heading", () => inside(doc, "címsor"), false],
    ["a list item", () => inside(doc, "pont"), false],
    ["a quote", () => inside(doc, "idézet"), false],
    ["a table cell", () => inside(doc, "cella"), false],
    ["a list, between its items", () => nodeAt(doc, "bulletList") + 1, false],
    ["a table, between its rows", () => nodeAt(doc, "table") + 1, false],
  ])("judges a position in %s", (_name, position, expected) => {
    expect(isMediaPlace(doc.resolve(position()))).toBe(expected)
  })

  it.each([
    [
      "a cursor in a top-level paragraph",
      () => TextSelection.create(doc, inside(doc, "bekezdés")),
      true,
    ],
    [
      "a range between two top-level paragraphs",
      () =>
        TextSelection.create(doc, inside(doc, "bekezdés"), inside(doc, "vége")),
      true,
    ],
    [
      "a range that ends in a heading",
      () =>
        TextSelection.create(
          doc,
          inside(doc, "bekezdés"),
          inside(doc, "címsor")
        ),
      false,
    ],
    [
      "a range that starts in a list item",
      () => TextSelection.create(doc, inside(doc, "pont"), inside(doc, "vége")),
      false,
    ],
    [
      "a selected top-level table",
      () => NodeSelection.create(doc, nodeAt(doc, "table")),
      true,
    ],
    [
      "a selected list item",
      () => NodeSelection.create(doc, nodeAt(doc, "listItem")),
      false,
    ],
    ["the whole document", () => new AllSelection(doc), true],
    [
      "a selected table cell",
      () => CellSelection.create(doc, nodeAt(doc, "tableCell")),
      false,
    ],
  ])("judges %s", (_name, selection, expected) => {
    expect(canPlaceMedia(selection())).toBe(expected)
  })

  /** Blocks whose positions the property below visits. */
  const BLOCKS = [
    "<p>ab</p>",
    "<p></p>",
    "<h2>cd</h2>",
    "<ul><li>ef</li><li><p>gh</p><ul><li>ij</li></ul></li></ul>",
    '<ol start="3"><li>kl</li></ol>',
    "<blockquote><p>mn</p><p>op</p></blockquote>",
    TABLE,
    "<hr>",
    '<p><img src="/storage/uploads/a.jpg" alt="a" width="350" height="350" style="display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px"></p>',
  ]
  const image = schema.nodes[SITE_IMAGE_NODE].create({
    src: "/storage/uploads/b.jpg",
    alt: "b",
    displayWidth: 350,
    ratio: 1,
  })
  /** The top-level blocks that are neither paragraphs nor images. */
  const containers = (node: ProseMirrorNode) =>
    node.children
      .filter(
        (child) =>
          child.type.name !== "paragraph" && child.type.name !== SITE_IMAGE_NODE
      )
      .map((child) => child.toJSON())

  it("allows exactly the places where an image cuts no list, quote, table or heading", () => {
    fc.assert(
      fc.property(
        fc.array(fc.constantFrom(...BLOCKS), { minLength: 1, maxLength: 6 }),
        (blocks) => {
          const body = parseHtml(schema, blocks.join(""))
          for (let position = 0; position <= body.content.size; position += 1) {
            const $position = body.resolve(position)
            const placed = isMediaPlace($position)
            // ProseMirror fits the image in wherever it is put, cutting
            // whatever containers stand in its way.
            const inserted = new Transform(body).replaceRangeWith(
              position,
              position,
              image
            ).doc
            if (placed) {
              expect(containers(inserted), `${position}`).toEqual(
                containers(body)
              )
            }
            const inText =
              $position.parent.isTextblock &&
              $position.parentOffset > 0 &&
              $position.parentOffset < $position.parent.content.size
            const topParagraph =
              $position.depth === 1 &&
              $position.parent.type.name === "paragraph"
            if (inText && !topParagraph) {
              expect(placed, `${position}`).toBe(false)
              expect(containers(inserted), `${position}`).not.toEqual(
                containers(body)
              )
            }
          }
        }
      )
    )
  })
})

describe("the media insert commands (editor.md §4.2)", () => {
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
  })
  afterAll(() => editor.destroy())

  type Commands = Pick<
    Editor["commands"],
    "insertSiteImage" | "insertYoutubeEmbed" | "insertPadletEmbed"
  >
  const INSERTS: [string, (commands: Commands) => boolean][] = [
    [
      "insertSiteImage",
      (commands) =>
        commands.insertSiteImage({
          src: "/storage/uploads/a.jpg",
          alt: "a",
          width: 350,
          height: 350,
        }),
    ],
    [
      "insertYoutubeEmbed",
      (commands) =>
        commands.insertYoutubeEmbed({
          videoId: "dQw4w9WgXcQ",
          start: null,
          list: null,
          title: "Videó",
        }),
    ],
    [
      "insertPadletEmbed",
      (commands) =>
        commands.insertPadletEmbed({
          key: "KEY",
          height: 608,
          title: "Padlet",
        }),
    ],
  ]

  describe.each(INSERTS)("%s", (_name, insert) => {
    it.each([
      ["a heading", "címsor"],
      ["a list item", "pont"],
      ["a quote", "idézet"],
      ["a table cell", "cella"],
    ])("is refused in %s, and editor.can() says so", (_place, text) => {
      loadDocument(editor, BODY)
      editor.commands.setTextSelection(inside(editor.state.doc, text))
      const before = editor.state.doc
      expect(insert(editor.can())).toBe(false)
      expect(insert(editor.commands)).toBe(false)
      expect(editor.state.doc.eq(before)).toBe(true)
    })

    it("is refused on a cell selection", () => {
      loadDocument(editor, BODY)
      const { doc } = editor.state
      editor.view.dispatch(
        editor.state.tr.setSelection(
          CellSelection.create(doc, nodeAt(doc, "tableCell"))
        )
      )
      expect(insert(editor.can())).toBe(false)
      expect(insert(editor.commands)).toBe(false)
      expect(editor.state.doc.eq(doc)).toBe(true)
    })

    it("splits a top-level paragraph and leaves every container whole", () => {
      loadDocument(editor, BODY)
      const before = editor.state.doc
      editor.commands.setTextSelection(inside(before, "bekezdés"))
      expect(insert(editor.can())).toBe(true)
      expect(insert(editor.commands)).toBe(true)
      const after = editor.state.doc
      expect(after.childCount).toBe(before.childCount + 2)
      // b | media | ekezdés, then the heading, list, quote and table as before.
      expect(after.child(0).textContent).toBe("b")
      expect(after.child(2).textContent).toBe("ekezdés")
      for (let index = 1; index < before.childCount; index += 1) {
        expect(after.child(index + 2).eq(before.child(index))).toBe(true)
      }
    })
  })
})
