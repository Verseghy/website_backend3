import { describe, expect, it } from "vitest"

import { padletRules } from "../padlet-input"
import { CONTENT_V1_POLICY } from "../policy"
import { youtubeRules } from "../youtube-input"
import { inlineStyleSheets } from "./inline-styles"
import type { PasteContext } from "./paste-context"
import { emptyReportDraft } from "./paste-context"
import type { PasteSource } from "./paste-source"
import { SLICE_MARKER } from "./paste-source"
import {
  attributeValue,
  frameAddress,
  hasContent,
  normalizeStructure,
  normalizeTable,
  removeWithContent,
  tightenCellAlignment,
} from "./paste-structure"

const policy = CONTENT_V1_POLICY
const parser = new DOMParser()
const parse = (html: string) => parser.parseFromString(html, "text/html")

/** The context of a paste from `source`. */
function context(source: PasteSource): PasteContext {
  return {
    policy,
    source,
    youtube: youtubeRules(policy),
    padlet: padletRules(policy),
    report: emptyReportDraft(),
  }
}

/** Runs step 10 (with step 6 first, as the filter does) and returns the body's HTML. */
function structure(html: string, source: PasteSource = "generic") {
  const document = parse(html)
  if (source !== "internal") {
    inlineStyleSheets(document)
  }
  const paste = context(source)
  normalizeStructure(document.body, paste)
  return { html: document.body.innerHTML, report: paste.report }
}

/**
 * A table as the filter writes it: the forced `table` and `tbody` styles of
 * editor.md §3.3 are left to the schema, which writes them on every table.
 */
const TABLE = "<table><tbody>"

describe("hasContent", () => {
  it.each<[string, boolean]>([
    ["<p> \n\t</p>", false],
    ["<p>&nbsp;</p>", true],
    ["<p><span></span></p>", false],
    ["<p><br></p>", true],
    ["<p><img src=x></p>", true],
    ["<p>a</p>", true],
  ])("tells whether %s shows anything: %s", (html, expected) => {
    expect(hasContent(parse(html).querySelector("p")!)).toBe(expected)
  })
})

describe("removeWithContent (pass 1)", () => {
  it("removes comments, Word's prefixed elements and what the server removes with its content", () => {
    const document = parse(
      "<p>a<!--[if gte vml 1]><v:shape><v:imagedata src='x'></v:imagedata></v:shape><![endif]--><o:p></o:p></p><w:Sdt>sdt</w:Sdt><script>s</script><style>p{}</style><svg><text>t</text></svg><textarea>u</textarea><list>v</list><meta name=x><p>b</p>"
    )
    removeWithContent(document.body, policy)
    expect(document.body.innerHTML).toBe("<p>a<o:p></o:p></p><p>b</p>")
  })
})

describe("normalizeTable (pass 2)", () => {
  /** Runs the table pass on the first table and returns the body. */
  const table = (html: string) => {
    const document = parse(html)
    normalizeTable(document.querySelector("table")!, policy)
    return document.body.innerHTML
  }

  it("moves thead and tfoot rows into one tbody, header first, footer last", () => {
    expect(
      table(
        "<table><tfoot><tr><td>láb</td></tr></tfoot><tbody><tr><td>test</td></tr></tbody><thead><tr><th>fej</th></tr></thead></table>"
      )
    ).toBe(
      "<table><tbody><tr><th>fej</th></tr><tr><td>test</td></tr><tr><td>láb</td></tr></tbody></table>"
    )
  })

  it("moves column widths to the first-row cells, span- and colspan-aware, under a cell's own width", () => {
    expect(
      table(
        "<table><colgroup><col width=64 span=2><col style='width: 75pt'></colgroup><tr><td colspan=2>a</td><td>b</td></tr><tr><td>c</td><td>d</td><td>e</td></tr></table>"
      )
    ).toBe(
      '<table><tbody><tr><td colspan="2" style="width: 128px">a</td><td style="width: 100px">b</td></tr><tr><td>c</td><td>d</td><td>e</td></tr></tbody></table>'
    )
    expect(
      table(
        "<table><col width=40><col width=50%><tr><td width=10>a</td><td>b</td></tr></table>"
      )
    ).toBe(
      '<table><tbody><tr><td width="10">a</td><td>b</td></tr></tbody></table>'
    )
  })

  it("reads a colgroup without cols as one column (LibreOffice)", () => {
    expect(
      table(
        "<table><colgroup width=85></colgroup><colgroup width=120></colgroup><tr><td>a</td><td>b</td></tr></table>"
      )
    ).toBe(
      '<table><tbody><tr><td style="width: 85px">a</td><td style="width: 120px">b</td></tr></tbody></table>'
    )
  })

  it("moves a row's height to its first cell, under the cell's own", () => {
    expect(
      table(
        "<table><tr height=20 style='height: 15.0pt'><td>a</td><td>b</td></tr><tr height=30><td style='height: 10px'>c</td></tr></table>"
      )
    ).toBe(
      '<table><tbody><tr height="20" style="height: 15.0pt"><td style="height: 20px">a</td><td>b</td></tr><tr height="30"><td style="height: 10px">c</td></tr></tbody></table>'
    )
  })

  it("puts the caption before the table and removes a table without rows", () => {
    expect(
      table("<table><caption>Órarend</caption><tr><td>a</td></tr></table>")
    ).toBe("<p>Órarend</p><table><tbody><tr><td>a</td></tr></tbody></table>")
    expect(table("<p>x</p><table><caption>üres</caption></table>")).toBe(
      "<p>x</p><p>üres</p>"
    )
  })
})

