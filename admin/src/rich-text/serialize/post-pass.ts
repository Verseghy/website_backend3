/**
 * The post-pass of the stored-HTML serialiser (editor.md §6 step 2): what
 * turns the schema's rendered elements into the one stored form of
 * content-v1, so that loading and saving a body changes no byte (P-fix).
 *
 * The schema renders what a document holds, in TipTap's terms: attributes in
 * the order TipTap merges them (`target`, `rel`, `href` on a link), several
 * declarations of one `span` in the order its attributes are registered, a
 * `<p>` in every list item. A reload reads back less than that and in one
 * form only, so the post-pass writes that form:
 *
 * 1. **Canonical styles.** Every `style` goes through `canonicalStyle`
 *    (editor.md §3.1); a style with nothing left is removed (§6 steps 2.1
 *    and 2.5).
 * 2. **Bare spans and equal neighbours.** A `span` without attributes is
 *    unwrapped, because a reload makes no text-style mark of it (§6 step
 *    2.2). For the same reason two inline elements side by side with the
 *    same tag and the same attributes are merged into one: a reload reads
 *    them as one mark, so storing them apart would not be a fixed point.
 *    The schema renders such a pair only for two marks whose values differ
 *    in something it does not write, such as a colour outside the grammar.
 * 3. **Tightening** (§3.7, §6 step 2.3). A lone paragraph without attributes
 *    is stored without its `<p>` when it is the only child of a `td`, `th`,
 *    `blockquote` or `li`, or the first child of an `li` whose other
 *    children are all lists. A lone cell paragraph whose only attribute is
 *    its `text-align` is tightened too, and the alignment moves to the cell.
 *    TipTap wraps such text in a paragraph again on load. §3.7 makes one
 *    exception, after the maintainer's answer of 2026-11-15 (item D148): an
 *    empty first paragraph in front of an `li`'s lists is kept, because
 *    `<li><ul>` does not load back as the same item.
 * 4. **Attribute order** (§3.2, §6 step 2.4). Each element's attributes are
 *    written in the policy's order for the element, so the bytes never
 *    depend on how TipTap merged them; `rel` comes last on a link. An
 *    attribute the policy does not list for the element (which the schema
 *    never renders, `site-extensions.policy.test.ts`) keeps its place after
 *    the listed ones, so the server's report shows it instead of the editor
 *    hiding it.
 *
 * The whitespace rule of §3.8 (§6 step 2.6) is applied to the document
 * before it is rendered (`whitespace.ts`), where each textblock is one node
 * with its text runs and hard breaks; it changes only text, which none of
 * the steps here looks at.
 *
 * Every step is a pure function from a stored tree to a new one.
 */
import type { RichTextPolicy } from "../policy"
import { elementOf } from "../policy"
import { canonicalStyle } from "../css/canonical-style"
import type { StoredElement, StoredNode } from "./stored-tree"
import { attributeOf, element } from "./stored-tree"

/** The inline elements a mark renders, which step 2 may merge. */
export const MARK_ELEMENTS: ReadonlySet<string> = new Set([
  "a",
  "em",
  "s",
  "span",
  "strong",
  "sub",
  "sup",
  "u",
])

/** The containers in which a lone paragraph is tightened (§3.7). */
export const TIGHT_CONTAINERS: ReadonlySet<string> = new Set([
  "blockquote",
  "li",
  "td",
  "th",
])

/** The table cells, whose lone paragraph also gives them its alignment. */
const CELLS: ReadonlySet<string> = new Set(["td", "th"])

/** The list elements, after which an `li`'s first paragraph is still tight. */
const LISTS: ReadonlySet<string> = new Set(["ol", "ul"])

/** Applies `change` to every element of a tree, children first. */
function mapElements(
  nodes: readonly StoredNode[],
  change: (node: StoredElement) => StoredNode[]
): StoredNode[] {
  return nodes.flatMap((node) =>
    node.kind === "text"
      ? [node]
      : change({ ...node, children: mapElements(node.children, change) })
  )
}

/** A copy of an element with one attribute set, or removed when `value` is `null`. */
function withAttribute(
  node: StoredElement,
  name: string,
  value: string | null
): StoredElement {
  const others = node.attributes.filter(([key]) => key !== name)
  const exists = others.length !== node.attributes.length
  if (value === null) {
    return exists ? { ...node, attributes: others } : node
  }
  return {
    ...node,
    attributes: exists
      ? node.attributes.map(([key, old]) => [key, key === name ? value : old])
      : [...node.attributes, [name, value]],
  }
}

/**
 * Step 1: every `style` in its canonical spelling, and none left empty.
 *
 * @param nodes - the tree.
 * @param policy - the content-v1 policy.
 */
export function canonicalStyles(
  nodes: readonly StoredNode[],
  policy: RichTextPolicy
): StoredNode[] {
  return mapElements(nodes, (node) => {
    const style = attributeOf(node, "style")
    return [
      style === null
        ? node
        : withAttribute(node, "style", canonicalStyle(policy, node.tag, style)),
    ]
  })
}

/** Whether two attribute lists are the same, in the same order. */
function sameAttributes(a: StoredElement, b: StoredElement): boolean {
  return (
    a.attributes.length === b.attributes.length &&
    a.attributes.every(
      ([name, value], index) =>
        b.attributes[index][0] === name && b.attributes[index][1] === value
    )
  )
}

/**
 * Merges neighbouring mark elements with the same tag and attributes, and
 * neighbouring text nodes, in one list of siblings; the children of a merged
 * pair are merged the same way.
 */
