import { afterAll, describe, expect, it } from "vitest"

import { table, TABLE_INPUTS, TABLE_SHAPES } from "@/test/editor-fixtures"
import { reparse, storedHtmlHarness } from "@/test/stored-html"
import type { StoredNode } from "./stored-tree"
import { attributeOf } from "./stored-tree"

/**
 * editor.md §13.4, L2, for tables: every canonical table shape is a fixed
 * point of `J` (P-fix), and every table input reaches its fixed point in one
 * load, with the stored form pinned here so a change to the table nodes,
 * prosemirror-tables or the serialiser shows as a changed expectation. The
 * random tables are in `tables.property.test.ts`.
 */
const harness = storedHtmlHarness()
afterAll(() => harness.destroy())
const { J } = harness

/** What each table input is stored as after its first load. */
const EXPECTED: Readonly<Record<keyof typeof TABLE_INPUTS, string>> = {
  "ragged-rows": table(
    "<tr><td>a</td><td>b</td><td>c</td></tr><tr><td>d</td><td></td><td></td></tr><tr><td></td><td></td><td></td></tr>"
  ),
  "overlong-rowspan": table(
    '<tr><td rowspan="2">a</td><td>b</td></tr><tr><td>c</td></tr>'
  ),
  "spans-outside-the-rule": table(
    '<tr><td>a</td><td>b</td><td colspan="2">c</td></tr>'
  ),
  "legacy-cell-attributes": table(
    '<tr><td style="background-color: #d9d9d9; box-sizing: content-box; height: 20.5px; text-align: center; vertical-align: top; width: 120px">a</td><td style="background-color: #ffffff; text-align: justify; vertical-align: baseline">b</td><td style="text-align: center">c</td></tr>'
  ),
  "style-over-hints": table(
    '<tr><td style="background-color: #0000ff; box-sizing: content-box; text-align: right; vertical-align: bottom; width: 80px">a</td></tr>'
  ),
  "thead-tfoot-caption-colgroup": table(
    "<tr><td></td><td>Felirat</td></tr><tr><th>h</th><th>i</th></tr><tr><td>a</td><td>b</td></tr><tr><td>f</td><td>g</td></tr>"
  ),
  "cssom-spellings": table(
    '<tr><td style="background-color: #d9d9d9; border-bottom: 1px solid #000000; border-left: 1px solid #000000; border-right: 1px solid #000000; border-top: 1px solid #000000; box-sizing: content-box; width: 128px">a</td></tr>'
  ),
  "excel-cell": table(
    '<tr><td style="background-color: #d9d9d9; border-bottom: 0.67px solid #000000; border-left: 0.67px solid #000000; border-right: 0.67px solid #000000; border-top: none; box-sizing: content-box; height: 20px; text-align: center; vertical-align: middle; white-space: nowrap; width: 128px"><strong>Eredmény</strong></td></tr>'
  ),
  "word-cell": table(
    '<tr><td style="border-bottom: 1.33px solid #000000; border-left: 1.33px solid #000000; border-right: 1.33px solid #000000; border-top: 1.33px solid #000000; text-align: center; vertical-align: top">szöveg</td></tr>'
  ),
  "aligned-cell-paragraph": table(
    '<tr><td style="text-align: center">a</td><td style="text-align: right">b</td><td style="text-align: center"></td></tr>'
  ),
  "dropped-declarations": table("<tr><td><strong>a</strong></td></tr>"),
  "table-first-in-list-item": `<ul><li></li></ul>${table("<tr><td>a</td></tr>")}`,
  "colliding-spans": table(
    `<tr><td></td><td></td><td></td><td></td></tr><tr><td></td><td>${table("<tr><td>A</td></tr>")}</td><td rowspan="2">A</td><td></td></tr><tr><td></td><td></td><td></td></tr>`
  ),
  "empty-table": "<p>a</p>",
  "span-past-the-grid": table(
    '<tr><td colspan="2">a</td></tr><tr><td>b</td><td>c</td></tr><tr><td>d</td><td></td></tr>'
  ),
  "widest-colspan": table(
    "<tr><td>Fejléc</td></tr><tr><td>a</td></tr><tr><td>b</td></tr>"
  ),
  "wide-colspan-between-cells": table(
    "<tr><td>a</td><td>b</td><td>c</td></tr><tr><td>d</td><td>e</td><td></td></tr>"
  ),
  // c keeps two columns (unanchoredSpans); prosemirror-tables 1.8.5 then
  // narrows it where it collides with b, adds cells both for the lost span
  // and for the short rows, and puts some at the start of a row, as its
  // padding rule does, so the cells move on and the table is four wide.
  "wide-colliding-colspan": table(
    '<tr><td></td><td>a</td><td rowspan="2">b</td><td></td></tr><tr><td></td><td></td><td>c</td></tr>'
  ),
  "hints-under-unstorable-styles": table(
    "<tr><td>a</td><td>b</td><td>c</td><td>d</td></tr>"
  ),
  "bgcolor-keywords": table(
    '<tr><td>a</td><td>b</td><td>c</td><td style="background-color: #f0e68c">d</td></tr>'
  ),
}

