/**
 * Paragraph and heading alignment (editor.md §4.3 row 2).
 *
 * TipTap 3.32's TextAlign on `paragraph` and `heading`, with the four
 * alignments of the policy and no default alignment, so an unaligned block
 * carries no style. Its `parseHTML` is replaced: stock reads
 * `element.style.textAlign`, the CSSOM, which the parse rules never use
 * (editor.md §2 stage 3); this one reads the raw `style` through
 * `readStoredDeclarations`, so `text-align: start` or a legacy `align`
 * attribute is not an alignment (the import converts `p[align]` before it
 * sanitises, editor.md §9).
 *
 * **In a table cell the commands align the cell** (§4.3 row 2, §4.5
 * "Toolbar"). When the selection is a cell selection, or lies inside one
 * cell, `setTextAlign`, `unsetTextAlign` and `toggleTextAlign` set the
 * cells' own `textAlign` (`site-table.ts`) and clear the alignment of the
 * paragraphs and headings in them. A cell's lone paragraph then has no
 * attribute and is stored tight, `<td style="text-align: center">x</td>`,
 * which is the shape frontend2 does not wrap (§3.7, "Tightening"). Stock
 * would align the paragraph, and the serialiser would move a lone
 * paragraph's alignment to the cell anyway, but a cell with two paragraphs
 * would keep one aligned paragraph and one not. The paragraphs inside a
 * nested table are left alone: their own cells align them. Everywhere else
 * the commands are stock.
 */
import type { CommandProps, RawCommands } from "@tiptap/core"
import { TextAlign } from "@tiptap/extension-text-align"
import type { Node as ProseMirrorNode } from "@tiptap/pm/model"
import type { Selection, Transaction } from "@tiptap/pm/state"
import { CellSelection } from "@tiptap/pm/tables"

import type { RichTextPolicy } from "../policy"
import { PolicyError, requireStyleGrammar } from "../policy"
import { readStoredDeclarations } from "../style-decls"

/** The node types that carry an alignment, and the tags they render. */
export const ALIGNED_TYPES: Readonly<Record<string, readonly string[]>> = {
  paragraph: ["p"],
  heading: ["h1", "h2", "h3", "h4"],
}

/** The table cell tags, whose alignment is a cell attribute. */
const CELL_TAGS = ["td", "th"] as const

/** Whether a node is a table cell, by its prosemirror-tables role. */
function isCell(node: ProseMirrorNode): boolean {
  const role: unknown = node.type.spec.tableRole
  return role === "cell" || role === "header_cell"
}

/**
 * The cells an alignment command aligns: every cell of a cell selection,
 * or the innermost cell that holds the whole selection. A selection that
 * runs from a nested table's cell into the text of the cell around that
 * table is in the outer cell, so the outer cell is aligned.
 *
 * @param selection - the editor's selection.
 * @returns the positions of the cells (before each cell node), in document
 *   order; empty when no one cell holds the selection (it starts or ends
 *   outside every table, or runs across two cells of one table), so the
 *   stock command aligns the blocks it covers.
 */
export function cellsToAlign(selection: Selection): number[] {
  if (selection instanceof CellSelection) {
    const cells: number[] = []
    selection.forEachCell((_cell, position) => cells.push(position))
    return cells.sort((a, b) => a - b)
  }
  const { $from, $to } = selection
  // The nodes down to the shared depth hold both ends; the deepest cell
  // among them is the innermost cell around the whole selection.
  for (let depth = $from.sharedDepth($to.pos); depth > 0; depth -= 1) {
    if (isCell($from.node(depth))) {
      return [$from.before(depth)]
    }
  }
  return []
}

/**
 * Sets the alignment of cells and clears it on the aligned blocks in them,
 * outside nested tables.
 *
 * @param tr - the transaction to add the steps to.
 * @param cells - the cell positions ({@link cellsToAlign}).
 * @param alignment - the cell's new `textAlign`, or `null` to remove it.
 * @param types - the node types whose `textAlign` is cleared.
 */
