/**
 * The site schema: the TipTap 3.32 extensions that make up the editor's
 * document model for content-v1 (ADR 0013 §2, editor.md §4.2 and §4.3).
 *
 * The schema is the editor's half of the fixed point "loading and saving a
 * body changes no byte" (ADR 0013 §2): every element and attribute it renders
 * is one the server keeps (`site-extensions.policy.test.ts` compares it with
 * the committed `policy.json`), and every parse rule reads only content-v1's
 * spellings, so what it loads is what it renders.
 *
 * What it holds, and why each choice is made, is in the extension modules
 * under `extensions/`:
 *
 * - StarterKit 3.32 for paragraphs, bold, italic, underline, strike, hard
 *   break, rule, quote, list items, undo, the drop and gap cursors and the
 *   list keymap. Its own document, heading, bullet list, ordered list and
 *   link are replaced by the site versions below. Code and code blocks are
 *   switched off (content-v1 has neither) and so is the trailing node,
 *   which would append `<p></p>` after a final table or embed. None of
 *   them sets a `class`, so none is ever stored.
 * - `site-document.ts`: the document, `(block | media)+`.
 * - `site-heading.ts`: headings 1-4 parsed (`h1` for the legacy bodies that
 *   use it), H2-H4 created by typing and shortcuts.
 * - `site-lists.ts`: `ol` with `start` and `type`, `ul` with `circle` and
 *   `square`.
 * - `site-link.ts`: links on the server's URL rule, autolinked while typing
 *   only.
 * - `site-text-align.ts`: paragraph and heading alignment; in a table cell,
 *   the alignment commands align the cell.
 * - `site-scripts.ts`: subscript and superscript, each excluding the other.
 * - `site-text-style.ts`: colour, highlight and `em` font size on `span`.
 * - `site-table.ts`: tables in content-v1's one shape, with the canonical
 *   cell declarations as cell attributes (rows 5-7).
 *
 * Images, embeds, the paste extension and image uploads join this list as
 * their own modules (editor.md §4.3 rows 8-13).
 */
import type { AnyExtension } from "@tiptap/core"
import { callOrReturn, getExtensionField, Node } from "@tiptap/core"
import { StarterKit } from "@tiptap/starter-kit"

import { createSiteDocument, MEDIA_GROUP } from "./extensions/site-document"
import { SiteHeading } from "./extensions/site-heading"
import { createSiteLink } from "./extensions/site-link"
import {
  createSiteBulletList,
  createSiteOrderedList,
} from "./extensions/site-lists"
import { SiteSubscript, SiteSuperscript } from "./extensions/site-scripts"
import { createSiteTable } from "./extensions/site-table"
import { createSiteTextAlign } from "./extensions/site-text-align"
import { createSiteTextStyle } from "./extensions/site-text-style"
import type { RichTextPolicy } from "./policy"

export {
  CREATABLE_HEADING_LEVELS,
  HEADING_LEVELS,
} from "./extensions/site-heading"

/** What {@link siteExtensions} is built from. */
export interface SiteExtensionsOptions {
  /**
   * The content-v1 policy the parse and render rules follow: pass
   * `CONTENT_V1_POLICY` from `./policy` (editor.md §1.2 D6). Tests pass
   * changed copies to check that the schema follows the policy.
   */
  readonly policy: RichTextPolicy
  /**
   * The nodes of the `media` group (editor.md §4.2): the image and embed
   * nodes, legal only at the top level of the document. Each must be a
   * node whose only group is `media`. Default: none, and the document holds
   * blocks only (`site-document.ts` says why that is the same schema).
   */
  readonly media?: readonly Node[]
}

/**
 * Checks that every media node is a node whose only group is `media`. A
 * node that is also a block would be legal inside quotes and list items,
 * which §4.2 forbids.
 */
function checkMedia(
  media: readonly unknown[]
): asserts media is readonly Node[] {
  media.forEach((extension, index) => {
    if (!(extension instanceof Node)) {
      throw new TypeError(
        `siteExtensions: options.media[${index}] must be a TipTap Node (Node.create), not ${typeof extension}.`
      )
    }
    // `group` may be a function of the node's options; getExtensionField
    // binds it and callOrReturn calls it, as TipTap does when it builds the
    // schema.
    const group: unknown = callOrReturn(
      getExtensionField(extension, "group", {
        name: extension.name,
        options: extension.options,
        storage: extension.storage,
      })
    )
    const groups =
      typeof group === "string" ? group.split(/\s+/).filter(Boolean) : []
    if (groups.length !== 1 || groups[0] !== MEDIA_GROUP) {
      throw new TypeError(
        `siteExtensions: media node "${extension.name}" has the group ${JSON.stringify(group ?? null)}; it must be exactly "${MEDIA_GROUP}", so that it is legal only at the top level (editor.md §4.2).`
      )
    }
  })
}

/**
 * The extensions of the site schema, for a TipTap `Editor` or `getSchema`.
 *
 * @param options - the policy and the media nodes; see
 *   {@link SiteExtensionsOptions}.
 * @returns the extensions, in the order TipTap registers them.
 * @throws {TypeError} when `options` has no policy or a media node is not in
 *   the `media` group alone.
 * @throws {PolicyError} when the policy lacks a rule the schema reads.
 */
export function siteExtensions(options: SiteExtensionsOptions): AnyExtension[] {
  if (typeof options !== "object" || options === null || !options.policy) {
    throw new TypeError(
      "siteExtensions: options.policy is required; pass CONTENT_V1_POLICY from admin/src/rich-text/policy.ts."
    )
  }
  const { policy } = options
  const media = options.media ?? []
  checkMedia(media)
  return [
    StarterKit.configure({
      document: false,
      bulletList: false,
      orderedList: false,
      link: false,
      code: false,
      codeBlock: false,
      trailingNode: false,
      heading: false,
    }),
    createSiteDocument(media.length > 0),
    SiteHeading,
    createSiteBulletList(policy),
    createSiteOrderedList(policy),
    createSiteLink(policy),
    createSiteTextAlign(policy),
    SiteSubscript,
    SiteSuperscript,
    ...createSiteTextStyle(policy),
    ...createSiteTable(policy),
    ...media,
  ]
}
