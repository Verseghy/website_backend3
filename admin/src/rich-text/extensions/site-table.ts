/**
 * The site schema's tables (editor.md §4.3 rows 5 to 7, ADR 0013 §2):
 * `Table`, `TableRow`, `TableCell` and `TableHeader` from TipTap 3.32,
 * narrowed to content-v1's one table shape (editor.md §3.7):
 *
 * ```html
 * <table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td style="…">…</td></tr></tbody></table>
 * ```
 *
 * That shape is what makes a table look the same on both public sites: no
 * table width, no `colgroup`, no `thead`, widths on the cells and a fixed
 * box model (ADR 0013, Context). Stock TipTap writes another one, so each
 * node is changed where it differs:
 *
 * - **Table** (row 5). `resizable: false`, `renderWrapper: false` and
 *   `View: null`. In 3.32, `addNodeView` installs its `TableView` even when
 *   the table is not resizable, and that view writes a `colgroup` and a
 *   `min-width` into the editor; with no view, the editor shows the render
 *   rule below, the public shape. The render rule is replaced too: the
 *   stock one always writes a `colgroup` and a `width` or `min-width`. This
 *   one writes the table's and the tbody's forced styles of the policy
 *   (editor.md §3.3, locked declarations), which the server forces on every
 *   table as well (§7.3).
 * - **TableRow** (row 6) is stock: a `tr` without attributes.
 * - **TableCell and TableHeader** (row 7) hold the canonical cell
 *   declarations as attributes, one per stored property of §3.3 that the
 *   policy lets a cell carry or leave out: `backgroundColor`, the four
 *   `border…` sides, `height`, `textAlign`, `verticalAlign`, `whiteSpace`
 *   and `width`. They are named after the policy's properties, so the
 *   schema follows the policy rather than restating it. `textAlign`
 *   replaces the stock `align`, which knows no `justify` (17 cells
 *   **[500]**) and reads the CSSOM. Every attribute is parsed from one
 *   memoised {@link readCellDecls}, which reads the raw `style` together
 *   with the `align`, `valign`, `bgcolor`, `width` and `height` attributes
 *   and returns the declarations the server stores, so the border
 *   shorthands and sides are resolved together, in source order, by the
 *   one cascade of `canonicalStyle` (editor.md §3.1 rule 1). No attribute
 *   renders itself; the node's render rule writes them all as one sorted
 *   style string ({@link cellStyle}), so the order in which TipTap merges
 *   attributes can never change the bytes, and `box-sizing: content-box`
 *   comes with a `width` or a `height` (§3.3). `colspan` and `rowspan` are
 *   read with the server's integer rule and `1` is left out. `colwidth`,
 *   which prosemirror-tables keeps for column resizing, is never read or
 *   rendered: content-v1 has no column widths (§3.4), and the paste filter
 *   moves a `colgroup`'s widths to the first-row cells (§5.2 step 10).
 *
 * A cell holds `block+`, never a `media` node, so images and embeds stay at
 * the top level (§4.2). A cell's lone paragraph is tightened by the
 * serialiser, which also moves its alignment to the cell (§3.7, §6 step
 * 2.3); in a cell, the alignment commands of `site-text-align.ts` align the
 * cell, so it stays tight.
 *
 * prosemirror-tables' `tableEditing` plugin stays: it keeps every table
 * rectangular (`fixTables` adds the cells a short row lacks, shortens a
 * rowspan that runs past the table and narrows a cell that collides with
 * another) and gives cell selections. The table adds a plugin of its own,
 * {@link fixTablesFully}, which runs before `tableEditing`'s and does two
 * things that `fixTables` alone does not (editor.md §4.3 row 5):
 *
 * - **It narrows a span that crosses a column in which no cell starts,**
 *   in a table that `fixTables` would pad ({@link unanchoredSpans}).
 *   prosemirror-tables counts a table as wide as its widest row's spans
 *   and pads every other row to that width, so one cell with
 *   `colspan="1000"`, which the server keeps (§7.3), turned each short row
 *   of its table into a thousand empty cells: a 619-byte body was stored
 *   as 270 KB, and a few KB of such rows outgrew the server's 2 MiB input
 *   limit after one load. HTML calls a column in which no cell starts a
 *   table model error, and a browser gives it no width of its own, so the
 *   table is stored without it.
 * - **It repeats the fix until no table has a problem left.** One pass is
 *   not always enough: fixing a collision can leave rows of different
 *   lengths, which only the next pass sees, so a body with colliding spans
 *   would otherwise be stored differently after each of its first loads.
 *
 * A ragged legacy table is so stored rectangular after its first save and
 * loads back unchanged (editor.md §6, P-fix; `serialize/tables.test.ts`,
 * `serialize/tables.property.test.ts`).
 */
