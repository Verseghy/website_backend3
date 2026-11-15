import type { Node as ProseMirrorNode } from "@tiptap/pm/model"
import fc from "fast-check"
import { afterAll, describe, expect, it } from "vitest"

import { canonicalStyle } from "../css/canonical-style"
import { CONTENT_V1_POLICY } from "../policy"
import { reparse, storedHtmlHarness } from "@/test/stored-html"
import type { StoredNode } from "./stored-tree"

/**
 * editor.md §13.4, L2, fast-check over tables:
 *
 * - every table the schema can hold, rectangular with merged cells and any
 *   cell declarations, is stored as a fixed point:
 *   `toStoredHtml(load(toStoredHtml(d))) == toStoredHtml(d)`;
 * - any table HTML, ragged, with legacy attributes, Office declarations,
 *   captions, column groups and header sections, reaches its fixed point in
 *   one load: `J(J(x)) == J(x)`;
 * - and in both, every stored cell carries only `colspan`, `rowspan` and a
 *   `style` in the server's canonical spelling, and the table its forced
 *   styles.
 */
const harness = storedHtmlHarness()
afterAll(() => harness.destroy())
const { J, save, editor } = harness
const POLICY = CONTENT_V1_POLICY
const schema = editor.schema
const { nodes } = schema

/** Everything in a stored body that a stored table must not hold. */
function tableProblems(stored: readonly StoredNode[]): string[] {
  const problems: string[] = []
  const visit = (node: StoredNode) => {
    if (node.kind === "text") {
      return
    }
    if (node.tag === "td" || node.tag === "th") {
      for (const [name, value] of node.attributes) {
        if (name === "style") {
          const canonical = canonicalStyle(POLICY, node.tag, value)
          if (canonical !== value) {
            problems.push(
              `${node.tag}[style="${value}"] is stored as "${canonical}"`
            )
          }
        } else if (!["colspan", "rowspan"].includes(name)) {
          problems.push(`${node.tag}[${name}]`)
        }
      }
    }
    if (["table", "tbody"].includes(node.tag)) {
      const forced = POLICY.elements[node.tag].attributes.find(
        ({ name }) => name === "style"
      )?.value
      const expected = forced?.kind === "forced" ? forced.value : null
      if (
        node.attributes.length !== 1 ||
        node.attributes[0][0] !== "style" ||
        node.attributes[0][1] !== expected
      ) {
        problems.push(`${node.tag} ${JSON.stringify(node.attributes)}`)
      }
    }
    if (["colgroup", "col", "thead", "tfoot", "caption"].includes(node.tag)) {
      problems.push(`<${node.tag}>`)
    }
    node.children.forEach(visit)
  }
  stored.forEach(visit)
  return problems
}

// Random tables of the schema.

const word = fc.stringMatching(/^[a-zA-Z0-9áéőúű.,!?-]{1,8}$/)
const side = fc.constantFrom(
  "1px solid #000000",
  "none",
  "hidden",
  "2px dashed #ff0000",
  "0px solid currentcolor",
  "3px double #0000ff",
  "0.67px dotted #00000080"
)
const borders = fc.oneof(
  fc.constant({}),
  side.map((value) => ({
    borderTop: value,
    borderRight: value,
    borderBottom: value,
    borderLeft: value,
  })),
  fc.record({
    borderTop: side,
    borderRight: side,
    borderBottom: side,
    borderLeft: side,
  }),
  fc
    .tuple(
      fc.constantFrom("borderTop", "borderRight", "borderBottom", "borderLeft"),
      side
    )
    .map(([name, value]) => ({ [name]: value }))
)
const cellAttrs = fc
  .tuple(
    fc.record({
      backgroundColor: fc.constantFrom(
        null,
        "#d9d9d9",
        "#ffff0080",
        "transparent",
        "currentcolor"
      ),
      width: fc.constantFrom(null, "1px", "128px", "20.5px", "2000px"),
      height: fc.constantFrom(null, "20px", "13.33px"),
      textAlign: fc.constantFrom(null, "left", "center", "right", "justify"),
      verticalAlign: fc.constantFrom(
        null,
        "top",
        "middle",
        "bottom",
        "baseline"
      ),
      whiteSpace: fc.constantFrom(null, "nowrap", "normal"),
    }),
    borders
  )
  .map(([attrs, sides]) => ({ ...attrs, ...sides }))