function mergeSiblings(nodes: readonly StoredNode[]): StoredNode[] {
  const merged: StoredNode[] = []
  for (const node of nodes) {
    const last = merged[merged.length - 1]
    if (last?.kind === "text" && node.kind === "text") {
      merged[merged.length - 1] = { kind: "text", text: last.text + node.text }
    } else if (
      last?.kind === "element" &&
      node.kind === "element" &&
      MARK_ELEMENTS.has(node.tag) &&
      last.tag === node.tag &&
      sameAttributes(last, node)
    ) {
      merged[merged.length - 1] = {
        ...last,
        children: mergeSiblings([...last.children, ...node.children]),
      }
    } else {
      merged.push(node)
    }
  }
  return merged
}

/**
 * Step 2: bare spans unwrapped, and equal neighbouring mark elements merged.
 *
 * @param nodes - the tree, with canonical styles (step 1), so a span whose
 *   style was dropped has no attributes left.
 */
export function unwrapAndMerge(nodes: readonly StoredNode[]): StoredNode[] {
  const unwrapped = mapElements(nodes, (node) =>
    node.tag === "span" && node.attributes.length === 0
      ? [...node.children]
      : [{ ...node, children: mergeSiblings(node.children) }]
  )
  return mergeSiblings(unwrapped)
}

/** Whether a node is a paragraph element without attributes. */
const isBareParagraph = (node: StoredNode | undefined): node is StoredElement =>
  node?.kind === "element" && node.tag === "p" && node.attributes.length === 0

/**
 * The `text-align` of a cell paragraph whose only attribute is a style of
 * that one declaration (canonical after step 1), or `null`.
 */
function loneAlignment(node: StoredNode): string | null {
  if (node.kind !== "element" || node.tag !== "p") {
    return null
  }
  const [only, ...rest] = node.attributes
  if (only === undefined || rest.length > 0 || only[0] !== "style") {
    return null
  }
  return /^text-align: [a-z]+$/.test(only[1]) ? only[1] : null
}

/**
 * Tightens one container, or returns it as it is.
 *
 * @param node - the container, its children already tightened.
 * @param policy - the content-v1 policy, for the cell's merged style.
 */
function tighten(node: StoredElement, policy: RichTextPolicy): StoredElement {
  if (!TIGHT_CONTAINERS.has(node.tag)) {
    return node
  }
  const [first, ...rest] = node.children
  if (first === undefined) {
    return node
  }
  if (rest.length === 0 && isBareParagraph(first)) {
    return { ...node, children: first.children }
  }
  // Before lists, only a paragraph with content is tightened: an empty one
  // would leave `<li><ul>`, and a reload makes no paragraph from nothing
  // there but moves the nested list out of the item, so the stored bytes
  // would change on the next save (ADR 0013 §2, "Fixed point"). editor.md
  // §3.7 keeps it, as the maintainer answered on 2026-11-15 (item D148,
  // §16 item 7), and post-pass.test.ts pins it.
  if (
    node.tag === "li" &&
    isBareParagraph(first) &&
    first.children.length > 0 &&
    rest.every((child) => child.kind === "element" && LISTS.has(child.tag))
  ) {
    return { ...node, children: [...first.children, ...rest] }
  }
  const alignment = rest.length === 0 ? loneAlignment(first) : null
  if (alignment !== null && CELLS.has(node.tag) && first.kind === "element") {
    // The paragraph's declaration comes last, so it wins over the cell's
    // own alignment, as the paragraph's did on screen.
    const own = attributeOf(node, "style")
    const style = canonicalStyle(
      policy,
      node.tag,
      own === null ? alignment : `${own}; ${alignment}`
    )
    return { ...withAttribute(node, "style", style), children: first.children }
  }
  return node
}

/**
 * Step 3: lone paragraphs tightened (editor.md §3.7, "Tightening").
 *
 * @param nodes - the tree, with canonical styles (step 1).
 * @param policy - the content-v1 policy.
 */
export function tightenParagraphs(
  nodes: readonly StoredNode[],
  policy: RichTextPolicy
): StoredNode[] {
  return mapElements(nodes, (node) => [tighten(node, policy)])
}

/**
 * Step 4: each element's attributes in the policy's order for its tag
 * (editor.md §3.2), unlisted ones after them in their own order.
 *
 * @param nodes - the tree.
 * @param policy - the content-v1 policy.
 */
export function orderAttributes(
  nodes: readonly StoredNode[],
  policy: RichTextPolicy
): StoredNode[] {
  return mapElements(nodes, (node) => {
    const order = elementOf(policy, node.tag)?.attributes.map(
      ({ name }) => name
    )
    if (order === undefined) {
      return [node]
    }
    const rank = (name: string) => {
      const index = order.indexOf(name)
      return index === -1 ? order.length : index
    }
    // Array.prototype.sort is stable, so unlisted attributes keep their order.
    const attributes = [...node.attributes].sort(
      ([a], [b]) => rank(a) - rank(b)
    )
    return [element(node.tag, attributes, node.children)]
  })
}

/**
 * The whole post-pass: steps 1 to 4, in that order.
 *
 * @param nodes - the tree that the schema rendered.
 * @param policy - the content-v1 policy (pass `CONTENT_V1_POLICY`).
 * @returns the stored tree.
 */
export function postPass(
  nodes: readonly StoredNode[],
  policy: RichTextPolicy
): StoredNode[] {
  return orderAttributes(
    tightenParagraphs(unwrapAndMerge(canonicalStyles(nodes, policy)), policy),
    policy
  )
}