import type { AnyExtension } from "@tiptap/core"
import type { Node as ProseMirrorNode } from "@tiptap/pm/model"
import type { Transaction } from "@tiptap/pm/state"
import { EditorState, Plugin, PluginKey } from "@tiptap/pm/state"
import { fixTables, TableMap } from "@tiptap/pm/tables"
import {
  Table,
  TableCell,
  TableHeader,
  TableRow,
} from "@tiptap/extension-table"

import { canonicalStyle, SHORTHAND_TARGETS } from "../css/canonical-style"
import { asciiLowerCase, declaredProperties } from "../css/declarations"
import { NAMED_COLOURS } from "../css/named-colours"
import type { RichTextPolicy, StyleRule } from "../policy"
import { elementOf, PolicyError, requireAttributeRule } from "../policy"
import { readStoredDeclarations } from "../style-decls"
import { isStoredInteger, readIntegerAttribute } from "./site-lists"

/** The two cell tags, `td` for `tableCell` and `th` for `tableHeader`. */
export const CELL_TAGS = ["td", "th"] as const

/** A cell tag. */
export type CellTag = (typeof CELL_TAGS)[number]

/** The node type names of the table nodes, as TipTap 3.32 names them. */
export const TABLE_NODE_NAMES = {
  table: "table",
  row: "tableRow",
  cell: "tableCell",
  header: "tableHeader",
} as const

/** One cell attribute and the CSS property whose canonical value it holds. */
export interface CellStyleAttribute {
  /** The node attribute, the property in camel case (`borderTop`). */
  readonly attribute: string
  /** The stored CSS property (`border-top`). */
  readonly property: string
}

/** `border-top` as `borderTop`. */
function camelCase(property: string): string {
  return property.replace(/-([a-z])/g, (_, letter: string) =>
    letter.toUpperCase()
  )
}

/** A style rule in a form two rules can be compared by. */
function ruleKey(rule: StyleRule): string {
  return JSON.stringify([rule.property, rule.role, rule.presence, rule.grammar])
}

/**
 * The cell attributes of the policy: one per stored property of `td` whose
 * presence is `optional`. A locked property that is always present, or
 * `box-sizing` (present exactly when the cell is sized), is not an
 * attribute: `canonicalStyle` writes it from the others.
 *
 * @param policy - the content-v1 policy.
 * @returns the attributes, in the policy's order of the properties.
 * @throws {PolicyError} when `td` or `th` has no `style` attribute or no
 *   style grammar, or when their grammars differ: `tableCell` and
 *   `tableHeader` share one set of attributes, and toggling a header cell
 *   keeps them.
 */
export function cellStyleAttributes(
  policy: RichTextPolicy
): readonly CellStyleAttribute[] {
  const [td, th] = CELL_TAGS.map((tag) => {
    requireAttributeRule(policy, tag, "style", "style")
    const rules = elementOf(policy, tag)?.style
    if (!rules) {
      throw new PolicyError(
        `${tag} has a style attribute but no style grammar in the policy; the table cells read their declarations from it (editor.md §3.3).`
      )
    }
    return rules
  })
  if (td.map(ruleKey).join("\n") !== th.map(ruleKey).join("\n")) {
    throw new PolicyError(
      "td and th have different style grammars in the policy; the table cells share one set of attributes, so header and body cells must store the same declarations (editor.md §3.3)."
    )
  }
  return td
    .filter((rule) => rule.role === "stored" && rule.presence === "optional")
    .map((rule) => ({
      attribute: camelCase(rule.property),
      property: rule.property,
    }))
}

