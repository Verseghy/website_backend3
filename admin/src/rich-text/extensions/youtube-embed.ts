/**
 * The YouTube embed of the site schema (editor.md §4.3 row 9, §3.7;
 * ADR 0013 §2, amended by ADR 0043).
 *
 * An own node, not stock `@tiptap/extension-youtube`, which writes about a
 * dozen option attributes and parses only its own wrapper. It is an atom in
 * the `media` group, so it lives at the top level only (§4.2), and it holds
 * what the stored address says: `videoId`, `start`, `list`, plus the
 * iframe's `title`. It writes content-v1's one shape (§3.7):
 *
 * ```html
 * <p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/<id>?start=…&amp;list=…" width="640" height="360" title="…" style="aspect-ratio: 16 / 9; height: auto; max-width: 640px; min-height: 200px; width: 100%" allow="…" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"></iframe></p>
 * ```
 *
 * Every input goes through the one reader of `youtube-input.ts`: the parse
 * rules below, the lone-URL paste rule, the insert dialog, and the
 * plain-text route that `paste-generic` adds. The parse rules, highest
 * priority first:
 *
 * - 70: `p[data-youtube-video]`, the stored shape, and
 *   `div[data-youtube-video]`, the wrapper of other TipTap installs;
 * - 65: a `p` whose only content is one YouTube iframe (legacy bodies);
 * - 60: a bare YouTube `iframe`, YouTube's Share > Embed code from an HTML
 *   clipboard or the plain-text route.
 *
 * Loading keeps the stored title as it is, or none (D135), so a stored body
 * loads back unchanged. The insert routes apply the title rule instead.
 *
 * The lone-URL paste rule turns a paste that is exactly one YouTube URL
 * into an embed titled „Videó” (D130, D131, D135) and reports the playback
 * options it dropped (D134). A YouTube link it cannot embed (a wrong id, a
 * user-uploads or search list) stays text, and staff are told why. So does a
 * link pasted where media cannot go (a list item, quote, table cell or
 * heading, `isMediaPlace`), which the insert command refuses too (§4.2).
 */
import { Node, PasteRule } from "@tiptap/core"
import type { DOMOutputSpec } from "@tiptap/pm/model"

import type { RichTextPolicy } from "../policy"
import type { YoutubeReading, YoutubeVideo } from "../youtube-input"
import {
  isYoutubeVideo,
  readYoutubeIframe,
  readYoutubeUrl,
  YOUTUBE_DEFAULT_TITLE,
  youtubeEmbedSrc,
  youtubeRules,
} from "../youtube-input"
import type { EditorNotice, NotifyEditor } from "./editor-notice"
import {
  canPlaceMedia,
  forcedFrameAttributes,
  isMediaPlace,
  loneElementChild,
  requireStoredStyle,
  wrapperIframe,
} from "./embed-frame"
import { MEDIA_GROUP } from "./site-document"

/** The node's name in the schema. */
export const YOUTUBE_EMBED_NODE = "youtubeEmbed"

/**
 * The player's style (editor.md §3.7): as wide as its column up to 640 px,
 * 16:9, and never lower than YouTube's minimum of 200 px (ADR 0043, D133).
 */
export const YOUTUBE_STYLE =
  "aspect-ratio: 16 / 9; height: auto; max-width: 640px; min-height: 200px; width: 100%"

/** The `width` and `height` attributes of the stored player. */
export const YOUTUBE_FRAME_SIZE = { width: "640", height: "360" } as const

/** The node's attributes: what the player plays, and the iframe's title. */
export interface YoutubeEmbedAttributes extends YoutubeVideo {
  /** The iframe's `title`, or `null` for none. */
  readonly title: string | null
}

/** The messages of a refused YouTube link and of dropped options. */
export const YOUTUBE_NOTICES = {
  optionsDropped: "editor.youtube.options_dropped",
  videoId: "editor.youtube.video_id",
  listType: "editor.youtube.list_type",
  notTopLevel: "editor.youtube.not_top_level",
} as const satisfies Record<string, EditorNotice>

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    youtubeEmbed: {
      /**
       * Inserts a YouTube embed at the selection, as the insert dialog
       * does. The title is stored as given; the dialog applies the title
       * rule first (D135).
       *
       * @param attributes - what the embed plays, and its title.
       * @returns `false`, changing nothing, when the server would refuse
       *   the address (D129, D132), or when the selection is not a place
       *   for media (`isMediaPlace`: a list item, quote, table cell or
       *   heading), so `editor.can()` disables the toolbar button there.
       */
      insertYoutubeEmbed: (attributes: YoutubeEmbedAttributes) => ReturnType
    }
  }
}

/** The attributes of a reading, or `false` for a parse rule to decline. */
function attributesOf(reading: YoutubeReading): YoutubeEmbedAttributes | false {
  return reading.kind === "video"
    ? { ...reading.video, title: reading.title }
    : false
}

