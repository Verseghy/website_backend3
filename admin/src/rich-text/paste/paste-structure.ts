/**
 * Step 10 of the paste filter (editor.md §5.2), with the declarations of
 * step 8 and the inline marks it rebuilds: Office and web structure is
 * removed, and what remains is content-v1's elements with content-v1's
 * attributes only (P-paste, §6).
 *
 * It runs in three passes over the clipboard document's body:
 *
 * 1. **Removed with their content**: comments (Word's conditional comments,
 *    the VML fallbacks and Excel's `StartFragment` markers among them),
 *    `head`, `meta`, `link` and `base`, Word's `v:*`, `w:*` and `m:*`
 *    elements, and the elements the server removes with their content: the
 *    first row of §3.5 and ammonia's namespace check (`policy.json`'s
 *    `removed_with_content` and `removed_by_namespace_check`, §3.9), so the
 *    filter keeps no text that the server would drop.
 * 2. **Tables**: `thead` and `tfoot` rows move into one `tbody` (`thead`
 *    first, `tfoot` last, as a browser shows them); `colgroup` and `col`
 *    widths move to the first-row cells that have none, span- and
 *    colspan-aware; a `tr` height moves to its first cell; a `caption`
 *    becomes a paragraph before its table; a table without rows goes.
 * 3. **Elements, children before parents**:
 *    - `h5` and `h6` become `h4`, as the import maps them (data-migration.md
 *      N5); `menu` and `dir` become `ul`;
 *    - the inline formatting elements become content-v1 marks
 *      (`inline-marks.ts`);
 *    - a link keeps an `href` the server keeps (§3.6) and is unwrapped
 *      otherwise, a Word bookmark (`a[name]` without `href`) included; the
 *      editor's own clipboard keeps its links as written;
 *    - an `img` keeps a `src` the server keeps, written as the server
 *      stores it, and is removed otherwise. Which pictures a paste may keep
 *      at all is the picture rule of §5.3, which joins the filter in its own
 *      slice;
 *    - an `iframe` keeps a YouTube or Padlet embed address, written in the
 *      node's canonical form: YouTube on `www.youtube-nocookie.com`, `http`
 *      and `listType=playlist` mapped and every other parameter dropped
 *      (ADR 0043, through `youtube-input.ts`), the dropped playback options
 *      reported (D134); any other `iframe` is removed and counted;
 *    - every other content-v1 element keeps the attributes of its §3.2 row
 *      whose values the server keeps, and its step-8 `style`. Forced values
 *      (`target`, `rel`, `allow`, `allowfullscreen`, `referrerpolicy`) are
 *      left to the schema, which writes them. The editor's own clipboard
 *      also keeps ProseMirror's `data-pm-slice`, which tells prosemirror-view
 *      how far the slice is open and which no parse rule reads, with its
 *      context's attributes left out (`keptSliceMarker`);
 *    - block containers outside content-v1 (`div`, `center`, `section`,
 *      `dl` and the rest of {@link BLOCK_CONTAINERS}) are unwrapped, and each
 *      run of inline content in them becomes a paragraph, so `<div>a</div>
 *      <div>b</div>` stays two paragraphs. A `center` gives its paragraphs
 *      and headings `text-align: center` (§5.2 step 10);
 *    - every other element (`o:p`, `font` without marks, `abbr`, `code`,
 *      Word's smart tags, …) is unwrapped.
 *
 * Finally a cell whose only child is one paragraph takes that paragraph's
 * `text-align` (§3.7, "Tightening").
 *
 * The embed wrappers of other TipTap installs (`div[data-youtube-video]`)
 * are unwrapped like any `div`; step 11a puts every embed into the
 * canonical wrapper again.
 */
import { canonicalStyle } from "../css/canonical-style"
import { isAllowedHref } from "../link-href"
import { embedSrc, imageSrc } from "../media-src"
import { padletEmbedSrc, readPadletAddress } from "../padlet-input"
import type { AttributeValueRule, RichTextPolicy } from "../policy"
import { elementOf } from "../policy"
import { readStoredDeclarations } from "../style-decls"
import { readYoutubeEmbedSrc, youtubeEmbedSrc } from "../youtube-input"
import { readIntegerAttribute } from "../extensions/site-lists"
import type { DeclarationReader } from "./inline-marks"
import {
  FORMATTING_TAGS,
  ownMarks,
  replaceWithMarks,
  spanStyle,
} from "./inline-marks"
import type { PasteContext } from "./paste-context"
import {
  listTypeFromStyle,
  pastedStyle,
  writeDeclarations,
} from "./paste-declarations"
import { keptSliceMarker, SLICE_MARKER } from "./paste-source"

