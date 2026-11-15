/**
 * The tree that the stored-HTML serialiser's post-pass and writer work on
 * (editor.md §6 steps 2 and 3).
 *
 * Step 1 renders the document to DOM nodes; this module copies them into
 * plain objects at once: tag, attributes in their order, children. From
 * then on no DOM object is touched, so no engine's CSSOM, attribute
 * normalisation or `innerHTML` can reach the stored bytes, and the post-pass
 * and the writer are pure functions that tests can call on trees written by
 * hand.
 */

/** An element of the stored tree. */
export interface StoredElement {
  readonly kind: "element"
  /** The tag name, lower case. */
  readonly tag: string
  /** The attributes, as `[name, value]` pairs in the order they are written. */
  readonly attributes: readonly (readonly [string, string])[]
  /** The child nodes. */
  readonly children: readonly StoredNode[]
}

/** A text node of the stored tree. */
export interface StoredText {
  readonly kind: "text"
  /** The text, unescaped. */
  readonly text: string
}

/** A node of the stored tree. */
export type StoredNode = StoredElement | StoredText

/**
 * An element of the stored tree.
 *
 * @param tag - the tag name, lower case.
 * @param attributes - the attributes, in order.
 * @param children - the child nodes.
 */
export function element(
  tag: string,
  attributes: readonly (readonly [string, string])[] = [],
  children: readonly StoredNode[] = []
): StoredElement {
  return { kind: "element", tag, attributes, children }
}

/**
 * A text node of the stored tree.
 *
 * @param text - the text, unescaped.
 */
export function text(text: string): StoredText {
  return { kind: "text", text }
}

/**
 * The value of an attribute of an element.
 *
 * @param node - the element.
 * @param name - the attribute name.
 * @returns the value, or `null` when the element does not have it.
 */
export function attributeOf(node: StoredElement, name: string): string | null {
  return node.attributes.find(([key]) => key === name)?.[1] ?? null
}

/** `Node.ELEMENT_NODE`, `Node.TEXT_NODE` and `Node.DOCUMENT_FRAGMENT_NODE`. */
const ELEMENT_NODE = 1
const TEXT_NODE = 3
const DOCUMENT_FRAGMENT_NODE = 11

/**
 * Copies DOM nodes into the stored tree.
 *
 * Elements keep their tag (`localName`, lower case for HTML elements) and
 * their attributes in DOM order, read with `getAttribute`-equivalent
 * `Attr.value`, never through `element.style`. Text nodes keep their data.
 * The children of a document fragment are taken in its place. Every other
 * node (a comment, a processing instruction) is left out: the schema's
 * render rules make none, and content-v1 stores none (editor.md §3.8).
 *
 * @param nodes - the DOM nodes, in order.
 * @returns the stored tree.
 */
export function treeFromDom(nodes: Iterable<Node>): StoredNode[] {
  const tree: StoredNode[] = []
  for (const node of nodes) {
    if (node.nodeType === TEXT_NODE) {
      tree.push(text((node as Text).data))
    } else if (node.nodeType === ELEMENT_NODE) {
      const dom = node as Element
      tree.push(
        element(
          dom.localName,
          Array.from(dom.attributes, (attribute) => [
            attribute.name,
            attribute.value,
          ]),
          treeFromDom(dom.childNodes)
        )
      )
    } else if (node.nodeType === DOCUMENT_FRAGMENT_NODE) {
      tree.push(...treeFromDom(node.childNodes))
    }
  }
  return tree
}
