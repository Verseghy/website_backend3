/**
 * What the image and embed nodes share (editor.md §4.3 rows 8-10): the
 * forced iframe attributes, the check that a style the node writes is
 * already in content-v1's one spelling, the reading of a wrapper whose
 * only content is one element, and where the nodes may be placed.
 *
 * The nodes take every stored value from the policy or check it against the
 * policy when the schema is built, so a policy change that would make the
 * server rewrite or drop what a node writes stops the editor at once, with
 * the value it expected, instead of reaching the server as a drift
 * (editor.md §7.7, "Drift canary").
 */
import type { ResolvedPos } from "@tiptap/pm/model"
import type { Selection } from "@tiptap/pm/state"

import { canonicalStyle } from "../css/canonical-style"
import type { RichTextPolicy } from "../policy"
import { PolicyError, requireAttributeRule } from "../policy"

/** The attributes every stored iframe carries with one value (§3.2). */
export interface ForcedFrameAttributes {
  /** The `allow` value. */
  readonly allow: string
  /** The `allowfullscreen` value, the empty string. */
  readonly allowfullscreen: string
  /** The `referrerpolicy` value. */
  readonly referrerpolicy: string
}

/**
 * The forced iframe attributes of a policy.
 *
 * @param policy - the content-v1 policy.
 * @returns the three values every embed writes.
 * @throws {PolicyError} when one of them is not a forced attribute of
 *   `iframe`.
 */
export function forcedFrameAttributes(
  policy: RichTextPolicy
): ForcedFrameAttributes {
  const forced = (name: string) =>
    requireAttributeRule(policy, "iframe", name, "forced").value
  return {
    allow: forced("allow"),
    allowfullscreen: forced("allowfullscreen"),
    referrerpolicy: forced("referrerpolicy"),
  }
}

/**
 * Checks that a style a node writes is stored as it is: the server's
 * canonical spelling of it is itself.
 *
 * @param policy - the content-v1 policy.
 * @param tag - the element the style is written on.
 * @param style - the style the node writes.
 * @returns `style`.
 * @throws {PolicyError} when the server would store another style, naming
 *   both.
 */
export function requireStoredStyle(
  policy: RichTextPolicy,
  tag: string,
  style: string
): string {
  const stored = canonicalStyle(policy, tag, style)
  if (stored !== style) {
    throw new PolicyError(
      `The style "${style}" that the editor writes on ${tag} would be stored as ${JSON.stringify(stored)} (editor.md §3.3, §3.7); update the node with the policy.`
    )
  }
  return style
}

/** Whether a node is text of ASCII whitespace only, which a wrapper may hold. */
function isBlank(node: Node): boolean {
  return (
    node.nodeType === node.TEXT_NODE &&
    /^[\t\n\f\r ]*$/.test(node.textContent ?? "")
  )
}

/**
 * The one element of a wrapper whose only content, apart from ASCII
 * whitespace, is that element.
 *
 * @param wrapper - the wrapper, such as a `p`.
 * @returns its only element child, or `null` when it holds anything else:
 *   other text, a comment, or more or fewer than one element.
 */
export function loneElementChild(wrapper: Element): Element | null {
  const elements: Element[] = []
  for (const child of wrapper.childNodes) {
    if (child.nodeType === child.ELEMENT_NODE) {
      elements.push(child as Element)
    } else if (!isBlank(child)) {
      return null
    }
  }
  return elements.length === 1 ? elements[0] : null
}

/**
 * The first `iframe` child of an embed wrapper (`p[data-youtube-video]`,
 * `div[data-youtube-video]`, `p[data-padlet-embed]`).
 *
 * @param wrapper - the wrapper element.
 * @returns the iframe, or `null` when the wrapper has none.
 */
export function wrapperIframe(wrapper: Element): Element | null {
  return (
    [...wrapper.children].find((child) => child.localName === "iframe") ?? null
  )
}

/** The paragraph node of StarterKit, the one textblock media may split. */
const PARAGRAPH_NODE = "paragraph"

/**
 * Whether an image or embed may be placed at a position (editor.md §4.2):
 * between two top-level blocks, or in a top-level paragraph, which the
 * insertion splits around it, as the paste filter does with media in running
 * text (§5.2 step 11a).
 *
 * Anywhere else (a list item, a quote, a table cell, a heading) the
 * document's content expression has no room for a `media` node, and
 * ProseMirror would fit one in by cutting the list, quote or table in two
 * around it. The insert commands and the YouTube paste rule refuse there
 * instead, so `editor.can()` disables the toolbar buttons (§4.2).
 *
 * @param $pos - a position resolved in the document.
 * @returns `true` when a media node may replace or split the content there.
 */
export function isMediaPlace($pos: ResolvedPos): boolean {
  return (
    $pos.depth === 0 ||
    ($pos.depth === 1 && $pos.parent.type.name === PARAGRAPH_NODE)
  )
}

/**
 * Whether an image or embed may replace a selection: both of its ends are a
 * place for media ({@link isMediaPlace}). A node selection of a top-level
 * block is replaced; a cell selection, or a range that ends in a list, a
 * quote, a table or a heading, is refused.
 *
 * @param selection - the selection a command would replace.
 * @returns `true` when the media node may replace it.
 */
export function canPlaceMedia(selection: Selection): boolean {
  return isMediaPlace(selection.$from) && isMediaPlace(selection.$to)
}
