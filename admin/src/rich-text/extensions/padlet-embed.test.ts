import { Editor } from "@tiptap/core"
import { describe, expect, it } from "vitest"

import { loadDocument } from "../load-document"
import { CONTENT_V1_POLICY, PolicyError } from "../policy"
import { toStoredHtml } from "../serialize/to-stored-html"
import { siteExtensions } from "../site-extensions"
import type { PadletEmbedAttributes } from "./padlet-embed"
import { createPadletEmbed, PADLET_EMBED_NODE } from "./padlet-embed"
import { policyWith } from "@/test/rich-text-policy"
import { parseHtml, siteSchema } from "@/test/site-schema"
import { storedHtmlHarness } from "@/test/stored-html"

const FORCED =
  'allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"'

/** The stored Padlet shape of editor.md §3.7. */
function stored(key: string, height = 608, title: string | null = "Padlet") {
  const titled = title === null ? "" : ` title="${title}"`
  return `<p data-padlet-embed=""><iframe src="https://padlet.com/embed/${key}" width="100%" height="${height}"${titled} style="height: ${height}px; width: 100%" ${FORCED}></iframe></p>`
}

describe("the Padlet embed's stored shape (editor.md §3.7)", () => {
  const harness = storedHtmlHarness()

  it.each([
    stored("KEY"),
    stored("bbcgkx6oq8valf3l", 400, "A tábla"),
    stored("k", 2000, null),
    stored("k", 100),
  ])("is a fixed point: %s", (body) => {
    expect(harness.J(body)).toBe(body)
  })
})

describe("the Padlet parse rules", () => {
  const schema = siteSchema()
  const attributesOf = (html: string) => {
    const found: unknown[] = []
    parseHtml(schema, html).forEach((node) => {
      if (node.type.name === PADLET_EMBED_NODE) {
        found.push(node.attrs)
      }
    })
    return found
  }
  const board = (
    key: string,
    rest: Partial<PadletEmbedAttributes> = {}
  ): PadletEmbedAttributes => ({ key, height: 608, title: null, ...rest })

  it.each<[string, string, PadletEmbedAttributes]>([
    [
      "the stored wrapper (70)",
      '<p data-padlet-embed=""><iframe src="https://padlet.com/embed/KEY" height="500" style="height: 400px" title="T"></iframe></p>',
      board("KEY", { height: 400, title: "T" }),
    ],
    [
      "a legacy paragraph with one Padlet iframe (65)",
      '<p><iframe allowfullscreen="" src="https://padlet.com/embed/bbcgkx6oq8valf3l" style="border: 0; width: 100%; height: 608px;" tabindex="-1"></iframe></p>',
      board("bbcgkx6oq8valf3l"),
    ],
    [
      "a lone iframe with a height attribute only (60)",
      '<iframe src="https://padlet.com/embed/KEY" height="700"></iframe>',
      board("KEY", { height: 700 }),
    ],
    [
      "a height outside the iframe's range, which falls back to 608",
      '<iframe src="https://padlet.com/embed/KEY" height="3000" style="height: 50px"></iframe>',
      board("KEY"),
    ],
    [
      "a fractional style height, which the attribute replaces",
      '<iframe src="https://padlet.com/embed/KEY" height="500" style="height: 400.5px"></iframe>',
      board("KEY", { height: 500 }),
    ],
  ])("read %s", (_name, html, expected) => {
    expect(attributesOf(html)).toEqual([expected])
  })

  it.each([
    '<iframe src="https://padlet.com/embed/KEY?x=1"></iframe>',
    '<iframe src="http://padlet.com/embed/KEY"></iframe>',
    '<iframe src="https://padlet.com/someone/KEY"></iframe>',
    '<p data-padlet-embed=""></p>',
  ])("make no Padlet embed of %s", (html) => {
    expect(attributesOf(html)).toEqual([])
  })
})

describe("the Padlet render rule", () => {
  const schema = siteSchema()

  it("writes no address and the default height for invalid attributes", () => {
    const node = schema.nodes[PADLET_EMBED_NODE].create({
      key: "a b",
      height: 5,
      title: "Padlet",
    })
    const html = toStoredHtml(schema.nodes.doc.create(null, [node]), {
      policy: CONTENT_V1_POLICY,
      document,
    })
    expect(html).toBe(
      `<p data-padlet-embed=""><iframe width="100%" height="608" title="Padlet" style="height: 608px; width: 100%" ${FORCED}></iframe></p>`
    )
  })
})

describe("insertPadletEmbed", () => {
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
  })
  const save = () =>
    toStoredHtml(editor.state.doc, { policy: CONTENT_V1_POLICY, document })

  it("inserts a board", () => {
    loadDocument(editor, "<p></p>")
    expect(
      editor.commands.insertPadletEmbed({
        key: "KEY",
        height: 608,
        title: "Padlet",
      })
    ).toBe(true)
    expect(save()).toBe(stored("KEY"))
  })

  it.each([
    { key: "", height: 608, title: "Padlet" },
    { key: "a/b", height: 608, title: "Padlet" },
    { key: "KEY", height: 99, title: "Padlet" },
    { key: "KEY", height: 608.5, title: "Padlet" },
  ])("refuses %j", (attributes) => {
    loadDocument(editor, "<p>x</p>")
    expect(editor.commands.insertPadletEmbed(attributes)).toBe(false)
    expect(save()).toBe("<p>x</p>")
  })
})

describe("createPadletEmbed", () => {
  it("refuses a policy without the iframe's height grammar", () => {
    const policy = policyWith((json) => {
      const elements = json.elements as {
        iframe: { style: { property: string }[] }
      }
      elements.iframe.style = elements.iframe.style.filter(
        (rule) => rule.property !== "height"
      )
    })
    expect(() => createPadletEmbed(policy)).toThrow(PolicyError)
  })
})
