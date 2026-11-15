import { Editor } from "@tiptap/core"
import type { Node as ProseMirrorNode } from "@tiptap/pm/model"
import { EditorState } from "@tiptap/pm/state"
import { fixTables, TableMap } from "@tiptap/pm/tables"
import { afterEach, describe, expect, it } from "vitest"

import { CONTENT_V1_POLICY, PolicyError } from "../policy"
import { siteExtensions } from "../site-extensions"
import {
  CELL_TAGS,
  cellHintStyle,
  cellStyle,
  cellStyleAttributes,
  createSiteTable,
  fixTablesFully,
  MAX_FIX_PASSES,
  readCellDecls,
  TABLE_NODE_NAMES,
  unanchoredSpans,
} from "./site-table"
import type { Json } from "@/test/rich-text-policy"
import { policyWith } from "@/test/rich-text-policy"
import { storedHtmlHarness } from "@/test/stored-html"
import { parseHtml, renderHtml, siteSchema } from "@/test/site-schema"

/**
 * The table nodes of the site schema (editor.md §4.3 rows 5-7): the cell
 * declarations, the presentational hints, the one sorted style string, and
 * the table shape in the schema and in a live editor. The fixed points over
 * whole bodies are in `serialize/tables.test.ts`.
 */
const POLICY = CONTENT_V1_POLICY
const ATTRIBUTES = cellStyleAttributes(POLICY)

type JsonObject = { [key: string]: Json }

/** A cell element with the given attributes, for the readers. */
function cell(attributes: Record<string, string>, tag = "td"): HTMLElement {
  const element = document.createElement(tag)
  for (const [name, value] of Object.entries(attributes)) {
    element.setAttribute(name, value)
  }
  return element
}

/** The element policy of one tag in a mutable policy copy. */
const elementJson = (json: JsonObject, tag: string) =>
  (json.elements as JsonObject)[tag] as JsonObject

describe("cellStyleAttributes", () => {
  it("names the cell attributes of editor.md §4.3 row 7 after the policy's properties", () => {
    expect(ATTRIBUTES.map(({ attribute }) => attribute)).toEqual([
      "borderTop",
      "borderRight",
      "borderBottom",
      "borderLeft",
      "verticalAlign",
      "whiteSpace",
      "width",
      "height",
      "textAlign",
      "backgroundColor",
    ])
    for (const { attribute, property } of ATTRIBUTES) {
      expect(
        property.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase())
      ).toBe(attribute)
    }
  })

  it("leaves out the input-only shorthands and box-sizing", () => {
    const properties = ATTRIBUTES.map(({ property }) => property)
    for (const absent of [
      "border",
      "border-width",
      "border-style",
      "border-color",
      "background",
      "box-sizing",
    ]) {
      expect(properties).not.toContain(absent)
    }
  })

  it("follows the policy rather than restating it", () => {
    const policy = policyWith((json) => {
      for (const tag of CELL_TAGS) {
        const element = elementJson(json, tag)
        element.style = (element.style as JsonObject[]).filter(
          (rule) => rule.property !== "white-space"
        )
      }
    })
    expect(
      cellStyleAttributes(policy).map(({ attribute }) => attribute)
    ).not.toContain("whiteSpace")
  })

  it.each<[string, (json: JsonObject) => void, string]>([
    [
      "td and th storing different declarations",
      (json) => {
        const th = elementJson(json, "th")
        th.style = (th.style as JsonObject[]).filter(
          (rule) => rule.property !== "height"
        )
      },
      "td and th have different style grammars in the policy",
    ],
    [
      "a cell without a style attribute",
      (json) => {
        const td = elementJson(json, "td")
        td.attributes = (td.attributes as JsonObject[]).filter(
          (attribute) => attribute.name !== "style"
        )
        td.style = null
      },
      "The policy has no style attribute on td with a style rule",
    ],
  ])("refuses a policy with %s", (_name, change, message) => {
    const policy = policyWith(change)
    expect(() => cellStyleAttributes(policy)).toThrow(PolicyError)
    expect(() => cellStyleAttributes(policy)).toThrow(message)
  })
})

