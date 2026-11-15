/**
 * The editor fixtures (editor.md §13.4, §13.6 "P2, P3"): named bodies and
 * command scripts whose stored HTML the L2 fixed-point tests check and the
 * golden files of `tests/fixtures/e1/` record, so the server's P2 and P3
 * tests (`tests/rich_text/golden.rs`) run over exactly what the editor
 * writes.
 *
 * - {@link CANONICAL_BODIES}: canonical bodies beyond editor.md §3.7's
 *   shapes, each a fixed point of `J` (P-fix).
 * - {@link TABLE_SHAPES}: canonical tables (§3.7 "Table"), each a fixed
 *   point of `J`.
 * - {@link TABLE_INPUTS}: table HTML that the editor rewrites on its first
 *   load (ragged rows, legacy attributes, CSSOM spellings, Office cells);
 *   `J` reaches its fixed point after one load.
 * - {@link MEDIA_BODIES}: canonical images and embeds (§3.7), each a fixed
 *   point of `J`.
 * - {@link MEDIA_INPUTS}: image and embed HTML that the editor rewrites on
 *   its first load (legacy and other editors' shapes, media inside cells,
 *   lists, quotes, links and text).
 * - {@link COMMAND_FIXTURES}: documents built with the editor's commands,
 *   the ones the toolbar's buttons call (§4.5 "Toolbar"), so that what a
 *   button writes is checked against the server too.
 * - {@link MEDIA_COMMAND_FIXTURES}: documents built with the media
 *   commands (insert a video, a Padlet board, an image at a width preset).
 *
 * Every body is invented markup (CLAUDE.md, public-safety rules). Names are
 * lower-case words joined by `-`; the golden file of a fixture is named
 * after its group and its name.
 */
import type { Editor } from "@tiptap/core"
import { CellSelection } from "@tiptap/pm/tables"

/** The forced styles of every stored table (editor.md §3.3). */
const TABLE =
  '<table style="border-collapse: collapse; max-width: fit-content">'
const TBODY = '<tbody style="display: table-row-group">'
const END = "</tbody></table>"

/** A stored table around the given rows. */
export const table = (rows: string) => `${TABLE}${TBODY}${rows}${END}`

/** The cell style of a 1 px black border on all four sides. */
const BLACK_BOX =
  "border-bottom: 1px solid #000000; border-left: 1px solid #000000; border-right: 1px solid #000000; border-top: 1px solid #000000"

/**
 * Canonical bodies beyond the canonical shapes: what the editor writes for
 * spaces, breaks, headings, lists, quotes, marks and links.
 */
export const CANONICAL_BODIES: Readonly<Record<string, string>> = {
  "empty-paragraph": "<p></p>",
  rule: "<hr>",
  // Typed spaces as the whitespace rule writes them (editor.md §3.8).
  "typed-spaces": "<p>két&nbsp; szóköz és&nbsp;&nbsp; három</p>",
  "hard-breaks": "<p>sor<br>új sor<br><br>üres sor előtte</p>",
  "space-before-break": "<p>szóköz a sortörés előtt <br>után</p>",
  headings:
    '<h1>Régi cím</h1><h3>Alcím</h3><h4 style="text-align: justify">Sorkizárt</h4>',
  "aligned-paragraph": '<p style="text-align: right">jobbra</p>',
  "nested-ordered-lists":
    '<ol type="I"><li>a<ol start="3"><li>b</li></ol></li><li></li></ol>',
  "nested-mixed-lists":
    '<ul style="list-style-type: square"><li>a<ul><li>b</li></ul><ol><li>c</li></ol></li></ul>',
  "aligned-list-item":
    '<ul><li><p style="text-align: center">középre</p></li></ul>',
  "quote-paragraphs": "<blockquote><p>két</p><p>bekezdés</p></blockquote>",
  "quote-list": "<blockquote><ul><li>lista</li></ul></blockquote>",
  "empty-quote": "<blockquote></blockquote>",
  "strike-underline-scripts":
    "<p><s>áthúzott</s> <u>aláhúzott</u> x<sub>2</sub> x<sup>3</sup></p>",
  "translucent-highlights":
    '<p><span style="background-color: transparent">átlátszó</span> <span style="background-color: #000000b3">áttetsző</span></p>',
  "all-text-styles":
    '<p><span style="background-color: #ffff00; color: #c00000; font-size: 1.82em">minden</span></p>',
  "link-schemes":
    '<p><a href="mailto:titkarsag@example.org" target="_blank" rel="noopener noreferrer">levél</a>, <a href="tel:+3612345678" target="_blank" rel="noopener noreferrer">telefon</a>, <a href="/hirek/12" target="_blank" rel="noopener noreferrer">hír</a>, <a href="#top" target="_blank" rel="noopener noreferrer">fel</a></p>',
  "links-without-target":
    '<p><a target="_blank" rel="noopener noreferrer">cél nélkül</a> <a href="" target="_blank" rel="noopener noreferrer">üres cél</a></p>',
  "url-as-text": "<p>Lásd example.org</p><p>Vége</p>",
  "escaped-text": '<p>&lt;script&gt; &amp; "idézet" „Videó”</p>',
  // editor.md §3.7 (item D148, answered on 2026-11-15): an empty first
  // list-item paragraph is kept in front of the item's lists, because
  // `<li><ul>` reloads with the nested list moved out of the item.
  "empty-paragraph-before-bullet-list":
    "<ul><li><p></p><ul><li>b</li></ul></li><li></li></ul><p></p>",
  "empty-paragraph-before-ordered-list":
    "<ol><li><p></p><ol><li>b</li></ol><ul><li></li></ul></li></ol>",
}

