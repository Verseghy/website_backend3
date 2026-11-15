/**
 * Step 8 of the paste filter (editor.md §5.2): the declarations of each
 * content-v1 element are mapped onto content-v1's vocabulary, so the
 * filter's output holds only the properties of §3.3 in their one spelling
 * (P-paste, §6).
 *
 * **Where the declarations come from.** From any source but the editor's
 * own clipboard, an element's `style` attribute holds its inlined rules
 * (step 6), possibly far more than the 32 declarations a style reader reads
 * (§7.3; Word for the web writes about forty computed properties on every
 * run). So the filter reads from the element's CSSOM the cascaded value of
 * each property that the policy lets the element store, and only those go
 * through `canonicalStyle`. The editor's own clipboard carries what the
 * editor wrote, in the browser's CSSOM spelling (`rgb()`, grouped borders),
 * so its `style` attribute goes through `canonicalStyle` as it is (§5.2,
 * "The editor's own clipboard").
 *
 * **The paste-time rules** of step 8 that apply to every source but the
 * editor's own clipboard:
 *
 * - a cell's widths and heights are rounded to whole px, and its border
 *   widths too, a width above 0 to at least 1 px, so `0.5pt` (0.67 px) is
 *   1 px and `1.0pt` is 1 px. Excel desktop will map its border weights to
 *   its on-screen widths instead (`.5pt` 1 px, `1.0pt` 2 px, …), which waits
 *   for the Windows captures (§16 item 1) and the Excel slice; until then
 *   Excel's borders take this rule. The editor's own clipboard is not
 *   rounded: it holds stored values already (`128.25px` is a stored
 *   width), and an internal copy keeps them;
 * - a cell without its own `white-space` gets `white-space: normal`, so it
 *   wraps as it did in the source, except from Excel desktop, whose cells
 *   rely on their own value or on the sites' `nowrap`;
 * - `align` on a paragraph or heading, and the `align`, `valign`,
 *   `bgcolor`, `width` and `height` hints of a cell, become declarations
 *   under the element's own (step 10 lists them; the cell hints are read as
 *   the schema reads them, `cellHintStyle`);
 * - `list-style-type` on an `ol` becomes its `type` (`lower-alpha` to `a`,
 *   `upper-roman` to `I`, …), the one way content-v1 stores it.
 *
 * A text colour, highlight and size come to `span`s through
 * `inline-marks.ts`, which also drops black text and white or transparent
 * highlights (§3.3, §5.2 step 9).
 */
import { BORDER_SIDES, canonicalStyle } from "../css/canonical-style"
import { asciiLowerCase } from "../css/declarations"
import { cellHintStyle } from "../extensions/site-table"
import type { RichTextPolicy } from "../policy"
import { elementOf } from "../policy"
import { readStoredDeclarations } from "../style-decls"
import type { PasteContext } from "./paste-context"

/** The tags whose `align` attribute is a `text-align` hint. */
const ALIGNABLE_BLOCKS = new Set(["p", "h1", "h2", "h3", "h4"])

/** The `align` values a paragraph or heading reads as `text-align`. */
const ALIGN_VALUES = new Set(["left", "center", "right", "justify"])

/** The `ol` `type` of each `list-style-type` that content-v1 stores. */
export const LIST_TYPE_OF_STYLE: Readonly<Record<string, string>> = {
  "lower-alpha": "a",
  "lower-latin": "a",
  "upper-alpha": "A",
  "upper-latin": "A",
  "lower-roman": "i",
  "upper-roman": "I",
}

/**
 * The properties an element may store, in the policy's order: its stored
 * `style` rules, without the input-only shorthands (the CSSOM has already
 * expanded them).
 *
 * @param policy - the content-v1 policy.
 * @param tag - the element's tag.
 * @returns the property names; empty for an element without a `style`.
 */
export function storedProperties(
  policy: RichTextPolicy,
  tag: string
): string[] {
  return (elementOf(policy, tag)?.style ?? [])
    .filter((rule) => rule.role === "stored")
    .map((rule) => rule.property)
}

/**
 * The cascaded declarations of some properties, from an element's CSSOM.
 *
 * @param element - an element whose `style` attribute holds its inlined
 *   rules.
 * @param properties - the properties to read.
 * @returns `property: value` pairs joined with `; `, each with the value the
 *   engine serialises; properties without a value are left out.
 */
export function cssomDeclarations(
  element: Element,
  properties: readonly string[]
): string {
  const style = (element as Partial<HTMLElement>).style
  if (style === undefined || style === null) {
    return ""
  }
  return properties
    .map((property) => [property, style.getPropertyValue(property).trim()])
    .filter(([, value]) => value !== "")
    .map(([property, value]) => `${property}: ${value}`)
    .join("; ")
}