describe("cellHintStyle", () => {
  it.each<[string, Record<string, string>, string | null]>([
    ["no hint", {}, null],
    ["align", { align: "left" }, "text-align: left"],
    ["align right", { align: "right" }, "text-align: right"],
    ["align justify", { align: "justify" }, "text-align: justify"],
    ["align center", { align: "Center" }, "text-align: center"],
    ["align middle", { align: " MIDDLE " }, "text-align: center"],
    ["align absmiddle", { align: "absmiddle" }, "text-align: center"],
    ["an align HTML does not know", { align: "char" }, null],
    ["an align named like an object member", { align: "constructor" }, null],
    ["valign", { valign: "Top" }, "vertical-align: top"],
    ["valign baseline", { valign: "baseline" }, "vertical-align: baseline"],
    ["an unknown valign", { valign: "center" }, null],
    ["bgcolor in hex", { bgcolor: "#D9D9D9" }, "background-color: #D9D9D9"],
    ["bgcolor short hex", { bgcolor: "#fff" }, "background-color: #fff"],
    [
      "bgcolor hex without #",
      { bgcolor: "d9d9d9" },
      "background-color: #d9d9d9",
    ],
    ["bgcolor named", { bgcolor: " red " }, "background-color: red"],
    ["bgcolor transparent", { bgcolor: "transparent" }, null],
    // CSS keywords that HTML's legacy colour rules read as hex digits.
    ["bgcolor currentcolor", { bgcolor: "currentColor" }, null],
    ["bgcolor a system colour", { bgcolor: "windowtext" }, null],
    ["bgcolor another system colour", { bgcolor: "Canvas" }, null],
    [
      "bgcolor named in capitals",
      { bgcolor: "Khaki" },
      "background-color: Khaki",
    ],
    ["bgcolor a word that is no colour", { bgcolor: "reddish" }, null],
    ["bgcolor in another spelling", { bgcolor: "rgb(1, 2, 3)" }, null],
    ["bgcolor with a declaration", { bgcolor: "red; width: 9px" }, null],
    ["width", { width: "120" }, "width: 120px"],
    ["width in px", { width: "120px" }, "width: 120px"],
    ["width with a fraction", { width: " 20.5 " }, "width: 20.5px"],
    ["width with trailing text", { width: "96pt" }, "width: 96px"],
    ["width in percent", { width: "50%" }, null],
    ["width zero", { width: "0" }, null],
    ["width without digits", { width: "auto" }, null],
    ["height", { height: "20" }, "height: 20px"],
    [
      "every hint",
      {
        align: "center",
        valign: "middle",
        bgcolor: "#d9d9d9",
        width: "128",
        height: "20",
      },
      "text-align: center; vertical-align: middle; background-color: #d9d9d9; width: 128px; height: 20px",
    ],
  ])("reads %s", (_name, attributes, expected) => {
    expect(cellHintStyle(cell(attributes))).toBe(expected)
  })

  const EVERY_HINT = {
    align: "center",
    valign: "middle",
    bgcolor: "#d9d9d9",
    width: "128",
    height: "20",
  }

  it.each<[string, string, string | null]>([
    [
      "a width the server cannot store",
      "width: 50%",
      "text-align: center; vertical-align: middle; background-color: #d9d9d9; height: 20px",
    ],
    [
      "an alignment the server cannot store",
      "text-align: start",
      "vertical-align: middle; background-color: #d9d9d9; width: 128px; height: 20px",
    ],
    [
      "the background shorthand, for the fill",
      "background: url(x.png)",
      "text-align: center; vertical-align: middle; width: 128px; height: 20px",
    ],
    [
      "a declaration the reader cannot read",
      "vertical-align: calc(1px); height: /* x */ 2px",
      "text-align: center; background-color: #d9d9d9; width: 128px",
    ],
    [
      "a name behind a comment or an escape",
      "/* a */ Width: 1px; h\\65ight: 2px",
      "text-align: center; vertical-align: middle; background-color: #d9d9d9",
    ],
    ["all, which sets every property", "all: initial", null],
  ])(
    "leaves out each hint the style declares: %s",
    (_name, style, expected) => {
      expect(cellHintStyle(cell({ ...EVERY_HINT, style }))).toBe(expected)
    }
  )

  it("keeps the hints whose property the style does not declare", () => {
    expect(
      cellHintStyle(
        cell({
          ...EVERY_HINT,
          style:
            "min-width: 10px; background-image: none; font-family: width; --height: 1px; inline-size: 5px",
        })
      )
    ).toBe(
      "text-align: center; vertical-align: middle; background-color: #d9d9d9; width: 128px; height: 20px"
    )
  })
})

