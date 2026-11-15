/**
 * The image of the site schema (editor.md §4.3 row 8, §3.7; ADR 0013 §2).
 *
 * An own node: nothing is reused from stock `@tiptap/extension-image`,
 * whose attributes and inline placement content-v1 does not have. It is an
 * atom in the `media` group, so an image lives at the top level only, never
 * in a cell, list item, quote, heading or link (§4.2): Angular's page
 * renderer moves every `<img>` in front of its parent element. It writes
 * content-v1's one image shape (§3.7):
 *
 * ```html
 * <p><img src="/storage/uploads/…" alt="…" width="350" height="495" style="display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px"></p>
 * ```
 *
 * Its attributes:
 *
 * - `src`, the stored path, which must pass the server's image rule
 *   (`media-src.ts`); an image the server would refuse is not loaded;
 * - `alt`, the alternative text, `null` when it is missing. A missing or
 *   blank alt shows a red badge in the editor (`isAltMissing`), and the
 *   editor input refuses to save it (POS-C03, §4.5 `requireImageAlt`);
 * - `displayWidth`, the shown width in px: the style's `width`, else the
 *   `width` attribute within the style's range. The editor offers the
 *   presets 175, 350, 525 and 700 px, a quarter to the whole 700 px column;
 *   legacy widths up to the style's limit are kept;
 * - `ratio`, height over width, from the `width` and `height` attributes
 *   (the natural size scaled to the display width), so the box is known
 *   before the image loads;
 * - `uploadState` (`pending` or `failed`) and `localKey`, which the image
 *   uploads keep while a file is on its way (§4.3 row 13); neither is
 *   stored.
 *
 * The stored `height` is computed from the stored `width`, the display
 * width rounded to a whole pixel, and the ratio: a body loads back with the
 * same two numbers, so the shape is a fixed point (§6).
 *
 * The parse rules: ① a `p` whose only content is one `img[src]`, alone or
 * inside a chain of `a`, `strong`, `em` or `span`, which is dropped
 * (priority 60, the stored shape and legacy wrappers); ② a bare `img[src]`
 * (priority 50), a fallback only, because ProseMirror's own placement would
 * cut a table or list in two around it. Pasted outside pictures never reach
 * the schema (§5.3), and the import moves legacy images out of cells,
 * lists, quotes, headings and links (§9).
 *
 * In the editor the node is a React node view (`SiteImageView`) inside a
 * `p`, so the editor shows the stored paragraph's margins.
 */
import { Node } from "@tiptap/core"
import type { DOMOutputSpec } from "@tiptap/pm/model"
import { ReactNodeViewRenderer } from "@tiptap/react"

import { canonicalStyle } from "../css/canonical-style"
import type { NaturalImage, SiteImageAttributes } from "../image-attributes"
import { IMAGE_COLUMN_WIDTH, imageSize } from "../image-attributes"
import { imageSrc } from "../media-src"
import type { RichTextPolicy } from "../policy"
import { requireAttributeRule, requireStyleGrammar } from "../policy"
import { readStyleDecls } from "../style-decls"
import { SiteImageView } from "../views/SiteImageView"
import {
  canPlaceMedia,
  loneElementChild,
  requireStoredStyle,
} from "./embed-frame"
import { MEDIA_GROUP } from "./site-document"
import { readIntegerAttribute } from "./site-lists"

/** The node's name in the schema. */
export const SITE_IMAGE_NODE = "siteImage"

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    siteImage: {
      /**
       * Inserts an image at the selection at its natural width, at most the
       * column's 700 px, with its natural aspect ratio.
       *
       * @param image - the stored path, the alt text and the natural size.
       * @returns `false`, changing nothing, when the server would refuse
       *   the path, the size is not a positive whole number of px, or the
       *   selection is not a place for media (`isMediaPlace`: a list item,
       *   quote, table cell or heading), so `editor.can()` disables the
       *   toolbar button there.
       */
      insertSiteImage: (image: NaturalImage) => ReturnType
    }
  }
}

/**
 * The image in a `p` that holds only it, alone or inside a chain of `a`,
 * `strong`, `em` or `span` (parse rule ①).
 *
 * @param paragraph - the `p` a parse rule matched.
 * @returns the `img`, or `null` when the paragraph holds anything else.
 */
export function wrappedImage(paragraph: Element): Element | null {
  let element = loneElementChild(paragraph)
  while (
    element !== null &&
    ["a", "strong", "em", "span"].includes(element.localName)
  ) {
    element = loneElementChild(element)
  }
  return element?.localName === "img" && element.hasAttribute("src")
    ? element
    : null
}

