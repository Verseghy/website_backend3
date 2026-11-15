/**
 * `OfficePaste` on the editor's paste path (editor.md §4.3 row 11, §5.1,
 * §5.5; ADR 0043 D128). Runs under jsdom (`npm test`) and in Chromium,
 * Firefox and WebKit (`npm run test:browser`).
 */
import { Editor, Extension } from "@tiptap/core"
import type { AnyExtension } from "@tiptap/core"
import { Plugin, PluginKey } from "@tiptap/pm/state"
import { afterAll, describe, expect, it, vi } from "vitest"

import { loadDocument } from "../load-document"
import type { PasteFilter } from "../paste/normalize-pasted-html"
import type { PasteReport } from "../paste/paste-context"
import type { PasteOutcome } from "../paste/paste-filter"
import { escapeText } from "../paste/paste-filter"
import type { RichTextPolicy } from "../policy"
import { CONTENT_V1_POLICY } from "../policy"
import { toStoredHtml } from "../serialize/to-stored-html"
import { siteExtensions } from "../site-extensions"
import type { EditorNotice, NotifyEditor } from "./editor-notice"
import {
  createOfficePaste,
  isEmbedCodePlace,
  OFFICE_PASTE_EXTENSION,
  outcomeNotices,
  PASTE_NOTICES,
} from "./office-paste"
import { YOUTUBE_NOTICES, YOUTUBE_STYLE } from "./youtube-embed"
import { clipboardPasteEvent } from "@/test/paste-fixtures"
import { pasteEvent } from "@/test/site-schema"
import { youtubeFixture, youtubeFixtures } from "@/test/youtube-fixtures"

const policy = CONTENT_V1_POLICY

const FORCED =
  'allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"'

/** The stored YouTube embed of editor.md §3.7 for a src and a title. */
const youtube = (src: string, title = "Videó") =>
  `<p data-youtube-video=""><iframe src="${src}" width="640" height="360" title="${title}" style="${YOUTUBE_STYLE}" ${FORCED}></iframe></p>`

const VIDEO = "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ"

/** An editor whose notices are collected, and its paste helpers. */
function pasteEditor(extensions?: (notify: NotifyEditor) => AnyExtension[]) {
  const notices: EditorNotice[] = []
  const notify: NotifyEditor = (notice) => notices.push(notice)
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: extensions?.(notify) ?? siteExtensions({ policy, notify }),
  })
  const save = () => toStoredHtml(editor.state.doc, { policy, document })
  /** Pastes clipboard data into an empty body and saves. */
  const paste = (data: Record<string, string>) => {
    notices.length = 0
    loadDocument(editor, "<p></p>")
    editor.commands.setTextSelection(1)
    editor.view.dom.dispatchEvent(clipboardPasteEvent(data))
    return save()
  }
  return { editor, notices, paste, save }
}

describe("createOfficePaste", () => {
  it.each<[string, () => unknown]>([
    ["no options", () => createOfficePaste(undefined as never)],
    [
      "no policy",
      () =>
        createOfficePaste({
          policy: undefined as unknown as RichTextPolicy,
          notify: () => {},
        }),
    ],
    [
      "no notice callback",
      () =>
        createOfficePaste({
          policy,
          notify: undefined as unknown as NotifyEditor,
        }),
    ],
  ])("refuses %s", (_name, call) => {
    expect(call).toThrow(TypeError)
    expect(call).toThrow("options.policy and options.notify are required")
  })

  it("is part of the site schema's extensions, and adds no node or mark", () => {
    const names = siteExtensions({ policy }).map(({ name }) => name)
    expect(names).toContain(OFFICE_PASTE_EXTENSION)
    const { editor } = pasteEditor()
    expect(editor.schema.nodes[OFFICE_PASTE_EXTENSION]).toBeUndefined()
    expect(editor.schema.marks[OFFICE_PASTE_EXTENSION]).toBeUndefined()
    editor.destroy()
  })
})

