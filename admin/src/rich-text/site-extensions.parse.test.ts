import { describe, expect, it } from "vitest"

import { parseHtml, renderHtml, siteSchema } from "@/test/site-schema"

/**
 * The schema's parse rules, case by case (editor.md §13.3, L1): what a body
 * becomes when it is parsed, shown as the schema renders it again with
 * styles verbatim. Two things here are the serialiser's job, not the
 * schema's, and show in the expected HTML: a lone paragraph in a list item
 * or quote is not tightened yet (`<li><p>…</p></li>`), and attributes come in
 * TipTap's order (`toStoredHtml` writes the §3.2 order, with `rel` last).
 */
const schema = siteSchema()
const parsed = (html: string) =>
  renderHtml(schema, parseHtml(schema, html)).innerHTML

const A = 'target="_blank" rel="noopener noreferrer"'

describe("parse rules", () => {
  describe.each<[string, [string, string, string][]]>([
    [
      "the canonical shapes of editor.md §3.7",
      [
        [
          "an aligned heading",
          '<h2 style="text-align: center">Eredmények</h2>',
          '<h2 style="text-align: center">Eredmények</h2>',
        ],
        [
          "a paragraph with a strong run and a styled span",
          '<p>Szöveg <strong>félkövér</strong>, <span style="color: #c00000; font-size: 1.25em">piros és nagy</span>.</p>',
          '<p>Szöveg <strong>félkövér</strong>, <span style="color: #c00000; font-size: 1.25em">piros és nagy</span>.</p>',
        ],
        [
          "an ordered list with a start and a type",
          '<ol start="5" type="a"><li>első</li><li><p>két</p><p>bekezdés</p></li></ol>',
          '<ol start="5" type="a"><li><p>első</p></li><li><p>két</p><p>bekezdés</p></li></ol>',
        ],
        [
          "a bullet list with circles",
          '<ul style="list-style-type: circle"><li>pont</li></ul>',
          '<ul style="list-style-type: circle"><li><p>pont</p></li></ul>',
        ],
        [
          "a quote",
          "<blockquote>Egy bekezdéses idézet</blockquote>",
          "<blockquote><p>Egy bekezdéses idézet</p></blockquote>",
        ],
        [
          "a link",
          '<p><a href="https://example.org/" target="_blank" rel="noopener noreferrer">link</a></p>',
          `<p><a ${A} href="https://example.org/">link</a></p>`,
        ],
      ],
    ],
    [
      "blocks",
      [
        ["h1, kept for legacy bodies", "<h1>Cím</h1>", "<h1>Cím</h1>"],
        ["h3 and h4", "<h3>a</h3><h4>b</h4>", "<h3>a</h3><h4>b</h4>"],
        ["h5 and h6 as paragraphs", "<h5>a</h5><h6>b</h6>", "<p>a</p><p>b</p>"],
        [
          "a code block as a paragraph",
          "<pre><code>x = 1</code></pre>",
          "<p>x = 1</p>",
        ],
        ["inline code as text", "<p><code>x</code> y</p>", "<p>x y</p>"],
        ["a rule and a hard break", "<p>a<br>b</p><hr>", "<p>a<br>b</p><hr>"],
        ["a trailing hard break", "<p>a<br></p>", "<p>a<br></p>"],
        ["a div's text as a paragraph", "<div>szöveg</div>", "<p>szöveg</p>"],
        [
          "no class, id, lang or data attribute",
          '<p class="text-node" id="x" lang="hu" data-x="1" dir="ltr">a</p><h2 class="heading-node">b</h2>',
          "<p>a</p><h2>b</h2>",
        ],
        ["no script", "<p>a</p><script>alert(1)</script>", "<p>a</p>"],
        ["an empty body as one empty paragraph", "", "<p></p>"],
      ],
    ],
    [
      "alignment",
      [
        [
          "the four alignments",
          '<p style="text-align: left">a</p><p style="text-align: right">b</p><p style="text-align: justify">c</p><h4 style="text-align: center">d</h4>',
          '<p style="text-align: left">a</p><p style="text-align: right">b</p><p style="text-align: justify">c</p><h4 style="text-align: center">d</h4>',
        ],
        [
          "a value outside the policy",
          '<p style="text-align: start">a</p>',
          "<p>a</p>",
        ],
        ["another spelling", '<p style="TEXT-ALIGN: CENTER">a</p>', "<p>a</p>"],
        ["the align attribute", '<p align="center">a</p>', "<p>a</p>"],
        [
          "only the alignment of a styled heading",
          '<h2 style="text-align: center; color: #c00000">a</h2>',
          '<h2 style="text-align: center">a</h2>',
        ],
      ],
    ],
    [
      "ordered lists",
      [
        [
          "the default start and type",
          '<ol start="1" type="1"><li>a</li></ol>',
          "<ol><li><p>a</p></li></ol>",
        ],
        [
          "start 0",
          '<ol start="0"><li>a</li></ol>',
          '<ol start="0"><li><p>a</p></li></ol>',
        ],
        [
          "the largest start",
          '<ol start="9999"><li>a</li></ol>',
          '<ol start="9999"><li><p>a</p></li></ol>',
        ],
        [
          "a start with leading zeros",
          '<ol start="007"><li>a</li></ol>',
          '<ol start="7"><li><p>a</p></li></ol>',
        ],
        [
          "a start above the maximum",
          '<ol start="10000"><li>a</li></ol>',
          "<ol><li><p>a</p></li></ol>",
        ],
        [
          "a start with trailing text",
          '<ol start="5x"><li>a</li></ol>',
          "<ol><li><p>a</p></li></ol>",
        ],
        [
          "a negative start",
          '<ol start="-2"><li>a</li></ol>',
          "<ol><li><p>a</p></li></ol>",
        ],
        [
          "each type, case kept",
          '<ol type="a"><li>a</li></ol><ol type="A"><li>b</li></ol><ol type="i"><li>c</li></ol><ol type="I"><li>d</li></ol>',
          '<ol type="a"><li><p>a</p></li></ol><ol type="A"><li><p>b</p></li></ol><ol type="i"><li><p>c</p></li></ol><ol type="I"><li><p>d</p></li></ol>',
        ],
        [
          "a type outside the policy",
          '<ol type="disc"><li>a</li></ol>',
          "<ol><li><p>a</p></li></ol>",
        ],
        [
          "a type from the list's CSS",
          '<ol style="list-style-type: upper-roman"><li>a</li></ol>',
          '<ol type="I"><li><p>a</p></li></ol>',
        ],
        [
          "a type from the first item's CSS (Google Docs)",
          '<ol><li style="list-style-type: lower-alpha">a</li><li>b</li></ol>',
          '<ol type="a"><li><p>a</p></li><li><p>b</p></li></ol>',
        ],
        [
          "the CSS type when the attribute is outside the policy",
          '<ol type="x" style="list-style-type: lower-roman"><li>a</li></ol>',
          '<ol type="i"><li><p>a</p></li></ol>',
        ],
        [
          "no type from CSS without one",
          '<ol style="list-style-type: decimal"><li>a</li></ol>',
          "<ol><li><p>a</p></li></ol>",
        ],
      ],
    ],
    [
      "bullet lists",
      [
        [
          "square",
          '<ul style="list-style-type: square"><li>a</li></ul>',
          '<ul style="list-style-type: square"><li><p>a</p></li></ul>',
        ],
        [
          "disc, the default",
          '<ul style="list-style-type: disc"><li>a</li></ul>',
          "<ul><li><p>a</p></li></ul>",
        ],
        [
          "a style outside the policy",
          '<ul style="list-style-type: decimal"><li>a</li></ul>',
          "<ul><li><p>a</p></li></ul>",
        ],
        [
          "the list-style shorthand",
          '<ul style="list-style: circle"><li>a</li></ul>',
          "<ul><li><p>a</p></li></ul>",
        ],
        [
          "nested lists",
          '<ul><li><p>a</p><ol type="i"><li>b</li></ol></li></ul>',
          '<ul><li><p>a</p><ol type="i"><li><p>b</p></li></ol></li></ul>',
        ],
      ],
    ],
    [
      "links",
      [
        [
          "the forced target and rel, and nothing else",
          '<p><a href="https://example.org/" target="_self" rel="nofollow" class="link" title="Cím">a</a></p>',
          `<p><a ${A} href="https://example.org/">a</a></p>`,
        ],
        [
          "mailto, tel, a fragment and relative links",
          '<p><a href="mailto:a@b.example">a</a> <a href="tel:+3612345">b</a> <a href="#top">c</a> <a href="/hirek/12">d</a> <a href="../x">e</a></p>',
          `<p><a ${A} href="mailto:a@b.example">a</a> <a ${A} href="tel:+3612345">b</a> <a ${A} href="#top">c</a> <a ${A} href="/hirek/12">d</a> <a ${A} href="../x">e</a></p>`,
        ],
        [
          "a javascript link as text",
          '<p><a href="javascript:alert(1)">a</a></p>',
          "<p>a</p>",
        ],
        [
          "a scheme-relative link as text",
          '<p><a href="//evil.example/">a</a></p>',
          "<p>a</p>",
        ],
        [
          "a link to a document under /storage/ as text",
          '<p><a href="/storage/uploads/x.svg">a</a></p>',
          "<p>a</p>",
        ],
        [
          "an ftp link as text",
          '<p><a href="ftp://example.org/">a</a></p>',
          "<p>a</p>",
        ],
        [
          "an empty href, which the server keeps",
          '<p><a href="">a</a></p>',
          `<p><a ${A} href="">a</a></p>`,
        ],
        [
          "a link without href, as the server keeps it",
          `<p><a ${A}>a</a></p>`,
          `<p><a ${A}>a</a></p>`,
        ],
        [
          // The server stores a Word bookmark as an `a` without `href`; the
          // paste filter unwraps bookmarks before the parse (editor.md §5.2
          // step 10), so the schema keeps what the server keeps.
          "a Word bookmark as the server stores it",
          '<p><a name="_Toc1">a</a></p>',
          `<p><a ${A}>a</a></p>`,
        ],
      ],
    ],
    [
      "marks",
      [
        [
          "b, i, strike and del as strong, em and s",
          "<p><b>a</b> <i>b</i> <strike>c</strike> <del>d</del> <u>e</u></p>",
          "<p><strong>a</strong> <em>b</em> <s>c</s> <s>d</s> <u>e</u></p>",
        ],
        [
          "a sub inside a sup: the inner one wins",
          "<p><sup><sub>2</sub></sup> <sub><sup>3</sup></sub></p>",
          "<p><sub>2</sub> <sup>3</sup></p>",
        ],
        [
          "a raised span as a superscript (stock 3.32's style rule)",
          '<p>x<span style="vertical-align: super">2</span></p>',
          "<p>x<sup>2</sup></p>",
        ],
      ],
    ],
    [
      "text style",
      [
        [
          "every stored declaration",
          '<p><span style="background-color: #ffff00; color: #c00000; font-size: 1.82em">a</span></p>',
          '<p><span style="color: #c00000; background-color: #ffff00; font-size: 1.82em">a</span></p>',
        ],
        [
          "a translucent highlight",
          '<p><span style="background-color: #000000b3">a</span></p>',
          '<p><span style="background-color: #000000b3">a</span></p>',
        ],
        [
          "the size bounds",
          '<p><span style="font-size: 0.5em">a</span><span style="font-size: 3em">b</span></p>',
          '<p><span style="font-size: 0.5em">a</span><span style="font-size: 3em">b</span></p>',
        ],
        [
          "no span for a font family",
          '<p><span style="font-family: Calibri">a</span></p>',
          "<p>a</p>",
        ],
        [
          "no span without a style",
          '<p><span class="x">a</span></p>',
          "<p>a</p>",
        ],
        [
          "no transparent text",
          '<p><span style="color: transparent">a</span></p>',
          "<p>a</p>",
        ],
        [
          "no translucent text",
          '<p><span style="color: #c0000080">a</span></p>',
          "<p>a</p>",
        ],
        [
          "no size below 0.5em",
          '<p><span style="font-size: 0.4em">a</span></p>',
          "<p>a</p>",
        ],
        [
          "no size in points",
          '<p><span style="font-size: 12pt">a</span></p>',
          "<p>a</p>",
        ],
        [
          "no kit palette variable",
          '<p><span style="color: var(--mt-accent-red)">a</span></p>',
          "<p>a</p>",
        ],
        [
          "the stored part of a mixed style",
          '<p><span style="color: #c00000; line-height: 2">a</span></p>',
          '<p><span style="color: #c00000">a</span></p>',
        ],
        [
          "a nested span, which inherits the outer colour",
          '<p><span style="color: #c00000">a<span style="font-size: 1.5em">b</span></span></p>',
          '<p><span style="color: #c00000">a</span><span style="color: #c00000; font-size: 1.5em">b</span></p>',
        ],
        [
          "an inner colour over the outer one",
          '<p><span style="color: #c00000"><span style="color: #0000ff">a</span></span></p>',
          '<p><span style="color: #0000ff">a</span></p>',
        ],
        [
          "an inner style past the outer one's declarations, read on its own",
          `<p><span style="${Array.from({ length: 32 }, () => "color: #c00000").join("; ")}"><span style="color: #0000ff">a</span></span></p>`,
          '<p><span style="color: #0000ff">a</span></p>',
        ],
        [
          "an inner style that sets a colour the server would rewrite, as no colour",
          '<p><span style="color: #c00000; font-size: 2em"><span style="color: red; font-size: 1.5em">a</span></span></p>',
          '<p><span style="font-size: 1.5em">a</span></p>',
        ],
        [
          "an unstyled span between two styled ones",
          '<p><span style="color: #c00000"><span>a<span style="font-size: 2em">b</span></span></span></p>',
          '<p><span style="color: #c00000">a</span><span style="color: #c00000; font-size: 2em">b</span></p>',
        ],
        [
          "no highlight when a later background shorthand sets it",
          '<p><span style="background-color: #00ff00; background: #ff0000">a</span></p>',
          "<p>a</p>",
        ],
        [
          "the highlight that a later background-color sets",
          '<p><span style="background: #ff0000; background-color: #00ff00">a</span></p>',
          '<p><span style="background-color: #00ff00">a</span></p>',
        ],
      ],
    ],
  ])("%s", (_group, cases) => {
    it.each(cases)("parses %s", (_name, input, expected) => {
      expect(parsed(input)).toBe(expected)
    })
  })
})