describe("readCellDecls", () => {
  const read = (attributes: Record<string, string>, tag = "td") =>
    Object.fromEntries(readCellDecls(POLICY, cell(attributes, tag)))

  it("reads the canonical declarations of the style", () => {
    expect(
      read({ style: "BACKGROUND: #D9D9D9; Width: 96pt; text-align: CENTER" })
    ).toEqual({
      "background-color": "#d9d9d9",
      "box-sizing": "content-box",
      "text-align": "center",
      width: "128px",
    })
  })

  it("resolves the border shorthands and sides together, in source order", () => {
    expect(
      read({ style: "border: 1px solid red; border-top: dashed" })
    ).toEqual({
      "border-bottom": "1px solid #ff0000",
      "border-left": "1px solid #ff0000",
      "border-right": "1px solid #ff0000",
      "border-top": "3px dashed currentcolor",
    })
    expect(
      read({ style: "border-top: 2px dotted; border-width: thin medium" })
    ).toEqual({
      "border-bottom": "none",
      "border-left": "none",
      "border-right": "none",
      "border-top": "1px dotted currentcolor",
    })
  })

  it("reads the hints under the style, which wins", () => {
    expect(
      read({
        align: "left",
        valign: "top",
        bgcolor: "red",
        width: "100",
        style: "text-align: right; background-color: #0000ff",
      })
    ).toEqual({
      "background-color": "#0000ff",
      "box-sizing": "content-box",
      "text-align": "right",
      "vertical-align": "top",
      width: "100px",
    })
  })

  it("keeps no hint under a style declaration the server cannot store", () => {
    // A browser shows 50% and start here, not the hints, so the cell keeps
    // no width and no alignment (content-v1 stores px and four keywords).
    expect(read({ width: "100", style: "width: 50%" })).toEqual({})
    expect(read({ align: "center", style: "text-align: start" })).toEqual({})
    expect(
      read({ bgcolor: "red", style: "background: url(x.png); color: red" })
    ).toEqual({})
  })

  it("sizes a cell whose size comes from a hint", () => {
    expect(read({ height: "20", style: "width: 10px" })).toEqual({
      "box-sizing": "content-box",
      height: "20px",
      width: "10px",
    })
  })

  it("keeps nothing the server drops", () => {
    expect(
      read({
        style:
          "padding: 5px; color: red; font-weight: bold; width: 50%; height: 2001px; box-sizing: border-box",
      })
    ).toEqual({})
    // A border declaration that sets no style still writes all four sides,
    // each as `none` (editor.md §7.3).
    expect(read({ style: "border-width: 1px" })).toEqual({
      "border-bottom": "none",
      "border-left": "none",
      "border-right": "none",
      "border-top": "none",
    })
    expect(read({ width: "50%", bgcolor: "nonsense!" })).toEqual({})
  })

  it("reads a header cell like a body cell", () => {
    expect(
      read({ style: "vertical-align: bottom", align: "middle" }, "th")
    ).toEqual({ "text-align": "center", "vertical-align": "bottom" })
  })

  it("reads an element once while it is unchanged, and again after a change", () => {
    const element = cell({ style: "text-align: center" })
    const first = readCellDecls(POLICY, element)
    expect(readCellDecls(POLICY, element)).toBe(first)
    element.setAttribute("valign", "top")
    const second = readCellDecls(POLICY, element)
    expect(Object.fromEntries(second)).toEqual({
      "text-align": "center",
      "vertical-align": "top",
    })
    const policy = policyWith(() => {})
    expect(readCellDecls(policy, element)).not.toBe(second)
  })
})

