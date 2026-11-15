import fc from "fast-check"
import { describe, expect, it } from "vitest"

import type { RichTextPolicy } from "../policy"
import { CONTENT_V1_POLICY } from "../policy"
import {
  normalizePastedHtml,
  preCleanPastedHtml,
} from "./normalize-pasted-html"
import { SLICE_MARKER } from "./paste-source"
import { clipboardHtml } from "@/test/paste-arbitrary"
import { pasteFixtures } from "@/test/paste-fixtures"
import { pasteViolations } from "@/test/paste-invariants"
import { youtubeFixture } from "@/test/youtube-fixtures"

const policy = CONTENT_V1_POLICY
const parser = new DOMParser()
const normalize = (html: string) =>
  normalizePastedHtml(html, { policy, parser })

/**
 * A table as the filter writes it: the forced `table` and `tbody` styles of
 * editor.md §3.3 are left to the schema, which writes them on every table.
 */
const TABLE = "<table><tbody>"
const END = "</tbody></table>"

describe("preCleanPastedHtml (editor.md §5.2 step 2)", () => {
  it.each([
    [
      "<html><body>a</body></html>\r\n<!--EndFragment-->x",
      "<html><body>a</body></html>",
    ],
    ["<html><body>a</body></HTML >trailing", "<html><body>a</body></HTML >"],
    ["<p>no document</p>", "<p>no document</p>"],
    [
      "<table><!--StartFragment--><tr><td>a</td></tr><!--EndFragment--></table>",
      "<table><!--StartFragment--><tr><td>a</td></tr><!--EndFragment--></table>",
    ],
  ])("cuts %j after </html> only", (html, expected) => {
    expect(preCleanPastedHtml(html)).toBe(expected)
  })
})

describe("normalizePastedHtml's options", () => {
  it.each<[string, () => unknown, string]>([
    [
      "a non-string",
      () => normalizePastedHtml(42 as unknown as string, { policy, parser }),
      "html must be the clipboard's text/html as a string",
    ],
    [
      "no policy",
      () =>
        normalizePastedHtml("", {
          policy: undefined as unknown as RichTextPolicy,
          parser,
        }),
      "options.policy is required",
    ],
    [
      "no parser",
      () =>
        normalizePastedHtml("", {
          policy,
          parser: {} as DOMParser,
        }),
      "options.parser must be a DOMParser",
    ],
  ])("refuses %s", (_name, call, message) => {
    expect(call).toThrow(TypeError)
    expect(call).toThrow(message)
  })
})

