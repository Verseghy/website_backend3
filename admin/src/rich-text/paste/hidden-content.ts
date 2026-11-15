/**
 * Step 7 of the paste filter (editor.md §5.2): content that the source
 * hides is removed before anything else reads it.
 *
 * Excel keeps hidden rows and columns in its clipboard HTML, styled
 * `display: none`. content-v1 has no `display` or `visibility` (editor.md
 * §3.4), so a hidden row would become visible on both sites; it is dropped
 * instead (§5.6, "Hidden rows and columns"). The step runs after step 6, so
 * a row hidden by a class rule is found too: it reads each element's
 * cascaded `display` and `visibility` through the CSSOM of its inlined
 * `style` attribute.
 *
 * An element counts as hidden when its `display` is `none` or its
 * `visibility` is `hidden` or `collapse` (`collapse` hides a table row or
 * column as `hidden` hides any other box). A hidden element goes with its
 * whole content. The editor's own clipboard skips this step (§5.2, "The
 * editor's own clipboard").
 *
 * Excel's `mso-ignore:colspan` and the exact format of a hidden column wait
 * for the Windows captures (§16 item 1); the Excel slices adjust spans when
 * they read them.
 */
import { asciiLowerCase } from "../css/declarations"

/** What step 7 removed, for the paste report (§5.2 step 12). */
export interface HiddenContent {
  /** Hidden table rows. */
  readonly rows: number
  /** Hidden table cells: hidden columns, cell by cell. */
  readonly cells: number
  /** Every other hidden element. */
  readonly other: number
}

/** The cascaded value of a property, trimmed and lower case, or `""`. */
function cascaded(element: Element, property: string): string {
  // Elements outside the HTML namespace, such as an SVG element left in the
  // document, may have no CSSOM; they are never hidden here.
  const style = (element as Partial<HTMLElement>).style
  if (style === undefined || style === null) {
    return ""
  }
  return asciiLowerCase(style.getPropertyValue(property).trim())
}

/**
 * Whether an element's own style hides it.
 *
 * @param element - an element whose `style` attribute holds its inlined
 *   rules (step 6).
 * @returns `true` for `display: none`, `visibility: hidden` and
 *   `visibility: collapse`.
 */
export function isHiddenElement(element: Element): boolean {
  const visibility = cascaded(element, "visibility")
  return (
    cascaded(element, "display") === "none" ||
    visibility === "hidden" ||
    visibility === "collapse"
  )
}

/**
 * Removes every hidden element below `root`, with its content.
 *
 * @param root - the clipboard document's body; changed in place.
 * @returns how many rows, cells and other elements were removed (an element
 *   inside a removed one is not counted).
 */
export function removeHiddenContent(root: Element): HiddenContent {
  let rows = 0
  let cells = 0
  let other = 0
  const pending: Element[] = [...root.children]
  while (pending.length > 0) {
    const element = pending.pop()!
    if (isHiddenElement(element)) {
      const tag = element.localName
      if (tag === "tr") {
        rows += 1
      } else if (tag === "td" || tag === "th") {
        cells += 1
      } else {
        other += 1
      }
      element.remove()
    } else {
      pending.push(...element.children)
    }
  }
  return { rows, cells, other }
}
