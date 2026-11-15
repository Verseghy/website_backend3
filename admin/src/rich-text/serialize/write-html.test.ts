import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { element, text } from "./stored-tree"
import type { StoredNode } from "./stored-tree"
import {
  escapeHtml,
  RAW_TEXT_ELEMENTS,
  VOID_ELEMENTS,
  writeHtml,
} from "./write-html"
import { regressionFixtures, reparse } from "@/test/stored-html"

describe("escapeHtml", () => {
  it.each<[string, string, boolean, string]>([
    ["text", "a & b < c > d", false, "a &amp; b &lt; c &gt; d"],
    ["a quote in text, kept", 'say "hi"', false, 'say "hi"'],
    ["a quote in an attribute", 'say "hi"', true, "say &quot;hi&quot;"],
    ["U+00A0 in text", "a\u00a0b", false, "a&nbsp;b"],
    ["U+00A0 in an attribute", "a\u00a0b", true, "a&nbsp;b"],
    ["an apostrophe, kept", "it's", true, "it's"],
    [
      "other characters, literal",
      "é ő „Videó” \u2003 \u200b",
      false,
      "é ő „Videó” \u2003 \u200b",
    ],
    ["an existing reference, escaped again", "&amp;", false, "&amp;amp;"],
    ["a lone high surrogate", "a\ud800b", false, "a�b"],
    ["a lone low surrogate", "a\udc00b", true, "a�b"],
    ["a surrogate pair, kept", "a😀b", false, "a😀b"],
    // The writer's own rules: what an HTML parse reads back.
    ["U+0000 in text, left out", "a\u0000b\u0000", false, "ab"],
    ["U+0000 in an attribute, as U+FFFD", "a\u0000b", true, "a�b"],
    ["CR LF in text, as LF", "a\r\nb", false, "a\nb"],
    ["a lone CR in an attribute, as LF", "a\rb", true, "a\nb"],
    ["CR before CR LF, as two LF", "a\r\r\nb", true, "a\n\nb"],
    ["LF, kept", "a\nb", true, "a\nb"],
  ])("escapes %s", (_name, value, attribute, expected) => {
    expect(escapeHtml(value, attribute)).toBe(expected)
  })
})

describe("writeHtml", () => {
  it("writes elements, attributes and text in html5ever's dialect", () => {
    expect(
      writeHtml([
        element(
          "p",
          [["style", "text-align: center"]],
          [
            text("a "),
            element(
              "a",
              [
                ["href", "/x?a=1&b=2"],
                ["target", "_blank"],
                ["rel", "noopener noreferrer"],
              ],
              [text("b")]
            ),
          ]
        ),
        element(
          "p",
          [["data-youtube-video", ""]],
          [element("iframe", [["allowfullscreen", ""]])]
        ),
      ])
    ).toBe(
      '<p style="text-align: center">a <a href="/x?a=1&amp;b=2" target="_blank" rel="noopener noreferrer">b</a></p><p data-youtube-video=""><iframe allowfullscreen=""></iframe></p>'
    )
  })

  it("writes void elements without an end tag or children", () => {
    expect(
      writeHtml([
        element("p", [], [text("a"), element("br"), text("b")]),
        element("hr", [], [text("lost")]),
        element("img", [["alt", "x"]]),
      ])
    ).toBe('<p>a<br>b</p><hr><img alt="x">')
  })

  it("writes the text of a raw-text element unescaped", () => {
    expect(writeHtml([element("iframe", [], [text("<b>&</b>")])])).toBe(
      "<iframe><b>&</b></iframe>"
    )
    expect(writeHtml([element("p", [], [text("<b>&</b>")])])).toBe(
      "<p>&lt;b&gt;&amp;&lt;/b&gt;</p>"
    )
  })

  it("writes raw text as the raw-text state reads it back", () => {
    // In raw text the tokenizer turns U+0000 into U+FFFD rather than
    // dropping it, and CR into LF as everywhere.
    expect(
      writeHtml([element("iframe", [], [text("a\u0000b\r\nc\ud800")])])
    ).toBe("<iframe>a�b\nc�</iframe>")
  })

  it("writes nothing for no nodes", () => {
    expect(writeHtml([])).toBe("")
  })

  it("lists html5ever 0.40.1's void and raw-text elements", () => {
    expect(VOID_ELEMENTS.size).toBe(18)
    for (const tag of ["br", "hr", "img"]) {
      expect(VOID_ELEMENTS.has(tag), tag).toBe(true)
    }
    expect(RAW_TEXT_ELEMENTS.has("iframe")).toBe(true)
  })

  // The server's sanitiser writes every regression output with html5ever,
  // so the writer must give the same bytes for the same tree.
  it.each(regressionFixtures().map((fixture) => [fixture.name, fixture]))(
    "writes the html5ever bytes of the regression output %s",
    (_name, { output }) => {
      expect(writeHtml(reparse(output))).toBe(output)
    }
  )

  it("writes text and attribute values that parse back unchanged", () => {
    const value = fc.string({ unit: "grapheme", maxLength: 30 }).map((s) =>
      // jsdom's parser, like the server's, turns U+0000 and CR into other
      // characters, and so do the writer's own rules (the next property).
      s.replace(/[\0\r]/g, "")
    )
    fc.assert(
      fc.property(value, value, (body, title) => {
        const tree: StoredNode[] = [
          element("p", [["title", title]], [text(body)]),
        ]
        const html = writeHtml(tree)
        expect(reparse(html)).toEqual(
          body === "" ? [element("p", [["title", title]])] : tree
        )
        expect(writeHtml(reparse(html))).toBe(html)
      })
    )
  })

  it("writes any text and attribute value as an HTML parse reads it back", () => {
    // Values with U+0000, CR and lone surrogates, which a plain-text paste
    // or a dialog can put into a document without an HTML parse.
    const value = fc
      .array(
        fc.oneof(
          fc.string({ unit: "grapheme", maxLength: 4 }),
          fc.constantFrom("\u0000", "\r", "\r\n", "\n", "\ud800", "\udc00")
        ),
        { maxLength: 8 }
      )
      .map((pieces) => pieces.join(""))
    fc.assert(
      fc.property(value, value, (body, title) => {
        const html = writeHtml([element("p", [["title", title]], [text(body)])])
        expect(html).not.toMatch(/[\0\r]/)
        expect(writeHtml(reparse(html))).toBe(html)
      })
    )
  })
})
