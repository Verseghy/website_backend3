/**
 * Step 11a of the paste filter (editor.md §5.2): every image and embed
 * ends up in a top-level paragraph of its own, in content-v1's wrapper.
 *
 * The site schema keeps `SiteImage`, `YoutubeEmbed` and `PadletEmbed` at the
 * top level only (§4.2): Angular's page renderer moves every `<img>` in
 * front of its parent element, so media inside a cell, list, quote,
 * heading or link would not lay out the same on both sites. Left to
 * ProseMirror, a nested image or embed would cut its table or list in two.
 * So the filter places media itself, before ProseMirror parses:
 *
 * - **inside a `td`, `th`, `li`, `blockquote`, `h1`-`h4` or `a`**: the media
 *   moves into a paragraph of its own **before** the top-level block that
 *   holds it, and a link around it is dropped. Inline wrappers that held
 *   only the media go with it, and so does a top-level paragraph or heading
 *   that is left with nothing to show; a cell or list item stays, empty;
 * - **inside running text** of a top-level paragraph: the paragraph is
 *   split around it. Both halves keep the paragraph's attributes and the
 *   inline wrappers around the split, each half is trimmed at the split,
 *   and a half with no text and no hard break is dropped; the media
 *   paragraph goes between them;
 * - **directly in the body**, or in any other block: a paragraph of its own
 *   before that block;
 * - **alone in a top-level paragraph**: the paragraph becomes the canonical
 *   wrapper (§3.7): `<p>` for an image, `<p data-youtube-video="">` or
 *   `<p data-padlet-embed="">` for an embed.
 *
 * Every move is listed in the report. A YouTube embed also gets the title
 * rule of D135: a missing or blank title, or exactly "YouTube video player"
 * after trimming, becomes „Videó” (ADR 0043, §4.3 row 9).
 *
 * Step 10 has already kept only images and embeds whose addresses the
 * server keeps, so every `img` and `iframe` this step meets is media the
 * schema parses. The import's rule N5b places legacy media the same way
 * (data-migration.md, §9).
 */
import { readYoutubeEmbedSrc, youtubeTitle } from "../youtube-input"
import type { PasteContext, PastedMedia } from "./paste-context"
import { SLICE_MARKER } from "./paste-source"
import { hasContent } from "./paste-structure"

/** The containers out of which media moves before its top-level block. */
const MOVE_OUT_OF: ReadonlySet<string> = new Set([
  "td",
  "th",
  "li",
  "blockquote",
  "h1",
  "h2",
  "h3",
  "h4",
  "a",
])

/** Inline wrappers that go with media they held alone. */
const INLINE_WRAPPERS: ReadonlySet<string> = new Set([
  "a",
  "span",
  "strong",
  "em",
  "u",
  "s",
  "sub",
  "sup",
])

/** The textblocks that are removed when moving their media left them empty. */
const TEXTBLOCKS: ReadonlySet<string> = new Set(["p", "h1", "h2", "h3", "h4"])

/** The wrapper marker of each embed (editor.md §3.2). */
const MARKERS: Readonly<Record<Exclude<PastedMedia, "image">, string>> = {
  youtube: "data-youtube-video",
  padlet: "data-padlet-embed",
}

/** ASCII whitespace only. */
const ASCII_BLANK = /^[\t\n\f\r ]*$/

/** What kind of media an `img` or an `iframe` is. */
function mediaKind(element: Element, context: PasteContext): PastedMedia {
  if (element.localName === "img") {
    return "image"
  }
  const src = element.getAttribute("src") ?? ""
  return readYoutubeEmbedSrc(src, context.youtube).kind === "video"
    ? "youtube"
    : "padlet"
}

/** The child of `root` that holds `node`, or `null` when `node` is `root`. */
function topLevelBlock(root: Element, node: Element): Element | null {
  let current: Element = node
  while (current.parentElement !== null && current.parentElement !== root) {
    current = current.parentElement
  }
  return current.parentElement === root ? current : null
}

/** The nearest ancestor of `node` below `root` that media moves out of. */
function containerOf(root: Element, node: Element): Element | null {
  for (
    let current = node.parentElement;
    current !== null && current !== root;
    current = current.parentElement
  ) {
    if (MOVE_OUT_OF.has(current.localName)) {
      return current
    }
  }
  return null
}

/**
 * The canonical wrapper paragraph of one media element, which the media is
 * moved into; the caller inserts the paragraph.
 */
function mediaParagraph(media: Element, kind: PastedMedia): Element {
  const paragraph = media.ownerDocument.createElement("p")
  if (kind !== "image") {
    paragraph.setAttribute(MARKERS[kind], "")
  }
  paragraph.append(media)
  return paragraph
}

/** The text nodes below a node, in document order. */
function textNodes(node: Node): Text[] {
  const found: Text[] = []
  const pending: Node[] = [node]
  while (pending.length > 0) {
    const current = pending.pop()!
    if (current.nodeType === current.TEXT_NODE) {
      found.push(current as Text)
    } else {
      pending.push(...[...current.childNodes].reverse())
    }
  }
  return found
}