/** The `align` values of a cell and the `text-align` each means (WHATWG). */
const ALIGN_HINTS: Readonly<Record<string, string>> = {
  left: "left",
  right: "right",
  center: "center",
  middle: "center",
  absmiddle: "center",
  justify: "justify",
}

/** The `valign` values of a cell, each the `vertical-align` of its name. */
const VALIGN_HINTS: ReadonlySet<string> = new Set([
  "top",
  "middle",
  "bottom",
  "baseline",
])

/** Trims ASCII whitespace, as HTML attribute parsing does. */
const trimAscii = (value: string) =>
  value.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, "")

/**
 * A `bgcolor` value as a CSS colour, for the forms that HTML's legacy
 * colour rules and CSS read alike: a named colour of CSS Color 4
 * ({@link NAMED_COLOURS}), `#rgb` and `#rrggbb`, and `rrggbb` without its
 * `#`. Every other spelling is left out, so a cell keeps no fill rather
 * than one that the legacy rules make up from an arbitrary string. That
 * includes the keywords that CSS reads but the legacy rules do not count as
 * named colours: `transparent`, which they refuse; `currentcolor`, which
 * they read as hex digits (`#c0e000`), so CSS would fill the cell with its
 * text colour; and the system colours (`windowtext`, `Canvas` and the
 * rest), which they read the same way.
 */
function colourHint(value: string): string | null {
  const colour = trimAscii(value)
  if (/^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i.test(colour)) {
    return colour
  }
  if (/^[0-9a-f]{6}$/i.test(colour)) {
    return `#${colour}`
  }
  return NAMED_COLOURS.has(asciiLowerCase(colour)) ? colour : null
}

/**
 * Whether a cell's `style` attribute replaces its hint for `property`, as
 * an inline style replaces a presentational hint in a browser: when it
 * declares the property, an input-only shorthand that sets it
 * ({@link SHORTHAND_TARGETS}: `background` for the fill) or `all`, which
 * sets every property. The declaration counts whether or not the server
 * can store its value: `width: 50%` hides a `width="100"`, because a
 * browser shows 50%, and the cell then keeps no width (§3.3 stores px
 * only). The logical properties (`inline-size`, `block-size`) are not read
 * as widths or heights: which one they set depends on the writing mode.
 *
 * @param property - the hint's property (`background-color`).
 * @param declared - the names the style declares ({@link declaredProperties}).
 */
function styleReplacesHint(
  property: string,
  declared: ReadonlySet<string>
): boolean {
  return (
    declared.has(property) ||
    declared.has("all") ||
    Object.entries(SHORTHAND_TARGETS).some(
      ([shorthand, target]) => target === property && declared.has(shorthand)
    )
  )
}

/**
 * A `width` or `height` value as px, as HTML's rules for nonzero dimension
 * values read it: leading digits, an optional fraction, anything after it
 * ignored. A percentage is left out (content-v1 stores cell widths in px
 * only, §3.3), and so is zero.
 */
function lengthHint(value: string): string | null {
  const match = /^([0-9]+(?:\.[0-9]+)?)(%?)/.exec(trimAscii(value))
  if (match === null || match[2] === "%" || Number(match[1]) === 0) {
    return null
  }
  return `${match[1]}px`
}

/**
 * The presentational hints of a cell's attributes that apply, as a style
 * string, in the WHATWG rendering rules' meaning: `align` (with `middle`
 * and `absmiddle` centring), `valign`, `bgcolor`, `width` and `height`. A
 * hint whose property the cell's `style` declares is left out
 * ({@link styleReplacesHint}), as a browser leaves it out.
 *
 * @param element - the `td` or `th`.
 * @returns the declarations, joined with `; `, or `null` for none.
 */