/**
 * Canonical tables: the shapes of editor.md §3.7 "Table", every cell
 * declaration of §3.3, and tables where the schema allows them.
 */
export const TABLE_SHAPES: Readonly<Record<string, string>> = {
  "excel-paste-cell": table(
    '<tr><td colspan="2" style="background-color: #d9d9d9; border-bottom: 1px solid #000000; border-left: none; border-right: 1px solid #000000; border-top: 1px solid #000000; box-sizing: content-box; height: 20px; text-align: center; vertical-align: middle; white-space: nowrap; width: 128px"><strong>Eredmény</strong></td></tr>'
  ),
  "header-row": table(
    "<tr><th>Név</th><th>Pont</th></tr><tr><td>Anna</td><td>12</td></tr><tr><td>Bence</td><td>9</td></tr>"
  ),
  "header-column": table(
    "<tr><th>1. óra</th><td>matematika</td></tr><tr><th>2. óra</th><td>irodalom</td></tr>"
  ),
  "merged-cells": table(
    '<tr><th colspan="3">Fej</th></tr><tr><td rowspan="2">a</td><td>b</td><td>c</td></tr><tr><td colspan="2">d</td></tr>'
  ),
  "border-styles": table(
    '<tr><td style="border-bottom: 2px dashed #ff0000; border-left: 3px dotted #00ff00; border-right: 3px double #0000ff; border-top: 1px groove #000000">a</td><td style="border-bottom: 4px ridge #808080; border-left: 1px inset #000000; border-right: 1px outset #000000; border-top: hidden">b</td><td style="border-bottom: none; border-left: none; border-right: none; border-top: none">c</td><td style="border-bottom: 0px solid currentcolor; border-left: 20px solid #00000080; border-right: 0.5px solid currentcolor; border-top: 1.33px solid #000000">d</td></tr>'
  ),
  "aligned-cells": table(
    '<tr><td style="text-align: left; vertical-align: top">a</td><td style="text-align: center; vertical-align: middle">b</td><td style="text-align: right; vertical-align: bottom">c</td><td style="text-align: justify; vertical-align: baseline">d</td></tr>'
  ),
  "wrapping-cells": table(
    '<tr><td style="white-space: nowrap">egy hosszú sor</td><td style="white-space: normal">tördelt sor</td></tr>'
  ),
  "sized-cells": table(
    '<tr><td style="box-sizing: content-box; width: 1px">a</td><td style="box-sizing: content-box; height: 2000px">b</td><td style="box-sizing: content-box; height: 20.5px; width: 128.25px">c</td></tr>'
  ),
  "cell-fills": table(
    '<tr><td style="background-color: #d9d9d9">a</td><td style="background-color: #ffff0080">b</td><td style="background-color: transparent">c</td><td style="background-color: currentcolor">d</td></tr>'
  ),
  "every-cell-declaration": table(
    `<tr><th rowspan="2" style="background-color: #fce4d6; ${BLACK_BOX}; box-sizing: content-box; height: 40px; text-align: center; vertical-align: middle; white-space: nowrap; width: 200px">Fejléc</th><td>a</td></tr><tr><td style="${BLACK_BOX}">b</td></tr>`
  ),
  "cell-blocks": table(
    '<tr><td><p>két</p><p>bekezdés</p></td><td><h2>Cím</h2></td><td><ul><li>pont</li></ul><ol start="2"><li>szám</li></ol></td></tr><tr><td><blockquote>idézet</blockquote></td><td>sor<br>új sor</td><td><strong>félkövér</strong> <a href="https://example.org/" target="_blank" rel="noopener noreferrer">link</a> <span style="color: #c00000">piros</span></td></tr>'
  ),
  "aligned-paragraphs-in-cell": table(
    '<tr><td style="text-align: center"><p>a</p><p style="text-align: right">b</p></td><td><h3 style="text-align: center">c</h3></td></tr>'
  ),
  "empty-cells": table(
    '<tr><th></th><td></td></tr><tr><td style="text-align: center"></td><td style="background-color: #d9d9d9"></td></tr>'
  ),
  "nested-table": table(
    `<tr><td>kívül</td><td>${table('<tr><td style="text-align: right">belül</td></tr>')}</td></tr>`
  ),
  "table-in-list-item": `<ul><li><p>pont</p>${table("<tr><td>a</td></tr>")}</li><li><p></p>${table("<tr><td>b</td></tr>")}</li></ul>`,
  "table-in-quote": `<blockquote>${table("<tr><td>a</td></tr>")}</blockquote>`,
  "tables-between-paragraphs": `<p>előtte</p>${table("<tr><td>a</td></tr>")}${table("<tr><td>b</td></tr>")}<p>utána</p>`,
  "spaces-in-cells": table(
    "<tr><td>két&nbsp; szóköz</td><td>&nbsp;</td><td>a&nbsp;&nbsp; b</td></tr>"
  ),
}