/**
 * Trims ASCII whitespace at one edge of a half of a split paragraph.
 *
 * @param half - the half.
 * @param edge - `end` for the half before the media, `start` for the one
 *   after it.
 */
function trimAtSplit(half: Element, edge: "start" | "end"): void {
  const texts = textNodes(half)
  if (edge === "end") {
    texts.reverse()
  }
  for (const text of texts) {
    const trimmed =
      edge === "end"
        ? text.data.replace(/[\t\n\f\r ]+$/, "")
        : text.data.replace(/^[\t\n\f\r ]+/, "")
    text.data = trimmed
    if (trimmed !== "") {
      return
    }
  }
}

/** Removes inline wrappers, from `start` up to `stop`, that show nothing. */
function removeEmptyWrappers(start: Element | null, stop: Element): void {
  let current = start
  while (
    current !== null &&
    current !== stop &&
    INLINE_WRAPPERS.has(current.localName) &&
    !hasContent(current)
  ) {
    const parent = current.parentElement
    current.remove()
    current = parent
  }
}

/** Whether a paragraph holds nothing but `media` and ASCII whitespace. */
function holdsOnly(paragraph: Element, media: Element): boolean {
  return [...paragraph.childNodes].every(
    (node) =>
      node === media ||
      (node.nodeType === node.TEXT_NODE &&
        ASCII_BLANK.test(node.textContent ?? ""))
  )
}

/**
 * Splits a top-level paragraph around media in its running text.
 *
 * @returns the half before the media and the half after it, each `null`
 *   when it shows nothing.
 */
function splitAround(
  paragraph: Element,
  media: Element
): [Element | null, Element | null] {
  const document = paragraph.ownerDocument
  const range = document.createRange()
  range.setStart(paragraph, 0)
  range.setEndBefore(media)
  const before = range.extractContents()
  range.setStartAfter(media)
  range.setEnd(paragraph, paragraph.childNodes.length)
  const after = range.extractContents()
  const half = (content: DocumentFragment, edge: "start" | "end") => {
    const element = paragraph.cloneNode(false) as Element
    element.append(content)
    trimAtSplit(element, edge)
    return hasContent(element) ? element : null
  }
  return [half(before, "end"), half(after, "start")]
}

/**
 * The elements that replace a top-level paragraph, with ProseMirror's slice
 * marker of the editor's own clipboard on the first of them only: the
 * marker sits on the first element of the clipboard HTML, and a paragraph
 * that holds a pasted image or embed can be that element.
 *
 * @param replaced - the paragraph being replaced.
 * @param parts - what replaces it, in order; at least one element.
 * @returns `parts`, the marker moved onto the first.
 */
function keepSliceMarker(replaced: Element, parts: Element[]): Element[] {
  const marker = replaced.getAttribute(SLICE_MARKER)
  for (const part of parts) {
    part.removeAttribute(SLICE_MARKER)
  }
  if (marker !== null && parts.length > 0) {
    parts[0].setAttribute(SLICE_MARKER, marker)
  }
  return parts
}

/**
 * Places every image and embed below `root` (editor.md §5.2 step 11a).
 *
 * @param root - the clipboard document's body, after step 10; changed in
 *   place.
 * @param context - the paste; its report receives every move.
 */
export function placeMedia(root: Element, context: PasteContext): void {
  for (const media of [...root.querySelectorAll("img, iframe")]) {
    const kind = mediaKind(media, context)
    if (kind === "youtube") {
      media.setAttribute("title", youtubeTitle(media.getAttribute("title")))
    }
    const top = topLevelBlock(root, media)
    if (top === null) {
      continue
    }
    if (top === media) {
      // Loose in the body: wrapped where it stands. `mediaParagraph` moves
      // the media into the new paragraph, so its place is kept first.
      const next = media.nextSibling
      root.insertBefore(mediaParagraph(media, kind), next)
      continue
    }
    const container = containerOf(root, media)
    if (container === null && top.localName === "p") {
      if (holdsOnly(top, media)) {
        top.replaceWith(...keepSliceMarker(top, [mediaParagraph(media, kind)]))
        continue
      }
      const [before, after] = splitAround(top, media)
      top.replaceWith(
        ...keepSliceMarker(
          top,
          [before, mediaParagraph(media, kind), after].filter(
            (part) => part !== null
          )
        )
      )
      // Media alone in inline wrappers leaves both halves empty: it was not
      // in running text, and nothing moved.
      if (before !== null || after !== null) {
        context.report.moved.push({ media: kind, from: "text" })
      }
      continue
    }
    const parent = media.parentElement
    top.before(mediaParagraph(media, kind))
    removeEmptyWrappers(parent, top)
    if (TEXTBLOCKS.has(top.localName) && !hasContent(top)) {
      top.remove()
    }
    context.report.moved.push({
      media: kind,
      from: container?.localName ?? top.localName,
    })
  }
}
