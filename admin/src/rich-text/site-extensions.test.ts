import { Editor } from "@tiptap/core"
import { afterEach, describe, expect, it } from "vitest"

import { DOCUMENT_CONTENT, MEDIA_GROUP } from "./extensions/site-document"
import { LIST_TYPE_ATTRIBUTE } from "./extensions/site-lists"
import { loadDocument } from "./load-document"
import type { RichTextPolicy } from "./policy"
import { CONTENT_V1_POLICY, PolicyError } from "./policy"
import { HEADING_LEVELS, siteExtensions } from "./site-extensions"
import type { Json } from "@/test/rich-text-policy"
import { policyWith } from "@/test/rich-text-policy"
import {
  parseHtml,
  pasteEvent,
  renderedElements,
  renderHtml,
  siteSchema,
} from "@/test/site-schema"

type JsonObject = { [key: string]: Json }

/** The `style` list of one element in a mutable policy copy. */
function styleOf(json: JsonObject, tag: string): JsonObject[] {
  return ((json.elements as JsonObject)[tag] as JsonObject)
    .style as JsonObject[]
}

/** The grammar of one property of one element in a mutable policy copy. */
function grammarOf(
  json: JsonObject,
  tag: string,
  property: string
): JsonObject {
  const rule = styleOf(json, tag).find((entry) => entry.property === property)
  return rule!.grammar as JsonObject
}

describe("the site schema", () => {
  const schema = siteSchema()

  it("has the nodes and marks of editor.md §4.3, and no others", () => {
    expect(Object.keys(schema.nodes).sort()).toEqual(
      [
        "blockquote",
        "bulletList",
        "doc",
        "hardBreak",
        "heading",
        "horizontalRule",
        "listItem",
        "orderedList",
        "padletEmbed",
        "paragraph",
        "siteImage",
        "table",
        "tableCell",
        "tableHeader",
        "tableRow",
        "text",
        "youtubeEmbed",
      ].sort()
    )
    expect(Object.keys(schema.marks).sort()).toEqual(
      [
        "bold",
        "italic",
        "link",
        "strike",
        "subscript",
        "superscript",
        "textStyle",
        "underline",
      ].sort()
    )
  })

  it("has no code, code block, trailing node, font family or line height", () => {
    const editor = new Editor({
      element: document.createElement("div"),
      extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
    })
    const names = editor.extensionManager.extensions.map(({ name }) => name)
    editor.destroy()
    for (const absent of [
      "code",
      "codeBlock",
      "trailingNode",
      "fontFamily",
      "lineHeight",
    ]) {
      expect(names).not.toContain(absent)
    }
    for (const present of [
      "undoRedo",
      "dropCursor",
      "gapCursor",
      "listKeymap",
    ]) {
      expect(names).toContain(present)
    }
  })

  it("parses and renders headings 1 to 4 only", () => {
    expect(schema.nodes.heading.spec.parseDOM?.map((rule) => rule.tag)).toEqual(
      HEADING_LEVELS.map((level) => `h${level}`)
    )
    const doc = parseHtml(schema, "<h1>a</h1><h4>b</h4><h5>c</h5><h6>d</h6>")
    expect(renderHtml(schema, doc).innerHTML).toBe(
      "<h1>a</h1><h4>b</h4><p>c</p><p>d</p>"
    )
  })
})

