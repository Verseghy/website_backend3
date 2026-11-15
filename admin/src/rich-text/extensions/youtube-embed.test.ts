import { Editor } from "@tiptap/core"
import { afterEach, describe, expect, it } from "vitest"

import { loadDocument } from "../load-document"
import { CONTENT_V1_POLICY, PolicyError } from "../policy"
import { toStoredHtml } from "../serialize/to-stored-html"
import { siteExtensions } from "../site-extensions"
import type { EditorNotice } from "./editor-notice"
import type { YoutubeEmbedAttributes } from "./youtube-embed"
import {
  createYoutubeEmbed,
  pasteNotice,
  YOUTUBE_EMBED_NODE,
  YOUTUBE_STYLE,
} from "./youtube-embed"
import { policyWith } from "@/test/rich-text-policy"
import { parseHtml, pasteEvent, siteSchema } from "@/test/site-schema"
import { storedHtmlHarness } from "@/test/stored-html"
import { youtubeFixture, youtubeFixtures } from "@/test/youtube-fixtures"

const FORCED =
  'allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"'

/** The stored YouTube shape of editor.md §3.7 for a src and a title. */
function stored(src: string, title: string | null = "Videó"): string {
  const titled = title === null ? "" : ` title="${title}"`
  return `<p data-youtube-video=""><iframe src="${src}" width="640" height="360"${titled} style="${YOUTUBE_STYLE}" ${FORCED}></iframe></p>`
}

const VIDEO = "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ"

describe("the YouTube embed's stored shape (editor.md §3.7)", () => {
  const harness = storedHtmlHarness()
  afterEach(() => harness.load("<p></p>"))

  it("writes the §3.7 style, with ADR 0043's min-height", () => {
    expect(YOUTUBE_STYLE).toBe(
      "aspect-ratio: 16 / 9; height: auto; max-width: 640px; min-height: 200px; width: 100%"
    )
  })

  it.each([
    stored(`${VIDEO}?start=30`),
    stored(`${VIDEO}?start=0&amp;list=PL_a-1`),
    // The server keeps any digits (§3.6), so the node keeps them as written.
    stored(`${VIDEO}?start=007`),
    stored(`${VIDEO}?start=99999999999999999999`),
    stored(
      "https://www.youtube-nocookie.com/embed/videoseries?list=PLMOZDTW7ZvXM"
    ),
    // A stored title is kept as it is, or none (D135).
    stored(VIDEO, null),
    stored(VIDEO, "YouTube video player"),
    stored(VIDEO, "  Saját &amp; cím  "),
  ])("is a fixed point: %s", (body) => {
    expect(harness.J(body)).toBe(body)
  })
})

describe("the YouTube parse rules", () => {
  const schema = siteSchema()
  const attributesOf = (html: string) => {
    const doc = parseHtml(schema, html)
    const found: unknown[] = []
    doc.forEach((node) => {
      if (node.type.name === YOUTUBE_EMBED_NODE) {
        found.push(node.attrs)
      }
    })
    return found
  }
  const embed = (
    videoId: string,
    rest: Partial<YoutubeEmbedAttributes> = {}
  ): YoutubeEmbedAttributes => ({
    videoId,
    start: null,
    list: null,
    title: null,
    ...rest,
  })

  it.each<[string, string, YoutubeEmbedAttributes]>([
    [
      "the stored wrapper (70)",
      `<p data-youtube-video=""><iframe src="${VIDEO}?start=5" title="T"></iframe></p>`,
      embed("dQw4w9WgXcQ", { start: "5", title: "T" }),
    ],
    [
      "another TipTap install's wrapper (70)",
      '<div data-youtube-video=""><iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ?si=x&amp;rel=0"></iframe></div>',
      embed("dQw4w9WgXcQ"),
    ],
    [
      "a legacy paragraph with one iframe (65)",
      '<p>\n<iframe width="560" height="315" src="https://www.youtube.com/embed/dQw4w9WgXcQ" frameborder="0"></iframe>\n</p>',
      embed("dQw4w9WgXcQ"),
    ],
    [
      "a bare iframe (60)",
      youtubeFixture("youtube-embed-context-menu-playlist"),
      embed("71ya_JC_IbM", {
        list: "PLMOZDTW7ZvXM",
        title: "20 years of esports history explained",
      }),
    ],
    [
      "an http iframe (D136)",
      youtubeFixture("youtube-help-start-http"),
      embed("UkWd0azv3fQ", { start: "90" }),
    ],
    [
      "the documented playlist form (D129)",
      '<iframe src="https://www.youtube.com/embed?listType=playlist&amp;list=PL1"></iframe>',
      embed("videoseries", { list: "PL1" }),
    ],
  ])("read %s", (_name, html, expected) => {
    expect(attributesOf(html)).toEqual([expected])
  })

  it.each([
    '<p data-youtube-video=""><iframe src="https://www.youtube.com/embed/abc"></iframe></p>',
    '<p data-youtube-video=""><iframe></iframe></p>',
    '<iframe src="https://www.youtube.com/embed/videoseries"></iframe>',
    '<iframe src="https://www.youtube.com/embed?listType=user_uploads&amp;list=x"></iframe>',
    '<iframe src="https://evil.example/embed/dQw4w9WgXcQ"></iframe>',
    '<p>szöveg <iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ"></iframe></p>',
  ])("make no YouTube embed of %s", (html) => {
    expect(attributesOf(html)).toEqual(
      html.includes("szöveg") ? [embed("dQw4w9WgXcQ")] : []
    )
  })

  it("store every research fixture's code as its canonical embed", () => {
    const harness = storedHtmlHarness()
    for (const { name, kind, text } of youtubeFixtures()) {
      if (kind === "code") {
        const html = harness.J(text)
        expect(html, name).toMatch(
          /^<p data-youtube-video=""><iframe src="https:\/\/www\.youtube-nocookie\.com\/embed\/[^"]+" width="640" height="360"( title="[^"]*")? style="aspect-ratio/
        )
        expect(harness.J(html), name).toBe(html)
      }
    }
    harness.destroy()
  })
})