export function cellHintStyle(element: Element): string | null {
  const read = (name: string) => element.getAttribute(name)
  // Enumerated attribute values are ASCII case-insensitive (HTML §2.3.3).
  const keyword = (name: string) => asciiLowerCase(trimAscii(read(name) ?? ""))
  const align = keyword("align")
  const valign = keyword("valign")
  const hints: readonly (readonly [string, string | null])[] = [
    // Own keys only, so `align="constructor"` reads no inherited member.
    [
      "text-align",
      Object.hasOwn(ALIGN_HINTS, align) ? ALIGN_HINTS[align] : null,
    ],
    ["vertical-align", VALIGN_HINTS.has(valign) ? valign : null],
    ["background-color", colourHint(read("bgcolor") ?? "")],
    ["width", lengthHint(read("width") ?? "")],
    ["height", lengthHint(read("height") ?? "")],
  ]
  const declared = declaredProperties(read("style") ?? "")
  const declarations: string[] = []
  for (const [property, value] of hints) {
    if (value !== null && !styleReplacesHint(property, declared)) {
      declarations.push(`${property}: ${value}`)
    }
  }
  return declarations.length === 0 ? null : declarations.join("; ")
}

/** The memo of {@link readCellDecls}, per policy and element. */
const cellMemo = new WeakMap<
  RichTextPolicy,
  WeakMap<Element, { key: string; declarations: ReadonlyMap<string, string> }>
>()

/**
 * The declarations the server stores for a cell: its `style` attribute
 * through `canonicalStyle`, over the presentational hints of its `align`,
 * `valign`, `bgcolor`, `width` and `height` attributes
 * ({@link cellHintStyle}). The `style` attribute wins, as it does over a
 * presentational hint in a browser: a hint whose property the style
 * declares is not read at all, so a declaration the server cannot store
 * leaves the cell without that property rather than with the hint's value.
 *
 * The result is memoised per element: every cell attribute's parse rule
 * calls it, ten times per cell, and the memo re-reads the element only when
 * one of the six attributes changed.
 *
 * @param policy - the content-v1 policy.
 * @param element - the `td` or `th` a parse rule matched.
 * @returns the canonical declarations by property; the border sides are all
 *   four or none, and `box-sizing` comes exactly with a size. The map is
 *   shared and must not be changed.
 * @throws {PolicyError} as `canonicalStyle` does.
 */
export function readCellDecls(
  policy: RichTextPolicy,
  element: Element
): ReadonlyMap<string, string> {
  const style = element.getAttribute("style")
  const hints = cellHintStyle(element)
  const key = JSON.stringify([element.localName, style, hints])
  let memo = cellMemo.get(policy)
  if (memo === undefined) {
    memo = new WeakMap()
    cellMemo.set(policy, memo)
  }
  const cached = memo.get(element)
  if (cached?.key === key) {
    return cached.declarations
  }
  const tag = element.localName
  // Each part is read on its own, so a hint can never cut into the style's
  // own 32-declaration limit (editor.md §7.3). The two share no property
  // but `box-sizing`, which both write as `content-box` with a size:
  // cellHintStyle leaves out every hint the style declares, and the hints
  // hold no border side.
  const declarations: ReadonlyMap<string, string> = new Map([
    ...readStoredDeclarations(policy, tag, hints),
    ...readStoredDeclarations(policy, tag, style),
  ])
  memo.set(element, { key, declarations })
  return declarations
}

/**
 * The one sorted style string of a cell (editor.md §4.3 row 7): each
 * attribute that holds a value the server stores, in its canonical
 * spelling, then the whole through `canonicalStyle`, which sorts the
 * declarations, writes the sides a border attribute leaves unset as `none`
 * (§7.3) and adds `box-sizing: content-box` to a sized cell (§3.3).
 *
 * @param policy - the content-v1 policy.
 * @param tag - `td` or `th`.
 * @param attrs - the cell node's attributes.
 * @param attributes - the cell attributes ({@link cellStyleAttributes}).
 * @returns the style to render, or `null` when the cell has none. A value
 *   that the server would drop, or that holds a `;` and so would read as
 *   more than one declaration, renders nothing.
 */
