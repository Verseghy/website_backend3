import { describe, expect, it } from "vitest"

import { padletRules } from "../padlet-input"
import { CONTENT_V1_POLICY } from "../policy"
import { youtubeRules } from "../youtube-input"
import { placeMedia } from "./media-placement"
import type { PasteContext } from "./paste-context"
import { emptyReportDraft } from "./paste-context"

const policy = CONTENT_V1_POLICY
const parser = new DOMParser()

/** Places the media of `html` (already step-10 output) and returns the result. */
function place(html: string) {
  const document = parser.parseFromString(html, "text/html")
  const context: PasteContext = {
    policy,
    source: "generic",
    youtube: youtubeRules(policy),
    padlet: padletRules(policy),
    report: emptyReportDraft(),
  }
  placeMedia(document.body, context)
  return { html: document.body.innerHTML, moved: context.report.moved }
}

const VIDEO = "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ"
const YOUTUBE = `<iframe src="${VIDEO}" title="Videó"></iframe>`
const IMAGE = '<img src="/storage/uploads/a.jpg" alt="kép">'
const PADLET = '<iframe src="https://padlet.com/embed/invented0key"></iframe>'
const youtubeParagraph = `<p data-youtube-video="">${YOUTUBE}</p>`

describe("placeMedia (editor.md §5.2 step 11a)", () => {
  it("wraps media that is already alone, and media loose in the body, in the canonical wrapper", () => {
    expect(place(`<p style="text-align: center">${IMAGE}</p>`)).toEqual({
      html: `<p>${IMAGE}</p>`,
      moved: [],
    })
    expect(place(`${PADLET}<p>a</p>`)).toEqual({
      html: `<p data-padlet-embed="">${PADLET}</p><p>a</p>`,
      moved: [],
    })
    expect(place(youtubeParagraph).html).toBe(youtubeParagraph)
  })

  it.each([
    [
      "td",
      `<table><tbody><tr><td>a${YOUTUBE}</td></tr></tbody></table>`,
      `${youtubeParagraph}<table><tbody><tr><td>a</td></tr></tbody></table>`,
    ],
    [
      "li",
      `<ul><li>pont ${IMAGE}</li></ul>`,
      `<p>${IMAGE}</p><ul><li>pont </li></ul>`,
    ],
    [
      "blockquote",
      `<blockquote>${PADLET}idézet</blockquote>`,
      `<p data-padlet-embed="">${PADLET}</p><blockquote>idézet</blockquote>`,
    ],
    ["h2", `<h2>Cím ${YOUTUBE}</h2>`, `${youtubeParagraph}<h2>Cím </h2>`],
  ])(
    "moves media out of a %s before its top-level block",
    (from, html, expected) => {
      expect(place(html)).toEqual({
        html: expected,
        moved: [
          {
            media: html.includes("<img")
              ? "image"
              : html.includes("padlet")
                ? "padlet"
                : "youtube",
            from,
          },
        ],
      })
    }
  )

  it("drops a link around media and the paragraph it leaves empty", () => {
    expect(
      place(`<p><a href="/x"><strong>${IMAGE}</strong></a></p><p>b</p>`)
    ).toEqual({
      html: `<p>${IMAGE}</p><p>b</p>`,
      moved: [{ media: "image", from: "a" }],
    })
  })

  it("keeps the text around a link it takes media out of", () => {
    expect(
      place(`<p>előtte <a href="/x">link ${IMAGE}</a> utána</p>`).html
    ).toBe(`<p>${IMAGE}</p><p>előtte <a href="/x">link </a> utána</p>`)
  })

  it("splits running text around media, both halves keeping the paragraph's attributes", () => {
    expect(
      place(
        `<p style="text-align: right">első <em>dőlt ${YOUTUBE} még</em> vége</p>`
      )
    ).toEqual({
      html: `<p style="text-align: right">első <em>dőlt</em></p>${youtubeParagraph}<p style="text-align: right"><em>még</em> vége</p>`,
      moved: [{ media: "youtube", from: "text" }],
    })
  })

  it("drops a half with no text and no hard break", () => {
    expect(place(`<p>  ${IMAGE} szöveg</p>`).html).toBe(
      `<p>${IMAGE}</p><p>szöveg</p>`
    )
    expect(place(`<p>sor<br>${IMAGE}</p>`).html).toBe(
      `<p>sor<br></p><p>${IMAGE}</p>`
    )
  })

  it("records nothing for media alone in inline wrappers", () => {
    expect(
      place(`<p><span style="color: #c00000">${IMAGE}</span></p>`)
    ).toEqual({
      html: `<p>${IMAGE}</p>`,
      moved: [],
    })
  })

  it("gives a YouTube embed the title rule of D135", () => {
    expect(
      place(`<iframe src="${VIDEO}" title=" YouTube video player "></iframe>`)
        .html
    ).toBe(youtubeParagraph)
    expect(place(`<iframe src="${VIDEO}"></iframe>`).html).toBe(
      youtubeParagraph
    )
    expect(
      place(`<iframe src="${VIDEO}" title="Saját cím"></iframe>`).html
    ).toBe(
      `<p data-youtube-video=""><iframe src="${VIDEO}" title="Saját cím"></iframe></p>`
    )
  })

  it("keeps the editor's own slice marker on the paragraph that replaces its first one", () => {
    expect(place(`<p data-pm-slice="0 0 []">${IMAGE}</p><p>b</p>`).html).toBe(
      `<p data-pm-slice="0 0 []">${IMAGE}</p><p>b</p>`
    )
    expect(
      place(`<p data-pm-slice="0 0 []" data-youtube-video="">${YOUTUBE}</p>`)
        .html
    ).toBe(`<p data-youtube-video="" data-pm-slice="0 0 []">${YOUTUBE}</p>`)
  })

  it("places several media in document order", () => {
    expect(place(`<p>a ${IMAGE} b ${PADLET} c</p>`).html).toBe(
      `<p>a</p><p>${IMAGE}</p><p>b</p><p data-padlet-embed="">${PADLET}</p><p>c</p>`
    )
  })
})