describe("the YouTube render rule", () => {
  const schema = siteSchema()

  it("writes no address the server would refuse", () => {
    const node = schema.nodes[YOUTUBE_EMBED_NODE].create({
      videoId: "abc",
      start: null,
      list: null,
      title: "Videó",
    })
    const html = toStoredHtml(schema.nodes.doc.create(null, [node]), {
      policy: CONTENT_V1_POLICY,
      document,
    })
    expect(html).not.toContain("src=")
    expect(html).toContain('title="Videó"')
  })
})

describe("createYoutubeEmbed", () => {
  it("refuses a policy that would store the player's style differently", () => {
    const policy = policyWith((json) => {
      const elements = json.elements as {
        iframe: { style: { property: string }[] }
      }
      elements.iframe.style = elements.iframe.style.filter(
        (rule) => rule.property !== "min-height"
      )
    })
    expect(() => createYoutubeEmbed(policy, () => {})).toThrow(PolicyError)
    expect(() => createYoutubeEmbed(policy, () => {})).toThrow("min-height")
  })
})

describe("insertYoutubeEmbed", () => {
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
  })
  const save = () =>
    toStoredHtml(editor.state.doc, { policy: CONTENT_V1_POLICY, document })

  it("inserts a valid video and replaces an empty paragraph", () => {
    loadDocument(editor, "<p></p>")
    expect(
      editor.commands.insertYoutubeEmbed({
        videoId: "dQw4w9WgXcQ",
        start: "90",
        list: null,
        title: "Videó",
      })
    ).toBe(true)
    expect(save()).toBe(stored(`${VIDEO}?start=90`))
  })

  it("splits a paragraph around the embed: media lives at the top level", () => {
    loadDocument(editor, "<p>előtte</p>")
    editor.commands.setTextSelection(4)
    editor.commands.insertYoutubeEmbed({
      videoId: "videoseries",
      start: null,
      list: "PL1",
      title: "Videó",
    })
    expect(save()).toBe(
      `<p>elő</p>${stored("https://www.youtube-nocookie.com/embed/videoseries?list=PL1")}<p>tte</p>`
    )
  })

  it.each([
    { videoId: "abc", start: null, list: null, title: "Videó" },
    { videoId: "videoseries", start: null, list: null, title: "Videó" },
    { videoId: "dQw4w9WgXcQ", start: "-1", list: null, title: "Videó" },
  ])("refuses %j and changes nothing", (attributes) => {
    loadDocument(editor, "<p>x</p>")
    expect(editor.commands.insertYoutubeEmbed(attributes)).toBe(false)
    expect(save()).toBe("<p>x</p>")
  })
})