describe("normalizePastedHtml (editor.md §5.2)", () => {
  it("inlines class rules into cells and maps them onto content-v1", () => {
    expect(
      normalize(
        "<style>.xl65 { background: #CCCCCC; text-align: center; font-family: Calibri }</style><table><tr><td class=xl65>a</td></tr></table>"
      )
    ).toEqual({
      html: `${TABLE}<tr><td style="background-color: #cccccc; text-align: center; white-space: normal">a</td></tr>${END}`,
      source: "generic",
      report: {
        moved: { media: [] },
        dropped: {
          hiddenRows: 0,
          hiddenCells: 0,
          hiddenOther: 0,
          fontFamilies: ["Calibri"],
          frames: 0,
          youtubeRefusals: [],
          embedParams: [],
        },
      },
    })
  })

  it("drops hidden rows and reports them", () => {
    const { html, report } = normalize(
      "<style>.h { display: none }</style><table><tr><td>látható</td></tr><tr class=h><td>rejtett</td></tr></table>"
    )
    expect(html).not.toContain("rejtett")
    expect(report.dropped.hiddenRows).toBe(1)
  })

  it("keeps the editor's own clipboard as written, with its slice marker", () => {
    expect(
      normalize(
        `<meta charset="utf-8"><p ${SLICE_MARKER}="1 1 []">a <strong>b</strong> <span style="color: rgb(192, 0, 0)">c</span></p>`
      )
    ).toMatchObject({
      html: `<p ${SLICE_MARKER}="1 1 []">a <strong>b</strong> <span style="color: #c00000">c</span></p>`,
      source: "internal",
    })
  })

  it("does not inline or hide on the editor's own clipboard", () => {
    const { html, report } = normalize(
      `<style>p { display: none }</style><p ${SLICE_MARKER}="0 0 []">a</p>`
    )
    expect(html).toBe(`<p ${SLICE_MARKER}="0 0 []">a</p>`)
    expect(report.dropped.hiddenOther).toBe(0)
  })

  it("moves an embed out of a cell and reports the move and the dropped option", () => {
    const { html, report } = normalize(
      '<table><tr><td>x<iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1"></iframe></td></tr></table>'
    )
    expect(html).toBe(
      `<p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ" title="Videó"></iframe></p>${TABLE}<tr><td style="white-space: normal">x</td></tr>${END}`
    )
    expect(report.moved.media).toEqual([{ media: "youtube", from: "td" }])
    expect(report.dropped.embedParams).toEqual(["autoplay"])
  })

  it("takes an http YouTube iframe on an allowed host and path as an embed, with the title rule (D135, D136)", () => {
    const { html, report } = normalize(
      `<p>előtte</p>${youtubeFixture("youtube-help-start-http")}<p>utána</p>`
    )
    expect(html).toBe(
      '<p>előtte</p><p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/UkWd0azv3fQ?start=90" width="420" height="315" title="Videó"></iframe></p><p>utána</p>'
    )
    expect(report.dropped).toMatchObject({ frames: 0, embedParams: [] })
  })

  it.each<[string, string[]]>([
    ["http://www.youtube.com/watch?v=dQw4w9WgXcQ", []],
    ["http://evil.example/embed/dQw4w9WgXcQ", []],
    ["http://www.youtube.com/embed/abc", ["video_id"]],
    ["https://www.youtube.com/embed?listType=search&list=x", ["list_type"]],
  ])(
    "removes and counts an iframe at %s, which is no embed address",
    (src, refusals) => {
      const { html, report } = normalize(
        `<p>a</p><iframe src="${src}"></iframe>`
      )
      expect(html).toBe("<p>a</p>")
      expect(report.dropped.frames).toBe(1)
      // A refused YouTube address says why (§4.3 row 9); another is silent.
      expect(report.dropped.youtubeRefusals).toEqual(refusals)
    }
  )

  it("lists the font families it drops (§5.2 step 12)", () => {
    const { html, report } = normalize(
      '<style>p { font-family: "Times New Roman", serif }</style><p>a <span style="font-family: Arial">b</span></p>'
    )
    expect(html).toBe("<p>a b</p>")
    expect(report.dropped.fontFamilies).toHaveLength(2)
    expect(report.dropped.fontFamilies[1]).toBe("Arial")
  })

  it("leaves a print style sheet and a <noscript> style out, so nothing they hide is dropped", () => {
    const { html, report } = normalize(
      '<style media="print">.x { display: none }</style><noscript><style>p { display: none }</style></noscript><p class="x">a</p><p>b</p>'
    )
    expect(html).toBe("<p>a</p><p>b</p>")
    expect(report.dropped.hiddenOther).toBe(0)
  })

  it("reads Excel for the web's document, whose style sits in the body", () => {
    const excel = pasteFixtures().find(
      ({ name }) => name === "roosterjs-excel-online-e2e"
    )!
    const { html, source } = normalize(excel.html)
    expect(source).toBe("generic")
    expect(html).not.toMatch(/<style|<!--|<div|<col/)
    expect(html.match(/<td /g)).toHaveLength(2)
    expect(html).toContain("height: 20px")
    expect(html).toContain("width: 64px")
    expect(html).toContain("vertical-align: bottom")
  })

  it.each(pasteFixtures().map(({ name, html }) => [name, html]))(
    "writes only content-v1 for %s",
    (_name, html) => {
      expect(pasteViolations(normalize(html).html, policy)).toEqual([])
    }
  )

  // 300 runs of the whole filter outlast Vitest's default 5 s on a busy
  // machine.
  it("writes only content-v1 for any clipboard HTML, and never throws", () => {
    fc.assert(
      fc.property(clipboardHtml, (html) => {
        const { html: output, source } = normalize(html)
        expect(
          pasteViolations(output, policy, {
            sliceMarker: source === "internal",
          })
        ).toEqual([])
      }),
      { numRuns: 300 }
    )
  }, 30_000)
})
