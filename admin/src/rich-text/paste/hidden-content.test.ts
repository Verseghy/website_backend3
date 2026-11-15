import { describe, expect, it } from "vitest"

import { isHiddenElement, removeHiddenContent } from "./hidden-content"
import { inlineStyleSheets } from "./inline-styles"

const parser = new DOMParser()

/** Parses, inlines (step 6) and returns the body. */
function body(html: string): HTMLElement {
  const document = parser.parseFromString(html, "text/html")
  inlineStyleSheets(document)
  return document.body
}

/**
 * Whether the tests run under jsdom (`npm test`) rather than in a browser
 * (`npm run test:browser`). jsdom's CSSOM (cssstyle) drops a declaration
 * whose property name is not lower case, which CSS reads case-insensitively
 * and every engine keeps.
 */
const JSDOM = navigator.userAgent.includes("jsdom")

describe("isHiddenElement", () => {
  const upperCaseName: [string, boolean][] = JSDOM
    ? []
    : [["DISPLAY: None", true]]
  it.each<[string, boolean]>([
    ["display: none", true],
    ["display: NONE !important", true],
    ...upperCaseName,
    ["visibility: hidden", true],
    ["visibility: collapse", true],
    ["display: block", false],
    ["visibility: visible", false],
    ["", false],
  ])("reads %j as hidden: %s", (style, hidden) => {
    const element = body(`<p style="${style}">a</p>`).querySelector("p")!
    expect(isHiddenElement(element)).toBe(hidden)
  })
})

describe("removeHiddenContent (editor.md §5.2 step 7)", () => {
  it("drops Excel's hidden rows and hidden column cells, and counts them", () => {
    const root = body(
      "<style>.hid { display: none }</style><table><tr><td>a</td><td style='display:none'>x</td></tr><tr class=hid><td>rejtett</td><td>sor</td></tr><tr><td>b</td><td style='display: none'>y</td></tr></table>"
    )
    expect(removeHiddenContent(root)).toEqual({ rows: 1, cells: 2, other: 0 })
    expect(root.textContent).toBe("ab")
  })

  it("drops other hidden elements with their content, counting only the outermost", () => {
    const root = body(
      "<p>látható</p><div style='visibility: hidden'><p>rejtett</p><span style='display:none'>belső</span></div>"
    )
    expect(removeHiddenContent(root)).toEqual({ rows: 0, cells: 0, other: 1 })
    expect(root.innerHTML).toBe("<p>látható</p>")
  })

  it("leaves a document without hidden content unchanged", () => {
    const root = body("<p>a</p><table><tr><td>b</td></tr></table>")
    const before = root.innerHTML
    expect(removeHiddenContent(root)).toEqual({ rows: 0, cells: 0, other: 0 })
    expect(root.innerHTML).toBe(before)
  })
})
