/**
 * The site schema's lists (editor.md §4.3 rows 1c and 1d): `ol` keeps
 * `start` and `type`, `ul` keeps `list-style-type` `circle` or `square`.
 *
 * Both start from TipTap 3.32's own nodes and narrow them to the policy:
 *
 * - **OrderedList** parses `type` as 3.32 does (the `type` attribute, else
 *   `list-style-type` on the `ol`, else on its first `li`), but keeps only
 *   the policy's values, and parses `start` with the server's integer rule.
 *   Stock 3.32 keeps any `type` string and `parseInt`s `start`, so
 *   `type="disc"` or `start="5x"` would reach the stored HTML and be removed
 *   by the server. It renders `start` only when it is not 1 and `type` only
 *   when it is not `1`, as stock does, and nothing outside the policy.
 * - **BulletList** gains the `listStyleType` attribute, read through the raw
 *   style reader; `disc` is the default and becomes `null`.
 *
 * In the editor view only, a node decoration puts `data-vfg-list-type` on an
 * `ol` with a type (editor.md §11.2): the editor stylesheet's reset rolls
 * the author origin back, and with it the presentational meaning of
 * `<ol type="a">`, while a selector on `[type]` matches ASCII
 * case-insensitively and cannot tell `a` from `A`. Decorations exist only in
 * the view, so the attribute never reaches the document, `getHTML()` or the
 * stored HTML.
 */
import { mergeAttributes } from "@tiptap/core"
import { BulletList, OrderedList } from "@tiptap/extension-list"
import type { Node as ProseMirrorNode } from "@tiptap/pm/model"
import { Plugin, PluginKey } from "@tiptap/pm/state"
import { Decoration, DecorationSet } from "@tiptap/pm/view"

import type { AttributeValueRule, RichTextPolicy } from "../policy"
import { requireAttributeRule, requireStyleGrammar } from "../policy"
import { isCanonicalValue, readStoredDeclarations } from "../style-decls"

/** The view-only attribute that names an `ol`'s list style (§11.2). */
export const LIST_TYPE_ATTRIBUTE = "data-vfg-list-type"

/** The CSS list style of each stored `ol` type (HTML's own mapping). */
export const LIST_STYLE_OF_TYPE: Readonly<Record<string, string>> = {
  a: "lower-alpha",
  A: "upper-alpha",
  i: "lower-roman",
  I: "upper-roman",
}

/**
 * The `type` that a CSS `list-style-type` means, as TipTap 3.32 maps it for
 * Word and Google Docs HTML (`cssListStyleTypeToHtmlType`).
 */
const TYPE_OF_LIST_STYLE: Readonly<Record<string, string>> = {
  "lower-alpha": "a",
  "lower-latin": "a",
  "upper-alpha": "A",
  "upper-latin": "A",
  "lower-roman": "i",
  "upper-roman": "I",
}

type IntegerRule = Extract<AttributeValueRule, { readonly kind: "integer" }>
type EnumeratedRule = Extract<
  AttributeValueRule,
  { readonly kind: "enumerated" }
>

/**
 * Reads an integer attribute with the server's rule (`attrs.rs`, `integer`):
 * ASCII whitespace trimmed, at most ten ASCII digits, within the range.
 *
 * @param value - the attribute value, or `null` when it is absent.
 * @param rule - the policy's integer rule.
 * @returns the number, or `null` when the server would drop the attribute
 *   for its value (the omitted default is a number like any other here).
 */
export function readIntegerAttribute(
  value: string | null,
  rule: IntegerRule
): number | null {
  const digits = value?.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, "") ?? ""
  if (!/^[0-9]{1,10}$/.test(digits)) {
    return null
  }
  const number = Number(digits)
  return number >= rule.min && number <= rule.max ? number : null
}

/**
 * Whether an integer attribute value is one the server stores: an integer
 * in range that is not the omitted default.
 */
function isStoredInteger(value: unknown, rule: IntegerRule): value is number {
  return (
    typeof value === "number" &&
    Number.isInteger(value) &&
    value >= rule.min &&
    value <= rule.max &&
    value !== rule.omitted
  )
}

/** Whether an enumerated value is one the server stores (not the default). */
function isStoredEnumerated(
  value: unknown,
  rule: EnumeratedRule
): value is string {
  return (
    typeof value === "string" &&
    rule.values.includes(value) &&
    value !== rule.omitted
  )
}

/**
 * The `type` of an `ol`: its own `type` attribute when the policy has that
 * value, else the `list-style-type` of the `ol`, else that of its first `li`
 * (TipTap 3.32's order), mapped to a type the policy has.
 *
 * @param element - the `ol`.
 * @param rule - the `ol` `type` rule.
 * @returns the type, or `null` for the default (decimal).
 */
