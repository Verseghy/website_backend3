/**
 * A label's colour in the label list (parity LBL-01, "list shows name and
 * colour"): a swatch filled with the colour, and the colour's `#rrggbb`
 * text beside it.
 *
 * The swatch is decorative (`aria-hidden`): the text says the same to a
 * screen reader, and lets the editor tell two close colours apart. The
 * colour reaches the page only through the element's `style` object, which
 * React writes through the CSS object model, never as markup, and only when
 * it is a `#rrggbb` value; any other text leaves the swatch empty and is
 * shown as escaped text like every list cell (parity.md, "every list cell is
 * rendered as escaped text").
 */
import { useRecordContext } from "ra-core"

import type { LabelResponse } from "@/bindings/LabelResponse"

import { isLabelColour } from "./label-handlers"

/**
 * Renders the colour of the label in the record context.
 *
 * @returns the swatch and the colour's text, or nothing outside a record.
 */
export function LabelColorField() {
  const record = useRecordContext<LabelResponse>()
  if (record === undefined) {
    return null
  }
  const { color } = record
  return (
    <span className="inline-flex items-center gap-2">
      <span
        aria-hidden="true"
        data-testid="label-color-swatch"
        className="inline-block size-4 shrink-0 rounded-sm border"
        style={isLabelColour(color) ? { backgroundColor: color } : undefined}
      />
      <span className="font-mono text-sm">{color}</span>
    </span>
  )
}
