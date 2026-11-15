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
 * - `site-image.ts`, `youtube-embed.ts`, `padlet-embed.ts`: the image and
 *   the two embeds, the `media` group, top level only (rows 8-10).
 *
 * The paste extension and image uploads join this list as their own
 * modules (editor.md §4.3 rows 11-13).
 */
import type { AnyExtension } from "@tiptap/core"
import { StarterKit } from "@tiptap/starter-kit"

import type { NotifyEditor } from "./extensions/editor-notice"
import { IGNORE_NOTICES } from "./extensions/editor-notice"
import { createPadletEmbed } from "./extensions/padlet-embed"
import { SiteDocument } from "./extensions/site-document"
import { SiteHeading } from "./extensions/site-heading"
import { createSiteImage } from "./extensions/site-image"
import { createSiteLink } from "./extensions/site-link"
import {
  createSiteBulletList,
  createSiteOrderedList,
} from "./extensions/site-lists"
import { SiteSubscript, SiteSuperscript } from "./extensions/site-scripts"
import { createSiteTable } from "./extensions/site-table"
import { createSiteTextAlign } from "./extensions/site-text-align"
import { createSiteTextStyle } from "./extensions/site-text-style"
import { createYoutubeEmbed } from "./extensions/youtube-embed"
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
   * Shows a notice of the editor to the staff member, by its `editor.*` key:
   * the YouTube paste rule's dropped options and refused links (ADR 0043
   * §4). Default: notices are not shown, as in a headless editor.
   */
  readonly notify?: NotifyEditor
}

/**
 * The extensions of the site schema, for a TipTap `Editor` or `getSchema`.
 *
 * @param options - the policy and the notice callback; see
 *   {@link SiteExtensionsOptions}.
 * @returns the extensions, in the order TipTap registers them.
 * @throws {TypeError} when `options` has no policy, or a `notify` that is
 *   not a function.
 * @throws {PolicyError} when the policy lacks a rule the schema reads.
 */
export function siteExtensions(options: SiteExtensionsOptions): AnyExtension[] {
  if (typeof options !== "object" || options === null || !options.policy) {
    throw new TypeError(
      "siteExtensions: options.policy is required; pass CONTENT_V1_POLICY from admin/src/rich-text/policy.ts."
    )
  }
  // Checked here, because the paste rule calls it only later, inside a
  // transaction, where a wrong value would fail with a bare "not a function".
  if (options.notify !== undefined && typeof options.notify !== "function") {
    throw new TypeError(
      `siteExtensions: options.notify must be a function that takes an editor.* message key, or left out; got ${options.notify === null ? "null" : typeof options.notify}.`
    )
  }
  const { policy } = options
  const notify = options.notify ?? IGNORE_NOTICES
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
    SiteDocument,
    SiteHeading,
    createSiteBulletList(policy),
    createSiteOrderedList(policy),
    createSiteLink(policy),
    createSiteTextAlign(policy),
    SiteSubscript,
    SiteSuperscript,
    ...createSiteTextStyle(policy),
    ...createSiteTable(policy),
    createSiteImage(policy),
    createYoutubeEmbed(policy, notify),
    createPadletEmbed(policy),
  ]
}