describe("an HTML paste (editor.md §5.1 steps 2-4)", () => {
  const { editor, notices, paste } = pasteEditor()
  afterAll(() => editor.destroy())

  it("goes through the paste filter", () => {
    // ProseMirror opens a pasted slice at both ends, so the first and the
    // last paragraph merge into the paragraph pasted into and take its
    // attributes; the middle one keeps its own alignment.
    expect(
      paste({
        "text/html":
          '<p>első</p><p class=MsoNormal style="margin: 0cm; font-family: Calibri; text-align: center"><b>félkövér</b> <span style="color: rgb(192, 0, 0); font-size: 14pt">piros</span><o:p></o:p></p><p>utolsó</p>',
      })
    ).toBe(
      '<p>első</p><p style="text-align: center"><strong>félkövér</strong> <span style="color: #c00000">piros</span></p><p>utolsó</p>'
    )
    expect(notices).toEqual([])
  })

  it("shows the dropped-options notice once for an embed that loses playback options (D134)", () => {
    expect(
      paste({
        "text/html": `<p>előtte</p>${youtubeFixture("youtube-embed-dialog-controls-off")}`,
      })
    ).toBe(`<p>előtte</p>${youtube(VIDEO)}`)
    expect(notices).toEqual([YOUTUBE_NOTICES.optionsDropped])
  })

  it("pastes YouTube's embed code from an HTML clipboard through the filter, not the text route (D128)", () => {
    const code = youtubeFixture("youtube-embed-dialog-default")
    expect(paste({ "text/html": code, "text/plain": code })).toBe(
      youtube(VIDEO)
    )
    expect(notices).toEqual([])
  })

  it.each<[string, string, EditorNotice[]]>([
    [
      "a wrong id",
      '<iframe src="https://www.youtube.com/embed/abc"></iframe>',
      [YOUTUBE_NOTICES.videoId],
    ],
    [
      "a user-uploads list",
      '<iframe src="https://www.youtube.com/embed?listType=user_uploads&amp;list=someone"></iframe>',
      [YOUTUBE_NOTICES.listType],
    ],
    [
      "both, beside an embed that loses an option",
      `<iframe src="https://www.youtube.com/embed?listType=search&amp;list=x"></iframe><iframe src="https://www.youtube.com/embed/abc"></iframe><iframe src="https://www.youtube.com/embed/abd"></iframe>${youtubeFixture("youtube-embed-dialog-controls-off")}`,
      [
        YOUTUBE_NOTICES.optionsDropped,
        YOUTUBE_NOTICES.videoId,
        YOUTUBE_NOTICES.listType,
      ],
    ],
  ])(
    "says why it removed a YouTube iframe with %s, as the other routes do (§4.3 row 9)",
    (_name, html, expected) => {
      const stored = paste({ "text/html": `<p>előtte</p>${html}` })
      expect(stored).not.toContain("youtube.com/embed/abc")
      expect(notices).toEqual(expected)
    }
  )

  it("says nothing about an iframe from another host, which is no YouTube address", () => {
    expect(
      paste({
        "text/html":
          '<p>a</p><iframe src="https://evil.example/embed/x"></iframe>',
      })
    ).toBe("<p>a</p>")
    expect(notices).toEqual([])
  })

  it("shows no stale notice on the next paste", () => {
    paste({ "text/html": youtubeFixture("youtube-embed-dialog-controls-off") })
    expect(notices).toHaveLength(1)
    expect(paste({ "text/plain": "szöveg" })).toBe("<p>szöveg</p>")
    expect(notices).toEqual([])
  })
})