describe("cellStyle", () => {
  const style = (attrs: Record<string, unknown>, tag: "td" | "th" = "td") =>
    cellStyle(POLICY, tag, attrs, ATTRIBUTES)

  it("is null for a cell without declarations", () => {
    expect(style({})).toBeNull()
    expect(style({ width: null, textAlign: undefined })).toBeNull()
  })

  it("writes one sorted string with box-sizing for a sized cell", () => {
    expect(
      style({
        width: "128px",
        textAlign: "center",
        backgroundColor: "#d9d9d9",
        whiteSpace: "nowrap",
        verticalAlign: "middle",
        height: "20px",
      })
    ).toBe(
      "background-color: #d9d9d9; box-sizing: content-box; height: 20px; text-align: center; vertical-align: middle; white-space: nowrap; width: 128px"
    )
  })

  it("writes the sides a border attribute leaves unset as none", () => {
    expect(style({ borderBottom: "2px solid #000000" }, "th")).toBe(
      "border-bottom: 2px solid #000000; border-left: none; border-right: none; border-top: none"
    )
  })

  it("writes each value in its canonical spelling", () => {
    expect(
      style({
        backgroundColor: "yellow",
        borderTop: "medium dashed RED",
        height: "15pt",
      })
    ).toBe(
      "background-color: #ffff00; border-bottom: none; border-left: none; border-right: none; border-top: 3px dashed #ff0000; box-sizing: content-box; height: 20px"
    )
  })

  it.each<[string, Record<string, unknown>]>([
    ["a value outside the grammar", { width: "50%" }],
    ["a value of another property", { textAlign: "middle" }],
    ["a value that holds a second declaration", { width: "10px; color: red" }],
    ["a value that is no string", { width: 128 }],
    ["a border side over 20 px", { borderTop: "21px solid #000000" }],
  ])("renders nothing for %s", (_name, attrs) => {
    expect(style(attrs)).toBeNull()
  })
})