export function cellStyle(
  policy: RichTextPolicy,
  tag: CellTag,
  attrs: Readonly<Record<string, unknown>>,
  attributes: readonly CellStyleAttribute[]
): string | null {
  const declarations: string[] = []
  for (const { attribute, property } of attributes) {
    const value = attrs[attribute]
    if (typeof value !== "string" || value.includes(";")) {
      continue
    }
    const canonical = readStoredDeclarations(
      policy,
      tag,
      `${property}: ${value}`
    ).get(property)
    if (canonical !== undefined) {
      declarations.push(`${property}: ${canonical}`)
    }
  }
  return declarations.length === 0
    ? null
    : canonicalStyle(policy, tag, declarations.join("; "))
}

/** One cell that {@link unanchoredSpans} narrows. */
export interface SpanNarrowing {
  /** The cell's offset in its table, as prosemirror-tables' `TableMap` gives it. */
  readonly position: number
  /** The cell's new `colspan`, at least 1. */
  readonly colspan: number
}

/**
 * The spans of a table to narrow before prosemirror-tables pads it: in a
 * table whose fix adds cells (a short row, or a cell that collides with a
 * rowspan from above), every cell whose span crosses a column in which no
 * cell starts loses those columns (editor.md §4.3 row 5).
 *
 * prosemirror-tables makes a table as wide as its widest row's spans and
 * fills every other row up to that width, so a span far wider than the
 * real grid turns every short row into that many empty cells (the module
 * documentation gives the numbers). A column in which no cell starts is
 * what HTML's table model calls an error, and a browser gives it no width
 * of its own, so dropping it from every span that crosses it changes
 * nothing that a reader saw and leaves `fixTables` only the cells that the
 * real grid lacks. The table is then at most as wide as the number of
 * columns in which a cell starts, at most its cell count, and padding it
 * adds at most that many cells per row.
 *
 * A rectangular table is left alone, however wide its spans: it needs no
 * padding, and editor.md §3.7's canonical shapes include a lone cell with
 * `colspan="2"`, which must stay a fixed point.
 *
 * @param table - a table node.
 * @returns the cells to narrow, in table order; empty when the table needs
 *   no cell added or no span crosses such a column.
 */
export function unanchoredSpans(table: ProseMirrorNode): SpanNarrowing[] {
  const map = TableMap.get(table)
  const addsCells = map.problems?.some(
    ({ type }) => type === "missing" || type === "collision"
  )
  if (addsCells !== true) {
    return []
  }
  // A cell's first slot in row order is its top-left one (TableMap places
  // each cell at the first free slot of its row), and 0 marks a slot that
  // no cell covers: cell offsets start at 1, inside the first row.
  const starts = new Map<number, number>()
  const anchored = new Array<boolean>(map.width).fill(false)
  map.map.forEach((position, slot) => {
    if (position !== 0 && !starts.has(position)) {
      starts.set(position, slot % map.width)
      anchored[slot % map.width] = true
    }
  })
  const narrowings: SpanNarrowing[] = []
  for (const [position, left] of starts) {
    const colspan = Number(table.nodeAt(position)?.attrs.colspan ?? 1)
    const end = Math.min(left + colspan, map.width)
    let unanchored = 0
    for (let column = left + 1; column < end; column += 1) {
      if (!anchored[column]) {
        unanchored += 1
      }
    }
    if (unanchored > 0) {
      narrowings.push({ position, colspan: colspan - unanchored })
    }
  }
  return narrowings.sort((a, b) => a.position - b.position)
}

/**
 * Calls `visit` for each table of `after` that `before` does not share at
 * the same place, outer tables before the tables in their cells. Like
 * `fixTables`, it relies on ProseMirror's persistent documents: a node an
 * edit did not touch is the same object in both, so an edit outside tables
 * costs one comparison per block on its path. The comparison looks at the
 * same index and at the index shifted by the change in the child count, so
 * one inserted or removed block does not make every block after it look
 * new; a table that only moved is visited again, which costs a cached
 * `TableMap` lookup.
 */