/**
 * Table HTML that the editor rewrites on its first load. Each comment says
 * what the first load changes.
 */
export const TABLE_INPUTS: Readonly<Record<string, string>> = {
  // Short rows get empty cells (prosemirror-tables' fixTables).
  "ragged-rows":
    "<table><tr><td>a</td><td>b</td><td>c</td></tr><tr><td>d</td></tr><tr></tr></table>",
  // A rowspan past the last row is shortened.
  "overlong-rowspan":
    '<table><tr><td rowspan="9">a</td><td>b</td></tr><tr><td>c</td></tr></table>',
  // colspan and rowspan the server would drop read as 1.
  "spans-outside-the-rule":
    '<table><tr><td colspan="abc" rowspan="0">a</td><td colspan="1" rowspan="1">b</td><td colspan=" 2 ">c</td></tr></table>',
  // The presentational hints of a cell become its declarations.
  "legacy-cell-attributes":
    '<table border="1" width="500" align="center"><tr><td align="middle" valign="TOP" bgcolor="d9d9d9" width="120" height="20.5px">a</td><td align="justify" valign="baseline" bgcolor="#FFF" width="50%" height="0">b</td><td align="absmiddle" bgcolor="transparent">c</td></tr></table>',
  // The style attribute wins over a hint.
  "style-over-hints":
    '<table><tr><td align="left" valign="top" bgcolor="red" width="100" style="text-align: right; vertical-align: bottom; background: #00f; width: 80px">a</td></tr></table>',
  // thead and tfoot rows join the body, a caption becomes a row, colgroup
  // widths are not kept.
  "thead-tfoot-caption-colgroup":
    '<table><caption>Felirat</caption><colgroup><col width="300"><col width="100"></colgroup><thead><tr><th>h</th><th>i</th></tr></thead><tbody><tr><td>a</td><td>b</td></tr></tbody><tfoot><tr><td>f</td><td>g</td></tr></tfoot></table>',
  // What a browser's CSSOM writes into the editor's own clipboard.
  "cssom-spellings":
    '<table style="border-collapse: collapse; max-width: fit-content;"><tbody style="display: table-row-group;"><tr><td style="background-color: rgb(217, 217, 217); border-width: 1px; border-style: solid; border-color: rgb(0, 0, 0); width: 96pt; box-sizing: content-box;">a</td></tr></tbody></table>',
  // An Excel desktop cell: class-free inline styles with Office noise.
  "excel-cell":
    '<table width="128" style="border-collapse:collapse;width:96pt"><tr height="20" style="height:15.0pt"><td height="20" class="xl65" width="128" style="height:15.0pt;width:96pt;border:.5pt solid windowtext;border-top:none;background:#D9D9D9;mso-pattern:black none;text-align:center;vertical-align:middle;white-space:nowrap;font-weight:700;font-family:Calibri">Eredmény</td></tr></table>',
  // A Word cell: padding, percent widths and paragraph classes.
  "word-cell":
    '<table class="MsoTableGrid" border="1" cellspacing="0" cellpadding="0" width="100%" style="width:100.0%;border-collapse:collapse;border:none"><tr><td width="50%" valign="top" style="width:50.0%;border:solid windowtext 1.0pt;padding:0cm 5.4pt 0cm 5.4pt"><p class="MsoNormal" align="center" style="text-align:center"><span lang="HU">szöveg</span></p></td></tr></table>',
  // A lone aligned paragraph moves its alignment to the cell.
  "aligned-cell-paragraph":
    '<table><tr><td><p style="text-align: center">a</p></td><td style="text-align: left"><p style="text-align: right">b</p></td><td><p style="text-align: center"></p></td></tr></table>',
  // Declarations a cell does not store are dropped, and so is a table
  // width.
  "dropped-declarations":
    '<table style="width: 700px; min-width: 100px; float: left"><tr><td style="padding: 5px; font-weight: bold; color: red; font-family: Arial; display: none; width: 100%px">a</td></tr></table>',
  // A table that opens a list item moves out of the item (TipTap's list
  // item starts with a paragraph); the editor never writes this form.
  "table-first-in-list-item":
    "<ul><li><table><tr><td>a</td></tr></table></li></ul>",
  // Spans that collide: narrowing the colliding cell leaves rows of other
  // lengths, which one fixTables pass does not see (fixTablesFully in
  // site-table.ts); the shrunk counterexample of tables.property.test.ts.
  "colliding-spans":
    '<table><tbody><tr></tr></tbody><tfoot><tr><td><table><tr><td>A</td></tr></table></td><td rowspan="2"><p>A</p></td></tr><tr><td colspan="2"></td></tr></tfoot></table>',
  // A table with no cell holds nothing to keep.
  "empty-table": "<p>a</p><table><tr></tr></table>",
  // A span past the columns where a cell starts loses them before the
  // short rows are padded (unanchoredSpans in site-table.ts).
  "span-past-the-grid":
    '<table><tr><td colspan="3">a</td></tr><tr><td>b</td><td>c</td></tr><tr><td>d</td></tr></table>',
  // The widest span the server keeps adds no cell to the short rows: the
  // table stays one column wide instead of a thousand.
  "widest-colspan":
    '<table><tr><td colspan="1000">Fejléc</td></tr><tr><td>a</td></tr><tr><td>b</td></tr></table>',
  // Columns between cells in which no cell starts are dropped too.
  "wide-colspan-between-cells":
    '<table><tr><td>a</td><td colspan="500">b</td><td>c</td></tr><tr><td>d</td><td>e</td></tr></table>',
  // A wide span that also collides with a rowspan from above.
  "wide-colliding-colspan":
    '<table><tr><td>a</td><td rowspan="2">b</td></tr><tr><td colspan="1000">c</td></tr></table>',
  // A style declaration the server cannot store still hides the hint for
  // its property, as in a browser, so the cell keeps neither.
  "hints-under-unstorable-styles":
    '<table><tr><td width="120" style="width: 50%">a</td><td align="center" style="text-align: start">b</td><td bgcolor="#d9d9d9" style="background: url(x.png)">c</td><td valign="top" style="vertical-align: -2px">d</td></tr></table>',
  // bgcolor keywords that CSS reads but HTML's legacy colour rules do not
  // count as named colours give no fill; a named colour does.
  "bgcolor-keywords":
    '<table><tr><td bgcolor="currentcolor">a</td><td bgcolor="windowtext">b</td><td bgcolor="Canvas">c</td><td bgcolor="Khaki">d</td></tr></table>',
}