export function alignCells(
  tr: Transaction,
  cells: readonly number[],
  alignment: string | null,
  types: readonly string[]
): void {
  for (const position of cells) {
    const cell = tr.doc.nodeAt(position)
    if (cell === null || !isCell(cell)) {
      continue
    }
    tr.setNodeMarkup(position, undefined, {
      ...cell.attrs,
      textAlign: alignment,
    })
    // Changing attributes moves no position, so the offsets read from the
    // cell before the first step stay valid.
    cell.descendants((child, offset) => {
      if (child.type.spec.tableRole === "table") {
        return false
      }
      if (types.includes(child.type.name) && child.attrs.textAlign != null) {
        tr.setNodeMarkup(position + 1 + offset, undefined, {
          ...child.attrs,
          textAlign: null,
        })
      }
      return true
    })
  }
}

/**
 * The text-align extension of the site schema.
 *
 * @param policy - the content-v1 policy; supplies the `text-align` keywords,
 *   which must be the same on every aligned element and on the table cells.
 * @returns the `textAlign` extension.
 * @throws {PolicyError} when an aligned element or a cell lacks the keyword
 *   grammar or two of them differ.
 */
export function createSiteTextAlign(policy: RichTextPolicy) {
  const tags = [...Object.values(ALIGNED_TYPES).flat(), ...CELL_TAGS]
  const grammars = tags.map((tag) =>
    requireStyleGrammar(policy, tag, "text-align", "keyword")
  )
  const alignments = grammars[0].values
  tags.forEach((tag, index) => {
    if (grammars[index].values.join() !== alignments.join()) {
      throw new PolicyError(
        `${tag} text-align allows ${grammars[index].values.join(", ")}, but ${tags[0]} allows ${alignments.join(", ")}; the alignment commands are one for paragraphs, headings and table cells (editor.md §4.3 row 2).`
      )
    }
  })
  return TextAlign.extend({
    addGlobalAttributes() {
      return [
        {
          types: this.options.types,
          attributes: {
            textAlign: {
              default: null,
              parseHTML: (element) =>
                readStoredDeclarations(
                  policy,
                  element.tagName.toLowerCase(),
                  element.getAttribute("style")
                ).get("text-align") ?? null,
              renderHTML: (attributes) =>
                typeof attributes.textAlign === "string" &&
                this.options.alignments.includes(attributes.textAlign)
                  ? { style: `text-align: ${attributes.textAlign}` }
                  : {},
            },
          },
        },
      ]
    },

    addCommands() {
      const stock = this.parent?.() as Pick<
        RawCommands,
        "setTextAlign" | "unsetTextAlign" | "toggleTextAlign"
      >
      const { options } = this
      // In a cell, align the cells; `null` there means "no cell here", and
      // the stock command runs.
      const inCells = (
        { state, tr, dispatch }: CommandProps,
        alignment: (cells: readonly ProseMirrorNode[]) => string | null
      ): boolean | null => {
        const cells = cellsToAlign(state.selection)
        if (cells.length === 0) {
          return null
        }
        if (dispatch) {
          const nodes = cells.map((position) => state.doc.nodeAt(position)!)
          alignCells(tr, cells, alignment(nodes), options.types)
        }
        return true
      }
      return {
        ...stock,
        setTextAlign: (alignment) => (props) => {
          if (!options.alignments.includes(alignment)) {
            return false
          }
          return (
            inCells(props, () => alignment) ??
            stock.setTextAlign(alignment)(props)
          )
        },
        unsetTextAlign: () => (props) =>
          inCells(props, () => null) ?? stock.unsetTextAlign()(props),
        toggleTextAlign: (alignment) => (props) => {
          if (!options.alignments.includes(alignment)) {
            return false
          }
          // Off only when every cell already has it, as a toolbar button
          // shows a mixed selection as not active.
          return (
            inCells(props, (cells) =>
              cells.every((cell) => cell.attrs.textAlign === alignment)
                ? null
                : alignment
            ) ?? stock.toggleTextAlign(alignment)(props)
          )
        },
      }
    },
  }).configure({
    types: Object.keys(ALIGNED_TYPES),
    alignments: [...alignments],
    defaultAlignment: null,
  })
}