/**
 * The notice for a lone URL the paste rule read: why it stays text, or the
 * dropped options of an embed.
 *
 * @param reading - what the reader made of the URL.
 * @returns the notice to show, or `null` for none.
 */
export function pasteNotice(reading: YoutubeReading): EditorNotice | null {
  switch (reading.kind) {
    case "refused":
      return reading.reason === "list_type"
        ? YOUTUBE_NOTICES.listType
        : YOUTUBE_NOTICES.videoId
    case "video":
      return reading.dropped.length > 0 ? YOUTUBE_NOTICES.optionsDropped : null
    default:
      return null
  }
}

/**
 * The YouTube embed node.
 *
 * @param policy - the content-v1 policy: the provider's hosts and id rules,
 *   the forced iframe attributes and the iframe's CSS grammar.
 * @param notify - shows the notices of the lone-URL paste rule.
 * @returns the node extension.
 * @throws {PolicyError} when the policy lacks a rule the node reads, or
 *   would store the player's style differently.
 */
export function createYoutubeEmbed(
  policy: RichTextPolicy,
  notify: NotifyEditor
) {
  const rules = youtubeRules(policy)
  const forced = forcedFrameAttributes(policy)
  const style = requireStoredStyle(policy, "iframe", YOUTUBE_STYLE)
  const fromIframe = (iframe: Element | null) =>
    iframe === null ? false : attributesOf(readYoutubeIframe(iframe, rules))
  // Each attribute is filled by the parse rules' `getAttrs` alone. TipTap
  // also reads an attribute of the same name from the matched element unless
  // `parseHTML` says otherwise, which would put an iframe's raw `title` or a
  // wrapper's stray attribute over what the reader read.
  const fromRules = { default: null, rendered: false, parseHTML: () => null }

  return Node.create({
    name: YOUTUBE_EMBED_NODE,
    group: MEDIA_GROUP,
    atom: true,
    draggable: true,

    addAttributes() {
      return {
        videoId: fromRules,
        start: fromRules,
        list: fromRules,
        title: fromRules,
      }
    },

    parseHTML() {
      return [
        {
          tag: "p[data-youtube-video]",
          priority: 70,
          getAttrs: (element) => fromIframe(wrapperIframe(element)),
        },
        {
          tag: "div[data-youtube-video]",
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
      const attributes = node.attrs as YoutubeEmbedAttributes
      // An address the server would refuse is never written; the parse
      // rules and the command only make valid ones.
      const src = isYoutubeVideo(attributes, rules)
        ? youtubeEmbedSrc(attributes)
        : null
      return [
        "p",
        { "data-youtube-video": "" },
        [
          "iframe",
          {
            src,
            ...YOUTUBE_FRAME_SIZE,
            title: attributes.title,
            style,
            ...forced,
          },
        ],
      ]
    },

    addCommands() {
      return {
        insertYoutubeEmbed:
          (attributes) =>
          ({ commands, tr }) =>
            canPlaceMedia(tr.selection) &&
            isYoutubeVideo(attributes, rules) &&
            commands.insertContent({ type: this.name, attrs: attributes }),
      }
    },

    addPasteRules() {
      return [
        new PasteRule({
          // Only a paste that is exactly one URL: when the clipboard event
          // is known, its whole text must be this textblock's text, so a
          // URL on one line of a longer paste stays text.
          find: (text, event) => {
            const trimmed = text.trim()
            const pasted = event?.clipboardData?.getData("text/plain")
            if (
              trimmed === "" ||
              (pasted !== undefined && pasted.trim() !== trimmed)
            ) {
              return null
            }
            const reading = readYoutubeUrl(trimmed, rules)
            return reading.kind === "none"
              ? null
              : [
                  {
                    index: text.indexOf(trimmed),
                    text: trimmed,
                    data: { reading },
                  },
                ]
          },
          handler: ({ state, match, chain, range }) => {
            const reading = match.data?.reading as YoutubeReading
            // In a list item, quote, table cell or heading the embed would
            // cut its container in two (§4.2), so the link stays text there,
            // as the toolbar's insert button is disabled there.
            const placeable = isMediaPlace(state.doc.resolve(range.from))
            const notice =
              reading.kind === "video" && !placeable
                ? YOUTUBE_NOTICES.notTopLevel
                : pasteNotice(reading)
            if (notice !== null) {
              notify(notice)
            }
            if (reading.kind !== "video" || !placeable) {
              // The link stays text; `null` tells TipTap nothing changed.
              return null
            }
            const attrs: YoutubeEmbedAttributes = {
              ...reading.video,
              title: YOUTUBE_DEFAULT_TITLE,
            }
            // As TipTap's `nodePasteRule` does: the chain works on the
            // paste rule's own transaction, so it is not run.
            chain()
              .deleteRange(range)
              .insertContentAt(range.from, { type: this.name, attrs })
          },
        }),
      ]
    },
  })
}