/** The locked declarations of every stored image (editor.md §3.3). */
const IMAGE_STYLE =
  "display: block; height: auto; margin: 0 auto; max-width: 100%"

/** The forced attributes of every stored iframe (editor.md §3.2). */
const FRAME =
  'allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"'

/** A stored YouTube embed of `src` (editor.md §3.7 "YouTube"). */
const youtube = (src: string, title = "Videó") =>
  `<p data-youtube-video=""><iframe src="${src}" width="640" height="360" title="${title}" style="aspect-ratio: 16 / 9; height: auto; max-width: 640px; min-height: 200px; width: 100%" ${FRAME}></iframe></p>`

/** A stored image (editor.md §3.7 "Image"). */
const image = (src: string, alt: string, width: number, height: number) =>
  `<p><img src="${src}" alt="${alt}" width="${width}" height="${height}" style="${IMAGE_STYLE}; width: ${width}px"></p>`

/**
 * Canonical images and embeds (editor.md §3.7 "Image", "YouTube",
 * "Padlet"; §4.3 rows 8-10), each a fixed point of `J`.
 */
export const MEDIA_BODIES: Readonly<Record<string, string>> = {
  image: image(
    "/storage/uploads/posts/7/udvar.jpg",
    "Az iskola udvara",
    525,
    394
  ),
  "image-without-size": `<p><img src="/storage/uploads/posts/7/logo.png" alt="Logó" style="${IMAGE_STYLE}"></p>`,
  // Legacy widths up to the style's 2000 px are kept (53 are wider than
  // 700 px [500]).
  "image-wide-legacy": image(
    "/storage/uploads/posts/7/tabla.png",
    "Tabló",
    1200,
    800
  ),
  "youtube-video": youtube(
    "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=30"
  ),
  "youtube-playlist": youtube(
    "https://www.youtube-nocookie.com/embed/videoseries?list=PLMOZDTW7ZvXM"
  ),
  "youtube-video-in-playlist": youtube(
    "https://www.youtube-nocookie.com/embed/71ya_JC_IbM?start=8&amp;list=PLMOZDTW7ZvXM"
  ),
  // A stored title is kept as it is (D135).
  "youtube-own-title": youtube(
    "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
    "A ballagás"
  ),
  padlet: `<p data-padlet-embed=""><iframe src="https://padlet.com/embed/KEY" width="100%" height="608" title="Padlet" style="height: 608px; width: 100%" ${FRAME}></iframe></p>`,
  "padlet-tall": `<p data-padlet-embed=""><iframe src="https://padlet.com/embed/KEY" width="100%" height="900" title="Padlet" style="height: 900px; width: 100%" ${FRAME}></iframe></p>`,
  "media-between-paragraphs": `<p>Előtte</p>${image("/storage/uploads/posts/7/a.jpg", "a", 175, 175)}${youtube("https://www.youtube-nocookie.com/embed/B5T4ZCfxp1Y")}<p>Utána</p>`,
}