function readListType(element: Element, rule: EnumeratedRule): string | null {
  const own = element.getAttribute("type")
  if (own !== null && rule.values.includes(own)) {
    return own === rule.omitted ? null : own
  }
  const firstItem = element.querySelector("li")
  for (const source of [element, firstItem]) {
    const style = source?.getAttribute("style")
    if (style === null || style === undefined) {
      continue
    }
    // Read every declaration, not only stored ones: list-style-type is not a
    // stored property of ol or li, it only tells which type the list has.
    const listStyle = style
      .split(";")
      .map((piece) => piece.split(":"))
      .filter((parts) => parts.length === 2)
      .find(([name]) => name.trim().toLowerCase() === "list-style-type")?.[1]
      ?.trim()
      .toLowerCase()
    const type =
      listStyle === undefined ? undefined : TYPE_OF_LIST_STYLE[listStyle]
    if (type !== undefined && isStoredEnumerated(type, rule)) {
      return type
    }
  }
  return null
}

/**
 * The view decorations that name each typed `ol`'s list style.
 *
 * @param doc - the document.
 * @param typeName - the ordered list's node type name.
 * @returns one node decoration per `ol` whose `type` has a list style.
 */
export function listTypeDecorations(
  doc: ProseMirrorNode,
  typeName: string
): DecorationSet {
  const decorations: Decoration[] = []
  doc.descendants((node, position) => {
    if (node.type.name === typeName) {
      const style = LIST_STYLE_OF_TYPE[String(node.attrs.type)]
      if (style !== undefined) {
        decorations.push(
          Decoration.node(position, position + node.nodeSize, {
            [LIST_TYPE_ATTRIBUTE]: style,
          })
        )
      }
    }
    return true
  })
  return DecorationSet.create(doc, decorations)
}

/** The plugin that keeps the list-type decorations of the view. */
const listTypeKey = new PluginKey<DecorationSet>("vfgListType")

/**
 * The ordered list of the site schema.
 *
 * @param policy - the content-v1 policy; supplies the `start` and `type`
 *   rules of `ol`.
 * @returns the `orderedList` node extension.
 * @throws {PolicyError} when the policy has no such rules.
 */
export function createSiteOrderedList(policy: RichTextPolicy) {
  const startRule = requireAttributeRule(policy, "ol", "start", "integer")
  const typeRule = requireAttributeRule(policy, "ol", "type", "enumerated")
  return OrderedList.extend({
    addAttributes() {
      return {
        start: {
          default: 1,
          parseHTML: (element) =>
            readIntegerAttribute(element.getAttribute("start"), startRule) ?? 1,
        },
        type: {
          default: null,
          parseHTML: (element) => readListType(element, typeRule),
        },
      }
    },

    renderHTML({ HTMLAttributes }) {
      const { start, type, ...rest } = HTMLAttributes
      const attributes = mergeAttributes(this.options.HTMLAttributes, rest)
      if (isStoredInteger(start, startRule)) {
        attributes.start = String(start)
      }
      if (isStoredEnumerated(type, typeRule)) {
        attributes.type = type
      }
      return ["ol", attributes, 0]
    },

    addProseMirrorPlugins() {
      const typeName = this.name
      return [
        ...(this.parent?.() ?? []),
        new Plugin<DecorationSet>({
          key: listTypeKey,
          state: {
            init: (_, state) => listTypeDecorations(state.doc, typeName),
            // Rebuilt on every document change: one walk over the document,
            // which ProseMirror does on each transaction anyway, keeps the
            // decorations exact without mapping rules for every edit.
            apply: (transaction, decorations) =>
              transaction.docChanged
                ? listTypeDecorations(transaction.doc, typeName)
                : decorations,
          },
          props: {
            decorations: (state) => listTypeKey.getState(state),
          },
        }),
      ]
    },
  })
}

/**
 * The bullet list of the site schema.
 *
 * @param policy - the content-v1 policy; supplies the `ul`
 *   `list-style-type` grammar.
 * @returns the `bulletList` node extension.
 * @throws {PolicyError} when the policy has no such grammar.
 */
export function createSiteBulletList(policy: RichTextPolicy) {
  const grammar = requireStyleGrammar(
    policy,
    "ul",
    "list-style-type",
    "keyword"
  )
  return BulletList.extend({
    addAttributes() {
      return {
        ...this.parent?.(),
        listStyleType: {
          default: null,
          parseHTML: (element) =>
            readStoredDeclarations(
              policy,
              "ul",
              element.getAttribute("style")
            ).get("list-style-type") ?? null,
          renderHTML: (attributes) =>
            typeof attributes.listStyleType === "string" &&
            isCanonicalValue(grammar, attributes.listStyleType)
              ? { style: `list-style-type: ${attributes.listStyleType}` }
              : {},
        },
      }
    },
  })
}