describe("attributeValue", () => {
  const rule = (tag: string, name: string) =>
    policy.elements[tag].attributes.find(
      (attribute) => attribute.name === name
    )!.value

  it.each<[string, string, string, string | null]>([
    ["td", "colspan", "2", "2"],
    ["td", "colspan", " 3 ", "3"],
    ["td", "colspan", "1", null],
    ["td", "colspan", "1001", null],
    ["td", "colspan", "x", null],
    ["ol", "start", "0", "0"],
    ["ol", "start", "1", null],
    ["ol", "type", "a", "a"],
    ["ol", "type", "A", "A"],
    ["ol", "type", "1", null],
    ["ol", "type", "disc", null],
    ["iframe", "width", "100%", "100%"],
    ["iframe", "width", "560", "560"],
    ["iframe", "width", "50%", null],
    ["iframe", "title", " Videó ", " Videó "],
    ["iframe", "allow", "autoplay", null],
    ["p", "data-youtube-video", "anything", ""],
    ["a", "target", "_self", null],
    ["a", "rel", "nofollow", null],
  ])("keeps %s[%s=%j] as %j", (tag, name, raw, expected) => {
    expect(attributeValue(rule(tag, name), raw)).toBe(expected)
  })
})

describe("normalizeStructure (editor.md §5.2 step 10)", () => {
  it("strips every attribute outside content-v1 and maps Office paragraphs", () => {
    expect(
      structure(
        "<p class=MsoNormal lang=HU dir=ltr id=x style='margin: 0cm; text-align: center; font-family: Calibri'>a<o:p></o:p></p>"
      ).html
    ).toBe('<p style="text-align: center">a</p>')
  })

  it("maps h5 and h6 to h4, and menu to ul", () => {
    expect(
      structure("<h5 align=right>a</h5><h6>b</h6><menu><li>c</li></menu>").html
    ).toBe('<h4 style="text-align: right">a</h4><h4>b</h4><ul><li>c</li></ul>')
  })

  it("turns formatting elements into content-v1 marks", () => {
    expect(
      structure(
        '<p><b>bold</b> <i>dőlt</i> <strike>át</strike> <span style="font-weight: 700; color: #c00000">piros</span> <font color=blue>kék</font></p>'
      ).html
    ).toBe(
      '<p><strong>bold</strong> <em>dőlt</em> <s>át</s> <span style="color: #c00000"><strong>piros</strong></span> <span style="color: #0000ff">kék</span></p>'
    )
  })

  it("does not make Google Docs' wrapper bold", () => {
    expect(
      structure(
        '<b style="font-weight:normal;" id="docs-internal-guid-0a1b"><p dir="ltr"><span style="font-weight:700">a</span><span style="font-weight:400">b</span></p></b>'
      ).html
    ).toBe("<p><strong>a</strong>b</p>")
  })

  it("keeps links the server keeps and unwraps the rest, Word's bookmarks included", () => {
    expect(
      structure(
        '<p><a href="https://example.org/" target=_self rel=nofollow class=x>jó</a> <a href="javascript:alert(1)">rossz</a> <a name="_Toc1">könyvjelző</a> <a href="">üres</a></p>'
      ).html
    ).toBe('<p><a href="https://example.org/">jó</a> rossz könyvjelző üres</p>')
  })

  it("keeps the editor's own links as written, without a target too", () => {
    expect(
      structure(
        `<p ${SLICE_MARKER}="1 1 []"><a target="_blank" rel="noopener noreferrer">cél nélkül</a></p>`,
        "internal"
      ).html
    ).toBe(`<p ${SLICE_MARKER}="1 1 []"><a>cél nélkül</a></p>`)
  })

  it("keeps images the server keeps, written as it stores them", () => {
    expect(
      structure(
        '<p><img src="https://verseghy-gimnazium.net/storage/uploads/a.jpg" alt="kép" width=350 height=0 class=x v:shapes="y"><img src="file:///invented/clip_image001.png"><img src="data:image/png;base64,AAAA"></p>'
      ).html
    ).toBe(
      '<p><img src="/storage/uploads/a.jpg" alt="kép" width="350" style="display: block; height: auto; margin: 0 auto; max-width: 100%"></p>'
    )
  })

  it("keeps embed iframes in the node's form, reports dropped options, and removes the rest", () => {
    const { html, report } = structure(
      '<iframe width="560" height="315" src="http://www.youtube.com/embed/dQw4w9WgXcQ?si=invented_si-0099&amp;controls=0&amp;start=30" title="YouTube video player" frameborder="0" allow="autoplay" allowfullscreen>szöveg</iframe><iframe src="https://padlet.com/embed/invented0key"></iframe><iframe src="https://evil.example/embed/x"></iframe><iframe src="https://www.youtube.com/embed/short"></iframe>'
    )
    expect(html).toBe(
      '<iframe src="https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=30" width="560" height="315" title="YouTube video player"></iframe><iframe src="https://padlet.com/embed/invented0key"></iframe>'
    )
    expect(report.frames).toBe(2)
    expect([...report.embedParams]).toEqual(["controls"])
    // The short id is a YouTube address the server refuses (D132); the
    // evil.example frame is no YouTube address at all.
    expect([...report.youtubeRefusals]).toEqual(["video_id"])
  })

  it("reports a refused YouTube list type, and no options of a refused embed", () => {
    const { html, report } = structure(
      '<iframe src="https://www.youtube.com/embed?listType=user_uploads&amp;list=someone&amp;autoplay=1"></iframe><iframe src="https://www.youtube.com/embed/abc?controls=0"></iframe>'
    )
    expect(html).toBe("")
    expect(report.frames).toBe(2)
    expect(new Set(report.youtubeRefusals)).toEqual(
      new Set(["list_type", "video_id"])
    )
    expect([...report.embedParams]).toEqual([])
  })

  it("lists the font families it drops, each once, in document order", () => {
    const { html, report } = structure(
      '<p style="font-family: Calibri">a <span style="font-family: Arial; color: red">b</span></p><style>td { font-family: Calibri }</style><table><tr><td>c</td></tr></table>'
    )
    expect(html).not.toContain("font-family")
    expect(html).toContain('<span style="color: #ff0000">b</span>')
    expect([...report.fontFamilies]).toEqual(["Calibri", "Arial"])
  })

  it("unwraps block containers, keeping each run of inline content a paragraph", () => {
    expect(
      structure(
        "<div>első</div><div>második <b>sor</b><p>bekezdés</p>harmadik</div><section><div> </div></section>"
      ).html
    ).toBe(
      "<p>első</p><p>második <strong>sor</strong></p><p>bekezdés</p><p>harmadik</p> "
    )
  })

  it("centres the blocks of a center element", () => {
    expect(
      structure(
        "<center>szöveg<p>a</p><p style='text-align: right'>b</p><table><tr><td>c</td></tr></table></center>"
      ).html
    ).toBe(
      `<p style="text-align: center">szöveg</p><p style="text-align: center">a</p><p style="text-align: right">b</p>${TABLE}<tr><td style="white-space: normal">c</td></tr></tbody></table>`
    )
  })

  it("unwraps every other element", () => {
    expect(
      structure(
        "<p><abbr title=x>rövid</abbr> <code>kód</code> <mark>jel</mark> <st1:place>hely</st1:place> <small>kicsi</small></p>"
      ).html
    ).toBe("<p>rövid kód jel hely kicsi</p>")
  })

  it("keeps the editor's own slice marker, and only on the editor's own clipboard", () => {
    expect(
      structure(`<p ${SLICE_MARKER}="0 0 []">a</p>`, "internal").html
    ).toBe(`<p ${SLICE_MARKER}="0 0 []">a</p>`)
    expect(structure(`<p ${SLICE_MARKER}="0 0 []">a</p>`).html).toBe("<p>a</p>")
  })

  it("keeps the slice marker without its context, and drops one ProseMirror does not write", () => {
    const forged =
      '1 1 ["blockquote",{"style":"background-image: url(https://evil.example/)"}]'
    expect(
      structure(`<p ${SLICE_MARKER}='${forged}'>a</p>`, "internal").html
    ).toBe(`<p ${SLICE_MARKER}="1 1 []">a</p>`)
    expect(
      structure(`<p ${SLICE_MARKER}="1 1 not json">a</p>`, "internal").html
    ).toBe("<p>a</p>")
  })

  // A rewrite that moves an element's children moves the subtree below it,
  // so nested runs that each moved cost jsdom about 5 s for 1,200 nested
  // spans, growing faster than the square of the depth. A span that keeps
  // only its colour is rewritten in place and moves nothing: the pass makes
  // no change to the tree's structure at all. (The engines' parsers cap the
  // nesting at about 512 levels, so there the input is shallower.)
  it("rewrites nested spans that keep only their colour in place, moving no subtree", () => {
    const depth = 1000
    const document = parse(
      `<p>${'<span style="color: red; font-family: Arial">'.repeat(depth)}x${"</span>".repeat(depth)}</p>`
    )
    const observer = new MutationObserver(() => {})
    observer.observe(document.body, { childList: true, subtree: true })
    normalizeStructure(document.body, context("generic"))
    const moves = observer.takeRecords()
    observer.disconnect()
    expect(moves).toEqual([])
    const spans = [...document.querySelectorAll("span")]
    expect(spans).toHaveLength(depth)
    expect(
      spans.every((span) => span.getAttribute("style") === "color: #ff0000")
    ).toBe(true)
  })

  it("maps an ordered list's style to its type and keeps its start", () => {
    expect(
      structure(
        "<ol style='list-style-type: lower-roman' start=5><li>a</li></ol><ol type=A start=1><li>b</li></ol>"
      ).html
    ).toBe('<ol start="5" type="i"><li>a</li></ol><ol type="A"><li>b</li></ol>')
  })
})