function forEachChangedTable(
  before: ProseMirrorNode | null,
  after: ProseMirrorNode,
  offset: number,
  visit: (table: ProseMirrorNode, position: number) => void
): void {
  const shift = before === null ? 0 : before.childCount - after.childCount
  after.forEach((child, childOffset, index) => {
    const same = (at: number) =>
      before !== null &&
      at >= 0 &&
      at < before.childCount &&
      before.child(at) === child
    // A textblock or a leaf holds no table.
    if (
      same(index) ||
      same(index + shift) ||
      child.isTextblock ||
      child.isLeaf
    ) {
      return
    }
    const position = offset + childOffset
    if (child.type.spec.tableRole === "table") {
      visit(child, position)
    }
    const old =
      before !== null && index < before.childCount ? before.child(index) : null
    forEachChangedTable(old, child, position + 1, visit)
  })
}

/**
 * The narrowing of {@link unanchoredSpans} for every table that changed.
 *
 * @param doc - the document to narrow.
 * @param before - a document whose tables need no fix.
 * @returns a transaction on `doc` that narrows the spans, or `null` when no
 *   span needs it.
 */
function narrowUnanchoredSpans(
  doc: ProseMirrorNode,
  before: ProseMirrorNode
): Transaction | null {
  let tr: Transaction | null = null
  forEachChangedTable(before, doc, 0, (table, tablePosition) => {
    for (const { position, colspan } of unanchoredSpans(table)) {
      tr ??= EditorState.create({ doc }).tr
      const at = tr.mapping.map(tablePosition + 1 + position)
      const cell = tr.doc.nodeAt(at)
      if (cell !== null) {
        tr.setNodeMarkup(at, undefined, { ...cell.attrs, colspan })
      }
    }
  })
  return tr
}

/**
 * The most passes one call of {@link fixTablesFully} runs. A pass narrows
 * the spans of {@link unanchoredSpans} and then runs `fixTables`, which
 * fixes every problem it finds; a later pass finds only what an earlier
 * fix left (a row that a narrowed collision made short). Two passes
 * settled every table the tests build, so eight leave a wide margin.
 *
 * The limit bounds the work of one call, not the editor's. In the editor,
 * ProseMirror calls a plugin's `appendTransaction` again after every
 * transaction that another plugin appends, and sets no limit of its own:
 * when this plugin's fix is not complete, `tableEditing`'s fix, which runs
 * after it, appends another, this plugin runs again, and the two take
 * turns until neither changes anything. A table therefore always reaches
 * its fixed point within the transaction that changed it, and a defect in
 * prosemirror-tables that never settled would loop there whatever this
 * limit is.
 */
export const MAX_FIX_PASSES = 8

/**
 * Fixes the tables of a state until none has a problem left: the
 * narrowing of {@link unanchoredSpans}, then prosemirror-tables'
 * `fixTables`, repeated on their own result.
 *
 * @param state - the state after the transactions to check.
 * @param oldState - the state before them; only tables that changed since
 *   are checked, as `fixTables` does, so an edit outside tables costs one
 *   comparison per block on its path.
 * @returns one transaction on `state` with the steps of every pass, or
 *   `null` when no table needed a fix.
 */
export function fixTablesFully(
  state: EditorState,
  oldState: EditorState
): Transaction | null {
  let tr: Transaction | null = null
  let before: ProseMirrorNode = oldState.doc
  let doc: ProseMirrorNode = state.doc
  const add = (steps: Transaction) => {
    tr ??= state.tr
    for (const step of steps.steps) {
      tr.step(step)
    }
  }
  for (let pass = 0; pass < MAX_FIX_PASSES; pass += 1) {
    const narrowed = narrowUnanchoredSpans(doc, before)
    if (narrowed !== null) {
      add(narrowed)
      doc = narrowed.doc
    }
    // Bare states of the documents: `fixTables` reads only `doc` and `tr`,
    // and a state without plugins runs no appendTransaction when the fix is
    // built, so this plugin never calls itself. `before` stays the document
    // of the pass's start, so the tables the narrowing left alone are
    // checked too.
    const fix = fixTables(
      EditorState.create({ doc }),
      EditorState.create({ doc: before })
    )
    if (fix === undefined) {
      break
    }
    add(fix)
    before = doc
    doc = fix.doc
  }
  return tr
}