/** A paragraph's or heading's `align` attribute as a declaration, or `null`. */
function alignHint(element: Element): string | null {
  const value = asciiLowerCase(
    (element.getAttribute("align") ?? "").replace(
      /^[\t\n\f\r ]+|[\t\n\f\r ]+$/g,
      ""
    )
  )
  return ALIGN_VALUES.has(value) ? `text-align: ${value}` : null
}

/** A px length rounded to whole px: `20.5px` to `21px`; other values as they are. */
function roundLength(value: string): string {
  const match = /^([0-9]+(?:\.[0-9]+)?)px$/.exec(value)
  return match === null ? value : `${Math.round(Number(match[1]))}px`
}

/** A border side with its width rounded, a width above 0 to at least 1 px. */
function roundBorderSide(value: string): string {
  const [width, ...rest] = value.split(" ")
  const match = /^([0-9]+(?:\.[0-9]+)?)px$/.exec(width)
  if (match === null) {
    return value
  }
  const px = Number(match[1])
  const rounded = px > 0 ? Math.max(1, Math.round(px)) : 0
  return [`${rounded}px`, ...rest].join(" ")
}

/**
 * Writes declarations back as a canonical style string.
 *
 * @param policy - the content-v1 policy.
 * @param tag - the element's tag.
 * @param declarations - canonical declarations by property.
 * @returns the canonical string, or `null` when nothing is stored.
 */
export function writeDeclarations(
  policy: RichTextPolicy,
  tag: string,
  declarations: ReadonlyMap<string, string>
): string | null {
  return canonicalStyle(
    policy,
    tag,
    [...declarations]
      .map(([property, value]) => `${property}: ${value}`)
      .join("; ")
  )
}

/**
 * A cell style with the paste-time rounding of step 8: widths, heights and
 * border widths in whole px.
 *
 * @param policy - the content-v1 policy.
 * @param tag - `td` or `th`.
 * @param style - the cell's canonical style, or `null`.
 * @returns the rounded canonical style, or `null`; a length that rounds to
 *   0 px leaves the cell, as the `px` grammar's minimum of 1 px refuses it.
 */
export function roundCellStyle(
  policy: RichTextPolicy,
  tag: string,
  style: string | null
): string | null {
  const declarations = new Map(readStoredDeclarations(policy, tag, style))
  if (declarations.size === 0) {
    return null
  }
  for (const [property, value] of declarations) {
    if (property === "width" || property === "height") {
      declarations.set(property, roundLength(value))
    } else if ((BORDER_SIDES as readonly string[]).includes(property)) {
      declarations.set(property, roundBorderSide(value))
    }
  }
  return writeDeclarations(policy, tag, declarations)
}

/**
 * The canonical `style` of a content-v1 element after step 8.
 *
 * @param context - the paste: the policy and the source.
 * @param element - the element, with its inlined `style` (step 6) unless
 *   the source is the editor's own clipboard.
 * @param tag - the element's content-v1 tag (an `h5` already mapped to
 *   `h4`, so not always `element.localName`).
 * @returns the style to store, or `null` for none.
 * @throws {PolicyError} as `canonicalStyle` does.
 */
export function pastedStyle(
  context: PasteContext,
  element: Element,
  tag: string
): string | null {
  const { policy, source } = context
  const properties = storedProperties(policy, tag)
  if (properties.length === 0) {
    return null
  }
  const own =
    source === "internal"
      ? (element.getAttribute("style") ?? "")
      : cssomDeclarations(element, properties)
  const hint =
    tag === "td" || tag === "th"
      ? cellHintStyle(element)
      : ALIGNABLE_BLOCKS.has(tag)
        ? alignHint(element)
        : null
  // The hint first, so the element's own declaration wins (the canonical
  // cascade keeps the last occurrence of a property).
  const style = canonicalStyle(
    policy,
    tag,
    [hint, own].filter((part) => part !== null && part !== "").join("; ")
  )
  if ((tag !== "td" && tag !== "th") || source === "internal") {
    return style
  }
  const rounded = roundCellStyle(policy, tag, style)
  if (source === "excel") {
    return rounded
  }
  const declarations = new Map(readStoredDeclarations(policy, tag, rounded))
  if (!declarations.has("white-space")) {
    declarations.set("white-space", "normal")
  }
  return writeDeclarations(policy, tag, declarations)
}

/**
 * The `type` attribute an `ol` gets from its `list-style-type`, when it has
 * no `type` of its own.
 *
 * @param element - the `ol`, with its inlined `style`.
 * @returns `a`, `A`, `i` or `I`, or `null` when the style gives none (a
 *   decimal list, or a type content-v1 does not store).
 */
export function listTypeFromStyle(element: Element): string | null {
  const style = (element as Partial<HTMLElement>).style
  const value = asciiLowerCase(
    style?.getPropertyValue("list-style-type").trim() ?? ""
  )
  return Object.hasOwn(LIST_TYPE_OF_STYLE, value)
    ? LIST_TYPE_OF_STYLE[value]
    : null
}
