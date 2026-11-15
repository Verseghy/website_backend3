import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { CONTENT_V1_POLICY } from "../policy"
import {
  canonicalStyles,
  MARK_ELEMENTS,
  orderAttributes,
  postPass,
  TIGHT_CONTAINERS,
  tightenParagraphs,
  unwrapAndMerge,
} from "./post-pass"
import type { StoredElement, StoredNode } from "./stored-tree"
import { element, text } from "./stored-tree"
import { writeHtml } from "./write-html"
import { regressionFixtures, reparse } from "@/test/stored-html"

const POLICY = CONTENT_V1_POLICY
const html = (nodes: readonly StoredNode[]) => writeHtml(nodes)
const tree = (source: string) => reparse(source)

describe("canonicalStyles", () => {
  it.each<[string, string, string]>([
    [
      "rewrites a style in another spelling",
      '<p><span style="font-size: 1.5em; color: rgb(192, 0, 0)">a</span></p>',
      '<p><span style="color: #c00000; font-size: 1.5em">a</span></p>',
    ],
    [
      "sorts TipTap's merged declarations",
      '<span style="color: #c00000; background-color: #ffff00; font-size: 1.82em">a</span>',
      '<span style="background-color: #ffff00; color: #c00000; font-size: 1.82em">a</span>',
    ],
    [
      "removes a style with nothing left",
      '<p style="color: red">a</p>',
      "<p>a</p>",
    ],
    ["removes an empty style", '<p style="">a</p>', "<p>a</p>"],
    [
      "removes the style of an element without a grammar",
      '<li style="text-align: center">a</li>',
      "<li>a</li>",
    ],
    [
      "keeps the attributes around the style in place",
      '<td colspan="2" style="width: 96pt" rowspan="3">a</td>',
      '<td colspan="2" style="box-sizing: content-box; width: 128px" rowspan="3">a</td>',
    ],
  ])("%s", (_name, source, expected) => {
    const wrapped = source.startsWith("<td")
      ? `<table><tbody><tr>${source}</tr></tbody></table>`
      : source
    const result = html(canonicalStyles(tree(wrapped), POLICY))
    expect(result).toContain(expected)
  })
})

describe("unwrapAndMerge", () => {
  it.each<[string, string, string]>([
    ["unwraps a bare span", "<p>a<span>b</span>c</p>", "<p>abc</p>"],
    [
      "unwraps nested bare spans",
      "<p><span><span><em>b</em></span></span></p>",
      "<p><em>b</em></p>",
    ],
    [
      "keeps a span with a style",
      '<p><span style="color: #c00000">b</span></p>',
      '<p><span style="color: #c00000">b</span></p>',
    ],
    [
      "merges equal neighbouring spans",
      '<p><span style="color: #c00000">a</span><span style="color: #c00000">b</span></p>',
      '<p><span style="color: #c00000">ab</span></p>',
    ],
    [
      "merges neighbours that meet after a bare span is unwrapped",
      "<p><strong>a</strong><span><strong>b</strong></span></p>",
      "<p><strong>ab</strong></p>",
    ],
    [
      "merges the children of merged neighbours too",
      '<p><a href="/x"><em>a</em></a><a href="/x"><em>b</em>c</a></p>',
      '<p><a href="/x"><em>ab</em>c</a></p>',
    ],
    [
      "keeps neighbours with other attributes apart",
      '<p><a href="/x">a</a><a href="/y">b</a></p>',
      '<p><a href="/x">a</a><a href="/y">b</a></p>',
    ],
    [
      "keeps neighbours with the same attributes in another order apart",
      '<p><a href="/x" rel="r">a</a><a rel="r" href="/x">b</a></p>',
      '<p><a href="/x" rel="r">a</a><a rel="r" href="/x">b</a></p>',
    ],
    [
      "never merges blocks",
      "<p>a</p><p>b</p><blockquote>c</blockquote><blockquote>d</blockquote>",
      "<p>a</p><p>b</p><blockquote>c</blockquote><blockquote>d</blockquote>",
    ],
    ["never merges hard breaks", "<p>a<br><br>b</p>", "<p>a<br><br>b</p>"],
  ])("%s", (_name, source, expected) => {
    expect(html(unwrapAndMerge(tree(source)))).toBe(expected)
  })

  it("merges only the elements that marks render", () => {
    expect([...MARK_ELEMENTS].sort()).toEqual(
      ["a", "em", "s", "span", "strong", "sub", "sup", "u"].sort()
    )
  })
})