describe("the media group", () => {
  const schema = siteSchema()
  const MEDIA = ["siteImage", "youtubeEmbed", "padletEmbed"]
  const image =
    '<p><img src="/storage/uploads/a.jpg" alt="kép" style="display: block; height: auto; margin: 0 auto; max-width: 100%"></p>'

  it("makes the document (block | media)+", () => {
    expect(schema.topNodeType.spec.content).toBe(DOCUMENT_CONTENT)
    expect(DOCUMENT_CONTENT).toBe("(block | media)+")
  })

  it.each(MEDIA)("holds %s as an atom of the media group only", (name) => {
    const type = schema.nodes[name]
    expect(type.spec.group).toBe(MEDIA_GROUP)
    expect(type.isAtom).toBe(true)
    expect(schema.topNodeType.contentMatch.matchType(type)).not.toBeNull()
  })

  it.each(MEDIA)("keeps %s at the top level only", (name) => {
    const media = schema.nodes[name]
    for (const container of [
      "blockquote",
      "listItem",
      "heading",
      "paragraph",
      "tableCell",
      "tableHeader",
    ]) {
      const type = schema.nodes[container]
      const after = type.contentMatch.matchType(
        ["listItem", "tableCell", "tableHeader"].includes(container)
          ? schema.nodes.paragraph
          : media
      )
      expect(type.contentMatch.matchType(media), container).toBeNull()
      expect(after?.matchType(media) ?? null, container).toBeNull()
    }
  })

  it("moves an image out of a quote or list when it parses", () => {
    const doc = parseHtml(
      schema,
      `<blockquote><p>a</p>${image}</blockquote><ul><li>${image}</li></ul>`
    )
    const topLevel: string[] = []
    doc.forEach((node) => topLevel.push(node.type.name))
    let nested = 0
    doc.forEach((node) =>
      node.descendants((child) => {
        nested += MEDIA.includes(child.type.name) ? 1 : 0
      })
    )
    expect(nested).toBe(0)
    expect(topLevel.filter((name) => name === "siteImage")).toHaveLength(2)
  })
})

describe("siteExtensions options", () => {
  it.each<[string, () => unknown, string]>([
    [
      "no options",
      () => siteExtensions(undefined as unknown as { policy: RichTextPolicy }),
      "siteExtensions: options.policy is required",
    ],
    [
      "no policy",
      () => siteExtensions({} as { policy: RichTextPolicy }),
      "siteExtensions: options.policy is required",
    ],
    [
      "a notify that is a string",
      () =>
        siteExtensions({
          policy: CONTENT_V1_POLICY,
          notify: "editor.youtube.video_id",
        } as unknown as { policy: RichTextPolicy }),
      "siteExtensions: options.notify must be a function that takes an editor.* message key, or left out; got string.",
    ],
    [
      "a notify that is null",
      () =>
        siteExtensions({
          policy: CONTENT_V1_POLICY,
          notify: null,
        } as unknown as { policy: RichTextPolicy }),
      "options.notify must be a function that takes an editor.* message key, or left out; got null.",
    ],
  ])("refuses %s", (_name, call, message) => {
    expect(call).toThrow(TypeError)
    expect(call).toThrow(message)
  })

  it("passes the notice callback to the YouTube paste rule", () => {
    const notices: string[] = []
    const editor = new Editor({
      element: document.createElement("div"),
      extensions: siteExtensions({
        policy: CONTENT_V1_POLICY,
        notify: (notice) => notices.push(notice),
      }),
    })
    editor.view.pasteText(
      "https://www.youtube.com/embed/dQw4w9WgXcQ?controls=0",
      pasteEvent({})
    )
    editor.destroy()
    expect(notices).toEqual(["editor.youtube.options_dropped"])
  })

  it.each<[string, (json: JsonObject) => void, string]>([
    [
      "headings aligned differently from paragraphs",
      (json) => {
        const grammar = styleOf(json, "h2")[0].grammar as JsonObject
        grammar.values = ["left", "center"]
      },
      "h2 text-align allows left, center, but p allows left, center, right, justify",
    ],
    [
      "cells aligned differently from paragraphs",
      (json) => {
        grammarOf(json, "th", "text-align").values = ["left", "right"]
      },
      "th text-align allows left, right, but p allows left, center, right, justify",
    ],
    [
      "a span font size in another unit",
      (json) => {
        styleOf(json, "span")[3].grammar = { kind: "px", min: 1, max: 100 }
      },
      "The policy has no stored font-size on span with a em grammar",
    ],
    [
      "a bullet style that is no keyword",
      (json) => {
        styleOf(json, "ul")[0].grammar = { kind: "locked", value: "circle" }
      },
      "The policy has no stored list-style-type on ul with a keyword grammar",
    ],
  ])("refuses a policy with %s", (_name, change, message) => {
    const policy = policyWith(change)
    expect(() => siteExtensions({ policy })).toThrow(PolicyError)
    expect(() => siteExtensions({ policy })).toThrow(message)
  })

  it("follows the policy rather than restating it", () => {
    const policy = policyWith((json) => {
      ;(styleOf(json, "ul")[0].grammar as JsonObject).values = ["square"]
      for (const tag of ["p", "h1", "h2", "h3", "h4", "td", "th"]) {
        grammarOf(json, tag, "text-align").values = ["center"]
      }
    })
    const schema = siteSchema({ policy })
    const doc = parseHtml(
      schema,
      '<ul style="list-style-type: circle"><li>a</li></ul><ul style="list-style-type: square"><li>b</li></ul><p style="text-align: right">c</p><h2 style="text-align: center">d</h2>'
    )
    expect(renderHtml(schema, doc).innerHTML).toBe(
      '<ul><li><p>a</p></li></ul><ul style="list-style-type: square"><li><p>b</p></li></ul><p>c</p><h2 style="text-align: center">d</h2>'
    )
  })
})

