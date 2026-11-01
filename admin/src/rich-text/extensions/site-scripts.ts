/**
 * Subscript and superscript, each excluding the other (editor.md §4.3 row 3).
 *
 * The stock 3.32.0 marks set no `excludes`, so toggling both on one run
 * would store `<sub><sup>2</sup></sub>`, which the server's structural pass
 * rewrites (editor.md §3.2, §7.5). With `excludes`, adding one mark removes
 * the other: the mark applied last, or parsed innermost, wins. Neither mark
 * has attributes, so two marks of the same type are always equal and never
 * stack, although each no longer excludes its own type.
 */
import { Subscript } from "@tiptap/extension-subscript"
import { Superscript } from "@tiptap/extension-superscript"

/** The `subscript` mark; it removes `superscript` from the run it covers. */
export const SiteSubscript = Subscript.extend({ excludes: "superscript" })

/** The `superscript` mark; it removes `subscript` from the run it covers. */
export const SiteSuperscript = Superscript.extend({ excludes: "subscript" })