/** Elements removed with their content besides the policy's lists. */
const HEAD_ONLY = ["head", "meta", "link", "base"] as const

/** Word's prefixed elements that are removed with their content. */
const OFFICE_PREFIXES = ["v:", "w:", "m:"] as const

/** Elements renamed to their content-v1 counterpart. */
export const RENAMED: Readonly<Record<string, string>> = {
  h5: "h4",
  h6: "h4",
  menu: "ul",
  dir: "ul",
}

/**
 * Block containers outside content-v1: unwrapped, each run of inline
 * content in them becoming a paragraph.
 */
export const BLOCK_CONTAINERS: ReadonlySet<string> = new Set([
  "div",
  "center",
  "section",
  "article",
  "aside",
  "header",
  "footer",
  "main",
  "nav",
  "address",
  "figure",
  "figcaption",
  "details",
  "summary",
  "dl",
  "dt",
  "dd",
  "pre",
  "fieldset",
  "legend",
  "form",
  "hgroup",
  "search",
])

/** The content-v1 blocks that end a run of inline content. */
const BLOCKS: ReadonlySet<string> = new Set([
  "p",
  "h1",
  "h2",
  "h3",
  "h4",
  "ul",
  "ol",
  "li",
  "table",
  "blockquote",
  "hr",
])

/** The textblocks that a `center` aligns. */
const TEXTBLOCKS: ReadonlySet<string> = new Set(["p", "h1", "h2", "h3", "h4"])

/** The node types this module tells apart. */
const ELEMENT_NODE = 1
const TEXT_NODE = 3
const PROCESSING_INSTRUCTION_NODE = 7
const COMMENT_NODE = 8

/** ASCII whitespace, which HTML collapses and ProseMirror drops at block edges. */
const ASCII_BLANK = /^[\t\n\f\r ]*$/

/** Trims ASCII whitespace, as HTML trims attribute values. */
function trimAscii(value: string): string {
  return value.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, "")
}

/**
 * Whether a node shows anything: text other than ASCII whitespace, a hard
 * break, an image or an embed.
 *
 * @param node - a node of the clipboard document.
 */
export function hasContent(node: Node): boolean {
  if (node.nodeType === TEXT_NODE) {
    return !ASCII_BLANK.test(node.textContent ?? "")
  }
  if (node.nodeType !== ELEMENT_NODE) {
    return false
  }
  const element = node as Element
  return (
    ["br", "img", "iframe", "hr"].includes(element.localName) ||
    !ASCII_BLANK.test(element.textContent ?? "") ||
    element.querySelector("br, img, iframe, hr") !== null
  )
}

/** Moves an element's children into its place. */
function unwrap(element: Element): void {
  element.replaceWith(...[...element.childNodes])
}

/** Sets or removes an element's `style`. */
function setStyle(element: Element, style: string | null): void {
  if (style === null) {
    element.removeAttribute("style")
  } else {
    element.setAttribute("style", style)
  }
}

/** Puts a declaration in front of an element's own, which keep winning. */
function prependDeclaration(element: Element, declaration: string): void {
  const own = element.getAttribute("style") ?? ""
  element.setAttribute(
    "style",
    trimAscii(own) === "" ? declaration : `${declaration}; ${own}`
  )
}

/** The cascaded declarations of an element, through its CSSOM. */
function cssomReader(element: Element): DeclarationReader {
  const style = (element as Partial<HTMLElement>).style
  return (property) => style?.getPropertyValue(property) ?? ""
}

/**
 * Pass 1: removes comments and the elements removed with their content.
 *
 * @param root - the clipboard document's body.
 * @param policy - the content-v1 policy, for the server's removal lists.
 */