describe("the table nodes", () => {
  const schema = siteSchema()

  it("are named as TipTap 3.32 names them", () => {
    expect(Object.values(TABLE_NODE_NAMES).sort()).toEqual(
      ["table", "tableCell", "tableHeader", "tableRow"].sort()
    )
    for (const name of Object.values(TABLE_NODE_NAMES)) {
      expect(schema.nodes[name], name).toBeDefined()
    }
  })

  it("render the public table shape of editor.md §3.7", () => {
    const doc = parseHtml(
      schema,
      '<table width="700" style="width: 700px"><colgroup><col width="300"></colgroup><thead><tr><th>h</th></tr></thead><tr><td colspan="2" style="text-align: center">a</td></tr></table>'
    )
    expect(renderHtml(schema, doc).innerHTML).toBe(
      '<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><th><p>h</p></th></tr><tr><td colspan="2" style="text-align: center"><p>a</p></td></tr></tbody></table>'
    )
  })

  it("read colspan and rowspan with the server's integer rule", () => {
    const doc = parseHtml(
      schema,
      '<table><tr><td colspan="1001" rowspan="65535">a</td><td colspan=" 3 " rowspan="02">b</td><td colspan="2x">c</td></tr></table>'
    )
    const cells: Record<string, unknown>[] = []
    doc.descendants((node) => {
      if (node.type.name === "tableCell") {
        cells.push({ colspan: node.attrs.colspan, rowspan: node.attrs.rowspan })
      }
      return true
    })
    expect(cells).toEqual([
      { colspan: 1, rowspan: 1 },
      { colspan: 3, rowspan: 2 },
      { colspan: 1, rowspan: 1 },
    ])
  })

  it("never read or render column widths", () => {
    const doc = parseHtml(
      schema,
      '<table><colgroup><col width="300"></colgroup><tr><td colwidth="120" data-colwidth="120">a</td></tr></table>'
    )
    let colwidth: unknown = "unread"
    doc.descendants((node) => {
      if (node.type.name === "tableCell") {
        colwidth = node.attrs.colwidth
      }
      return true
    })
    expect(colwidth).toBeNull()
    const withWidth = schema.nodes.tableCell.create(
      { colwidth: [120] },
      schema.nodes.paragraph.create()
    )
    const rendered = document.createElement("div")
    rendered.innerHTML = renderHtml(
      schema,
      schema.nodes.doc.create(null, [
        schema.nodes.table.create(null, [
          schema.nodes.tableRow.create(null, [withWidth]),
        ]),
      ])
    ).innerHTML
    expect(rendered.querySelector("td")?.attributes).toHaveLength(0)
  })

  it("keep media out of cells", () => {
    const withMedia = siteSchema()
    const doc = parseHtml(
      withMedia,
      '<table><tr><td><p>a</p><p><img src="/storage/uploads/a.jpg" alt="a"></p></td></tr></table>'
    )
    expect(withMedia.nodes.tableCell.spec.content).toBe("block+")
    let inCell = false
    doc.descendants((node, _position, parent) => {
      if (node.type.name === "siteImage" && parent?.type.name !== "doc") {
        inCell = true
      }
      return true
    })
    expect(inCell).toBe(false)
    const topLevel: string[] = []
    doc.forEach((node) => topLevel.push(node.type.name))
    expect(topLevel).toContain("siteImage")
  })

  it("refuse a policy without the forced table style", () => {
    const policy = policyWith((json) => {
      const table = elementJson(json, "table")
      ;(table.attributes as JsonObject[])[0].value = { kind: "style" }
    })
    expect(() => createSiteTable(policy)).toThrow(PolicyError)
    expect(() => createSiteTable(policy)).toThrow(
      "The policy has no style attribute on table with a forced rule"
    )
  })

  it("refuse a policy without the cell spans", () => {
    const policy = policyWith((json) => {
      const td = elementJson(json, "td")
      td.attributes = (td.attributes as JsonObject[]).filter(
        (attribute) => attribute.name !== "rowspan"
      )
    })
    expect(() => createSiteTable(policy)).toThrow(
      "The policy has no rowspan attribute on td with a integer rule"
    )
  })
})

describe("tables in an editor", () => {
  let editor: Editor | undefined
  afterEach(() => editor?.destroy())

  const create = (content: string) => {
    editor = new Editor({
      element: document.createElement("div"),
      extensions: siteExtensions({ policy: POLICY }),
      content,
    })
    return editor
  }

  it("show the render rule's shape, with no table view or column resizing", () => {
    const live = create("<table><tr><td>a</td><td>b</td></tr></table>")
    const table = live.view.dom.querySelector("table")!
    expect(table.querySelector("colgroup")).toBeNull()
    expect(table.parentElement).toBe(live.view.dom)
    expect(table.getAttribute("style")).toMatch(
      /^border-collapse: collapse; max-width: fit-content;?$/
    )
    expect(table.style.minWidth).toBe("")
    expect(table.style.width).toBe("")
    const plugins = live.state.plugins.map(
      (plugin) => (plugin as unknown as { key: string }).key
    )
    expect(plugins.some((key) => key.startsWith("tableColumnResizing"))).toBe(
      false
    )
    expect(plugins.some((key) => key.startsWith("selectingCells"))).toBe(true)
  })

  it("keep a table rectangular as it is edited", () => {
    const { J, editor: live, save, destroy } = storedHtmlHarness()
    try {
      live.commands.setContent(
        "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td></tr></table>"
      )
      const stored = save(live.state.doc)
      expect(stored).toContain("<tr><td>c</td><td></td></tr>")
      expect(J(stored)).toBe(stored)
    } finally {
      destroy()
    }
  })

  it("move to the next cell with Tab, and add a row after the last", () => {
    const live = create("<table><tr><td>a</td><td>b</td></tr></table>")
    live.commands.setTextSelection(3)
    expect(live.commands.goToNextCell()).toBe(true)
    expect(live.state.selection.$from.parent.textContent).toBe("b")
    live.commands.keyboardShortcut("Tab")
    let rows = 0
    live.state.doc.descendants((node) => {
      rows += node.type.name === "tableRow" ? 1 : 0
      return true
    })
    expect(rows).toBe(2)
  })
})

