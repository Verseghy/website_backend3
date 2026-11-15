/**
 * Test support for the site schema (`src/rich-text/site-extensions.ts`).
 *
 * - {@link siteSchema} builds the ProseMirror schema of `siteExtensions`.
 * - {@link parseHtml} parses HTML with the schema's parse rules, as stage 3
 *   of the pipeline does (editor.md §2).
 * - {@link renderedElements} lists what the schema's own render rules
 *   (`toDOM`) write for a document, and {@link specElements} for one render
 *   rule: every element with its attributes as written, read from the DOM
 *   output specs, so no DOM implementation re-spells a style on the way.
 * - {@link renderHtml} renders a document to DOM with `style` written
 *   verbatim, through the stored-HTML serialiser's `verbatimDocument`
 *   (editor.md §6 step 1), without its post-pass, so tests see what the
 *   schema itself renders; the stored HTML proper is `toStoredHtml`.
 */
import { getSchema } from "@tiptap/core"
import type {
  DOMOutputSpec,
  Mark,
  Node as ProseMirrorNode,
  Schema,
} from "@tiptap/pm/model"
import { DOMParser, DOMSerializer } from "@tiptap/pm/model"

import type { SiteExtensionsOptions } from "@/rich-text/site-extensions"
import { siteExtensions } from "@/rich-text/site-extensions"
import { CONTENT_V1_POLICY } from "@/rich-text/policy"
import { verbatimDocument } from "@/rich-text/serialize/to-stored-html"

/**
 * The ProseMirror schema of the site extensions.
 *
 * @param options - overrides of the default options (the committed policy,
 *   no notices).
 */
export function siteSchema(
  options: Partial<SiteExtensionsOptions> = {}
): Schema {
  return getSchema(siteExtensions({ policy: CONTENT_V1_POLICY, ...options }))
}

/**
 * Parses HTML into a document with the schema's parse rules.
 *
 * @param schema - the schema.
 * @param html - the HTML of a body.
 */
export function parseHtml(schema: Schema, html: string): ProseMirrorNode {
  const container = document.createElement("div")
  container.innerHTML = html
  return DOMParser.fromSchema(schema).parse(container)
}

/** One element a render rule writes. */
export interface RenderedElement {
  /** The tag name, lower case. */
  readonly tag: string
  /** The attributes as written, `null` values left out. */
  readonly attributes: Readonly<Record<string, string>>
  /** The node or mark type that wrote it. */
  readonly writtenBy: string
}

/**
 * Collects the elements of one DOM output spec, recursively.
 *
 * @param spec - what a `toDOM` returned.
 * @param writtenBy - the node or mark type that returned it.
 * @param into - the list the elements are appended to.
 */
export function specElements(
  spec: DOMOutputSpec,
  writtenBy: string,
  into: RenderedElement[]
): void {
  if (typeof spec === "string" || !Array.isArray(spec)) {
    // A text node, or a DOM node a render rule built itself; the site
    // schema has neither, and the test that reads this fails on an unknown
    // tag if one appears.
    if (typeof spec !== "string") {
      into.push({ tag: "#dom-node", attributes: {}, writtenBy })
    }
    return
  }
  const [tag, ...rest] = spec as readonly [string, ...unknown[]]
  let children = rest
  const attributes: Record<string, string> = {}
  const first = rest[0]
  if (
    first !== null &&
    typeof first === "object" &&
    !Array.isArray(first) &&
    (first as { nodeType?: unknown }).nodeType === undefined
  ) {
    for (const [name, value] of Object.entries(first)) {
      if (value !== null && value !== undefined) {
        attributes[name] = String(value)
      }
    }
    children = rest.slice(1)
  }
  into.push({ tag: tag.toLowerCase(), attributes, writtenBy })
  for (const child of children) {
    if (child !== 0) {
      specElements(child as DOMOutputSpec, writtenBy, into)
    }
  }
}

/**
 * The elements the schema's render rules write for a document: one entry
 * per node (except the document and text) and per mark on each text or
 * inline node.
 *
 * @param doc - the document.
 */
export function renderedElements(doc: ProseMirrorNode): RenderedElement[] {
  const into: RenderedElement[] = []
  const renderMark = (mark: Mark) => {
    const toDOM = mark.type.spec.toDOM
    if (toDOM) {
      specElements(toDOM(mark, true), mark.type.name, into)
    }
  }
  doc.descendants((node) => {
    const toDOM = node.type.spec.toDOM
    if (!node.isText && toDOM) {
      specElements(toDOM(node), node.type.name, into)
    }
    node.marks.forEach(renderMark)
    return true
  })
  return into
}

/**
 * Renders a document's content into a `div`, with styles verbatim.
 *
 * @param schema - the document's schema.
 * @param doc - the document.
 */
export function renderHtml(schema: Schema, doc: ProseMirrorNode): HTMLElement {
  const container = document.createElement("div")
  container.appendChild(
    DOMSerializer.fromSchema(schema).serializeFragment(doc.content, {
      document: verbatimDocument(document),
    })
  )
  return container
}

/**
 * A `paste` event with clipboard data, for `view.pasteText` and
 * `view.pasteHTML`: jsdom has neither `ClipboardEvent` nor `DataTransfer`,
 * which prosemirror-view would otherwise construct.
 *
 * @param data - the clipboard formats, by MIME type.
 */
export function pasteEvent(
  data: Readonly<Record<string, string>>
): ClipboardEvent {
  return Object.assign(new Event("paste"), {
    clipboardData: {
      types: Object.keys(data),
      files: [],
      getData: (format: string) => data[format] ?? "",
    },
  }) as unknown as ClipboardEvent
}