/**
 * The image node.
 *
 * @param policy - the content-v1 policy: the image rule, the `width` and
 *   `height` attribute range and the `img` CSS grammar.
 * @returns the node extension.
 * @throws {PolicyError} when the policy lacks a rule the node reads.
 */
export function createSiteImage(policy: RichTextPolicy) {
  const widths = requireStyleGrammar(policy, "img", "width", "px")
  const sizeRule = requireAttributeRule(policy, "img", "width", "integer")
  requireAttributeRule(policy, "img", "height", "integer")
  requireAttributeRule(policy, "img", "src", "image_src")
  // The locked declarations alone, as the server writes an image without a
  // width; the node writes exactly this plus `width`.
  const lockedStyle = canonicalStyle(policy, "img", "") ?? ""
  requireStoredStyle(policy, "img", lockedStyle)
  const isDisplayWidth = (value: unknown): value is number =>
    typeof value === "number" &&
    Number.isFinite(value) &&
    value >= widths.min &&
    value <= widths.max
  const styleOf = (displayWidth: number | null) =>
    displayWidth === null
      ? lockedStyle
      : (canonicalStyle(policy, "img", `width: ${displayWidth}px`) ??
        lockedStyle)

  const fromImage = (image: Element | null): SiteImageAttributes | false => {
    const src =
      image === null
        ? null
        : imageSrc(image.getAttribute("src") ?? "", policy.urls)
    if (image === null || src === null) {
      return false
    }
    const styled = readStyleDecls(policy, image).get("width")
    const width = readIntegerAttribute(image.getAttribute("width"), sizeRule)
    const height = readIntegerAttribute(image.getAttribute("height"), sizeRule)
    const displayWidth =
      styled === undefined
        ? width === null
          ? null
          : Math.min(Math.max(width, widths.min), widths.max)
        : Number.parseFloat(styled)
    return {
      src,
      alt: image.getAttribute("alt"),
      displayWidth,
      ratio: width !== null && height !== null ? height / width : null,
      uploadState: null,
      localKey: null,
    }
  }
  // The parse rules alone fill the attributes (see youtube-embed.ts).
  const fromRules = { default: null, rendered: false, parseHTML: () => null }

  return Node.create({
    name: SITE_IMAGE_NODE,
    group: MEDIA_GROUP,
    atom: true,
    draggable: true,

    addAttributes() {
      return {
        src: fromRules,
        alt: fromRules,
        displayWidth: fromRules,
        ratio: fromRules,
        uploadState: fromRules,
        localKey: fromRules,
      }
    },

    parseHTML() {
      return [
        {
          tag: "p",
          priority: 60,
          getAttrs: (element) => fromImage(wrappedImage(element)),
        },
        {
          tag: "img[src]",
          priority: 50,
          getAttrs: (element) => fromImage(element),
        },
      ]
    },

    renderHTML({ node }): DOMOutputSpec {
      const attributes = node.attrs as SiteImageAttributes
      const displayWidth = isDisplayWidth(attributes.displayWidth)
        ? attributes.displayWidth
        : null
      const size = imageSize(displayWidth, attributes.ratio, sizeRule)
      const src =
        attributes.src === null ? null : imageSrc(attributes.src, policy.urls)
      return [
        "p",
        {},
        [
          "img",
          {
            src,
            alt: attributes.alt,
            width: size.width === null ? null : String(size.width),
            height: size.height === null ? null : String(size.height),
            style: styleOf(displayWidth),
          },
        ],
      ]
    },

    addNodeView() {
      // The wrapper is a `p`, so the editor shows the stored paragraph's
      // margins; the view itself writes phrasing content only.
      return ReactNodeViewRenderer(SiteImageView, { as: "p" })
    },

    addCommands() {
      return {
        insertSiteImage:
          (image) =>
          ({ commands, tr }) => {
            const src = imageSrc(image.src, policy.urls)
            const sized = [image.width, image.height].every(
              (value) => Number.isSafeInteger(value) && value > 0
            )
            if (src === null || !sized || !canPlaceMedia(tr.selection)) {
              return false
            }
            const attrs: SiteImageAttributes = {
              src,
              alt: image.alt,
              displayWidth: Math.min(image.width, IMAGE_COLUMN_WIDTH),
              ratio: image.height / image.width,
              uploadState: null,
              localKey: null,
            }
            return commands.insertContent({ type: this.name, attrs })
          },
      }
    },
  })
}