describe("tightenParagraphs", () => {
  it.each<[string, string, string]>([
    [
      "a lone list item paragraph",
      "<ul><li><p>a</p></li></ul>",
      "<ul><li>a</li></ul>",
    ],
    [
      "a list item's first paragraph before lists",
      "<ul><li><p>a</p><ol><li><p>b</p></li></ol><ul><li><p>c</p></li></ul></li></ul>",
      "<ul><li>a<ol><li>b</li></ol><ul><li>c</li></ul></li></ul>",
    ],
    [
      "not two paragraphs of a list item",
      "<ol><li><p>két</p><p>bekezdés</p></li></ol>",
      "<ol><li><p>két</p><p>bekezdés</p></li></ol>",
    ],
    [
      "not a paragraph before a quote in a list item",
      "<ul><li><p>a</p><blockquote><p>b</p></blockquote></li></ul>",
      "<ul><li><p>a</p><blockquote>b</blockquote></li></ul>",
    ],
    [
      "not an aligned paragraph in a list item",
      '<ul><li><p style="text-align: center">a</p></li></ul>',
      '<ul><li><p style="text-align: center">a</p></li></ul>',
    ],
    [
      "a lone quote paragraph",
      "<blockquote><p>Egy bekezdéses idézet</p></blockquote>",
      "<blockquote>Egy bekezdéses idézet</blockquote>",
    ],
    [
      "not a quote paragraph before a list",
      "<blockquote><p>a</p><ul><li><p>b</p></li></ul></blockquote>",
      "<blockquote><p>a</p><ul><li>b</li></ul></blockquote>",
    ],
    [
      "an empty lone paragraph",
      "<ul><li><p></p></li></ul><blockquote><p></p></blockquote>",
      "<ul><li></li></ul><blockquote></blockquote>",
    ],
    // editor.md §3.7 keeps this paragraph (item D148, answered on
    // 2026-11-15), because `<li><ul>` reloads with the nested list moved
    // out of the item. The property test found it with
    // doc(bulletList(listItem(paragraph, bulletList(listItem(paragraph))),
    // listItem(paragraph)), paragraph).
    [
      "not an empty first paragraph before a list item's lists",
      "<ul><li><p></p><ul><li><p>b</p></li></ul></li></ul>",
      "<ul><li><p></p><ul><li>b</li></ul></li></ul>",
    ],
    [
      "not an empty first paragraph before two lists",
      "<ol><li><p></p><ol><li><p>b</p></li></ol><ul><li><p></p></li></ul></li></ol>",
      "<ol><li><p></p><ol><li>b</li></ol><ul><li></li></ul></li></ol>",
    ],
    [
      "nested containers from the inside out",
      "<blockquote><blockquote><p>a</p></blockquote></blockquote>",
      "<blockquote><blockquote>a</blockquote></blockquote>",
    ],
    [
      "not a paragraph that is an embed wrapper",
      '<blockquote><p data-youtube-video=""></p></blockquote>',
      '<blockquote><p data-youtube-video=""></p></blockquote>',
    ],
  ])("tightens %s", (_name, source, expected) => {
    expect(html(tightenParagraphs(tree(source), POLICY))).toBe(expected)
  })

  const cell = (inner: string) =>
    `<table><tbody><tr>${inner}</tr></tbody></table>`

  it.each<[string, string, string]>([
    ["a lone cell paragraph", "<td><p>a</p></td>", "<td>a</td>"],
    ["a lone header paragraph", "<th><p>a</p></th>", "<th>a</th>"],
    [
      "a cell paragraph's alignment, moved to the cell",
      '<td><p style="text-align: right">a</p></td>',
      '<td style="text-align: right">a</td>',
    ],
    [
      "the paragraph's alignment over the cell's own",
      '<td style="text-align: left; width: 10px; box-sizing: content-box"><p style="text-align: center">a</p></td>',
      '<td style="box-sizing: content-box; text-align: center; width: 10px">a</td>',
    ],
    [
      "not a cell paragraph with another attribute",
      '<td><p style="text-align: right" data-x="1">a</p></td>',
      '<td><p style="text-align: right" data-x="1">a</p></td>',
    ],
    [
      "not two cell paragraphs",
      "<td><p>a</p><p>b</p></td>",
      "<td><p>a</p><p>b</p></td>",
    ],
  ])("tightens %s", (_name, source, expected) => {
    const result = html(tightenParagraphs(tree(cell(source)), POLICY))
    expect(result).toBe(cell(expected))
  })

  it("tightens only in the containers of editor.md §3.7", () => {
    expect([...TIGHT_CONTAINERS].sort()).toEqual(
      ["blockquote", "li", "td", "th"].sort()
    )
  })
})

