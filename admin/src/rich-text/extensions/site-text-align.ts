/**
 * Paragraph and heading alignment (editor.md §4.3 row 2).
 *
 * TipTap 3.32's TextAlign on `paragraph` and `heading`, with the four
 * alignments of the policy and no default alignment, so an unaligned block
 * carries no style. Its `parseHTML` is replaced: stock reads
 * `element.style.textAlign`, the CSSOM, which the parse rules never use
 * (editor.md §2 stage 3); this one reads the raw `style` through
 * `readStoredDeclarations`, so `text-align: start` or a legacy `align`
 * attribute is not an alignment (the import converts `p[align]` before it
 * sanitises, editor.md §9). In a table cell the toolbar aligns the cell
 * instead; that command belongs to the table nodes.
 */
import { TextAlign } from "@tiptap/extension-text-align"

import type { RichTextPolicy } from "../policy"
import { PolicyError, requireStyleGrammar } from "../policy"
import { readStoredDeclarations } from "../style-decls"

/** The node types that carry an alignment, and the tags they render. */
export const ALIGNED_TYPES: Readonly<Record<string, readonly string[]>> = {
  paragraph: ["p"],
  heading: ["h1", "h2", "h3", "h4"],
}

/**
 * The text-align extension of the site schema.
 *
 * @param policy - the content-v1 policy; supplies the `text-align` keywords,
 *   which must be the same on every aligned element.
 * @returns the `textAlign` extension.
 * @throws {PolicyError} when an aligned element lacks the keyword grammar or
 *   two of them differ.
 */
export function createSiteTextAlign(policy: RichTextPolicy) {
  const tags = Object.values(ALIGNED_TYPES).flat()
  const grammars = tags.map((tag) =>
    requireStyleGrammar(policy, tag, "text-align", "keyword")
  )
  const alignments = grammars[0].values
  tags.forEach((tag, index) => {
    if (grammars[index].values.join() !== alignments.join()) {
      throw new PolicyError(
        `${tag} text-align allows ${grammars[index].values.join(", ")}, but ${tags[0]} allows ${alignments.join(", ")}; the alignment attribute is one for paragraphs and headings (editor.md §4.3 row 2).`
      )
    }
  })
  return TextAlign.extend({
    addGlobalAttributes() {
      return [
        {
          types: this.options.types,
          attributes: {
            textAlign: {
              default: null,
              parseHTML: (element) =>
                readStoredDeclarations(
                  policy,
                  element.tagName.toLowerCase(),
                  element.getAttribute("style")
                ).get("text-align") ?? null,
              renderHTML: (attributes) =>
                typeof attributes.textAlign === "string" &&
                this.options.alignments.includes(attributes.textAlign)
                  ? { style: `text-align: ${attributes.textAlign}` }
                  : {},
            },
          },
        },
      ]
    },
  }).configure({
    types: Object.keys(ALIGNED_TYPES),
    alignments: [...alignments],
    defaultAlignment: null,
  })
}
