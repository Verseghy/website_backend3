import fc from "fast-check"
import { describe, expect, it } from "vitest"

import {
  detectPasteSource,
  EXCEL_NAMESPACE,
  keptSliceMarker,
  PASTE_SOURCES,
  SLICE_MARKER,
  WORD_NAMESPACE,
} from "./paste-source"

const parser = new DOMParser()
const detect = (html: string) =>
  detectPasteSource(parser.parseFromString(html, "text/html"))

describe("detectPasteSource (editor.md §5.2 step 1)", () => {
  it("lists the four sources in the order step 1 checks them", () => {
    expect(PASTE_SOURCES).toEqual(["word", "excel", "internal", "generic"])
  })

  it.each([
    [
      "Word's namespace on the root",
      `<html xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:w="${WORD_NAMESPACE}"><body><p class=MsoNormal>a</p></body></html>`,
    ],
    [
      "Word's namespace in another case and with spaces",
      `<HTML xmlns:w=" URN:schemas-microsoft-com:office:WORD "><body>a</body></HTML>`,
    ],
    [
      "Word's ProgId without the namespace",
      "<html><head><meta name=ProgId content=Word.Document></head><body>a</body></html>",
    ],
    [
      "Word's ProgId with the content first",
      '<meta content="word.document" name="progid"><p>a</p>',
    ],
    [
      "Word HTML after a clipboard header",
      `Version:0.9\r\nStartHTML:0000000105\r\n<html xmlns:w="${WORD_NAMESPACE}"><body>a</body></html>`,
    ],
  ])("detects Word desktop: %s", (_name, html) => {
    expect(detect(html)).toBe("word")
  })

  it("detects Excel desktop by its namespace", () => {
    expect(
      detect(
        `<html xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="${EXCEL_NAMESPACE}"><body><table><tr><td>1</td></tr></table></body></html>`
      )
    ).toBe("excel")
  })

  it("sends Excel for the web, which has Excel's ProgId but no namespace, down the generic path", () => {
    expect(
      detect(
        "<html><body><div ccp_infra_version='3'><html><head><meta name=ProgId content=Excel.Sheet></head><body><table><tr><td>1</td></tr></table></body></html></div></body></html>"
      )
    ).toBe("generic")
  })

  it("detects Word before Excel when a document carries both namespaces", () => {
    expect(
      detect(
        `<html xmlns:w="${WORD_NAMESPACE}" xmlns:x="${EXCEL_NAMESPACE}"><body>a</body></html>`
      )
    ).toBe("word")
  })

  it("detects the editor's own clipboard by ProseMirror's slice marker", () => {
    expect(detect(`<p ${SLICE_MARKER}="1 1 []">a</p>`)).toBe("internal")
    expect(
      detect(
        `<meta charset="utf-8"><table ${SLICE_MARKER}="1 1 [&quot;table&quot;,null]"><tbody><tr><td>a</td></tr></tbody></table>`
      )
    ).toBe("internal")
  })

  it.each([
    [
      "Google Docs",
      '<meta charset="utf-8"><b style="font-weight:normal;" id="docs-internal-guid-0a1b2c3d"><p dir="ltr">a</p></b>',
    ],
    ["a web page", "<h2>Cím</h2><p>szöveg</p>"],
    ["an empty clipboard", ""],
    [
      "a namespace URN that is only text",
      `<p>xmlns:w="${WORD_NAMESPACE}" and data-pm-slice</p>`,
    ],
    [
      "a namespace URN in an attribute value",
      `<p title='xmlns:w="${WORD_NAMESPACE}"'>a</p>`,
    ],
    [
      "another namespace on the w prefix",
      '<html xmlns:w="urn:other"><body>a</body></html>',
    ],
  ])("sends %s down the generic path", (_name, html) => {
    expect(detect(html)).toBe("generic")
  })

  it("returns one of the four sources for any string", () => {
    fc.assert(
      fc.property(fc.string({ maxLength: 200 }), (html) => {
        expect(PASTE_SOURCES).toContain(detect(html))
      }),
      { numRuns: 200 }
    )
  })
})

describe("keptSliceMarker (editor.md §5.5, the editor's own clipboard)", () => {
  /** prosemirror-view's own reading of the marker (`parseFromClipboard`). */
  const PROSEMIRROR_READING = /^(\d+) (\d+)(?: -(\d+))? (.*)/

  it.each([
    ["0 0 []", "0 0 []"],
    ["1 1 []", "1 1 []"],
    ['1 1 ["blockquote",null]', "1 1 []"],
    ['2 2 -1 ["bulletList",null,"listItem",null]', "2 2 -1 []"],
    ["007 1 -03 []", "7 1 -3 []"],
  ])("keeps %s as %s", (raw, kept) => {
    expect(keptSliceMarker(raw)).toBe(kept)
  })

  it("leaves out the context, so no node is built from its attributes", () => {
    expect(
      keptSliceMarker(
        '1 1 ["table",{"style":"x"},"tableRow",null,"tableCell",{"colspan":2,"style":"background-image: url(https://evil.example/)"}]'
      )
    ).toBe("1 1 []")
  })

  it.each([
    ["no context", "1 1"],
    ["a context that is no JSON", "1 1 not json"],
    ["a context that is no array", '1 1 {"table":null}'],
    ["an odd context", '1 1 ["table"]'],
    ["a node name that is no string", '1 1 [{"x":1},null]'],
    ["a node name that is no identifier", '1 1 ["table cell",null]'],
    ["a depth of four digits", "1000 1 []"],
    ["a negative depth", "-1 1 []"],
    ["an empty value", ""],
  ])("drops a marker with %s", (_name, raw) => {
    expect(keptSliceMarker(raw)).toBeNull()
  })

  it("keeps only values prosemirror-view reads, with an empty context", () => {
    const name = fc.constantFrom("table", "tableRow", "paragraph", "x y", "")
    const attrs = fc.constantFrom<unknown>(null, {}, { style: "color: red" }, 1)
    fc.assert(
      fc.property(
        fc.nat({ max: 1200 }),
        fc.nat({ max: 1200 }),
        fc.option(fc.nat({ max: 5 })),
        fc.array(fc.tuple(name, attrs), { maxLength: 4 }),
        (start, end, wrappers, context) => {
          const raw = `${start} ${end}${wrappers === null ? "" : ` -${wrappers}`} ${JSON.stringify(context.flat())}`
          const kept = keptSliceMarker(raw)
          const valid =
            start < 1000 &&
            end < 1000 &&
            context.every(([n]) => /^[A-Za-z][A-Za-z0-9_]*$/.test(n))
          expect(kept === null).toBe(!valid)
          if (kept !== null) {
            const match = PROSEMIRROR_READING.exec(kept)
            expect(match).not.toBeNull()
            expect([match![1], match![2], match![3]]).toEqual([
              String(start),
              String(end),
              wrappers === null ? undefined : String(wrappers),
            ])
            expect(match![4]).toBe("[]")
            expect(keptSliceMarker(kept)).toBe(kept)
          }
        }
      )
    )
  })
})
