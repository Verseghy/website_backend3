/**
 * fast-check clipboard HTML for the paste filter's properties (editor.md
 * §13.3, §13.5 "Degrade safely"): random documents over a pool of tags,
 * attributes, styles and addresses like those of the server's P1 fuzz
 * (§13.6), so that the filter meets Office and web markup, the elements the
 * server removes with their content, every URL rule's refusals and the CSS
 * that content-v1 rejects.
 *
 * The documents are strings, as the clipboard hands them over: tags are
 * written as they are drawn, so misnested and unclosed markup reaches the
 * HTML parser too.
 */
import fc from "fast-check"

/** The tags a random document is built from. */
const TAGS = [
  "p",
  "div",
  "span",
  "b",
  "i",
  "u",
  "s",
  "strong",
  "em",
  "sub",
  "sup",
  "font",
  "center",
  "h1",
  "h2",
  "h4",
  "h5",
  "h6",
  "ul",
  "ol",
  "li",
  "blockquote",
  "table",
  "tbody",
  "thead",
  "tfoot",
  "tr",
  "td",
  "th",
  "caption",
  "colgroup",
  "col",
  "a",
  "br",
  "hr",
  "img",
  "iframe",
  "o:p",
  "v:shape",
  "w:sdt",
  "svg",
  "math",
  "style",
  "script",
  "section",
  "pre",
  "code",
  "mark",
  "noscript",
  "template",
  "textarea",
  "button",
  "marquee",
  "title",
  "xmp",
  "picture",
  "figure",
  "dl",
  "dd",
] as const

/** The attribute names a random element may carry. */
const ATTRIBUTES = [
  "style",
  "class",
  "id",
  "align",
  "valign",
  "bgcolor",
  "width",
  "height",
  "colspan",
  "rowspan",
  "span",
  "start",
  "type",
  "href",
  "src",
  "alt",
  "title",
  "color",
  "onclick",
  "data-x",
  "dir",
  "lang",
] as const

/** Values of `style`: Office and web declarations, and CSS content-v1 rejects. */
const STYLES = [
  "color: red",
  "color:#000000",
  "background: #CCCCCC",
  "background-color: transparent",
  "font-weight: 700",
  "font-weight:normal",
  "font-style: italic",
  "text-decoration: underline line-through",
  "vertical-align: super",
  "display:none",
  "visibility: hidden",
  "border: .5pt solid windowtext",
  "border-top: 1.0pt dashed black; border-left: none",
  "width: 48pt",
  "width: 50%",
  "height: 15.0pt",
  "white-space: nowrap",
  "text-align: center",
  "text-align: general",
  "font-size: 14pt",
  "font-size: 1.5em",
  "font-family: Calibri, sans-serif",
  "mso-number-format: General; mso-list: l0 level1 lfo1",
  "list-style-type: lower-roman",
  "background: url(javascript:alert(1))",
  "width: expression(alert(1))",
  "--x: 1",
  "margin: 0 auto",
  "color: rgb(192, 0, 0) !important",
  "/* comment */ color: blue",
] as const

/** Addresses: kept, rewritten and refused by the URL rules of editor.md §3.6. */
const ADDRESSES = [
  "https://example.org/",
  "javascript:alert(1)",
  "/storage/uploads/a.jpg",
  "https://verseghy-gimnazium.net/storage/uploads/b.png",
  "/storage/uploads/../../x.jpg",
  "data:image/png;base64,AAAA",
  "blob:https://example.org/x",
  "file:///invented/x.png",
  "https://www.youtube.com/embed/dQw4w9WgXcQ?si=invented_si-0001&controls=0",
  "http://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=5",
  "https://www.youtube.com/embed?listType=playlist&list=PLinvented",
  "https://www.youtube.com/embed/abc",
  "https://padlet.com/embed/invented0key",
  "https://evil.example/embed/x",
  "//evil.example/x",
  "#top",
  "",
] as const

/** Other attribute values. */
const VALUES = [
  "2",
  "0",
  "1000",
  "100%",
  "x",
  "a",
  "I",
  "center",
  "top",
  "#ff0",
  "yellow",
  "1",
] as const

/** The text a random document holds. */
const TEXTS = [
  "a",
  "szöveg",
  " ",
  "\n",
  "&nbsp;",
  "&lt;",
  "&amp;",
  " ",
  "őű",
] as const

/** One random attribute, written as ` name="value"`. */
const attribute = fc
  .tuple(
    fc.constantFrom(...ATTRIBUTES),
    fc.oneof(
      fc.constantFrom(...STYLES),
      fc.constantFrom(...ADDRESSES),
      fc.constantFrom(...VALUES)
    )
  )
  .map(([name, value]) => ` ${name}="${value.replace(/"/g, "&quot;")}"`)

/**
 * Values of ProseMirror's `data-pm-slice`, the editor's own clipboard
 * (editor.md §5.2): open depths and wrapper counts that need not match the
 * document they come with, contexts (which the filter leaves out, so that
 * no mismatched one reaches prosemirror-view), forged attributes, and
 * values prosemirror-view does not write, which the filter drops.
 */
const SLICE_MARKERS = [
  "0 0 []",
  "1 1 []",
  "2 2 []",
  "0 0 -1 []",
  "1 1 -3 []",
  '1 1 ["blockquote",null]',
  '2 2 -1 ["bulletList",null,"listItem",null]',
  '3 3 ["table",null,"tableRow",null,"tableCell",{"colspan":1,"rowspan":1,"style":"background-color: #ff0000"}]',
  '1 1 ["paragraph",{"style":"color: red\\" onclick=\\"x"}]',
  '1 1 [{"x":1},null]',
  "1 1 not json",
  "99999 1 []",
  "",
] as const

/** Random clipboard HTML, from any source but the editor's own clipboard. */
const outsideHtml: fc.Arbitrary<string> = fc.letrec<{
  node: string
  text: string
  element: string
}>((tie) => ({
  node: fc.oneof(
    { maxDepth: 5, depthIdentifier: "clipboard-html" },
    tie("text"),
    tie("element")
  ),
  text: fc.constantFrom(...TEXTS),
  element: fc
    .tuple(
      fc.constantFrom(...TAGS),
      fc.array(attribute, { maxLength: 3 }),
      fc.array(tie("node"), { maxLength: 4 })
    )
    .map(
      ([tag, attributes, children]) =>
        `<${tag}${attributes.join("")}>${children.join("")}</${tag}>`
    ),
})).node

/**
 * Random clipboard HTML: in about one document of four, a paragraph with a
 * `data-pm-slice` marker comes first, as on the editor's own clipboard,
 * so the properties also take the filter's internal path (§5.2, "The
 * editor's own clipboard"), forged and broken markers included.
 */
export const clipboardHtml: fc.Arbitrary<string> = fc
  .tuple(
    fc.oneof(
      { arbitrary: fc.constant(null), weight: 3 },
      { arbitrary: fc.constantFrom(...SLICE_MARKERS), weight: 1 }
    ),
    outsideHtml
  )
  .map(([marker, html]) =>
    marker === null
      ? html
      : `<p data-pm-slice="${marker.replace(/&/g, "&amp;").replace(/"/g, "&quot;")}">a</p>${html}`
  )