/** Every cell element of a stored tree. */
function cells(nodes: readonly StoredNode[]): StoredNode[] {
  return nodes.flatMap((node) =>
    node.kind === "text"
      ? []
      : [
          ...(["td", "th"].includes(node.tag) ? [node] : []),
          ...cells(node.children),
        ]
  )
}

describe("the canonical table shapes (P-fix)", () => {
  it.each(Object.entries(TABLE_SHAPES))("keeps %s", (_name, shape) => {
    expect(J(shape)).toBe(shape)
  })

  it("keeps them all in one body", () => {
    const body = Object.values(TABLE_SHAPES).join("")
    expect(J(body)).toBe(body)
  })

  it("write every border side of a cell, or none", () => {
    const sides = ["border-top", "border-right", "border-bottom", "border-left"]
    for (const shape of Object.values(TABLE_SHAPES)) {
      for (const cell of cells(reparse(shape))) {
        const style =
          cell.kind === "element" ? (attributeOf(cell, "style") ?? "") : ""
        const present = sides.filter((side) => style.includes(`${side}:`))
        expect([0, 4], style).toContain(present.length)
      }
    }
  })
})

describe("the table inputs", () => {
  it("have an expectation each", () => {
    expect(Object.keys(EXPECTED).sort()).toEqual(
      Object.keys(TABLE_INPUTS).sort()
    )
  })

  it.each(Object.entries(TABLE_INPUTS))(
    "store %s in the expected form, a fixed point",
    (name, input) => {
      const once = J(input)
      expect(once).toBe(EXPECTED[name])
      expect(J(once)).toBe(once)
    }
  )
})

describe("a span far wider than its table", () => {
  it("loads quickly and stores no larger than the table it is", () => {
    // Before unanchoredSpans, each short row under a colspan="1000" was
    // padded to a thousand cells: 30 rows took about 2 s and 270 KB, and
    // about 250 rows of a few KB passed the server's 2 MiB input limit.
    const rows = 250
    const input = `<table><tr><td colspan="1000">a</td></tr>${"<tr><td>x</td></tr>".repeat(rows)}</table>`
    const start = performance.now()
    const once = J(input)
    const elapsed = performance.now() - start
    expect(once).toBe(
      table(`<tr><td>a</td></tr>${"<tr><td>x</td></tr>".repeat(rows)}`)
    )
    expect(once.length).toBeLessThan(input.length + 200)
    expect(J(once)).toBe(once)
    // Tens of milliseconds unloaded; the padded form took over 10 s. The
    // margin is for a loaded CI runner.
    expect(elapsed).toBeLessThan(3_000)
  }, 30_000)
})