describe("orderAttributes", () => {
  it.each<[string, string, string]>([
    [
      "a link, rel last",
      '<a target="_blank" rel="noopener noreferrer" href="https://example.org/">l</a>',
      '<a href="https://example.org/" target="_blank" rel="noopener noreferrer">l</a>',
    ],
    [
      "an ordered list",
      '<ol type="a" start="5"><li>a</li></ol>',
      '<ol start="5" type="a"><li>a</li></ol>',
    ],
    [
      "an embed, with an attribute the policy does not list kept after the rest",
      '<p data-youtube-video=""><iframe title="Videó" frameborder="0" allowfullscreen="" src="https://www.youtube-nocookie.com/embed/abcDEF12345?start=8&amp;list=PL1" height="360" width="640" allow="x" referrerpolicy="y" style="width: 100%"></iframe></p>',
      '<p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/abcDEF12345?start=8&amp;list=PL1" width="640" height="360" title="Videó" style="width: 100%" allow="x" allowfullscreen="" referrerpolicy="y" frameborder="0"></iframe></p>',
    ],
    [
      "nothing on an element outside content-v1",
      '<div b="1" a="2">x</div>',
      '<div b="1" a="2">x</div>',
    ],
  ])("orders %s", (_name, source, expected) => {
    expect(html(orderAttributes(tree(source), POLICY))).toBe(expected)
  })
})

/** The canonical shapes of editor.md §3.7, as the server stores them. */
const CANONICAL_SHAPES = regressionFixtures().find(
  ({ name }) => name === "canonical-shapes"
)?.output

/** Every element's attributes in reverse order. */
function reversed(nodes: readonly StoredNode[]): StoredNode[] {
  return nodes.map((node) =>
    node.kind === "text"
      ? node
      : element(
          node.tag,
          [...node.attributes].reverse(),
          reversed(node.children)
        )
  )
}

describe("postPass", () => {
  it("is the identity on the canonical shapes of editor.md §3.7", () => {
    expect(CANONICAL_SHAPES).toBeDefined()
    const shapes = tree(CANONICAL_SHAPES ?? "")
    expect(html(postPass(shapes, POLICY))).toBe(CANONICAL_SHAPES)
    // Whatever order the attributes come in.
    expect(html(postPass(reversed(shapes), POLICY))).toBe(CANONICAL_SHAPES)
  })

  it("writes the schema's rendering of the shapes in their stored form", () => {
    expect(
      html(
        postPass(
          tree(
            '<ul style="list-style-type: circle"><li><p>pont</p></li></ul><p><a target="_blank" rel="noopener noreferrer" href="https://example.org/">link</a></p><p><span style="color: #c00000; font-size: 1.25em">piros</span><span>!</span></p>'
          ),
          POLICY
        )
      )
    ).toBe(
      '<ul style="list-style-type: circle"><li>pont</li></ul><p><a href="https://example.org/" target="_blank" rel="noopener noreferrer">link</a></p><p><span style="color: #c00000; font-size: 1.25em">piros</span>!</p>'
    )
  })

  it.each(regressionFixtures().map((fixture) => [fixture.name, fixture]))(
    "is idempotent on the regression fixture %s",
    (_name, { input, output }) => {
      for (const body of [input, output]) {
        const once = postPass(tree(body), POLICY)
        expect(html(postPass(once, POLICY))).toBe(html(once))
      }
    }
  )

  it("is idempotent on random trees", () => {
    const leaf = fc.oneof(
      fc.constantFrom("a", " ", "b c").map((value) => text(value)),
      fc.constant(element("br"))
    )
    const style = fc.constantFrom(
      null,
      "color: red",
      "text-align: center",
      "TEXT-ALIGN: RIGHT",
      "font-size: 2em",
      ""
    )
    const node: fc.Arbitrary<StoredNode> = fc.letrec<{ node: StoredNode }>(
      (tie) => ({
        node: fc.oneof(
          { depthSize: "small", withCrossShrink: true },
          leaf,
          fc
            .tuple(
              fc.constantFrom(
                "p",
                "li",
                "ul",
                "blockquote",
                "span",
                "strong",
                "a",
                "td"
              ),
              style,
              fc.array(tie("node"), { maxLength: 3 })
            )
            .map(([tag, value, children]): StoredElement =>
              element(tag, value === null ? [] : [["style", value]], children)
            )
        ),
      })
    ).node
    fc.assert(
      fc.property(fc.array(node, { maxLength: 4 }), (nodes) => {
        const once = postPass(nodes, POLICY)
        expect(html(postPass(once, POLICY))).toBe(html(once))
      })
    )
  })
})