/** A cell's content: one or two paragraphs, or a paragraph and a list. */
const cellContent = fc.oneof(
  fc
    .tuple(fc.constantFrom(null, "center", "right"), word)
    .map(([textAlign, text]) => [
      nodes.paragraph.create({ textAlign }, schema.text(text)),
    ]),
  fc
    .tuple(word, fc.option(word, { nil: undefined }))
    .map(([first, second]) => [
      nodes.paragraph.create(null, schema.text(first)),
      nodes.paragraph.create(
        null,
        second === undefined ? undefined : schema.text(second)
      ),
    ]),
  word.map((text) => [
    nodes.paragraph.create(null, schema.text(text)),
    nodes.bulletList.create(null, [
      nodes.listItem.create(null, [
        nodes.paragraph.create(null, schema.text(text)),
      ]),
    ]),
  ]),
  fc.constant([nodes.paragraph.create()])
)

/**
 * A rectangular table of `rows` by `cols` cells with at most one merged
 * rectangle, so prosemirror-tables finds nothing to fix.
 */
const tables = fc
  .record({
    rows: fc.integer({ min: 1, max: 4 }),
    cols: fc.integer({ min: 1, max: 4 }),
  })
  .chain(({ rows, cols }) =>
    fc.record({
      rows: fc.constant(rows),
      cols: fc.constant(cols),
      merge: fc.option(
        fc
          .record({
            row: fc.nat(rows - 1),
            col: fc.nat(cols - 1),
          })
          .chain(({ row, col }) =>
            fc.record({
              row: fc.constant(row),
              col: fc.constant(col),
              height: fc.integer({ min: 1, max: rows - row }),
              width: fc.integer({ min: 1, max: cols - col }),
            })
          ),
        { nil: null }
      ),
      cells: fc.array(
        fc.record({
          header: fc.boolean(),
          attrs: cellAttrs,
          content: cellContent,
        }),
        { minLength: rows * cols, maxLength: rows * cols }
      ),
    })
  )
  .map(({ rows, cols, merge, cells }) => {
    const covered = (row: number, col: number) =>
      merge !== null &&
      row >= merge.row &&
      row < merge.row + merge.height &&
      col >= merge.col &&
      col < merge.col + merge.width &&
      !(row === merge.row && col === merge.col)
    const tableRows: ProseMirrorNode[] = []
    for (let row = 0; row < rows; row += 1) {
      const rowCells: ProseMirrorNode[] = []
      for (let col = 0; col < cols; col += 1) {
        if (covered(row, col)) {
          continue
        }
        const { header, attrs, content } = cells[row * cols + col]
        const spans =
          merge !== null && row === merge.row && col === merge.col
            ? { colspan: merge.width, rowspan: merge.height }
            : {}
        rowCells.push(
          (header ? nodes.tableHeader : nodes.tableCell).createChecked(
            { ...attrs, ...spans },
            content
          )
        )
      }
      tableRows.push(nodes.tableRow.createChecked(null, rowCells))
    }
    return nodes.table.createChecked(null, tableRows)
  })

const documents = fc
  .array(
    fc.oneof(
      { weight: 3, arbitrary: tables },
      {
        weight: 1,
        arbitrary: word.map((text) =>
          nodes.paragraph.create(null, schema.text(text))
        ),
      }
    ),
    { minLength: 1, maxLength: 3 }
  )
  .map((content) => nodes.doc.createChecked(null, content))

describe("random tables of the site schema", () => {
  it("are stored as fixed points, in content-v1", () => {
    fc.assert(
      fc.property(documents, (doc) => {
        const stored = save(doc)
        expect(J(stored)).toBe(stored)
        expect(tableProblems(reparse(stored)), stored).toEqual([])
      }),
      { numRuns: 100 }
    )
    // About 40 ms a body under jsdom, with a margin for a loaded CI runner.
  }, 120_000)
})

// Random table HTML.

