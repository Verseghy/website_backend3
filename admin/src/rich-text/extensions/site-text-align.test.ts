import { Editor } from "@tiptap/core"
import { TextSelection } from "@tiptap/pm/state"
import { CellSelection } from "@tiptap/pm/tables"
import { afterAll, afterEach, describe, expect, it } from "vitest"

import { CONTENT_V1_POLICY } from "../policy"
import { siteExtensions } from "../site-extensions"
import { alignCells, ALIGNED_TYPES, cellsToAlign } from "./site-text-align"
import { table } from "@/test/editor-fixtures"
import { storedHtmlHarness } from "@/test/stored-html"

/**
 * The alignment commands (editor.md §4.3 row 2): stock outside tables, and
 * in a table cell the cell is aligned and its paragraphs lose their own
 * alignment, so the cell stays tight (§3.7). The parse and render rules of
 * the alignment are in `site-extensions.parse.test.ts` and
 * `site-extensions.policy.test.ts`.
 */
const harness = storedHtmlHarness()
const { editor, load, save } = harness
afterAll(() => harness.destroy())
afterEach(() => load("<p></p>"))

/** The positions of every table cell, in document order. */
function cellPositions(): number[] {
  const cells: number[] = []
  editor.state.doc.descendants((node, position) => {
    if (["tableCell", "tableHeader"].includes(node.type.name)) {
      cells.push(position)
    }
    return true
  })
  return cells
}

/** Selects cells `anchor` to `head` (indexes in document order). */
function selectCells(anchor: number, head: number): void {
  const cells = cellPositions()
  editor.view.dispatch(
    editor.state.tr.setSelection(
      CellSelection.create(editor.state.doc, cells[anchor], cells[head])
    )
  )
}

/** Puts the cursor in the first paragraph of cell `index`. */
function inCell(index: number, offset = 0): void {
  editor.commands.setTextSelection(cellPositions()[index] + 2 + offset)
}

/**
 * A text selection from the start of the first text `from` to the end of
 * the first text `to`, so a case names the words instead of positions.
 */
function selectBetween(from: string, to: string): void {
  const find = (needle: string) => {
    let found = -1
    editor.state.doc.descendants((node, position) => {
      if (found === -1 && node.isText && node.text === needle) {
        found = position
      }
      return found === -1
    })
    if (found === -1) {
      throw new Error(`selectBetween: "${needle}" is not a text node.`)
    }
    return found
  }
  editor.view.dispatch(
    editor.state.tr.setSelection(
      TextSelection.create(editor.state.doc, find(from), find(to) + to.length)
    )
  )
}

/**
 * The stored HTML with the outermost table's forced styles shortened to
 * `<T>`, for short expectations; a nested table keeps its own.
 */
const stored = () =>
  save(editor.state.doc)
    .replace(
      '<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group">',
      "<T>"
    )
    .replace(/<\/tbody><\/table>(?!.*<\/tbody><\/table>)/s, "</T>")

const TWO_BY_TWO = table(
  '<tr><td>a</td><td><p style="text-align: right">b</p><p>c</p></td></tr><tr><td><ul><li><p style="text-align: justify">d</p></li></ul></td><td>e</td></tr>'
)