describe("a paste that a plugin before OfficePaste consumes", () => {
  let consume = true
  // A higher priority puts its plugin first, as prosemirror-tables' cell
  // paste handler can be.
  const Consumer = Extension.create({
    name: "consumer",
    priority: 1000,
    addProseMirrorPlugins() {
      return [
        new Plugin({
          key: new PluginKey("consumer"),
          props: { handlePaste: () => consume },
        }),
      ]
    },
  })
  const { editor, notices, paste } = pasteEditor((notify) => [
    ...siteExtensions({ policy, notify }),
    Consumer,
  ])
  afterAll(() => editor.destroy())

  it("leaves no notice behind for the next paste", () => {
    paste({ "text/html": youtubeFixture("youtube-embed-dialog-controls-off") })
    expect(notices).toEqual([])
    expect(editor.storage.officePaste.pending?.kind).toBe("filtered")
    consume = false
    expect(paste({ "text/plain": "szöveg" })).toBe("<p>szöveg</p>")
    expect(notices).toEqual([])
    expect(editor.storage.officePaste.pending).toBeNull()
  })
})

describe("the plain-text route for embed code (ADR 0043 D128)", () => {
  const { editor, notices, paste, save } = pasteEditor()
  afterAll(() => editor.destroy())

  it("pastes YouTube's embed code as an embed, with the title rule", () => {
    expect(
      paste({ "text/plain": youtubeFixture("youtube-embed-dialog-default") })
    ).toBe(youtube(VIDEO))
    expect(notices).toEqual([])
  })

  it("shows the dropped-options notice on the plain-text route", () => {
    expect(
      paste({
        "text/plain": youtubeFixture("youtube-embed-dialog-controls-off"),
      })
    ).toBe(youtube(VIDEO))
    expect(notices).toEqual([YOUTUBE_NOTICES.optionsDropped])
  })

  it("pastes a lone Padlet iframe as a board", () => {
    expect(
      paste({
        "text/plain":
          '  <iframe src="https://padlet.com/embed/invented0key"></iframe>\n',
      })
    ).toMatch(
      /^<p data-padlet-embed=""><iframe src="https:\/\/padlet\.com\/embed\/invented0key" /
    )
  })

  it("keeps a refused YouTube iframe as text and says why", () => {
    const code =
      '<iframe src="https://www.youtube.com/embed/abc" title="x"></iframe>'
    expect(paste({ "text/plain": code })).not.toContain("<iframe")
    expect(notices).toEqual([YOUTUBE_NOTICES.videoId])
  })

  it.each([
    '<iframe src="https://evil.example/embed/x"></iframe>',
    '<div class="padlet-embed"><iframe src="https://padlet.com/embed/invented0key"></iframe></div>',
    `előtte ${youtubeFixture("youtube-embed-dialog-default")}`,
    '<iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ"></iframe><iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ"></iframe>',
  ])("keeps anything else as text: %s", (code) => {
    const stored = paste({ "text/plain": code })
    expect(stored).not.toContain("data-youtube-video")
    expect(stored).not.toContain("data-padlet-embed")
    expect(notices).toEqual([])
  })

  it("keeps embed code as text in a plain-text paste (Shift held)", () => {
    const code = youtubeFixture("youtube-embed-dialog-default")
    loadDocument(editor, "<p></p>")
    editor.commands.setTextSelection(1)
    editor.view.pasteText(code, pasteEvent({ "text/plain": code }))
    expect(save()).not.toContain("data-youtube-video")
  })

  it("keeps embed code as text when Shift is down as the paste arrives (Ctrl+Shift+V)", () => {
    const code = youtubeFixture("youtube-embed-dialog-default")
    notices.length = 0
    loadDocument(editor, "<p></p>")
    editor.commands.setTextSelection(1)
    // prosemirror-view reads Shift from the last keydown and takes the
    // plain-text path while it is held.
    const dom = editor.view.dom
    dom.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Shift",
        keyCode: 16,
        shiftKey: true,
        bubbles: true,
      })
    )
    dom.dispatchEvent(clipboardPasteEvent({ "text/plain": code }))
    dom.dispatchEvent(
      new KeyboardEvent("keyup", { key: "Shift", keyCode: 16, bubbles: true })
    )
    const stored = save()
    expect(stored).not.toContain("data-youtube-video")
    expect(stored).toContain("&lt;iframe")
    expect(notices).toEqual([])
    // With Shift released, the same clipboard becomes an embed again.
    expect(paste({ "text/plain": code })).toBe(youtube(VIDEO))
  })

  it("upgrades an http embed address on the plain-text route (D136)", () => {
    expect(
      paste({ "text/plain": youtubeFixture("youtube-help-start-http") })
    ).toBe(youtube(`${VIDEO.replace("dQw4w9WgXcQ", "UkWd0azv3fQ")}?start=90`))
    expect(notices).toEqual([])
  })

  const CELLS =
    '<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td>%</td><td>b</td></tr></tbody></table>'
  const PADLET_CODE =
    '<iframe src="https://padlet.com/embed/invented0key"></iframe>'

  /** Loads `body` and puts the cursor in its first textblock. */
  const cursorInFirstTextblock = (body: string) => {
    loadDocument(editor, body)
    let textblock = -1
    editor.state.doc.descendants((node, pos) => {
      if (textblock < 0 && node.isTextblock) {
        textblock = pos + 1
      }
    })
    editor.commands.setTextSelection(textblock)
    return textblock
  }

  it.each<[string, string, string, EditorNotice]>([
    ["YouTube's", "a heading", "<h2>%</h2>", YOUTUBE_NOTICES.notTopLevel],
    [
      "YouTube's",
      "a list item",
      "<ul><li>%</li></ul>",
      YOUTUBE_NOTICES.notTopLevel,
    ],
    [
      "YouTube's",
      "a quote",
      "<blockquote>%</blockquote>",
      YOUTUBE_NOTICES.notTopLevel,
    ],
    ["YouTube's", "a table cell", CELLS, YOUTUBE_NOTICES.notTopLevel],
    ["Padlet's", "a table cell", CELLS, PASTE_NOTICES.padletNotTopLevel],
    [
      "Padlet's",
      "a list item",
      "<ul><li>%</li></ul>",
      PASTE_NOTICES.padletNotTopLevel,
    ],
  ])(
    "keeps %s embed code pasted in %s as text and says why, so the block is not cut in two",
    (provider, _place, body, notice) => {
      const code =
        provider === "YouTube's"
          ? youtubeFixture("youtube-embed-dialog-default")
          : PADLET_CODE
      notices.length = 0
      cursorInFirstTextblock(body.replace("%", ""))
      editor.view.dom.dispatchEvent(clipboardPasteEvent({ "text/plain": code }))
      expect(save()).toBe(body.replace("%", escapeText(code)))
      expect(notices).toEqual([notice])
    }
  )

  it("says why a refused YouTube code stays text, wherever it is pasted", () => {
    notices.length = 0
    cursorInFirstTextblock("<ul><li></li></ul>")
    editor.view.dom.dispatchEvent(
      clipboardPasteEvent({
        "text/plain":
          '<iframe src="https://www.youtube.com/embed/abc"></iframe>',
      })
    )
    expect(notices).toEqual([YOUTUBE_NOTICES.videoId])
  })

  it("keeps the code as text over a selection that ends in a table, which the paste replaces", () => {
    notices.length = 0
    loadDocument(editor, `<p>előtte</p>${CELLS.replace("%", "a")}`)
    let cell = -1
    editor.state.doc.descendants((node, pos) => {
      if (cell < 0 && node.type.name === "tableCell") {
        cell = pos + 2
      }
    })
    editor.commands.setTextSelection({ from: 3, to: cell + 1 })
    const code = youtubeFixture("youtube-embed-dialog-default")
    editor.view.dom.dispatchEvent(clipboardPasteEvent({ "text/plain": code }))
    const stored = save()
    expect(stored).not.toContain("data-youtube-video")
    expect(stored.match(/<table/g)).toHaveLength(1)
    expect(notices).toEqual([YOUTUBE_NOTICES.notTopLevel])
  })

  // A drop has no clipboard event that jsdom can place (posAtCoords needs
  // layout), so the hook is called as prosemirror-view's handleDrop calls
  // it: with the drop point as the context, the selection elsewhere.
  describe("a drop of embed code", () => {
    const dropText = (text: string, at: number) =>
      editor.view.someProp("clipboardTextParser", (parse) =>
        parse(text, editor.state.doc.resolve(at), false, editor.view)
      )
    const code = youtubeFixture("youtube-embed-dialog-default")

    it("into a table cell keeps the code as text, wherever the selection is", () => {
      notices.length = 0
      loadDocument(editor, `<p>előtte</p>${CELLS.replace("%", "a")}`)
      editor.commands.setTextSelection(1)
      let cell = -1
      editor.state.doc.descendants((node, pos) => {
        if (cell < 0 && node.type.name === "tableCell") {
          cell = pos + 2
        }
      })
      expect(dropText(code, cell)).toBeFalsy()
      expect(notices).toEqual([YOUTUBE_NOTICES.notTopLevel])
    })

    it("into a top-level paragraph is an embed, though the selection is in a cell", () => {
      notices.length = 0
      loadDocument(editor, `<p>előtte</p>${CELLS.replace("%", "a")}`)
      let cell = -1
      editor.state.doc.descendants((node, pos) => {
        if (cell < 0 && node.type.name === "tableCell") {
          cell = pos + 2
        }
      })
      editor.commands.setTextSelection(cell)
      const slice = dropText(code, 3)
      expect(slice?.content.firstChild?.type.name).toBe("youtubeEmbed")
      expect(notices).toEqual([])
    })
  })
})