describe("the site schema in an editor", () => {
  let editor: Editor | undefined

  afterEach(() => {
    editor?.destroy()
    editor = undefined
  })

  function createEditor(html = "<p></p>"): Editor {
    editor = new Editor({
      element: document.createElement("div"),
      extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
    })
    loadDocument(editor, html)
    return editor
  }

  /**
   * An editor after its `create` event, which fires a tick after
   * construction: Link registers its linkifyjs protocols (`tel`) only then.
   */
  async function createReadyEditor(html = "<p></p>"): Promise<Editor> {
    const target = createEditor(html)
    await new Promise<void>((resolve) => {
      target.on("create", () => resolve())
    })
    return target
  }

  /**
   * Types text at the cursor the way the browser's text input does, through
   * the view's `handleTextInput`, so input rules run on each character.
   */
  function type(target: Editor, text: string): void {
    for (const character of text) {
      const { view } = target
      const { from, to } = view.state.selection
      const handled = view.someProp("handleTextInput", (handle) =>
        handle(view, from, to, character, () =>
          view.state.tr.insertText(character, from, to)
        )
      )
      if (!handled) {
        view.dispatch(view.state.tr.insertText(character, from, to))
      }
    }
  }

  /** The rendered elements of the editor's document, as tag and attributes. */
  function rendered(target: Editor) {
    return renderedElements(target.state.doc).map(({ tag, attributes }) => ({
      tag,
      attributes,
    }))
  }

  describe("links", () => {
    it("autolinks a URL as it is typed", () => {
      const target = createEditor("<p>Lásd</p>")
      target.commands.focus("end")
      target.commands.insertContent(" example.org ")
      expect(rendered(target)).toEqual([
        { tag: "p", attributes: {} },
        {
          tag: "a",
          attributes: {
            target: "_blank",
            rel: "noopener noreferrer",
            href: "http://example.org",
          },
        },
      ])
    })

    it.each([
      ["tel:3612345", "tel:3612345"],
      ["tel:06-1-234-5678", "tel:06-1-234-5678"],
      ["mailto:titkarsag@example.org", "mailto:titkarsag@example.org"],
    ])("autolinks %s as it is typed", async (typed, href) => {
      const target = await createReadyEditor("<p>Hívja</p>")
      target.commands.focus("end")
      target.commands.insertContent(` ${typed} `)
      expect(rendered(target)[1]).toEqual({
        tag: "a",
        attributes: { target: "_blank", rel: "noopener noreferrer", href },
      })
    })

    it("does not autolink a tel: URI with a +, which linkifyjs does not find", async () => {
      const target = await createReadyEditor("<p>Hívja</p>")
      target.commands.focus("end")
      target.commands.insertContent(" tel:+3612345 ")
      expect(rendered(target)).toEqual([{ tag: "p", attributes: {} }])
      target.commands.setTextSelection({ from: 7, to: 19 })
      expect(target.commands.setLink({ href: "tel:+3612345" })).toBe(true)
      expect(rendered(target)[1].attributes.href).toBe("tel:+3612345")
    })

    it("autolinks nothing that the server would refuse", () => {
      const target = createEditor("<p>Lásd</p>")
      target.commands.focus("end")
      target.commands.insertContent(" ftp://example.org/x ")
      expect(
        target.state.doc.rangeHasMark(
          0,
          target.state.doc.content.size,
          target.schema.marks.link
        )
      ).toBe(false)
    })

    it.each([["paste"], ["drop"]])(
      "creates no link in text that a %s inserts",
      (event) => {
        const target = createEditor("<p>Lásd</p>")
        const end = target.state.doc.content.size - 1
        target.view.dispatch(
          target.state.tr
            .insertText(" example.org ", end)
            .setMeta("uiEvent", event)
        )
        expect(target.state.doc.textContent).toBe("Lásd example.org ")
        expect(rendered(target)).toEqual([{ tag: "p", attributes: {} }])
      }
    )

    it("creates no link when a URL is pasted as text", () => {
      const target = createEditor("<p>Lásd</p>")
      target.commands.focus("end")
      target.view.pasteText(
        " https://example.org/ ",
        pasteEvent({ "text/plain": " https://example.org/ " })
      )
      expect(target.state.doc.textContent).toBe("Lásd https://example.org/ ")
      expect(rendered(target)).toEqual([{ tag: "p", attributes: {} }])
    })

    it("does not turn a selection into a link when a URL is pasted over it", () => {
      const target = createEditor("<p>Lásd ezt</p>")
      target.commands.setTextSelection({ from: 6, to: 9 })
      target.view.pasteText(
        "https://example.org/",
        pasteEvent({ "text/plain": "https://example.org/" })
      )
      expect(target.state.doc.textContent).toBe("Lásd https://example.org/")
      expect(rendered(target)).toEqual([{ tag: "p", attributes: {} }])
    })

    it("keeps the links of pasted HTML that the server keeps", () => {
      const target = createEditor("<p></p>")
      const html =
        '<p><a href="https://example.org/">jó</a> <a href="javascript:alert(1)">rossz</a> https://example.net/</p>'
      target.view.pasteHTML(html, pasteEvent({ "text/html": html }))
      expect(rendered(target).filter(({ tag }) => tag === "a")).toEqual([
        {
          tag: "a",
          attributes: {
            target: "_blank",
            rel: "noopener noreferrer",
            href: "https://example.org/",
          },
        },
      ])
    })

    it("sets only links the server keeps, with the fixed target and rel", () => {
      const target = createEditor("<p>szöveg</p>")
      target.commands.setTextSelection({ from: 1, to: 7 })
      expect(target.commands.setLink({ href: "javascript:alert(1)" })).toBe(
        false
      )
      expect(target.commands.setLink({ href: "//evil.example/" })).toBe(false)
      expect(
        target.commands.setLink({
          href: "/hirek/12",
          target: "_self",
          rel: "nofollow",
          class: "x",
          title: "y",
        })
      ).toBe(true)
      const [link] = target.state.doc.firstChild?.firstChild?.marks ?? []
      expect(link.attrs).toEqual({ href: "/hirek/12" })
      expect(rendered(target)[1]).toEqual({
        tag: "a",
        attributes: {
          target: "_blank",
          rel: "noopener noreferrer",
          href: "/hirek/12",
        },
      })
    })

    it("does not follow a link on click", () => {
      const target = createEditor()
      const link = target.extensionManager.extensions.find(
        ({ name }) => name === "link"
      )
      expect(link?.options).toMatchObject({
        openOnClick: false,
        autolink: true,
        linkOnPaste: false,
        protocols: [{ scheme: "tel", optionalSlashes: true }],
      })
    })

    it("writes a link without href as the server keeps it, and loads it back", () => {
      const target = createEditor("<p>szöveg</p>")
      target.commands.setTextSelection({ from: 1, to: 7 })
      // Stock toggleLink accepts a link without href.
      expect(target.commands.toggleLink({} as { href: string })).toBe(true)
      expect(rendered(target)[1]).toEqual({
        tag: "a",
        attributes: { target: "_blank", rel: "noopener noreferrer" },
      })
      const html = renderHtml(target.schema, target.state.doc).innerHTML
      expect(html).toBe(
        '<p><a target="_blank" rel="noopener noreferrer">szöveg</a></p>'
      )
      expect(parseHtml(target.schema, html).eq(target.state.doc)).toBe(true)
    })

    it("writes no href that the server would drop, even when a mark holds one", () => {
      const target = createEditor("<p>szöveg</p>")
      target.commands.setTextSelection({ from: 1, to: 7 })
      // setMark skips setLink's check, as JSON content or another
      // extension could.
      target.commands.setMark("link", { href: "javascript:alert(1)" })
      expect(rendered(target)[1]).toEqual({
        tag: "a",
        attributes: { target: "_blank", rel: "noopener noreferrer" },
      })
    })
  })

  describe("headings", () => {
    it.each([
      ["## ", "<h2></h2>"],
      ["### ", "<h3></h3>"],
      ["#### ", "<h4></h4>"],
    ])("makes a heading of %j typed at the start", (typed, html) => {
      const target = createEditor()
      target.commands.focus("start")
      type(target, typed)
      expect(renderHtml(target.schema, target.state.doc).innerHTML).toBe(html)
    })

    it.each(["# ", "##### "])("makes no heading of %j", (typed) => {
      const target = createEditor()
      target.commands.focus("start")
      type(target, typed)
      expect(renderHtml(target.schema, target.state.doc).innerHTML).toBe(
        `<p>${typed}</p>`
      )
    })

    it.each([2, 3, 4])("toggles an H%i with Mod-Alt and its level", (level) => {
      const target = createEditor("<p>cím</p>")
      target.commands.focus("end")
      target.commands.keyboardShortcut(`Mod-Alt-${level}`)
      expect(renderHtml(target.schema, target.state.doc).innerHTML).toBe(
        `<h${level}>cím</h${level}>`
      )
    })

    it("makes no h1 with Mod-Alt-1", () => {
      const target = createEditor("<p>cím</p>")
      target.commands.focus("end")
      target.commands.keyboardShortcut("Mod-Alt-1")
      expect(renderHtml(target.schema, target.state.doc).innerHTML).toBe(
        "<p>cím</p>"
      )
    })

    it("keeps a legacy h1, which commands can still change", () => {
      const target = createEditor("<h1>régi</h1>")
      expect(renderHtml(target.schema, target.state.doc).innerHTML).toBe(
        "<h1>régi</h1>"
      )
      target.commands.focus("end")
      target.commands.setHeading({ level: 2 })
      expect(renderHtml(target.schema, target.state.doc).innerHTML).toBe(
        "<h2>régi</h2>"
      )
    })
  })

  describe("subscript and superscript", () => {
    it.each([
      [
        "subscript, then superscript",
        "toggleSubscript",
        "toggleSuperscript",
        "superscript",
      ],
      [
        "superscript, then subscript",
        "toggleSuperscript",
        "toggleSubscript",
        "subscript",
      ],
    ] as const)(
      "keeps the one applied last: %s",
      (_name, first, second, kept) => {
        const target = createEditor("<p>H2O</p>")
        target.commands.setTextSelection({ from: 2, to: 3 })
        target.commands[first]()
        target.commands[second]()
        const marks = target.state.doc.firstChild
          ?.child(1)
          .marks.map(({ type }) => type.name)
        expect(marks).toEqual([kept])
      }
    )
  })

  describe("text style", () => {
    it("renders only colours and sizes in content-v1's spelling", () => {
      const target = createEditor("<p>szín</p>")
      target.commands.setTextSelection({ from: 1, to: 5 })
      target.commands.setColor("red")
      target.commands.setFontSize("12pt")
      expect(rendered(target)).toEqual([
        { tag: "p", attributes: {} },
        { tag: "span", attributes: {} },
      ])
      target.commands.setColor("#c00000")
      target.commands.setBackgroundColor("#ffff00")
      target.commands.setFontSize("1.5em")
      expect(rendered(target)[1]).toEqual({
        tag: "span",
        attributes: {
          style: "color: #c00000; background-color: #ffff00; font-size: 1.5em",
        },
      })
    })
  })

  describe("ordered list types in the view", () => {
    const typeIn = (target: Editor) =>
      target.view.dom.querySelector("ol")?.getAttribute(LIST_TYPE_ATTRIBUTE) ??
      null

    it("decorates a typed list in the view, never in the document", () => {
      const target = createEditor('<ol type="a"><li><p>első</p></li></ol>')
      expect(typeIn(target)).toBe("lower-alpha")
      expect(target.getHTML()).not.toContain(LIST_TYPE_ATTRIBUTE)
      expect(renderHtml(target.schema, target.state.doc).innerHTML).toBe(
        '<ol type="a"><li><p>első</p></li></ol>'
      )
    })

    it("follows a change of the list's type", () => {
      const target = createEditor('<ol type="a"><li><p>első</p></li></ol>')
      target.commands.setTextSelection(3)
      target.commands.updateAttributes("orderedList", { type: "I" })
      expect(typeIn(target)).toBe("upper-roman")
      target.commands.updateAttributes("orderedList", { type: null })
      expect(typeIn(target)).toBeNull()
    })
  })
})