describe("the lone-URL paste rule (D130, D131, D134, D135)", () => {
  const notices: EditorNotice[] = []
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: siteExtensions({
      policy: CONTENT_V1_POLICY,
      notify: (notice) => notices.push(notice),
    }),
  })
  const save = () =>
    toStoredHtml(editor.state.doc, { policy: CONTENT_V1_POLICY, document })
  /** Pastes text through a DOM paste event, as a browser does. */
  const paste = (text: string, start = "<p></p>") => {
    notices.length = 0
    loadDocument(editor, start)
    // The end of the last paragraph's text.
    editor.commands.setTextSelection(editor.state.doc.content.size - 1)
    editor.view.dom.dispatchEvent(pasteEvent({ "text/plain": text }))
    return save()
  }

  it.each(
    youtubeFixtures()
      .filter(({ kind }) => kind === "link")
      .map(({ name }) => name)
  )("embeds the link %s, titled „Videó”", (name) => {
    const html = paste(youtubeFixture(name))
    expect(html).toMatch(
      /^<p data-youtube-video=""><iframe src="https:\/\/www\.youtube-nocookie\.com\/embed\/[^"]+" width="640" height="360" title="Videó" style=/
    )
  })

  it("gives the share link with a start time its start", () => {
    expect(paste(youtubeFixture("youtube-share-link-video-start"))).toBe(
      stored(`${VIDEO}?start=90`)
    )
    expect(notices).toEqual([])
  })

  it("shows the notice when it drops a playback option", () => {
    expect(paste(youtubeFixture("youtube-documented-embed-loop"))).toBe(
      stored(VIDEO)
    )
    expect(notices).toEqual(["editor.youtube.options_dropped"])
  })

  it("keeps a link it cannot embed as text and says why", () => {
    expect(paste("https://youtu.be/abc")).toBe("<p>https://youtu.be/abc</p>")
    expect(notices).toEqual(["editor.youtube.video_id"])
    expect(paste("https://www.youtube.com/embed?listType=search&list=x")).toBe(
      "<p>https://www.youtube.com/embed?listType=search&amp;list=x</p>"
    )
    expect(notices).toEqual(["editor.youtube.list_type"])
  })

  it("leaves a URL inside a longer paste as text", () => {
    expect(
      paste(`nézd meg: ${youtubeFixture("youtube-share-link-video")}`)
    ).toBe("<p>nézd meg: https://youtu.be/dQw4w9WgXcQ?si=invented_si-0001</p>")
    expect(
      paste(`első sor\n${youtubeFixture("youtube-share-link-video")}`)
    ).not.toContain("data-youtube-video")
    expect(paste("https://example.org/")).toBe("<p>https://example.org/</p>")
    expect(notices).toEqual([])
  })

  it.each([
    ["a heading", "<h2></h2>", "<h2>%</h2>"],
    ["a list item", "<ul><li></li></ul>", "<ul><li>%</li></ul>"],
    ["a quote", "<blockquote></blockquote>", "<blockquote>%</blockquote>"],
    [
      "a table cell",
      '<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td></td><td>b</td></tr></tbody></table>',
      '<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td>%</td><td>b</td></tr></tbody></table>',
    ],
  ])(
    "keeps a link pasted in %s as text and says why, as media goes only at the top level",
    (_place, body, expected) => {
      notices.length = 0
      loadDocument(editor, body)
      let textblock = -1
      editor.state.doc.descendants((node, pos) => {
        if (textblock < 0 && node.isTextblock) {
          textblock = pos + 1
        }
      })
      editor.commands.setTextSelection(textblock)
      const link = youtubeFixture("youtube-share-link-short")
      editor.view.dom.dispatchEvent(pasteEvent({ "text/plain": link }))
      expect(save()).toBe(expected.replace("%", link.trim()))
      expect(notices).toEqual(["editor.youtube.not_top_level"])
    }
  )

  it("inserts the embed between the halves of the paragraph it lands in", () => {
    expect(paste(youtubeFixture("youtube-share-link-short"), "<p>ab</p>")).toBe(
      `<p>ab</p>${stored("https://www.youtube-nocookie.com/embed/B5T4ZCfxp1Y")}`
    )
  })
})

describe("pasteNotice", () => {
  it("names the message of each reading", () => {
    expect(pasteNotice({ kind: "none" })).toBeNull()
    expect(pasteNotice({ kind: "refused", reason: "video_id" })).toBe(
      "editor.youtube.video_id"
    )
    expect(pasteNotice({ kind: "refused", reason: "list_type" })).toBe(
      "editor.youtube.list_type"
    )
    const video = { videoId: "dQw4w9WgXcQ", start: null, list: null }
    expect(
      pasteNotice({ kind: "video", video, title: null, dropped: [] })
    ).toBeNull()
    expect(
      pasteNotice({ kind: "video", video, title: null, dropped: ["end"] })
    ).toBe("editor.youtube.options_dropped")
  })
})