export function removeWithContent(root: Element, policy: RichTextPolicy): void {
  const removed = new Set<string>([
    ...policy.removed_with_content,
    ...policy.removed_by_namespace_check,
    ...HEAD_ONLY,
  ])
  const pending: Node[] = [...root.childNodes]
  while (pending.length > 0) {
    const node = pending.pop()!
    if (
      node.nodeType === COMMENT_NODE ||
      node.nodeType === PROCESSING_INSTRUCTION_NODE
    ) {
      node.parentNode?.removeChild(node)
      continue
    }
    if (node.nodeType !== ELEMENT_NODE) {
      continue
    }
    const element = node as Element
    const tag = element.localName
    if (
      removed.has(tag) ||
      OFFICE_PREFIXES.some((prefix) => tag.startsWith(prefix))
    ) {
      element.remove()
    } else {
      pending.push(...element.childNodes)
    }
  }
}

/**
 * The most columns whose widths the table pass reads: a `col` may span
 * 1000 columns, and a pasted table cannot make the filter build more.
 */
const MAX_COLUMNS = 10_000

/** A span or column count as HTML reads it: leading digits, 1 to 1000. */
function spanOf(value: string | null): number {
  const match = /^[0-9]+/.exec(trimAscii(value ?? ""))
  const number = match === null ? 1 : Number(match[0])
  return Math.min(Math.max(number, 1), 1000)
}

/**
 * A cell-like element's width or height in whole px, from its style (the
 * CSSOM) or its attribute, as a cell would store it; `null` when it has
 * none, or one that a cell does not store (a percentage).
 */
function lengthPx(
  policy: RichTextPolicy,
  element: Element,
  property: "width" | "height"
): number | null {
  const styled = cssomReader(element)(property).trim()
  const attribute = trimAscii(element.getAttribute(property) ?? "")
  const value =
    styled !== ""
      ? styled
      : /^[0-9]+(?:\.[0-9]+)?$/.test(attribute)
        ? `${attribute}px`
        : ""
  const stored = readStoredDeclarations(
    policy,
    "td",
    value === "" ? null : `${property}: ${value}`
  ).get(property)
  const match = /^([0-9.]+)px$/.exec(stored ?? "")
  return match === null ? null : Math.round(Number(match[1]))
}

/** Whether a cell declares its own width or height, in its style or as a hint. */
function hasOwnLength(element: Element, property: "width" | "height"): boolean {
  return (
    cssomReader(element)(property).trim() !== "" ||
    element.hasAttribute(property)
  )
}

/** The `col` elements a `colgroup` stands for: its `col`s, or itself. */
function columnsOf(colgroup: Element): Element[] {
  const cols = [...colgroup.children].filter((col) => col.localName === "col")
  return cols.length > 0 ? cols : [colgroup]
}

/** The rows of a table section. */
function rowsOf(section: Element): Element[] {
  return [...section.children].filter((row) => row.localName === "tr")
}

/** The cells of a row. */
function cellsOf(row: Element): Element[] {
  return [...row.children].filter(
    (cell) => cell.localName === "td" || cell.localName === "th"
  )
}

/**
 * Moves the widths of a table's columns to its first-row cells that have
 * no width of their own; a cell that spans columns gets their sum, when
 * every one of them has a width.
 */
function moveColumnWidths(
  policy: RichTextPolicy,
  columns: readonly Element[],
  firstRow: Element
): void {
  const widths: (number | null)[] = []
  for (const column of columns) {
    const width = lengthPx(policy, column, "width")
    let count = spanOf(column.getAttribute("span"))
    for (; count > 0 && widths.length < MAX_COLUMNS; count -= 1) {
      widths.push(width)
    }
  }
  let index = 0
  for (const cell of cellsOf(firstRow)) {
    const span = spanOf(cell.getAttribute("colspan"))
    const covered = widths.slice(index, index + span)
    index += span
    if (
      covered.length === span &&
      covered.every((width) => width !== null) &&
      !hasOwnLength(cell, "width")
    ) {
      const sum = (covered as number[]).reduce(
        (total, width) => total + width,
        0
      )
      prependDeclaration(cell, `width: ${sum}px`)
    }
  }
}

/**
 * Pass 2, for one table: one `tbody` with every row, the column widths on
 * the first row's cells, each row's height on its first cell, the caption
 * as a paragraph before the table.
 *
 * @param table - the table; it is removed when it has no row.
 * @param policy - the content-v1 policy, for the cell length rules.
 */