describe("isEmbedCodePlace", () => {
  const { editor } = pasteEditor()
  afterAll(() => editor.destroy())
  /** The first position inside the list item's paragraph of the document. */
  const inList = 7

  it.each<[string, number, [number, number], boolean]>([
    ["a paste in a top-level paragraph", 2, [2, 2], true],
    ["a paste over a range of top-level paragraphs", 2, [2, 13], true],
    ["a paste in a list item", inList, [inList, inList], false],
    ["a paste over a range that ends in a list item", 2, [2, inList], false],
    [
      "a drop in a top-level paragraph, the selection in a list",
      2,
      [inList, inList],
      true,
    ],
    ["a drop in a list item, the selection elsewhere", inList, [2, 2], false],
  ])("reads %s", (_name, context, [from, to], expected) => {
    loadDocument(editor, "<p>ab</p><ul><li><p>c</p></li></ul><p>de</p>")
    expect(editor.state.doc.resolve(inList).parent.textContent).toBe("c")
    editor.commands.setTextSelection({ from, to })
    expect(
      isEmbedCodePlace(
        editor.state.doc.resolve(context),
        editor.state.selection
      )
    ).toBe(expected)
  })
})

describe("outcomeNotices", () => {
  /** A filtered outcome whose report drops what is given. */
  const filtered = (
    dropped: Partial<PasteReport["dropped"]>
  ): PasteOutcome => ({
    kind: "filtered",
    html: "",
    source: "generic",
    report: {
      moved: { media: [] },
      dropped: {
        hiddenRows: 0,
        hiddenCells: 0,
        hiddenOther: 0,
        fontFamilies: [],
        frames: 0,
        youtubeRefusals: [],
        embedParams: [],
        ...dropped,
      },
    },
  })

  it("has nothing to say without an outcome or with an empty report", () => {
    expect(outcomeNotices(null)).toEqual([])
    expect(
      outcomeNotices(filtered({ frames: 2, fontFamilies: ["Calibri"] }))
    ).toEqual([])
  })

  it("shows the fallback's notice alone", () => {
    expect(
      outcomeNotices({
        kind: "fallback",
        html: "",
        source: null,
        error: new Error("x"),
      })
    ).toEqual([PASTE_NOTICES.fallback])
  })

  it("shows the dropped options first, then each refusal once", () => {
    expect(
      outcomeNotices(
        filtered({
          embedParams: ["autoplay", "loop"],
          youtubeRefusals: ["video_id", "list_type"],
        })
      )
    ).toEqual([
      YOUTUBE_NOTICES.optionsDropped,
      YOUTUBE_NOTICES.videoId,
      YOUTUBE_NOTICES.listType,
    ])
    expect(
      outcomeNotices(filtered({ youtubeRefusals: ["list_type"] }))
    ).toEqual([YOUTUBE_NOTICES.listType])
  })
})