/**
 * Image and embed HTML that the editor rewrites on its first load: legacy
 * and other editors' shapes, and media in places content-v1 does not keep
 * them (editor.md §4.2, §4.3 rows 8-10); `J` reaches its fixed point after
 * one load.
 */
export const MEDIA_INPUTS: Readonly<Record<string, string>> = {
  "image-in-link":
    '<p><a href="https://example.org/"><img src="/storage/uploads/a.jpg" alt="a" width="350" height="350"></a></p>',
  "image-in-text":
    '<p>Előtte <img src="/storage/uploads/a.jpg" alt="a"> utána</p>',
  "image-in-cell": `<table><tr><td>a<img src="/storage/uploads/a.jpg" alt="a"></td></tr></table>`,
  "image-in-list-item":
    '<ul><li>pont<img src="/storage/uploads/a.jpg" alt="a"></li></ul>',
  "image-own-host":
    '<p><img src="https://verseghy-gimnazium.net/storage/uploads/a.jpg" alt="a" width="800" height="600" style="height: 600px; width: 600pt"></p>',
  "youtube-tiptap-wrapper":
    '<div data-youtube-video=""><iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ?si=x&amp;rel=0" width="560" height="315"></iframe></div>',
  "youtube-legacy-paragraph":
    '<p><iframe width="560" height="315" src="https://www.youtube.com/embed/dQw4w9WgXcQ" frameborder="0" allowfullscreen></iframe></p>',
  "youtube-http":
    '<iframe src="http://www.youtube.com/embed/UkWd0azv3fQ?start=90" width="420" height="315"></iframe>',
  "youtube-list-type-playlist":
    '<iframe src="https://www.youtube.com/embed?listType=playlist&amp;list=PLC77007E23FF423C6"></iframe>',
  "youtube-in-quote":
    '<blockquote><p>idézet</p><p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ"></iframe></p></blockquote>',
  "padlet-legacy":
    '<p><iframe allowfullscreen="" src="https://padlet.com/embed/KEY" style="border: 0; width: 100%; height: 608px;" tabindex="-1"></iframe></p>',
}