export function normalizeTable(table: Element, policy: RichTextPolicy): void {
  const document = table.ownerDocument
  const head: Element[] = []
  const body: Element[] = []
  const foot: Element[] = []
  const columns: Element[] = []
  for (const child of [...table.children]) {
    switch (child.localName) {
      case "thead":
        head.push(...rowsOf(child))
        break
      case "tfoot":
        foot.push(...rowsOf(child))
        break
      case "tbody":
        body.push(...rowsOf(child))
        break
      case "tr":
        body.push(child)
        break
      case "colgroup":
        columns.push(...columnsOf(child))
        break
      case "col":
        columns.push(child)
        break
      case "caption": {
        const paragraph = document.createElement("p")
        paragraph.append(...[...child.childNodes])
        table.before(paragraph)
        break
      }
    }
  }
  const rows = [...head, ...body, ...foot]
  if (rows.length === 0) {
    table.remove()
    return
  }
  moveColumnWidths(policy, columns, rows[0])
  for (const row of rows) {
    const height = lengthPx(policy, row, "height")
    const [first] = cellsOf(row)
    if (
      height !== null &&
      first !== undefined &&
      !hasOwnLength(first, "height")
    ) {
      prependDeclaration(first, `height: ${height}px`)
    }
  }
  const tbody = document.createElement("tbody")
  tbody.append(...rows)
  table.replaceChildren(tbody)
}

/**
 * The value an attribute keeps under its §3.2 rule, or `null` when the
 * filter leaves it out: a value the server drops, the omitted default, and
 * the rules written elsewhere (`style`, the URL rules) or by the schema
 * (`forced`, `link_rel`).
 *
 * @param rule - the attribute's value rule.
 * @param raw - its value as the DOM holds it.
 */
export function attributeValue(
  rule: AttributeValueRule,
  raw: string
): string | null {
  switch (rule.kind) {
    case "text":
      return raw
    case "marker":
      return ""
    case "integer": {
      const number = readIntegerAttribute(raw, rule)
      return number === null || number === rule.omitted ? null : String(number)
    }
    case "integer_or_full": {
      if (trimAscii(raw) === rule.full) {
        return rule.full
      }
      const number = readIntegerAttribute(raw, {
        kind: "integer",
        min: rule.min,
        max: rule.max,
        omitted: null,
      })
      return number === null ? null : String(number)
    }
    case "enumerated": {
      // Enumerated values are compared as written: `a` and `A` are two
      // list types.
      const value = trimAscii(raw)
      return rule.values.includes(value) && value !== rule.omitted
        ? value
        : null
    }
    default:
      return null
  }
}

/**
 * Leaves an element with exactly the attributes of its §3.2 row that it
 * keeps, in canonical order.
 *
 * @param element - a content-v1 element.
 * @param tag - its tag.
 * @param context - the paste.
 * @param fixed - values the caller has already decided (an address).
 */
function keepAttributes(
  element: Element,
  tag: string,
  context: PasteContext,
  fixed: ReadonlyMap<string, string> = new Map()
): void {
  const kept: [string, string][] = []
  for (const { name, value: rule } of elementOf(context.policy, tag)
    ?.attributes ?? []) {
    const decided = fixed.get(name)
    if (decided !== undefined) {
      kept.push([name, decided])
    } else if (rule.kind === "style") {
      const style = pastedStyle(context, element, tag)
      if (style !== null) {
        kept.push([name, style])
      }
    } else {
      const raw = element.getAttribute(name)
      const value = raw === null ? null : attributeValue(rule, raw)
      if (value !== null) {
        kept.push([name, value])
      }
    }
  }
  const marker =
    context.source === "internal" ? element.getAttribute(SLICE_MARKER) : null
  const slice = marker === null ? null : keptSliceMarker(marker)
  for (const attribute of [...element.attributes]) {
    element.removeAttributeNode(attribute)
  }
  for (const [name, value] of kept) {
    element.setAttribute(name, value)
  }
  if (slice !== null) {
    element.setAttribute(SLICE_MARKER, slice)
  }
}

/** Gives an element another tag, with its children and its style hints. */
function rename(element: Element, tag: string): Element {
  const renamed = element.ownerDocument.createElement(tag)
  for (const name of ["style", "align", "type", "start"]) {
    const value = element.getAttribute(name)
    if (value !== null) {
      renamed.setAttribute(name, value)
    }
  }
  renamed.append(...[...element.childNodes])
  element.replaceWith(renamed)
  return renamed
}

