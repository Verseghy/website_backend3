/**
 * The site schema's headings (editor.md §4.3 row 1a, §3.2).
 *
 * TipTap 3.32's Heading parses and renders levels 1 to 4: `h1` must stay in
 * the list, or the legacy bodies' `h1` (§3.2) would load as paragraphs, and
 * `h5` and `h6` become paragraphs. New headings are H2 to H4 only: the
 * toolbar offers those (§4.5), and the paste filter maps Word's `h1` to
 * `h2` (row 1a). Stock Heading derives its input rules and keyboard
 * shortcuts from the same `levels` that its parse rules use, so with level 1
 * configured, typing `# ` at the start of a paragraph or pressing
 * Mod-Alt-1 would make a new `h1`. Here both come from
 * {@link CREATABLE_HEADING_LEVELS} instead, in stock's shape: `## `, `### `
 * and `#### ` and Mod-Alt-2 to Mod-Alt-4.
 *
 * The commands (`setHeading`, `toggleHeading`) still accept every parsed
 * level, so a legacy `h1` can be kept, changed or toggled back to a
 * paragraph.
 */
import { textblockTypeInputRule } from "@tiptap/core"
import { Heading } from "@tiptap/extension-heading"

/** The heading levels the schema parses and renders (editor.md §4.3 row 1a). */
export const HEADING_LEVELS = [1, 2, 3, 4] as const

/** The heading levels that typing and shortcuts create (§3.2, §4.5). */
export const CREATABLE_HEADING_LEVELS = [2, 3, 4] as const

/** The heading node of the site schema. */
export const SiteHeading = Heading.extend({
  addInputRules() {
    const lowest = Math.min(...CREATABLE_HEADING_LEVELS)
    // As stock: each level's rule matches from the lowest level's number of
    // `#` up to its own, and the first rule that matches wins, so `## `
    // makes an H2 and `#### ` an H4, while `# ` stays text.
    return CREATABLE_HEADING_LEVELS.map((level) =>
      textblockTypeInputRule({
        find: new RegExp(`^(#{${lowest},${level}})\\s$`),
        type: this.type,
        getAttributes: { level },
      })
    )
  },

  addKeyboardShortcuts() {
    return Object.fromEntries(
      CREATABLE_HEADING_LEVELS.map((level) => [
        `Mod-Alt-${level}`,
        () => this.editor.commands.toggleHeading({ level }),
      ])
    )
  },
}).configure({ levels: [...HEADING_LEVELS] })
