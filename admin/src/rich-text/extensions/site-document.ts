/**
 * The document node of the site schema (editor.md §4.2).
 *
 * Images and embeds belong to the node group `media`, and only the document
 * names that group in its content expression. That is what makes them legal
 * at the top level only, never in a list item, quote, heading or (later) a
 * table cell, because Angular's `processCustomTags` moves every `<img>` in
 * front of its parent element (editor.md §4.2).
 *
 * ProseMirror refuses a content expression that names a group without a
 * member ("No node type or group 'media' found"), so the expression names
 * `media` only once the schema has a media node. With no media node,
 * `(block | media)+` and `block+` describe the same documents, so the schema
 * is the one §4.2 specifies either way.
 */
import { Document } from "@tiptap/extension-document"

/** The node group of images and embeds. */
export const MEDIA_GROUP = "media"

/** The document's content: blocks and media, at least one (editor.md §4.2). */
export const DOCUMENT_CONTENT = `(block | ${MEDIA_GROUP})+`

/** The same content when the schema has no node in the media group. */
export const DOCUMENT_CONTENT_WITHOUT_MEDIA = "block+"

/**
 * The document node, replacing StarterKit's (whose `document` option is
 * `false`).
 *
 * @param hasMedia - whether the schema has at least one node in the
 *   {@link MEDIA_GROUP} group.
 * @returns the `doc` node extension.
 */
export function createSiteDocument(hasMedia: boolean) {
  return Document.extend({
    content: hasMedia ? DOCUMENT_CONTENT : DOCUMENT_CONTENT_WITHOUT_MEDIA,
  })
}
