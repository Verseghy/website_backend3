/**
 * The Padlet embed of the site schema (editor.md §4.3 row 10, §3.7).
 *
 * An atom in the `media` group, so it lives at the top level only (§4.2).
 * It holds the board's `key`, the player's `height` in px and the iframe's
 * `title`, and writes content-v1's one Padlet shape:
 *
 * ```html
 * <p data-padlet-embed=""><iframe src="https://padlet.com/embed/<key>" width="100%" height="608" title="Padlet" style="height: 608px; width: 100%" allow="…" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"></iframe></p>
 * ```
 *
 * The parse rules, highest priority first: `p[data-padlet-embed]` (70), a
 * `p` whose only content is one Padlet iframe (65), and a lone Padlet
 * `iframe` (60). Each reads the address with `padlet-input.ts`; the height
 * comes from the iframe's style (`height: <n>px`), else from its `height`
 * attribute, within the iframe's CSS height range, else 608. The stored
 * title is kept as it is, or none, so a stored body loads back unchanged.
 */
import { Node } from "@tiptap/core"
import type { DOMOutputSpec } from "@tiptap/pm/model"

import type { RichTextPolicy } from "../policy"
import { requireAttributeRule, requireStyleGrammar } from "../policy"
import {
  isPadletKey,
  PADLET_DEFAULT_HEIGHT,
  padletEmbedSrc,
  padletRules,
  readPadletAddress,
} from "../padlet-input"
import { readStyleDecls } from "../style-decls"
import {
  canPlaceMedia,
  forcedFrameAttributes,
  loneElementChild,
  requireStoredStyle,
  wrapperIframe,
} from "./embed-frame"
import { MEDIA_GROUP } from "./site-document"

/** The node's name in the schema. */
export const PADLET_EMBED_NODE = "padletEmbed"

/** The node's attributes. */
export interface PadletEmbedAttributes {
  /** The board's key, the last segment of its embed address. */
  readonly key: string
  /** The player's height in px. */
  readonly height: number
  /** The iframe's `title`, or `null` for none. */
  readonly title: string | null
}

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    padletEmbed: {
      /**
       * Inserts a Padlet board at the selection, as the insert dialog does.
       *
       * @param attributes - the board, its height and its title.
       * @returns `false`, changing nothing, when the server would refuse
       *   the address or the height, or when the selection is not a place
       *   for media (`isMediaPlace`: a list item, quote, table cell or
       *   heading), so `editor.can()` disables the toolbar button there.
       */
      insertPadletEmbed: (attributes: PadletEmbedAttributes) => ReturnType
    }
  }
}

/** Reads a `<n>px` length that is a whole number of pixels. */
function wholePixels(value: string | undefined): number | null {
  const match = /^([0-9]+)px$/.exec(value ?? "")
  return match === null ? null : Number(match[1])
}

/**
 * The Padlet embed node.
 *
 * @param policy - the content-v1 policy: the provider's host and key rule,
 *   the forced iframe attributes, the full-width value and the iframe's
 *   height range.
 * @returns the node extension.
 * @throws {PolicyError} when the policy lacks a rule the node reads, or
 *   would store the node's style differently.
 */
export function createPadletEmbed(policy: RichTextPolicy) {
  const rules = padletRules(policy)
  const forced = forcedFrameAttributes(policy)
  const fullWidth = requireAttributeRule(
    policy,
    "iframe",
    "width",
    "integer_or_full"
  ).full
  const heights = requireStyleGrammar(policy, "iframe", "height", "auto_or_px")
  const frameHeight = requireAttributeRule(
    policy,
    "iframe",
    "height",
    "integer"
  )
  const isHeight = (value: unknown): value is number =>
    typeof value === "number" &&
    Number.isInteger(value) &&
    value >= heights.min &&
    value <= heights.max &&
    value >= frameHeight.min &&
    value <= frameHeight.max
  const styleOf = (height: number) => `height: ${height}px; width: ${fullWidth}`
  requireStoredStyle(policy, "iframe", styleOf(PADLET_DEFAULT_HEIGHT))

  const fromIframe = (
    iframe: Element | null
  ): PadletEmbedAttributes | false => {
    const src =
      iframe?.localName === "iframe" ? iframe.getAttribute("src") : null
    const key = src === null ? null : readPadletAddress(src, rules)
    if (iframe === null || key === null) {
      return false
    }
    const fromStyle = wholePixels(readStyleDecls(policy, iframe).get("height"))
    const fromAttribute = /^[0-9]{1,4}$/.test(
      iframe.getAttribute("height") ?? ""
    )
      ? Number(iframe.getAttribute("height"))
      : null
    const height = [fromStyle, fromAttribute].find(isHeight)
    return {
      key,
      height: height ?? PADLET_DEFAULT_HEIGHT,
      title: iframe.getAttribute("title"),
    }
  }
  // See youtube-embed.ts: the parse rules alone fill the attributes.
  const fromRules = { rendered: false, parseHTML: () => null }

  return Node.create({
    name: PADLET_EMBED_NODE,
    group: MEDIA_GROUP,
    atom: true,
    draggable: true,

    addAttributes() {
      return {
        key: { ...fromRules, default: null },
        height: { ...fromRules, default: PADLET_DEFAULT_HEIGHT },
        title: { ...fromRules, default: null },
      }
    },

    parseHTML() {
      return [
        {
          tag: "p[data-padlet-embed]",
          priority: 70,
          getAttrs: (element) => fromIframe(wrapperIframe(element)),
        },
        {
          tag: "p",
          priority: 65,
          getAttrs: (element) => fromIframe(loneElementChild(element)),
        },
        {
          tag: "iframe",
          priority: 60,
          getAttrs: (element) => fromIframe(element),
        },
      ]
    },

    renderHTML({ node }): DOMOutputSpec {
      const { key, height, title } = node.attrs as PadletEmbedAttributes
      const validKey = typeof key === "string" && isPadletKey(key, rules)
      const validHeight = isHeight(height) ? height : PADLET_DEFAULT_HEIGHT
      return [
        "p",
        { "data-padlet-embed": "" },
        [
          "iframe",
          {
            src: validKey ? padletEmbedSrc(key) : null,
            width: fullWidth,
            height: String(validHeight),
            title,
            style: styleOf(validHeight),
            ...forced,
          },
        ],
      ]
    },

    addCommands() {
      return {
        insertPadletEmbed:
          (attributes) =>
          ({ commands, tr }) =>
            canPlaceMedia(tr.selection) &&
            isPadletKey(attributes.key, rules) &&
            isHeight(attributes.height) &&
            commands.insertContent({ type: this.name, attrs: attributes }),
      }
    },
  })
}