/** The positions of every table cell, in document order. */
function cellPositions(editor: Editor): number[] {
  const cells: number[] = []
  editor.state.doc.descendants((node, position) => {
    const role: unknown = node.type.spec.tableRole
    if (role === "cell" || role === "header_cell") {
      cells.push(position)
    }
    return true
  })
  return cells
}

/**
 * Selects the cells from `anchor` to `head`, each given as its index in
 * document order.
 */
function selectCells(editor: Editor, anchor: number, head: number): void {
  const cells = cellPositions(editor)
  editor.view.dispatch(
    editor.state.tr.setSelection(
      CellSelection.create(editor.state.doc, cells[anchor], cells[head])
    )
  )
}

/**
 * Puts the cursor at the start of the cell with the given index in
 * document order: inside the cell (+1) and its first paragraph (+1).
 */
function inCell(editor: Editor, index: number): void {
  editor.commands.setTextSelection(cellPositions(editor)[index] + 2)
}

/**
 * Selects the first occurrence of `needle` in the document's text, so a
 * script names the words it changes instead of counting positions.
 *
 * @throws {Error} when the text is not in the document.
 */
function selectText(editor: Editor, needle: string): Editor {
  const found: { from: number; to: number }[] = []
  editor.state.doc.descendants((node, position) => {
    const index = node.isText ? (node.text ?? "").indexOf(needle) : -1
    if (found.length === 0 && index !== -1) {
      found.push({
        from: position + index,
        to: position + index + needle.length,
      })
    }
    return found.length === 0
  })
  if (found.length === 0) {
    throw new Error(`selectText: "${needle}" is not in the document.`)
  }
  editor.commands.setTextSelection(found[0])
  return editor
}

/** Inserts a table of `rows` by `cols` with a header row at the end. */
function withTable(editor: Editor, rows: number, cols: number): void {
  editor.commands.focus("end")
  editor.commands.insertTable({ rows, cols, withHeaderRow: true })
}

/**
 * Documents built with the editor's commands. Each script starts on an
 * editor loaded with `<p>Táblázat:</p>` and leaves the document whose
 * stored HTML is the fixture.
 */
export const COMMAND_FIXTURES: Readonly<
  Record<string, (editor: Editor) => void>
