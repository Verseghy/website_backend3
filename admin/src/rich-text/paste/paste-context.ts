/**
 * What the steps of the paste filter share while they run over one paste:
 * the policy and the readers built from it, the detected source, and the
 * report they fill in (editor.md §5.2 step 12).
 *
 * The report is for the paste handlers and the tests: `handlePaste` shows
 * the dropped-options notice (§4.3 row 9, D134) and the message of each
 * refused YouTube address (D129, D132) from it, and the tests assert what
 * each step removed or moved. The picture counts of §5.3 and the files of
 * §5.1 join it with the picture rules, which are not part of the generic
 * path's first slice.
 *
 * Its shape as built differs from §5.2 step 12's sketch, as §5.5 ("As
 * built") records: the detected source sits beside the report, in
 * `NormalizedPaste`, not inside it; `dropped` also counts hidden cells,
 * other hidden elements and removed iframes, and lists the kinds of
 * refused YouTube addresses.
 */
import type { PadletRules } from "../padlet-input"
import type { RichTextPolicy } from "../policy"
import type {
  DroppedOption,
  YoutubeRefusal,
  YoutubeRules,
} from "../youtube-input"
import { DROPPED_OPTIONS } from "../youtube-input"
import type { PasteSource } from "./paste-source"

/** The kind of a media node that step 11a placed. */
export type PastedMedia = "image" | "youtube" | "padlet"

/** The kinds of refused YouTube addresses, in the order their notices show. */
export const YOUTUBE_REFUSALS: readonly YoutubeRefusal[] = [
  "video_id",
  "list_type",
]

/** One image or embed that step 11a moved to the top level. */
export interface MovedMedia {
  /** What was moved. */
  readonly media: PastedMedia
  /**
   * Where it was: the tag of the element it was taken out of (`td`, `th`,
   * `li`, `blockquote`, `h1`-`h4` or `a`), or `text` when it split a
   * paragraph of running text in two.
   */
  readonly from: string
}

/** What the filter removed or moved, besides the markup it rewrote. */
export interface PasteReport {
  /** Media moved to the top level (step 11a). */
  readonly moved: { readonly media: readonly MovedMedia[] }
  /** What was left out. */
  readonly dropped: {
    /** Hidden table rows (step 7). */
    readonly hiddenRows: number
    /** Hidden table cells, the cells of hidden columns (step 7). */
    readonly hiddenCells: number
    /** Other hidden elements (step 7). */
    readonly hiddenOther: number
    /**
     * The distinct `font-family` values the pasted elements carried, as the
     * engine's CSSOM serialises them, in document order. content-v1 stores
     * no font (§3.3), so every one of them is dropped, whatever the source.
     */
    readonly fontFamilies: readonly string[]
    /**
     * `iframe`s whose address is no embed the server keeps (editor.md
     * §3.6): another host, a refused YouTube id or list type, a broken
     * address. They are removed with nothing in their place.
     */
    readonly frames: number
    /**
     * Why the YouTube iframes among `frames` were refused (D129, D132),
     * each kind once, in the order of {@link YOUTUBE_REFUSALS}.
     * `handlePaste` shows the message of each, as the plain-text route and
     * the lone-URL paste rule do (§4.3 row 9).
     */
    readonly youtubeRefusals: readonly YoutubeRefusal[]
    /**
     * The YouTube player options that change playback and were dropped
     * (D134), each once, in the order of `DROPPED_OPTIONS`. When it is not
     * empty, `handlePaste` shows the notice of editor.md §4.3 row 9.
     */
    readonly embedParams: readonly DroppedOption[]
  }
}

/** The report while the steps fill it in. */
export interface PasteReportDraft {
  hiddenRows: number
  hiddenCells: number
  hiddenOther: number
  /** A `Set` keeps insertion order, so the report lists them as found. */
  readonly fontFamilies: Set<string>
  frames: number
  readonly youtubeRefusals: Set<YoutubeRefusal>
  readonly embedParams: Set<DroppedOption>
  readonly moved: MovedMedia[]
}

/** What every step of one paste reads. */
export interface PasteContext {
  /** The content-v1 policy (`CONTENT_V1_POLICY`). */
  readonly policy: RichTextPolicy
  /** The source step 1 detected. */
  readonly source: PasteSource
  /** The policy's YouTube rules, for the iframe addresses. */
  readonly youtube: YoutubeRules
  /** The policy's Padlet rules, for the iframe addresses. */
  readonly padlet: PadletRules
  /** The report the steps fill in. */
  readonly report: PasteReportDraft
}

/** An empty report draft. */
export function emptyReportDraft(): PasteReportDraft {
  return {
    hiddenRows: 0,
    hiddenCells: 0,
    hiddenOther: 0,
    fontFamilies: new Set(),
    frames: 0,
    youtubeRefusals: new Set(),
    embedParams: new Set(),
    moved: [],
  }
}

/**
 * The finished report of a draft.
 *
 * @param draft - the draft the steps filled in.
 * @returns the report, with the font families in the order they were
 *   found, the refusals in `YOUTUBE_REFUSALS` order and the dropped options
 *   in `DROPPED_OPTIONS` order.
 */
export function finishReport(draft: PasteReportDraft): PasteReport {
  return {
    moved: { media: [...draft.moved] },
    dropped: {
      hiddenRows: draft.hiddenRows,
      hiddenCells: draft.hiddenCells,
      hiddenOther: draft.hiddenOther,
      fontFamilies: [...draft.fontFamilies],
      frames: draft.frames,
      youtubeRefusals: YOUTUBE_REFUSALS.filter((reason) =>
        draft.youtubeRefusals.has(reason)
      ),
      embedParams: DROPPED_OPTIONS.filter((option) =>
        draft.embedParams.has(option)
      ),
    },
  }
}