describe("unanchoredSpans", () => {
  const schema = siteSchema()
  /** The first table of `html`, parsed without an editor (no fix yet). */
  const tableOf = (html: string) => parseHtml(schema, html).firstChild!
  /** Each narrowed cell as its text and its new colspan. */
  const narrowed = (html: string) => {
    const table = tableOf(html)
    return unanchoredSpans(table).map(({ position, colspan }) => [
      table.nodeAt(position)!.textContent,
      colspan,
    ])
  }

  it("narrows a span past the grid to the columns where a cell starts", () => {
    expect(
      narrowed(
        `<table><tr><td colspan="1000">a</td></tr>${"<tr><td>x</td></tr>".repeat(3)}</table>`
      )
    ).toEqual([["a", 1]])
    expect(
      narrowed(
        '<table><tr><td colspan="3">a</td></tr><tr><td>b</td><td>c</td></tr><tr><td>d</td></tr></table>'
      )
    ).toEqual([["a", 2]])
  })

  it("drops the columns between cells too, from every span that crosses them", () => {
    // Cells start in columns 0, 1, 400 (e) and 501 (c): b keeps 1 and 400,
    // d keeps 0 and 1.
    expect(
      narrowed(
        '<table><tr><td>a</td><td colspan="500">b</td><td>c</td></tr><tr><td colspan="400">d</td><td>e</td></tr></table>'
      )
    ).toEqual([
      ["b", 2],
      ["d", 2],
    ])
  })

  it("narrows a wide span that collides with a rowspan from above", () => {
    // c starts in column 0 and runs over b's column 1, where it collides;
    // the columns after 1 hold no cell's start, so c keeps two.
    expect(
      narrowed(
        '<table><tr><td>a</td><td rowspan="2">b</td></tr><tr><td colspan="1000">c</td></tr></table>'
      )
    ).toEqual([["c", 2]])
  })

  it("leaves a table alone that fixTables pads without a span past its grid", () => {
    expect(
      narrowed(
        "<table><tr><td>a</td><td>b</td><td>c</td></tr><tr><td>d</td></tr></table>"
      )
    ).toEqual([])
  })

  it("leaves a table alone whose fix adds no cell, however wide its spans", () => {
    // Rectangular: the §3.7 Excel cell keeps its colspan="2".
    expect(narrowed('<table><tr><td colspan="2">a</td></tr></table>')).toEqual(
      []
    )
    expect(
      narrowed(
        '<table><tr><td colspan="1000">a</td></tr><tr><td colspan="1000">b</td></tr></table>'
      )
    ).toEqual([])
    // Only an overlong rowspan, which fixTables shortens without adding.
    expect(
      narrowed('<table><tr><td colspan="9" rowspan="4">a</td></tr></table>')
    ).toEqual([])
  })
})