/** Adds `text-align: center` to a paragraph or heading that has no alignment. */
function centre(element: Element, policy: RichTextPolicy): void {
  const tag = element.localName
  if (!TEXTBLOCKS.has(tag)) {
    return
  }
  const declarations = new Map(
    readStoredDeclarations(policy, tag, element.getAttribute("style"))
  )
  if (!declarations.has("text-align")) {
    declarations.set("text-align", "center")
    setStyle(element, writeDeclarations(policy, tag, declarations))
  }
}

/**
 * Unwraps a block container, turning each run of inline content in it into
 * a paragraph.
 */
function unwrapBlockContainer(
  element: Element,
  centred: boolean,
  policy: RichTextPolicy
): void {
  const document = element.ownerDocument
  const out: Node[] = []
  let run: Node[] = []
  const flush = () => {
    if (run.some(hasContent)) {
      const paragraph = document.createElement("p")
      if (centred) {
        setStyle(paragraph, canonicalStyle(policy, "p", "text-align: center"))
      }
      paragraph.append(...run)
      out.push(paragraph)
    } else {
      out.push(...run)
    }
    run = []
  }
  for (const child of [...element.childNodes]) {
    if (
      child.nodeType === ELEMENT_NODE &&
      BLOCKS.has((child as Element).localName)
    ) {
      flush()
      if (centred) {
        centre(child as Element, policy)
      }
      out.push(child)
    } else {
      run.push(child)
    }
  }
  flush()
  element.replaceWith(...out)
}

/**
 * An `iframe`'s address in the node's canonical form, or `null` for an
 * iframe the paste removes. A YouTube address the reader refuses (a wrong
 * id, a user-uploads or search list) is reported with its reason, so
 * `handlePaste` can say why the video is missing (§4.3 row 9); the dropped
 * playback options of an accepted one are reported for the D134 notice.
 *
 * @param element - the `iframe`.
 * @param context - the paste, whose report this fills in.
 */
export function frameAddress(
  element: Element,
  context: PasteContext
): string | null {
  const raw = element.getAttribute("src") ?? ""
  const youtube = readYoutubeEmbedSrc(raw, context.youtube)
  let src: string | null = null
  if (youtube.kind === "video") {
    src = youtubeEmbedSrc(youtube.video)
  } else if (youtube.kind === "refused") {
    context.report.youtubeRefusals.add(youtube.reason)
  } else {
    const key = readPadletAddress(raw, context.padlet)
    src = key === null ? null : padletEmbedSrc(key)
  }
  // The server's own rule decides in the end, so the filter never hands on
  // an address that a policy change made unstorable.
  if (src === null || embedSrc(src, context.policy.urls) === null) {
    return null
  }
  if (youtube.kind === "video") {
    // Only an embed that stays loses its options; a removed one is counted
    // in `frames` and has nothing left to play.
    for (const option of youtube.dropped) {
      context.report.embedParams.add(option)
    }
  }
  return src
}

/**
 * Pass 3, for one element (its children are done).
 *
 * @param original - the element.
 * @param context - the paste.
 */
function transformElement(original: Element, context: PasteContext): void {
  const { policy, source } = context
  let element = original
  if (Object.hasOwn(RENAMED, element.localName)) {
    element = rename(element, RENAMED[element.localName])
  }
  const tag = element.localName
  if (FORMATTING_TAGS.has(tag)) {
    const read = cssomReader(element)
    replaceWithMarks(
      element,
      ownMarks(tag, read),
      spanStyle(policy, source, element, read)
    )
    return
  }
  switch (tag) {
    case "a": {
      const href = element.getAttribute("href")
      const refused =
        href === null
          ? source !== "internal"
          : !isAllowedHref(href, policy.urls) ||
            (source !== "internal" && trimAscii(href) === "")
      if (refused) {
        unwrap(element)
      } else {
        const fixed = new Map<string, string>(
          href === null ? [] : [["href", href]]
        )
        keepAttributes(element, tag, context, fixed)
      }
      return
    }
    case "img": {
      const src = imageSrc(element.getAttribute("src") ?? "", policy.urls)
      if (src === null) {
        element.remove()
      } else {
        keepAttributes(element, tag, context, new Map([["src", src]]))
      }
      return
    }
    case "iframe": {
      const src = frameAddress(element, context)
      if (src === null) {
        element.remove()
        context.report.frames += 1
      } else {
        // An iframe is childless in content-v1 (§7.5); the HTML parser keeps
        // the text between its tags as a child.
        element.replaceChildren()
        keepAttributes(element, tag, context, new Map([["src", src]]))
      }
      return
    }
    case "ol":
      if (source !== "internal" && !element.hasAttribute("type")) {
        const type = listTypeFromStyle(element)
        if (type !== null) {
          element.setAttribute("type", type)
        }
      }
      break
  }
  if (elementOf(policy, tag) !== undefined) {
    keepAttributes(element, tag, context)
  } else if (BLOCK_CONTAINERS.has(tag)) {
    unwrapBlockContainer(element, tag === "center", policy)
  } else {
    unwrap(element)
  }
}