describe("cellsToAlign", () => {
  it("is the cell that holds a text selection", () => {
    load(TWO_BY_TWO)
    inCell(1)
    expect(cellsToAlign(editor.state.selection)).toEqual([cellPositions()[1]])
  })

  it("is every cell of a cell selection, in document order", () => {
    load(TWO_BY_TWO)
    selectCells(3, 0)
    expect(cellsToAlign(editor.state.selection)).toEqual(cellPositions())
  })

  it("is the innermost cell of a nested table", () => {
    load(table(`<tr><td>${table("<tr><td>x</td></tr>")}</td></tr>`))
    inCell(1)
    expect(cellsToAlign(editor.state.selection)).toEqual([cellPositions()[1]])
  })

  it("is the outer cell for a selection from a nested cell into the outer cell's text", () => {
    load(
      table(`<tr><td>${table("<tr><td>belül</td></tr>")}<p>kívül</p></td></tr>`)
    )
    const [outer, inner] = cellPositions()
    selectBetween("belül", "kívül")
    expect(cellsToAlign(editor.state.selection)).toEqual([outer])
    expect(cellsToAlign(editor.state.selection)).not.toContain(inner)
    editor.commands.setTextAlign("center")
    expect(stored()).toBe(
      `<T><tr><td style="text-align: center">${table("<tr><td>belül</td></tr>")}<p>kívül</p></td></tr></T>`
    )
  })

  it("is empty for a selection from a cell to the paragraph after its table, which stock aligns", () => {
    load(`${table("<tr><td>a</td></tr>")}<p>utána</p>`)
    selectBetween("a", "utána")
    expect(cellsToAlign(editor.state.selection)).toEqual([])
    editor.commands.setTextAlign("right")
    // Stock aligns both paragraphs; the serialiser moves the lone cell
    // paragraph's alignment to the cell (§3.7).
    expect(stored()).toBe(
      '<T><tr><td style="text-align: right">a</td></tr></T><p style="text-align: right">utána</p>'
    )
  })

  it("is empty for a text selection across two cells of one table", () => {
    load(table("<tr><td>bal</td><td>jobb</td></tr>"))
    selectBetween("bal", "jobb")
    expect(cellsToAlign(editor.state.selection)).toEqual([])
  })

  it("is empty outside tables and for a selection that leaves its cell", () => {
    load(`<p>előtte</p>${table("<tr><td>a</td></tr>")}`)
    editor.commands.setTextSelection(2)
    expect(cellsToAlign(editor.state.selection)).toEqual([])
    const inside = cellPositions()[0] + 2
    editor.view.dispatch(
      editor.state.tr.setSelection(
        TextSelection.create(editor.state.doc, 2, inside)
      )
    )
    expect(cellsToAlign(editor.state.selection)).toEqual([])
  })
})

describe("alignCells", () => {
  it("skips a position that holds no cell", () => {
    load(`<p>a</p>${table("<tr><td>b</td></tr>")}`)
    const tr = editor.state.tr
    alignCells(tr, [0], "center", Object.keys(ALIGNED_TYPES))
    expect(tr.docChanged).toBe(false)
  })
})

describe("setTextAlign", () => {
  it("aligns paragraphs and headings outside tables, as stock does", () => {
    load("<h2>a</h2><p>b</p>")
    editor.commands.selectAll()
    expect(editor.commands.setTextAlign("center")).toBe(true)
    expect(stored()).toBe(
      '<h2 style="text-align: center">a</h2><p style="text-align: center">b</p>'
    )
  })

  it("aligns the cell and clears its blocks' alignment, so the cell stays tight", () => {
    load(TWO_BY_TWO)
    inCell(1)
    expect(editor.commands.setTextAlign("center")).toBe(true)
    expect(stored()).toBe(
      '<T><tr><td>a</td><td style="text-align: center"><p>b</p><p>c</p></td></tr><tr><td><ul><li><p style="text-align: justify">d</p></li></ul></td><td>e</td></tr></T>'
    )
    inCell(2, 2)
    editor.commands.setTextAlign("right")
    expect(stored()).toContain(
      '<td style="text-align: right"><ul><li>d</li></ul></td>'
    )
  })

  it("aligns every cell of a cell selection", () => {
    load(TWO_BY_TWO)
    selectCells(0, 3)
    editor.commands.setTextAlign("left")
    expect(stored()).toBe(
      '<T><tr><td style="text-align: left">a</td><td style="text-align: left"><p>b</p><p>c</p></td></tr><tr><td style="text-align: left"><ul><li>d</li></ul></td><td style="text-align: left">e</td></tr></T>'
    )
  })

  it("leaves the paragraphs of a nested table to their own cells", () => {
    load(
      table(
        `<tr><td><p>kívül</p>${table('<tr><td><p style="text-align: right">belül</p><p>x</p></td></tr>')}</td></tr>`
      )
    )
    inCell(0)
    editor.commands.setTextAlign("center")
    expect(stored()).toContain(
      '<td style="text-align: center"><p>kívül</p><table'
    )
    expect(stored()).toContain('<p style="text-align: right">belül</p>')
  })

  it("aligns a header cell", () => {
    load(table("<tr><th>fej</th></tr>"))
    inCell(0)
    editor.commands.setTextAlign("justify")
    expect(stored()).toBe(
      '<T><tr><th style="text-align: justify">fej</th></tr></T>'
    )
  })

  it("refuses an alignment the policy does not have, and changes nothing", () => {
    load(TWO_BY_TWO)
    inCell(0)
    const before = editor.state.doc
    expect(editor.commands.setTextAlign("start")).toBe(false)
    expect(editor.state.doc.eq(before)).toBe(true)
    load("<p>a</p>")
    editor.commands.setTextSelection(1)
    expect(editor.commands.setTextAlign("start")).toBe(false)
    expect(stored()).toBe("<p>a</p>")
  })

  it("can be asked about without changing the document", () => {
    load(TWO_BY_TWO)
    inCell(0)
    const before = editor.state.doc
    expect(editor.can().setTextAlign("center")).toBe(true)
    expect(editor.can().setTextAlign("start")).toBe(false)
    expect(editor.state.doc.eq(before)).toBe(true)
  })
})