/**
 * The plugin of {@link fixTablesFully}. It comes before `tableEditing`, so
 * its narrowing sees a loaded table before `tableEditing`'s own
 * `fixTables` pads it; `tableEditing` then finds nothing left to fix.
 */
const fixTablesFullyKey = new PluginKey("vfgFixTablesFully")

/**
 * A cell node of the site schema: TipTap's `TableCell` or `TableHeader`,
 * extended as the module documentation says.
 */
function createSiteCell(
  policy: RichTextPolicy,
  tag: CellTag,
  attributes: readonly CellStyleAttribute[]
) {
  const base = tag === "td" ? TableCell : TableHeader
  const colspan = requireAttributeRule(policy, tag, "colspan", "integer")
  const rowspan = requireAttributeRule(policy, tag, "rowspan", "integer")
  const span = (rule: typeof colspan, name: string) => ({
    default: 1,
    parseHTML: (element: HTMLElement) =>
      readIntegerAttribute(element.getAttribute(name), rule) ?? 1,
    renderHTML: (attrs: Record<string, unknown>) =>
      isStoredInteger(attrs[name], rule) ? { [name]: String(attrs[name]) } : {},
  })
  return base.extend({
    addAttributes() {
      return {
        colspan: span(colspan, "colspan"),
        rowspan: span(rowspan, "rowspan"),
        colwidth: { default: null, parseHTML: () => null, rendered: false },
        ...Object.fromEntries(
          attributes.map(({ attribute, property }) => [
            attribute,
            {
              default: null,
              parseHTML: (element: HTMLElement) =>
                readCellDecls(policy, element).get(property) ?? null,
              renderHTML: () => ({}),
            },
          ])
        ),
      }
    },

    renderHTML({ node, HTMLAttributes }) {
      // HTMLAttributes holds only colspan and rowspan: every other
      // attribute renders nothing, and the cell's options add no class.
      const rendered: Record<string, string> = {}
      for (const name of ["colspan", "rowspan"]) {
        if (typeof HTMLAttributes[name] === "string") {
          rendered[name] = HTMLAttributes[name]
        }
      }
      const style = cellStyle(policy, tag, node.attrs, attributes)
      if (style !== null) {
        rendered.style = style
      }
      return [tag, rendered, 0]
    },
  })
}

/**
 * The table nodes of the site schema.
 *
 * @param policy - the content-v1 policy; supplies the forced `table` and
 *   `tbody` styles, the cell grammars and the `colspan` and `rowspan` rules.
 * @returns the `table`, `tableRow`, `tableHeader` and `tableCell` node
 *   extensions.
 * @throws {PolicyError} when the policy lacks one of those rules, or when
 *   `td` and `th` store different declarations.
 */
export function createSiteTable(policy: RichTextPolicy): AnyExtension[] {
  const tableStyle = requireAttributeRule(
    policy,
    "table",
    "style",
    "forced"
  ).value
  const tbodyStyle = requireAttributeRule(
    policy,
    "tbody",
    "style",
    "forced"
  ).value
  const attributes = cellStyleAttributes(policy)
  const table = Table.extend({
    renderHTML() {
      return [
        "table",
        { style: tableStyle },
        ["tbody", { style: tbodyStyle }, 0],
      ]
    },

    addProseMirrorPlugins() {
      // Ours first: ProseMirror runs appendTransaction in plugin order, and
      // the narrowing must come before tableEditing's padding.
      return [
        new Plugin({
          key: fixTablesFullyKey,
          appendTransaction: (_transactions, oldState, state) =>
            fixTablesFully(state, oldState),
        }),
        ...(this.parent?.() ?? []),
      ]
    },
  }).configure({
    HTMLAttributes: {},
    resizable: false,
    renderWrapper: false,
    View: null,
    allowTableNodeSelection: false,
  })
  return [
    table,
    TableRow.configure({ HTMLAttributes: {} }),
    createSiteCell(policy, "th", attributes),
    createSiteCell(policy, "td", attributes),
  ]
}