describe("fixTablesFully", () => {
  const schema = siteSchema()
  // Parsed without an editor, so no plugin has fixed anything yet.
  const COLLIDING = parseHtml(
    schema,
    '<table><tr></tr><tr><td>a</td><td rowspan="2">b</td></tr><tr><td colspan="2">c</td></tr></table>'
  )

  /** The problems prosemirror-tables finds in every table of a document. */
  const problems = (doc: ProseMirrorNode) => {
    const found: unknown[] = []
    doc.descendants((node) => {
      if (node.type.name === "table") {
        found.push(...(TableMap.get(node).problems ?? []))
      }
      return true
    })
    return found
  }

  it("fixes what one fixTables pass leaves", () => {
    const empty = EditorState.create({ doc: parseHtml(schema, "<p></p>") })
    const state = EditorState.create({ doc: COLLIDING })
    expect(problems(COLLIDING)).not.toEqual([])
    const once = fixTables(state, empty)!.doc
    expect(problems(once)).not.toEqual([])
    const fixed = fixTablesFully(state, empty)
    expect(fixed).not.toBeNull()
    expect(problems(fixed!.doc)).toEqual([])
    expect(
      fixTablesFully(EditorState.create({ doc: fixed!.doc }), empty)
    ).toBeNull()
  })

  it("changes nothing when no table has a problem or nothing changed", () => {
    const whole = parseHtml(schema, "<table><tr><td>a</td></tr></table>")
    const empty = EditorState.create({ doc: parseHtml(schema, "<p></p>") })
    expect(fixTablesFully(EditorState.create({ doc: whole }), empty)).toBeNull()
    const state = EditorState.create({ doc: COLLIDING })
    expect(fixTablesFully(state, state)).toBeNull()
  })

  it("narrows a span past the grid before it pads the rows", () => {
    const empty = EditorState.create({ doc: parseHtml(schema, "<p></p>") })
    const wide = parseHtml(
      schema,
      `<table><tr><td colspan="1000">a</td></tr>${"<tr><td>x</td></tr>".repeat(30)}</table>`
    )
    // fixTables alone fills every short row up to the span's width.
    const padded = fixTables(EditorState.create({ doc: wide }), empty)!.doc
    expect(TableMap.get(padded.firstChild!).map.length).toBe(31 * 1000)
    const fixed = fixTablesFully(EditorState.create({ doc: wide }), empty)!.doc
    const map = TableMap.get(fixed.firstChild!)
    expect([map.width, map.height, map.problems]).toEqual([1, 31, null])
  })

  it("narrows the tables nested in a changed cell, and only changed tables", () => {
    const wide = (text: string) =>
      `<table><tr><td colspan="50">${text}</td></tr><tr><td>x</td></tr></table>`
    const outer = parseHtml(
      schema,
      `<table><tr><td>${wide("a")}</td></tr></table>`
    )
    const empty = EditorState.create({ doc: parseHtml(schema, "<p></p>") })
    const fixed = fixTablesFully(EditorState.create({ doc: outer }), empty)!.doc
    const inner = fixed.firstChild!.firstChild!.firstChild!.firstChild!
    expect(TableMap.get(inner).width).toBe(1)
    // A document whose old state already holds the same table checks none.
    const same = EditorState.create({ doc: outer })
    expect(fixTablesFully(same, same)).toBeNull()
  })

  it("is bounded", () => {
    expect(MAX_FIX_PASSES).toBeGreaterThanOrEqual(2)
  })

  it("runs before tableEditing's own fix in the editor", () => {
    const live = new Editor({
      element: document.createElement("div"),
      extensions: siteExtensions({ policy: POLICY }),
    })
    try {
      const keys = live.state.plugins.map(
        (plugin) => (plugin as unknown as { key: string }).key
      )
      const ours = keys.findIndex((key) => key.startsWith("vfgFixTablesFully"))
      const stock = keys.findIndex((key) => key.startsWith("selectingCells"))
      expect(ours).toBeGreaterThanOrEqual(0)
      expect(ours).toBeLessThan(stock)
    } finally {
      live.destroy()
    }
  })

  it("runs in the editor, so a body with colliding spans loads whole", () => {
    const { J, destroy } = storedHtmlHarness()
    try {
      const html =
        '<table><tr></tr><tr><td>a</td><td rowspan="2">b</td></tr><tr><td colspan="2">c</td></tr></table>'
      const once = J(html)
      expect(J(once)).toBe(once)
    } finally {
      destroy()
    }
  })
})