describe("unsetTextAlign", () => {
  it("removes the cell's alignment and its blocks' alignment", () => {
    load(
      table(
        '<tr><td style="text-align: center"><p style="text-align: right">a</p><p>b</p></td></tr>'
      )
    )
    inCell(0)
    expect(editor.commands.unsetTextAlign()).toBe(true)
    expect(stored()).toBe("<T><tr><td><p>a</p><p>b</p></td></tr></T>")
  })

  it("removes a paragraph's alignment outside tables, as stock does", () => {
    load('<p style="text-align: center">a</p>')
    editor.commands.setTextSelection(1)
    editor.commands.unsetTextAlign()
    expect(stored()).toBe("<p>a</p>")
  })
})

describe("toggleTextAlign", () => {
  it("sets the cells' alignment unless every cell has it, then removes it", () => {
    load(table('<tr><td style="text-align: center">a</td><td>b</td></tr>'))
    selectCells(0, 1)
    editor.commands.toggleTextAlign("center")
    expect(stored()).toBe(
      '<T><tr><td style="text-align: center">a</td><td style="text-align: center">b</td></tr></T>'
    )
    selectCells(0, 1)
    editor.commands.toggleTextAlign("center")
    expect(stored()).toBe("<T><tr><td>a</td><td>b</td></tr></T>")
  })

  it("refuses an alignment the policy does not have", () => {
    load(table("<tr><td>a</td></tr>"))
    inCell(0)
    expect(editor.commands.toggleTextAlign("start")).toBe(false)
  })

  it("toggles a paragraph outside tables, as stock does", () => {
    load("<p>a</p>")
    editor.commands.setTextSelection(1)
    editor.commands.toggleTextAlign("right")
    expect(stored()).toBe('<p style="text-align: right">a</p>')
    editor.commands.toggleTextAlign("right")
    expect(stored()).toBe("<p>a</p>")
  })
})

describe("the keyboard shortcuts", () => {
  let other: Editor | undefined
  afterEach(() => other?.destroy())

  it("align the cell in a cell", () => {
    other = new Editor({
      element: document.createElement("div"),
      extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
      content: table("<tr><td>a</td></tr>"),
    })
    other.commands.setTextSelection(4)
    other.commands.keyboardShortcut("Mod-Shift-e")
    let alignment: unknown = null
    other.state.doc.descendants((node) => {
      if (node.type.name === "tableCell") {
        alignment = node.attrs.textAlign
      }
      return true
    })
    expect(alignment).toBe("center")
  })
})
