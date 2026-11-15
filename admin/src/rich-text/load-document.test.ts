import { Editor } from "@tiptap/core"
import { afterEach, describe, expect, it } from "vitest"

import { loadDocument } from "./load-document"
import { CONTENT_V1_POLICY } from "./policy"
import { siteExtensions } from "./site-extensions"
import { renderHtml } from "@/test/site-schema"

describe("loadDocument", () => {
  let editor: Editor | undefined

  afterEach(() => {
    if (editor && !editor.isDestroyed) {
      editor.destroy()
    }
    editor = undefined
  })

  function createEditor(): Editor {
    editor = new Editor({
      element: document.createElement("div"),
      extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
    })
    return editor
  }

  const html = (target: Editor) =>
    renderHtml(target.schema, target.state.doc).innerHTML

  // setContent alone autolinks the last word of the first of several
  // blocks, so these bodies would gain a link on load.
  const urlBodies = [
    "<p>Lásd example.org</p><p>Vége</p>",
    "<p>Lásd https://example.org/</p><p>Vége</p>",
    "<ul><li><p>example.org</p></li><li><p>Vége</p></li></ul>",
  ]

  it.each(urlBodies)("creates no link when it loads %s", (body) => {
    const target = createEditor()
    loadDocument(target, body)
    expect(html(target)).toBe(body)
  })

  it("is needed: a plain setContent links the same bodies", () => {
    const target = createEditor()
    for (const body of urlBodies) {
      target.commands.setContent(body)
      expect(html(target)).toContain("<a ")
    }
  })

  // The server keeps an empty href and an `a` without href (adding target
  // and rel), so a stored body can hold either; loading it must keep the
  // link (editor.md §6, P-fix).
  it.each([
    '<p><a target="_blank" rel="noopener noreferrer" href="">Lásd</a></p>',
    '<p><a target="_blank" rel="noopener noreferrer">Lásd</a></p>',
  ])("keeps the link of %s", (body) => {
    const target = createEditor()
    loadDocument(target, body)
    expect(html(target)).toBe(body)
  })

  // A body without text loaded while the whole old document was selected,
  // as it is when the document is one rule: TipTap's clearDocument plugin
  // turned its first block into a plain paragraph.
  const emptyFirstBlocks = [
    '<h3 style="text-align: center"></h3>',
    "<h2></h2><ul><li><p></p></li></ul>",
    '<p style="text-align: right"></p>',
  ]

  it.each(emptyFirstBlocks)(
    "loads %s the same whatever the editor showed before",
    (body) => {
      const target = createEditor()
      for (const before of [
        "<hr>",
        "<hr><p>a</p>",
        "<ul><li>a</li></ul>",
        "",
      ]) {
        loadDocument(target, before)
        loadDocument(target, body)
        expect(html(target), before).toBe(body)
      }
    }
  )

  it("is needed: a plain setContent after a lone rule loses the first block's type", () => {
    const target = createEditor()
    for (const body of emptyFirstBlocks) {
      target.commands.setContent("<hr>")
      target.commands.setContent(body)
      expect(html(target)).not.toBe(body)
    }
  })

  it("keeps source indentation out of the document", () => {
    const target = createEditor()
    loadDocument(target, "<p>Első</p>\n    <p>Második</p>\n")
    expect(html(target)).toBe("<p>Első</p><p>Második</p>")
  })

  it("loads an empty body as one empty paragraph", () => {
    const target = createEditor()
    loadDocument(target, "")
    expect(html(target)).toBe("<p></p>")
  })

  it("is not undone by the first undo", () => {
    const target = createEditor()
    loadDocument(target, "<p>Betöltve</p>")
    expect(target.commands.undo()).toBe(false)
    expect(html(target)).toBe("<p>Betöltve</p>")
    target.commands.focus("end")
    target.commands.insertContent("!")
    expect(target.commands.undo()).toBe(true)
    expect(html(target)).toBe("<p>Betöltve</p>")
  })

  it.each<[string, (target: Editor) => void, string]>([
    [
      "a body that is not a string",
      (target) => loadDocument(target, null as unknown as string),
      "loadDocument: html must be the stored body as a string, not object.",
    ],
    [
      "a destroyed editor",
      (target) => {
        target.destroy()
        loadDocument(target, "<p>x</p>")
      },
      "loadDocument: the editor is destroyed",
    ],
  ])("refuses %s", (_name, call, message) => {
    const target = createEditor()
    expect(() => call(target)).toThrow(TypeError)
    expect(() => call(target)).toThrow(message)
  })
})
