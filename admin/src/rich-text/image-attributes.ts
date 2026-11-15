/**
 * The image node's attributes and the rules on them that both the node
 * (`extensions/site-image.ts`) and its editor view (`views/SiteImageView.tsx`)
 * read (editor.md §4.3 row 8): the width presets, the missing-alt rule and
 * the stored size. A module of its own, so that the node and the view do not
 * import each other.
 */

/**
 * The width presets in px: a quarter, a half, three quarters and the whole
 * of the 700 px column (editor.md §4.3 row 8).
 */
export const IMAGE_WIDTH_PRESETS = [175, 350, 525, 700] as const

/** The widest preset, the column's width. */
export const IMAGE_COLUMN_WIDTH = 700

/** The state of an image whose file is being uploaded (§4.3 row 13). */
export type UploadState = "pending" | "failed"

/** The node's attributes. */
export interface SiteImageAttributes {
  /** The stored path of the image. */
  readonly src: string | null
  /** The alternative text, or `null` when it is missing. */
  readonly alt: string | null
  /** The shown width in px, or `null` for the image's own width. */
  readonly displayWidth: number | null
  /** Height over width, or `null` when it is not known. */
  readonly ratio: number | null
  /** An upload in progress or failed, or `null`; never stored. */
  readonly uploadState: UploadState | null
  /** The upload's local key, or `null`; never stored. */
  readonly localKey: string | null
}

/** The natural size of an image file, as an upload or the library gives it. */
export interface NaturalImage {
  /** The stored path. */
  readonly src: string
  /** The alternative text, or `null` when there is none yet. */
  readonly alt: string | null
  /** The natural width in px. */
  readonly width: number
  /** The natural height in px. */
  readonly height: number
}

/**
 * Whether an image lacks its alternative text: none, or only whitespace.
 * The editor shows the red badge then, and saving is refused (POS-C03).
 *
 * @param alt - the `alt` attribute's value, or `null`.
 * @returns `true` when the alt text is missing.
 */
export function isAltMissing(alt: string | null): boolean {
  return alt === null || alt.trim() === ""
}

/**
 * The `width` and `height` attributes of an image: the display width in
 * whole px, and the height from it and the ratio. A value outside the
 * attributes' range is left out, as the server would drop it.
 *
 * @param displayWidth - the shown width in px, or `null`.
 * @param ratio - height over width, or `null`.
 * @param range - the attributes' integer range (`policy.json`, `img`).
 * @returns the two attribute values, each `null` when it is not written.
 */
export function imageSize(
  displayWidth: number | null,
  ratio: number | null,
  range: { readonly min: number; readonly max: number }
): { readonly width: number | null; readonly height: number | null } {
  const inRange = (value: number) =>
    Number.isInteger(value) && value >= range.min && value <= range.max
  const width = displayWidth === null ? null : Math.round(displayWidth)
  if (width === null || !inRange(width)) {
    return { width: null, height: null }
  }
  const height =
    ratio === null || !Number.isFinite(ratio) ? null : Math.round(width * ratio)
  return { width, height: height !== null && inRange(height) ? height : null }
}
