/**
 * `toStoredHtml`: the stored-HTML serialiser (editor.md §6, ADR 0013 §2),
 * the only way the editor turns a document into the string it saves.
 *
 * It never uses `editor.getHTML()`, which returns the engine's CSSOM
 * re-serialisation (`rgb()` colours, grouped `border-width`) and differs
 * between browsers and jsdom. Instead, in three steps:
 *
 * 1. **Verbatim DOM serialiser.** The whitespace rule of §3.8 is applied to
 *    the document's textblocks, which also drops U+0000 (`whitespace.ts`),
 *    then prosemirror-model's `DOMSerializer` renders it with a document
 *    whose elements have no `style` property ({@link verbatimDocument}),
 *    in an inert document ({@link inertDocument}), and the result is copied
 *    into a plain tree (`stored-tree.ts`).
 * 2. **Post-pass** (`post-pass.ts`): canonical styles, bare spans and equal
 *    neighbours, tightening, attribute order.
 * 3. **Writer** (`write-html.ts`): html5ever's dialect, so the server writes
 *    the same bytes again (§3.8).
 *
 * The result is a fixed point: for every document `d`,
 * `toStoredHtml(load(toStoredHtml(d))) == toStoredHtml(d)`, where `load` is
 * `loadDocument` (editor.md §6 P-fix, §13.4).
 */
import type { Node as ProseMirrorNode } from "@tiptap/pm/model"
import { DOMSerializer } from "@tiptap/pm/model"

import type { RichTextPolicy } from "../policy"
import { postPass } from "./post-pass"
import { treeFromDom } from "./stored-tree"
import { normalizeWhitespace } from "./whitespace"
import { writeHtml } from "./write-html"

/** What {@link toStoredHtml} needs besides the document. */
export interface StoredHtmlOptions {
  /** The content-v1 policy: pass `CONTENT_V1_POLICY` (editor.md §1.2 D6). */
  readonly policy: RichTextPolicy
  /**
   * A DOM document: the editor's own (`editor.view.dom.ownerDocument`), or
   * jsdom's under Node. Nothing is created in it: the rendered nodes are
   * created in the inert document that {@link inertDocument} makes from it
   * once, so a save never fetches an image.
   */
  readonly document: Document
}

/** The inert document made from each live one, created once and reused. */
const INERT_DOCUMENTS = new WeakMap<Document, Document>()

/**
 * An inert document made from `document`: created on the first call with
 * `implementation.createHTMLDocument("")` and returned again on every later
 * call for the same document.
 *
 * The serialiser renders into it rather than into the page's document
 * (editor.md §6 step 1). In a browser, an `img` of the page's document
 * starts fetching as soon as its `src` is set, attached or not, so every
 * throttled save (§4.5) would request the body's images again. A document
 * made by `createHTMLDocument` has no browsing context, so nothing created
 * in it loads anything; prosemirror-view parses pasted HTML in such a
 * document for the same reason (prosemirror-view@1.42.7:src/clipboard.ts,
 * `detachedDoc`). Creating it once also keeps each save cheap.
 *
 * @param document - a DOM document, live or inert.
 * @returns the inert document for it.
 */
export function inertDocument(document: Document): Document {
  let inert = INERT_DOCUMENTS.get(document)
  if (inert === undefined) {
    inert = document.implementation.createHTMLDocument("")
    INERT_DOCUMENTS.set(document, inert)
  }
  return inert
}

/**
 * Hides an element's own `style` property, so prosemirror-model writes the
 * `style` attribute with `setAttribute`, verbatim.
 */
function withoutStyleProperty<T extends Element>(created: T): T {
  Object.defineProperty(created, "style", {
    value: undefined,
    configurable: true,
  })
  return created
}

/**
 * A document whose elements have no `style` property (editor.md §6 step 1).
 *
 * prosemirror-model 1.25.12's `renderSpec` assigns a `style` attribute
 * through the CSSOM (`dom.style.cssText = value`) when the element has a
 * `style` property, and with `setAttribute` otherwise
 * (prosemirror-model@1.25.12:src/to_dom.ts). The CSSOM would re-spell the
 * value in the engine's own way, so every element this document creates
 * has that property shadowed with `undefined`. This relies on a
 * prosemirror-model internal: the version is pinned (editor.md §4.1), and
 * `to-stored-html.test.ts` fails if the branch changes.
 *
 * @param document - the DOM document that creates the nodes.
 * @returns a stand-in with the four factory methods prosemirror-model uses.
 */
export function verbatimDocument(document: Document): Document {
  return {
    createElement: (tag: string) =>
      withoutStyleProperty(document.createElement(tag)),
    createElementNS: (namespace: string | null, tag: string) =>
      withoutStyleProperty(document.createElementNS(namespace, tag)),
    createTextNode: (data: string) => document.createTextNode(data),
    createDocumentFragment: () => document.createDocumentFragment(),
  } as unknown as Document
}

/**
 * The stored HTML of a document (editor.md §6).
 *
 * @param doc - the editor's document, built with `siteExtensions`.
 * @param options - the policy and the DOM document; see
 *   {@link StoredHtmlOptions}.
 * @returns content-v1 HTML in html5ever's dialect. A document of one empty
 *   paragraph is `<p></p>`, as the server stores it, not `""`.
 * @throws {TypeError} when `doc` is not a ProseMirror document, an option
 *   is missing, or `options.document` cannot create an inert document, with
 *   what to pass instead.
 */
export function toStoredHtml(
  doc: ProseMirrorNode,
  options: StoredHtmlOptions
): string {
  if (
    typeof doc !== "object" ||
    doc === null ||
    typeof doc.type?.schema !== "object" ||
    doc.type !== doc.type.schema.topNodeType
  ) {
    throw new TypeError(
      "toStoredHtml: doc must be the editor's document (editor.state.doc), the top node of its schema."
    )
  }
  if (typeof options !== "object" || options === null || !options.policy) {
    throw new TypeError(
      "toStoredHtml: options.policy is required; pass CONTENT_V1_POLICY from admin/src/rich-text/policy.ts."
    )
  }
  if (
    typeof options.document?.implementation?.createHTMLDocument !== "function"
  ) {
    throw new TypeError(
      "toStoredHtml: options.document must be a DOM document, such as editor.view.dom.ownerDocument."
    )
  }
  const rendered = DOMSerializer.fromSchema(doc.type.schema).serializeFragment(
    normalizeWhitespace(doc).content,
    { document: verbatimDocument(inertDocument(options.document)) }
  )
  return writeHtml(postPass(treeFromDom(rendered.childNodes), options.policy))
}
