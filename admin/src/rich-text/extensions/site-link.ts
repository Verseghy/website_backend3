/**
 * The site schema's link mark (editor.md §4.3 row 1b, ADR 0013 §2 "Link").
 *
 * TipTap 3.32's Link, narrowed to content-v1's `a`:
 *
 * - **Attributes.** Only `href` is a mark attribute. `target` and `rel` are
 *   fixed `HTMLAttributes`, `_blank` and `noopener noreferrer`, taken from
 *   the policy, and stock's `class` and `title` attributes are gone. Stock
 *   parses all four from the element, so a pasted `target="_self"`,
 *   `rel="nofollow"`, `class` or `title` would be stored and then removed by
 *   the server. Attribute order is the serialiser's job (editor.md §4.3, last
 *   paragraph), which writes `rel` last.
 * - **Which links are kept.** `isAllowedUri` is the server's `a[href]` rule
 *   (`link-href.ts`): http, https, mailto and tel, relative links on the
 *   page's own origin, `#` fragments. It decides `setLink` and autolinking.
 * - **Links without a target.** The server keeps an `a` whose `href` is
 *   empty, and an `a` without `href` (it gains `target` and `rel`), so both
 *   are content-v1 and loading and saving them must change nothing
 *   (editor.md §6, P-fix). Stock's parse rule matches only `a[href]` and
 *   refuses an empty `href`, so it would drop both links, while its render
 *   rule writes `href=""` for every refused or missing `href`. Here the
 *   parse rule keeps an `a` whose `href` is missing, empty or allowed, and
 *   the render rule writes `href` only when the rule allows it, otherwise
 *   none, as the server drops a refused `href` and keeps the `a`. A
 *   `toggleLink` without `href` (stock allows it) therefore writes
 *   `<a target rel>`, which loads back unchanged.
 * - **Autolink while typing only.** `autolink` is on, so a URL followed by a
 *   space becomes a link as it is typed. Loading a body must not create
 *   links, so `loadDocument` carries `preventAutolink`. Pasting and dropping
 *   must not either: the paste is the paste filter's output parsed by this
 *   schema (editor.md §5.1), so stock's URL paste rule and its
 *   URL-over-selection paste handler (`linkOnPaste`) are switched off, and
 *   autolink skips paste and drop transactions. The stock paste handler would
 *   also set the `href` linkify finds without asking `isAllowedUri`, so a
 *   pasted `ftp:` URL would become a link whose `href` the render rule then
 *   drops, and as a `handlePaste` of an extension with priority 1000 it
 *   would run before the paste filter's own `handlePaste` (§5.1 step 4) at
 *   TipTap's default priority. `openOnClick` is off, so a click places the
 *   cursor instead of following the link.
 * - **Protocols.** linkifyjs, which autolink uses to find URLs in typed
 *   text, knows http, https and mailto by itself; `tel` is registered as a
 *   scheme without `//`. Two limits of linkifyjs 4.4 remain: it finds no
 *   `tel:` URL whose number starts with `+`, so `tel:+36…` is not
 *   autolinked (a link set with `setLink` is not affected), and it registers
 *   schemes globally, once, before its first use. Link registers them when
 *   an editor emits `create` (a tick after it is constructed) and resets
 *   them when any editor is destroyed, so typing before `create`, or in one
 *   editor after another one was destroyed, autolinks no `tel:` URL until
 *   the next editor is created.
 */
import { mergeAttributes } from "@tiptap/core"
import { Link } from "@tiptap/extension-link"
import type { Transaction } from "@tiptap/pm/state"
import { Plugin } from "@tiptap/pm/state"

import { isAllowedHref } from "../link-href"
import type { RichTextPolicy } from "../policy"
import { requireAttributeRule } from "../policy"

/**
 * The schemes registered with linkifyjs for autolinking, beyond its own
 * http, https and mailto: `tel`, which has no `//`.
 */
export const LINKIFY_PROTOCOLS = [
  { scheme: "tel", optionalSlashes: true },
] as const

/**
 * Whether a transaction comes from a paste or a drop: prosemirror-view marks
 * both with the `uiEvent` meta.
 */
function fromPasteOrDrop(transaction: Transaction): boolean {
  const event: unknown = transaction.getMeta("uiEvent")
  return event === "paste" || event === "drop"
}

/**
 * The link mark of the site schema.
 *
 * @param policy - the content-v1 policy; supplies the URL rules and the
 *   forced `target` and `rel` of `a`.
 * @returns the `link` mark extension.
 * @throws {PolicyError} when the policy's `a` lacks those rules.
 */
export function createSiteLink(policy: RichTextPolicy) {
  requireAttributeRule(policy, "a", "href", "link_href")
  const target = requireAttributeRule(policy, "a", "target", "forced").value
  const rel = requireAttributeRule(policy, "a", "rel", "link_rel").value
  const allowed = (href: unknown): href is string =>
    typeof href === "string" && isAllowedHref(href, policy.urls)
  return Link.extend({
    addAttributes() {
      return {
        href: {
          default: null,
          parseHTML: (element) => element.getAttribute("href"),
        },
      }
    },

    parseHTML() {
      return [
        {
          tag: "a",
          getAttrs: (element) => {
            const href = element.getAttribute("href")
            return href === null || allowed(href) ? null : false
          },
        },
      ]
    },

    renderHTML({ HTMLAttributes }) {
      const { href, ...rest } = HTMLAttributes as Record<string, unknown>
      return [
        "a",
        mergeAttributes(
          this.options.HTMLAttributes,
          rest,
          allowed(href) ? { href } : {}
        ),
        0,
      ]
    },

    addPasteRules() {
      return []
    },

    addProseMirrorPlugins() {
      // Stock autolink runs on every document change without
      // `preventAutolink`, so text pasted or dropped with a URL before a
      // space would become a link. Of the stock plugins (autolink and the
      // click handler; the paste handler is off), autolink is the one with
      // an `appendTransaction`; it is kept, but skips paste and drop.
      return (this.parent?.() ?? []).map((plugin) => {
        const append = plugin.spec.appendTransaction
        if (append === undefined) {
          return plugin
        }
        return new Plugin({
          ...plugin.spec,
          appendTransaction(transactions, oldState, newState) {
            return transactions.some(fromPasteOrDrop)
              ? null
              : append.call(this, transactions, oldState, newState)
          },
        })
      })
    },
  }).configure({
    autolink: true,
    openOnClick: false,
    linkOnPaste: false,
    protocols: LINKIFY_PROTOCOLS.map((protocol) => ({ ...protocol })),
    HTMLAttributes: { target, rel },
    isAllowedUri: (url) => allowed(url),
  })
}
