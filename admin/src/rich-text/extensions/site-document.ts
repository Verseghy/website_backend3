/**
 * The document node of the site schema (editor.md §4.2).
 *
 * Images and embeds belong to the node group `media`, and only the document
 * names that group in its content expression. That is what makes them legal
 * at the top level only, never in a list item, quote, heading or table
 * cell, because Angular's `processCustomTags` moves every `<img>` in front
 * of its parent element (editor.md §4.2). The group's members are
 * `SiteImage`, `YoutubeEmbed` and `PadletEmbed` (§4.3 rows 8-10).
 */
import { Document } from "@tiptap/extension-document"

/** The node group of images and embeds. */
export const MEDIA_GROUP = "media"

/** The document's content: blocks and media, at least one (editor.md §4.2). */
export const DOCUMENT_CONTENT = `(block | ${MEDIA_GROUP})+`

/** The document node, replacing StarterKit's (whose `document` option is `false`). */
export const SiteDocument = Document.extend({ content: DOCUMENT_CONTENT })