> = {
  "table-insert": (editor) => withTable(editor, 3, 3),
  "table-insert-text": (editor) => {
    withTable(editor, 2, 2)
    inCell(editor, 0)
    editor.commands.insertContent("Név")
    inCell(editor, 2)
    editor.commands.insertContent("Anna")
  },
  "table-add-rows-and-columns": (editor) => {
    withTable(editor, 2, 2)
    inCell(editor, 3)
    // One command at a time: prosemirror-tables' commands read the
    // selection from the state, which a TipTap chain does not map.
    editor.commands.addRowAfter()
    editor.commands.addColumnAfter()
    inCell(editor, 0)
    editor.commands.addRowBefore()
    editor.commands.addColumnBefore()
  },
  "table-delete-row-and-column": (editor) => {
    withTable(editor, 3, 3)
    inCell(editor, 4)
    editor.commands.deleteRow()
    editor.commands.deleteColumn()
  },
  "table-merge-cells": (editor) => {
    withTable(editor, 3, 3)
    selectCells(editor, 3, 7)
    editor.commands.mergeCells()
  },
  "table-split-cell": (editor) => {
    withTable(editor, 3, 3)
    selectCells(editor, 3, 4)
    editor.commands.mergeCells()
    inCell(editor, 3)
    editor.commands.splitCell()
  },
  "table-header-cells": (editor) => {
    withTable(editor, 3, 3)
    editor.commands.toggleHeaderRow()
    editor.commands.toggleHeaderColumn()
    inCell(editor, 8)
    editor.commands.toggleHeaderCell()
  },
  "table-cell-fill": (editor) => {
    withTable(editor, 2, 2)
    selectCells(editor, 0, 1)
    editor.commands.setCellAttribute("backgroundColor", "#d9d9d9")
    inCell(editor, 3)
    editor.commands.setCellAttribute("backgroundColor", "#ffff00")
  },
  "table-borders-all": (editor) => {
    withTable(editor, 2, 2)
    selectCells(editor, 0, 3)
    for (const side of [
      "borderTop",
      "borderRight",
      "borderBottom",
      "borderLeft",
    ]) {
      editor.commands.setCellAttribute(side, "1px solid #000000")
    }
  },
  "table-borders-none": (editor) => {
    withTable(editor, 2, 2)
    selectCells(editor, 0, 3)
    for (const side of [
      "borderTop",
      "borderRight",
      "borderBottom",
      "borderLeft",
    ]) {
      editor.commands.setCellAttribute(side, "none")
    }
  },
  "table-one-border-side": (editor) => {
    withTable(editor, 2, 2)
    inCell(editor, 2)
    editor.commands.setCellAttribute("borderBottom", "2px solid #000000")
  },
  "table-vertical-align": (editor) => {
    withTable(editor, 2, 2)
    selectCells(editor, 0, 3)
    editor.commands.setCellAttribute("verticalAlign", "middle")
  },
  "table-wrap-toggle": (editor) => {
    withTable(editor, 2, 2)
    selectCells(editor, 2, 3)
    editor.commands.setCellAttribute("whiteSpace", "nowrap")
  },
  "table-cell-sizes": (editor) => {
    withTable(editor, 2, 2)
    inCell(editor, 2)
    editor.commands.setCellAttribute("width", "128px")
    inCell(editor, 3)
    editor.commands.setCellAttribute("height", "20px")
  },
  "table-align-in-cell": (editor) => {
    withTable(editor, 2, 2)
    inCell(editor, 2)
    editor.commands.insertContent("jobbra")
    editor.commands.setTextAlign("right")
    selectCells(editor, 0, 1)
    editor.commands.setTextAlign("center")
  },
  "table-delete": (editor) => {
    withTable(editor, 2, 2)
    inCell(editor, 0)
    editor.commands.deleteTable()
  },
  "text-headings-and-marks": (editor) => {
    editor.commands.setContent("<p>Cím</p><p>félkövér dőlt aláhúzott</p>")
    selectText(editor, "Cím").commands.setHeading({ level: 3 })
    selectText(editor, "félkövér").commands.toggleBold()
    selectText(editor, "dőlt").commands.toggleItalic()
    selectText(editor, "aláhúzott").commands.toggleUnderline()
  },
  "text-style-and-align": (editor) => {
    editor.commands.setContent("<p>piros, kiemelt és nagy</p>")
    selectText(editor, "piros").commands.setColor("#c00000")
    selectText(editor, "kiemelt").commands.setBackgroundColor("#ffff00")
    selectText(editor, "nagy").commands.setFontSize("1.5em")
    editor.commands.setTextAlign("center")
  },
  "lists-and-quote": (editor) => {
    editor.commands.setContent("<p>pont</p><p>szám</p><p>idézet</p>")
    selectText(editor, "pont").commands.toggleBulletList()
    selectText(editor, "szám").commands.toggleOrderedList()
    selectText(editor, "idézet").commands.toggleBlockquote()
    editor.commands.focus("end")
    editor.commands.setHorizontalRule()
  },
}

/**
 * Documents built with the media commands that the toolbar's image, video
 * and Padlet buttons and the image's width presets call (editor.md §4.3
 * rows 8-10, §4.5). Each script starts like {@link COMMAND_FIXTURES}.
 */
export const MEDIA_COMMAND_FIXTURES: Readonly<
  Record<string, (editor: Editor) => void>
> = {
  "insert-youtube": (editor) => {
    editor.commands.focus("end")
    editor.commands.insertYoutubeEmbed({
      videoId: "dQw4w9WgXcQ",
      start: "90",
      list: null,
      title: "Videó",
    })
  },
  "insert-padlet": (editor) => {
    editor.commands.focus("end")
    editor.commands.insertPadletEmbed({
      key: "KEY",
      height: 608,
      title: "Padlet",
    })
  },
  "insert-image-at-a-preset": (editor) => {
    editor.commands.focus("end")
    editor.commands.insertSiteImage({
      src: "/storage/uploads/posts/7/kep.jpg",
      alt: "Kép",
      width: 1600,
      height: 1200,
    })
    // A width preset, as the image's view sets it.
    let position = -1
    editor.state.doc.forEach((node, offset) => {
      if (node.type.name === "siteImage") {
        position = offset
      }
    })
    editor.commands.command(({ tr }) => {
      tr.setNodeAttribute(position, "displayWidth", 350)
      return true
    })
  },
}