describe("frameAddress", () => {
  /** The frame of an address, in a paste whose policy is `paste.policy`. */
  const frame = (src: string) => {
    const element = parse("").createElement("iframe")
    element.setAttribute("src", src)
    return element
  }

  it("writes the node's canonical address of YouTube and Padlet frames", () => {
    const paste = context("generic")
    expect(
      frameAddress(
        frame("http://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1"),
        paste
      )
    ).toBe("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ")
    expect(
      frameAddress(frame("https://padlet.com/embed/invented0key"), paste)
    ).toBe("https://padlet.com/embed/invented0key")
    expect([...paste.report.embedParams]).toEqual(["autoplay"])
  })

  // The readers come from the policy the schema was built with; the
  // server's embed rule of the paste's policy still has the last word, so a
  // policy change can never let the filter hand on an address the server
  // would drop.
  it("drops an address the reader accepts but the paste's policy refuses", () => {
    const paste = {
      ...context("generic"),
      policy: {
        ...policy,
        urls: {
          ...policy.urls,
          embed: {
            ...policy.urls.embed,
            providers: policy.urls.embed.providers
              .filter(({ hosts }) => !hosts.includes("padlet.com"))
              .map((provider) => ({
                ...provider,
                hosts: provider.hosts.filter(
                  (host) => host !== "www.youtube-nocookie.com"
                ),
              })),
          },
        },
      },
    }
    expect(
      frameAddress(
        frame("https://www.youtube.com/embed/dQw4w9WgXcQ?controls=0"),
        paste
      )
    ).toBeNull()
    expect(
      frameAddress(frame("https://padlet.com/embed/invented0key"), paste)
    ).toBeNull()
    expect([...paste.report.embedParams]).toEqual([])
    expect([...paste.report.youtubeRefusals]).toEqual([])
  })
})

describe("tightenCellAlignment", () => {
  it("moves the alignment of a cell's only paragraph to the cell", () => {
    const document = parse(
      "<table><tr><td> <p style='text-align: center'>a</p> </td><td><p style='text-align: right'>b</p><p>c</p></td></tr></table>"
    )
    tightenCellAlignment(document.body, policy)
    expect(document.querySelector("tr")!.innerHTML).toBe(
      '<td style="text-align: center"> <p>a</p> </td><td><p style="text-align: right">b</p><p>c</p></td>'
    )
  })
})
