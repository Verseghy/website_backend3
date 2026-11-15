/**
 * The whitespace rule of content-v1 (editor.md §3.8, §6 step 2.6): the
 * stored text of every textblock is exactly what prosemirror-model 1.25.12
 * keeps when it loads the body again, so loading and saving changes no
 * byte.
 *
 * On load, prosemirror-model (`from_dom.ts`, `addTextNode` and `finish`)
 * collapses every run of collapsible ASCII whitespace (space, tab, LF, CR,
 * FF) to one space, drops that space at the start of a textblock and after
 * a hard break, and drops trailing whitespace at the end of the textblock.
 * U+00A0 is not collapsible. So the stored text is written in the form that
 * those steps leave unchanged:
 *
 * - collapsible whitespace is removed at the start of a textblock, at its
 *   end and directly after a hard break;
 * - every other run of n collapsible characters is written as n - 1
 *   U+00A0 followed by one space, which keeps two typed spaces two spaces
 *   wide and loads back as the same characters.
 *
 * A run is counted across mark boundaries (the characters keep their own
 * text node and marks), and a hard break or any other inline leaf node ends
 * it. A run just before a hard break is an inner run, because
 * prosemirror-model strips trailing whitespace only at the end of the
 * textblock.
 *
 * U+0000 is dropped first. Every HTML parse ignores it in text: the
 * tree builder's "in body" rule (html5ever@0.40.1:src/tree_builder/rules.rs,
 * `Token::NullCharacter`), which the server's sanitiser and the next load
 * both run. A document can still hold it, because text typed or pasted as
 * plain text never passes an HTML parse. Written as it is, it would be lost
 * on the next load, so the stored bytes would change (ADR 0013 §2). It is
 * dropped here, before the whitespace runs are counted (it would otherwise
 * split a run that the reload joins) and before the post-pass (a paragraph
 * that held only U+0000 must reach tightening as the empty paragraph it
 * becomes on reload).
 *
 * The rule works on the document rather than the rendered DOM because a
 * textblock is one node there, while in the stored HTML a tightened
 * paragraph has no element of its own.
 */
import type { Node as ProseMirrorNode } from "@tiptap/pm/model"
import { Fragment } from "@tiptap/pm/model"

/** The characters prosemirror-model collapses. */
const COLLAPSIBLE = new Set([" ", "\t", "\n", "\r", "\f"])

/** The character that no HTML parse keeps in text. */
const NUL = "\u0000"

/** One character of a textblock, by the text node it belongs to. */
interface Slot {
  /** The index of the text node in the textblock's content. */
  readonly node: number
  /** The character, or `null` once it is removed. */
  character: string | null
}

/**
 * The whitespace rule for one textblock's inline content, after U+0000 is
 * dropped.
 *
 * @param content - the inline nodes, in order.
 * @param isBreak - whether a node is a hard break.
 * @returns the content with its text rewritten; text nodes that end up
 *   empty are left out, and neighbouring text nodes with the same marks
 *   are joined.
 */
export function normalizeInlineWhitespace(
  content: readonly ProseMirrorNode[],
  isBreak: (node: ProseMirrorNode) => boolean
): ProseMirrorNode[] {
  // The textblock as a sequence of characters and leaf nodes. U+0000 never
  // enters it, so the runs below are counted as the reload sees them.
  const sequence: (Slot | ProseMirrorNode)[] = []
  content.forEach((node, index) => {
    if (node.isText) {
      for (const character of node.text ?? "") {
        if (character !== NUL) {
          sequence.push({ node: index, character })
        }
      }
    } else {
      sequence.push(node)
    }
  })
  // A ProseMirror node has a `type`; a slot has none.
  const isSlot = (item: Slot | ProseMirrorNode): item is Slot =>
    !("type" in item)
  const collapsible = (item: Slot | ProseMirrorNode) =>
    isSlot(item) && item.character !== null && COLLAPSIBLE.has(item.character)

  // At the start, and after every hard break: strip up to the first other
  // character or leaf.
  let stripping = true
  for (const item of sequence) {
    if (!isSlot(item)) {
      stripping = isBreak(item)
    } else if (stripping && collapsible(item)) {
      item.character = null
    } else {
      stripping = false
    }
  }
  // At the end: strip back to the last other character or leaf.
  for (let index = sequence.length - 1; index >= 0; index -= 1) {
    const item = sequence[index]
    if (!isSlot(item)) {
      break
    }
    if (item.character === null) {
      continue
    }
    if (!collapsible(item)) {
      break
    }
    item.character = null
  }
  // Every inner run: n - 1 no-break spaces and one space.
  let run: Slot[] = []
  const closeRun = () => {
    run.forEach((slot, index) => {
      slot.character = index === run.length - 1 ? " " : "\u00a0"
    })
    run = []
  }
  for (const item of sequence) {
    if (collapsible(item)) {
      run.push(item as Slot)
    } else if (!isSlot(item) || item.character !== null) {
      closeRun()
    }
  }
  closeRun()

  const texts = content.map(() => "")
  for (const item of sequence) {
    if (isSlot(item) && item.character !== null) {
      texts[item.node] += item.character
    }
  }
  const rewritten: ProseMirrorNode[] = []
  content.forEach((node, index) => {
    if (!node.isText) {
      rewritten.push(node)
    } else if (texts[index] !== "") {
      rewritten.push(
        texts[index] === node.text
          ? node
          : node.type.schema.text(texts[index], node.marks)
      )
    }
  })
  const joined: ProseMirrorNode[] = []
  Fragment.fromArray(rewritten).forEach((node) => joined.push(node))
  return joined
}

/**
 * Applies the whitespace rule, and the drop of U+0000, to every textblock
 * of a document.
 *
 * @param doc - the document.
 * @returns the document, or a copy whose textblocks hold the stored text.
 *   Nodes that the rule does not change are shared, not copied.
 */
export function normalizeWhitespace(doc: ProseMirrorNode): ProseMirrorNode {
  const isBreak = (node: ProseMirrorNode) =>
    node.type === node.type.schema.linebreakReplacement
  const visit = (node: ProseMirrorNode): ProseMirrorNode => {
    if (node.isLeaf) {
      return node
    }
    const children: ProseMirrorNode[] = []
    node.forEach((child) => children.push(child))
    const next = node.isTextblock
      ? normalizeInlineWhitespace(children, isBreak)
      : children.map(visit)
    const unchanged =
      next.length === children.length &&
      next.every((child, index) => child === children[index])
    return unchanged ? node : node.copy(Fragment.fromArray(next))
  }
  return visit(doc)
}
