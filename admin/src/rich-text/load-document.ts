/**
 * Loading a stored body into the editor (editor.md §4.5 "Loading", §6 `J`).
 *
 * `loadDocument` is the one way a body enters an editor. It is TipTap's
 * `setContent` with the default whitespace handling: the kit's
 * `preserveWhitespace: "full"` turns legacy source indentation into empty
 * paragraphs (post 532: 12 to 24 `<p>`, ADR 0013). The transaction also
 * carries two flags:
 *
 * - `preventAutolink`, so the Link mark's autolink, which runs on every
 *   document change, does not turn a URL at the end of a body's first
 *   paragraph into a link: autolink is for typing only (ADR 0013 §2);
 * - `addToHistory: false`, so the first undo after loading does not empty
 *   the editor.
 */
import type { Editor } from "@tiptap/core"

/**
 * Replaces the editor's document with a stored body.
 *
 * @param editor - the editor, built with `siteExtensions` and not destroyed.
 * @param html - the body, content-v1 HTML as `serve_content` returns it; `""`
 *   gives an empty paragraph.
 * @throws {TypeError} when `html` is not a string or the editor is destroyed.
 */
export function loadDocument(editor: Editor, html: string): void {
  if (typeof html !== "string") {
    throw new TypeError(
      `loadDocument: html must be the stored body as a string, not ${typeof html}.`
    )
  }
  if (editor.isDestroyed) {
    throw new TypeError(
      "loadDocument: the editor is destroyed; load into a live editor."
    )
  }
  editor
    .chain()
    .setMeta("preventAutolink", true)
    .setMeta("addToHistory", false)
    .setContent(html)
    .run()
}