const declaration = fc.constantFrom(
  "border: 1px solid red",
  "border: .5pt solid windowtext",
  "border-top: none",
  "border-left: medium dashed",
  "border-width: thin medium",
  "border-style: solid",
  "border-color: black red",
  "background: #D9D9D9",
  "background-color: rgb(217, 217, 217)",
  "width: 96pt",
  "width: 50%",
  "width: 100%px",
  "height: 15.0pt",
  "box-sizing: border-box",
  "text-align: CENTER",
  "text-align: start",
  "vertical-align: middle",
  "white-space: nowrap",
  "padding: 0cm 5.4pt",
  "mso-pattern: black none",
  "font-weight: 700",
  "color: red",
  "display: none",
  "BORDER-BOTTOM: 2px DOUBLE #00F"
)
const cellAttribute = fc.oneof(
  fc
    .array(declaration, { minLength: 1, maxLength: 5 })
    .map((list) => ["style", list.join("; ")] as const),
  fc
    .constantFrom("left", "middle", "absmiddle", "JUSTIFY", "char")
    .map((value) => ["align", value] as const),
  fc
    .constantFrom("top", "BASELINE", "center")
    .map((value) => ["valign", value] as const),
  fc
    .constantFrom("#d9d9d9", "fff", "red", "transparent", "x")
    .map((value) => ["bgcolor", value] as const),
  fc
    .constantFrom("120", "50%", "0", "20.5px")
    .map((value) => ["width", value] as const),
  fc.constantFrom("20", "0").map((value) => ["height", value] as const),
  fc
    .constantFrom("2", "3", "0", "abc", "40", "1000", "1001")
    .map((value) => ["colspan", value] as const),
  fc
    .constantFrom("2", "9", "0", "-1")
    .map((value) => ["rowspan", value] as const),
  fc.constantFrom("x", "MsoNormal").map((value) => ["class", value] as const)
)
const cellHtml = fc
  .tuple(
    fc.constantFrom("td", "th"),
    fc.uniqueArray(cellAttribute, { selector: ([name]) => name, maxLength: 4 }),
    fc.oneof(
      word,
      word.map((text) => `<p>${text}</p>`),
      word.map((text) => `<p style="text-align: center">${text}</p>`),
      word.map((text) => `<p>${text}</p><p>${text}</p>`),
      word.map((text) => `<table><tr><td>${text}</td></tr></table>`),
      fc.constant("")
    )
  )
  .map(
    ([tag, attributes, content]) =>
      `<${tag}${attributes.map(([name, value]) => ` ${name}="${value}"`).join("")}>${content}</${tag}>`
  )
const rowsHtml = fc
  .array(
    fc
      .array(cellHtml, { maxLength: 4 })
      .map((cells) => `<tr>${cells.join("")}</tr>`),
    { minLength: 1, maxLength: 4 }
  )
  .map((rows) => rows.join(""))
const tableHtml = fc
  .record({
    caption: fc.option(word, { nil: undefined }),
    colgroup: fc.boolean(),
    head: fc.option(rowsHtml, { nil: undefined }),
    body: rowsHtml,
    foot: fc.option(rowsHtml, { nil: undefined }),
    style: fc.constantFrom(
      "",
      ' style="width: 700px; border-collapse: collapse"',
      ' width="500" border="1"'
    ),
  })
  .map(
    ({ caption, colgroup, head, body, foot, style }) =>
      `<table${style}>${caption === undefined ? "" : `<caption>${caption}</caption>`}${colgroup ? '<colgroup><col width="300"></colgroup>' : ""}${head === undefined ? "" : `<thead>${head}</thead>`}<tbody>${body}</tbody>${foot === undefined ? "" : `<tfoot>${foot}</tfoot>`}</table>`
  )
const bodies = fc
  .array(
    fc.oneof(
      { weight: 3, arbitrary: tableHtml },
      { weight: 1, arbitrary: word.map((text) => `<p>${text}</p>`) }
    ),
    { minLength: 1, maxLength: 3 }
  )
  .map((parts) => parts.join(""))

/** How many of each kind of element an HTML string holds, parsed as a body. */
function countElements(html: string, selector: string): number {
  return new DOMParser()
    .parseFromString(`<body>${html}</body>`, "text/html")
    .querySelectorAll(selector).length
}

describe("random table HTML", () => {
  it("reaches a fixed point of J in one load, in content-v1", () => {
    fc.assert(
      fc.property(bodies, (body) => {
        const once = J(body)
        expect(J(once), body).toBe(once)
        expect(tableProblems(reparse(once)), body).toEqual([])
      }),
      { numRuns: 100 }
    )
    // About 40 ms a body under jsdom, with a margin for a loaded CI runner.
  }, 120_000)

  it("adds no more cells than its rows times twice its cells, whatever its spans", () => {
    // After unanchoredSpans a table is at most as wide as the columns in
    // which a cell starts, plus the columns a collision overlaps: at most
    // twice its cells. A caption becomes a row of one cell, so it counts
    // as both. Padding to a colspan's width instead broke this bound by
    // about a thousand cells a row.
    fc.assert(
      fc.property(bodies, (body) => {
        const rows = countElements(body, "tr, caption")
        const cells = countElements(body, "td, th, caption")
        const stored = countElements(J(body), "td, th")
        expect(stored, body).toBeLessThanOrEqual(
          Math.max(1, rows) * 2 * Math.max(1, cells)
        )
      }),
      { numRuns: 100 }
    )
  }, 120_000)
})