/**
 * Moves the `text-align` of a cell's only paragraph to the cell.
 *
 * @param root - the clipboard document's body.
 * @param policy - the content-v1 policy.
 */
export function tightenCellAlignment(
  root: Element,
  policy: RichTextPolicy
): void {
  for (const cell of [...root.querySelectorAll("td, th")]) {
    const children = [...cell.childNodes].filter(
      (node) =>
        !(
          node.nodeType === TEXT_NODE &&
          ASCII_BLANK.test(node.textContent ?? "")
        )
    )
    const [paragraph] = children
    if (
      children.length !== 1 ||
      paragraph.nodeType !== ELEMENT_NODE ||
      (paragraph as Element).localName !== "p"
    ) {
      continue
    }
    const own = new Map(
      readStoredDeclarations(
        policy,
        "p",
        (paragraph as Element).getAttribute("style")
      )
    )
    const align = own.get("text-align")
    if (align === undefined) {
      continue
    }
    own.delete("text-align")
    setStyle(paragraph as Element, writeDeclarations(policy, "p", own))
    const tag = cell.localName
    const declarations = new Map(
      readStoredDeclarations(policy, tag, cell.getAttribute("style"))
    )
    declarations.set("text-align", align)
    setStyle(cell, writeDeclarations(policy, tag, declarations))
  }
}

/**
 * Adds the `font-family` of every element under a root to the report, in
 * document order. content-v1 stores no font (§3.3), so pass 3 drops each
 * one; the report lists what was dropped (§5.2 step 12, `fontFamilies`).
 *
 * @param root - the clipboard document's body, after passes 1 and 2.
 * @param context - the paste, whose report this fills in.
 */
export function collectFontFamilies(
  root: Element,
  context: PasteContext
): void {
  for (const element of root.querySelectorAll("*")) {
    const family = cssomReader(element)("font-family").trim()
    if (family !== "") {
      context.report.fontFamilies.add(family)
    }
  }
}

/**
 * Step 10 with steps 8 and 9 as this filter has them: the three passes and
 * the cell alignment.
 *
 * **Cost.** Pass 3 rewrites an element by moving its children: into new
 * mark wrappers (`replaceWithMarks`), up into its parent (an unwrap) or
 * into new paragraphs (a block container). Moving a child moves its whole
 * subtree, and the DOM visits every node of a moved subtree (its insertion
 * and removal steps), so `n` elements nested `d` deep cost about `n × d`
 * node visits, and jsdom, which also walks to the root for each visited
 * node, about `n × d²`. A formatting element that is already the one wrapper
 * it needs (a `span` that keeps only its colour, a `strong`) is rewritten in
 * place and moves nothing. Browsers' HTML parsers, `DOMParser` included,
 * cap the nesting depth (Chromium and WebKit at 512 levels), which bounds
 * `d` for a real paste; only the pasting staff member's tab pays it.
 *
 * @param root - the clipboard document's body; changed in place.
 * @param context - the paste.
 * @throws {PolicyError} as `canonicalStyle` does for a policy it cannot read.
 */
export function normalizeStructure(root: Element, context: PasteContext): void {
  removeWithContent(root, context.policy)
  for (const table of [...root.querySelectorAll("table")]) {
    normalizeTable(table, context.policy)
  }
  collectFontFamilies(root, context)
  // Document order is pre-order, so walking it backwards visits every
  // element after its descendants: a parent sees its children done.
  const elements = [...root.querySelectorAll("*")]
  for (let index = elements.length - 1; index >= 0; index -= 1) {
    const element = elements[index]
    if (element.isConnected && root.contains(element)) {
      transformElement(element, context)
    }
  }
  tightenCellAlignment(root, context.policy)
}
