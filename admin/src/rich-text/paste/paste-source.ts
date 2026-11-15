/**
 * Step 1 of the paste filter (editor.md §5.2): which program wrote the
 * clipboard HTML.
 *
 * The filter promises fidelity only for Windows desktop Word and Excel
 * pasted into Chrome, Edge or Firefox (research: DECISIONS 15e, ADR 0013
 * §3), and the editor's own clipboard gets rules of its own (15f, §5.3).
 * Every other source takes the generic path (§5.5). The markers are the
 * ones editor.md §5.2 step 1 names:
 *
 * - **Word desktop**: the root element's `xmlns:w` is Word's namespace, or a
 *   `<meta name=ProgId content=Word.Document>` is present. Safari removes
 *   every `<meta>` from Office HTML but keeps the namespace (research: editor
 *   design, clipboard §5.3), so either marker is enough.
 * - **Excel desktop**: the root element's `xmlns:x` is Excel's namespace.
 *   Excel for the web writes the `ProgId` of Excel without the namespace,
 *   and takes the generic path.
 * - **The editor's own clipboard**: an element carries ProseMirror's slice
 *   marker, `data-pm-slice`.
 * - **Generic**: anything else.
 *
 * The markers are read from the parsed document, never from the raw string
 * with a regular expression, so a namespace URN that appears in a pasted
 * page's text or in an attribute value cannot select the Office rules. The
 * HTML parser moves the attributes of a late `<html>` start tag onto the
 * root element, so the namespaces are found whatever precedes the tag.
 * macOS Office writes the same dialect as Windows, so its HTML may be
 * detected as Word or Excel desktop; that is best effort (§5.2 step 1).
 */
import { asciiLowerCase } from "../css/declarations"

/** The sources the paste filter tells apart, in the order step 1 checks them. */
export const PASTE_SOURCES = ["word", "excel", "internal", "generic"] as const

/** One source of pasted HTML. */
export type PasteSource = (typeof PASTE_SOURCES)[number]

/** Word's XML namespace, the value of `xmlns:w` on Word's root element. */
export const WORD_NAMESPACE = "urn:schemas-microsoft-com:office:word"

/** Excel's XML namespace, the value of `xmlns:x` on Excel's root element. */
export const EXCEL_NAMESPACE = "urn:schemas-microsoft-com:office:excel"

/** The `ProgId` that Word desktop writes into a `<meta>`, lower case. */
const WORD_PROG_ID = "word.document"

/**
 * The attribute ProseMirror writes on the first element of its own
 * clipboard HTML (prosemirror-view 1.42.7, `serializeForClipboard`). It
 * also tells prosemirror-view how far the pasted slice is open, so the
 * filter keeps it on the editor's own HTML (editor.md §5.2).
 */
export const SLICE_MARKER = "data-pm-slice"

/**
 * The value of {@link SLICE_MARKER} as prosemirror-view 1.42.7 writes it:
 * the slice's open depths at its start and end, the number of wrappers it
 * added around table parts (`-N`), and the JSON context, `[name, attrs,
 * …]` of the nodes the copied range sat in (`serializeForClipboard`;
 * `parseFromClipboard` reads it with the same shape). The numbers are
 * bounded to three digits: a content-v1 document is never that deep.
 */
const SLICE_MARKER_VALUE =
  /^([0-9]{1,3}) ([0-9]{1,3})(?: -([0-9]{1,3}))? (.*)$/s

/** A node type's name as a schema writes it: an identifier. */
const NODE_NAME = /^[A-Za-z][A-Za-z0-9_]*$/

/**
 * The slice marker the filter keeps on the editor's own clipboard
 * (editor.md §5.5, "As built"): its open depths and wrapper count, with an
 * empty context.
 *
 * prosemirror-view reads the marker to know how far the pasted slice is
 * open, and wraps the parsed slice in the context's nodes, built with
 * `NodeType.create(attrs, …)` (`addContext`), outside the schema's parse
 * rules and the paste filter. Any web page can write the attribute, so the
 * context could hand those nodes attribute values that §3.2 never allows.
 * And a context that does not match the slice it travels with makes the
 * paste throw in prosemirror-transform's fitting (`closeNodeStart`,
 * `closeFragment`): `<p data-pm-slice='1 1 ["blockquote",null]'>a</p>b`
 * does, and so could the editor's own clipboard once step 11a has split a
 * paragraph around an embed. So the filter keeps the context out. The
 * copied nodes still carry their own attributes in the HTML; the context
 * only re-creates the nodes the copied range sat in (a list, a quote, a
 * table), where the paste cannot place the slice without them.
 *
 * @param raw - the attribute's value as the clipboard holds it.
 * @returns the value to keep, or `null` for one prosemirror-view does not
 *   write (a context that is no JSON array of node names and attributes),
 *   which then leaves the paste as an outside one's slice.
 */
export function keptSliceMarker(raw: string): string | null {
  const match = SLICE_MARKER_VALUE.exec(raw)
  if (match === null) {
    return null
  }
  let context: unknown
  try {
    context = JSON.parse(match[4])
  } catch {
    return null
  }
  if (
    !Array.isArray(context) ||
    context.length % 2 !== 0 ||
    context.some(
      (entry, index) =>
        index % 2 === 0 && (typeof entry !== "string" || !NODE_NAME.test(entry))
    )
  ) {
    return null
  }
  const wrappers = match[3] === undefined ? "" : ` -${Number(match[3])}`
  return `${Number(match[1])} ${Number(match[2])}${wrappers} []`
}

/** Trims ASCII whitespace and lower-cases ASCII letters, as HTML compares keywords. */
function keyword(value: string | null): string {
  return asciiLowerCase(
    (value ?? "").replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, "")
  )
}

/**
 * Detects the program that wrote pasted HTML (editor.md §5.2 step 1).
 *
 * @param document - the clipboard HTML, parsed whole by an inert
 *   `DOMParser` (step 3); it is only read.
 * @returns `word` or `excel` for Office desktop HTML, `internal` for the
 *   editor's own clipboard, and `generic` for every other source.
 */
export function detectPasteSource(document: Document): PasteSource {
  const root = document.documentElement
  const namespace = (prefix: string) =>
    root === null ? "" : keyword(root.getAttribute(`xmlns:${prefix}`))
  const wordProgId = [...document.querySelectorAll("meta")].some(
    (meta) =>
      keyword(meta.getAttribute("name")) === "progid" &&
      keyword(meta.getAttribute("content")) === WORD_PROG_ID
  )
  if (namespace("w") === WORD_NAMESPACE || wordProgId) {
    return "word"
  }
  if (namespace("x") === EXCEL_NAMESPACE) {
    return "excel"
  }
  if (document.querySelector(`[${SLICE_MARKER}]`) !== null) {
    return "internal"
  }
  return "generic"
}