describe("every YouTube embed code of the research (ADR 0043, D128-D136)", () => {
  const { editor, notices, paste } = pasteEditor()
  afterAll(() => editor.destroy())
  const codes = youtubeFixtures().filter(({ kind }) => kind === "code")
  /** The fixtures whose code carries a playback option the node drops (D134). */
  const DROPS = new Set([
    "youtube-dev-sample-autoplay-origin",
    "youtube-embed-dialog-all-options",
    "youtube-embed-dialog-controls-off",
    "youtube-embed-dialog-playlist-all-options",
    "youtube-help-autoplay",
  ])
  /** The title the paste gives a fixture's code (D135). */
  const titleOf = (code: string) => {
    const iframe = new DOMParser()
      .parseFromString(code, "text/html")
      .querySelector("iframe")!
    const title = iframe.getAttribute("title")?.trim() ?? ""
    return title === "" || title === "YouTube video player"
      ? "Videó"
      : iframe.getAttribute("title")!
  }

  it("has every embed code fixture", () => {
    expect(codes.map(({ name }) => name)).toEqual(
      expect.arrayContaining([...DROPS, "youtube-help-start-http"])
    )
  })

  it.each(codes.map(({ name, text }) => [name, text]))(
    "pastes %s as one embed, as text and as HTML alike",
    (name, code) => {
      const asText = paste({ "text/plain": code })
      const textNotices = [...notices]
      const asHtml = paste({ "text/html": code, "text/plain": code })
      expect(asHtml).toBe(asText)
      expect(notices).toEqual(textNotices)
      expect(textNotices).toEqual(
        DROPS.has(name) ? [YOUTUBE_NOTICES.optionsDropped] : []
      )
      const iframe = new DOMParser()
        .parseFromString(asText, "text/html")
        .querySelector("p[data-youtube-video] > iframe")
      expect(iframe, name).not.toBeNull()
      // D132: an 11-character id, or a playlist's videoseries with its list.
      expect(iframe!.getAttribute("src")).toMatch(
        /^https:\/\/www\.youtube-nocookie\.com\/embed\/(?:[A-Za-z0-9_-]{11}(?:\?start=[0-9]+)?(?:[?&]list=[A-Za-z0-9_-]+)?|videoseries\?list=[A-Za-z0-9_-]+)$/
      )
      expect(iframe!.getAttribute("title")).toBe(titleOf(code))
    }
  )
})

describe("the plain-text fallback (editor.md §5.5)", () => {
  const reportError = vi.fn()
  const throwing: PasteFilter = () => {
    throw new Error("unforeseen input")
  }
  const { editor, notices, paste } = pasteEditor((notify) => [
    ...siteExtensions({ policy, notify }).filter(
      ({ name }) => name !== OFFICE_PASTE_EXTENSION
    ),
    createOfficePaste({ policy, notify, filter: throwing, reportError }),
  ])
  afterAll(() => editor.destroy())

  it("pastes the text alone, shows the notice once and reports the error with the source", () => {
    expect(
      paste({
        "text/html":
          "<p>első <b>sor</b></p><p>második &lt;sor&gt;</p><table><tr><td>a</td></tr></table>",
      })
    ).toBe("<p>első sor</p><p>második &lt;sor&gt;</p><p>a</p>")
    expect(notices).toEqual([PASTE_NOTICES.fallback])
    expect(reportError).toHaveBeenCalledOnce()
    expect(reportError.mock.calls[0][0]).toContain("generic HTML")
    expect(reportError.mock.calls[0][1]).toEqual(new Error("unforeseen input"))
  })
})
