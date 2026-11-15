/**
 * The data-provider handlers of the labels, „címkék” (parity LBL-01): the
 * generic list reads of `src/data-provider/list-handlers.ts` over
 * `/api/admin/labels`, with the reader of one label.
 *
 * The server answers a label as the `LabelResponse` binding; the list and
 * the record routes let in `edit labels`, and `edit posts` for the post
 * form's label picker (ADR 0007 §4). The writes come with their own slices.
 */
import type { LabelResponse } from "@/bindings/LabelResponse"
import type { ResourceHandlers } from "@/data-provider/data-provider"
import { isRecordId } from "@/data-provider/list-contract"
import { createListHandlers } from "@/data-provider/list-handlers"

/** The ra-core resource name of the labels, as the access map names it. */
export const LABELS_RESOURCE = "labels"

/** The labels' path below the admin API. */
export const LABELS_PATH = "/labels"

/**
 * A label's colour: `#rrggbb`, upper- or lower-case hex digits, the one form
 * the `labels` table's CHECK accepts.
 */
const LABEL_COLOUR = /^#[0-9A-Fa-f]{6}$/

/**
 * Whether `value` is a colour a label can have.
 *
 * @param value - any value, such as a member of a JSON answer.
 * @returns `true` for a `#rrggbb` string.
 */
export function isLabelColour(value: unknown): value is string {
  return typeof value === "string" && LABEL_COLOUR.test(value)
}

/**
 * Reads one label of an answer.
 *
 * @param value - one row of the list's `data`, or the record route's body.
 * @returns the label, with exactly the binding's three fields, when `id` is
 *   a record id, `name` a string and `color` a `#rrggbb` colour; otherwise
 *   `undefined`.
 */
export function parseLabel(value: unknown): LabelResponse | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return undefined
  }
  const { id, name, color } = value as Record<string, unknown>
  if (!isRecordId(id) || typeof name !== "string" || !isLabelColour(color)) {
    return undefined
  }
  return { id, name, color }
}

/**
 * Builds the labels' handlers. They send through the data provider's
 * `request`, as every handler does (`src/data-provider/list-handlers.ts`).
 *
 * @returns `getList`, `getOne` and `getMany` for `labels`.
 */
export function createLabelHandlers(): ResourceHandlers {
  return createListHandlers({ path: LABELS_PATH, parseRow: parseLabel })
}
