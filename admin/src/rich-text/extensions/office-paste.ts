/**
 * `OfficePaste`, the site schema's paste extension (editor.md §4.3 row 11,
 * §5.1; ADR 0013 §3).
 *
 * It wires the paste filter into the editor's paste path, at the hooks of
 * editor.md §5.1:
 *
 * - **`transformPastedHTML`** (step 2), TipTap 3.32's extension-level hook,
 *   which TipTap chains with every other extension's: every HTML paste and
 *   every drop that carries HTML goes through `filterPastedHtml`, the paste
 *   filter with its plain-text fallback (§5.2, §5.5). Its outcome waits in
 *   the extension's storage for the handler below. When the filter falls
 *   back, the error is logged with the detected source.
 * - **`handlePaste` and `handleDrop`** (step 4): they take the waiting
 *   outcome and show its notices, then let ProseMirror insert the slice
 *   (they return `false`). The notices are the fallback's
 *   (`editor.paste.fallback`, §5.5), the YouTube dropped-options notice
 *   when the filter dropped a playback option (D134, §5.2 step 12), and the
 *   message of each kind of YouTube iframe the filter removed because the
 *   server refuses its address (a wrong id, a user-uploads or search list;
 *   §4.3 row 9, D129, D132), as the plain-text route and the lone-URL paste
 *   rule show it. The picture notice and the file rules of §5.3 join these
 *   handlers with the picture slice.
 * - **`clipboardTextParser`**, the plain-text route of ADR 0043 (D128,
 *   §5.1): a paste that has no `text/html` and whose trimmed text is exactly
 *   one `<iframe …></iframe>` with a YouTube or Padlet embed address (for
 *   YouTube `http` too, D136) is parsed in an inert document and goes
 *   through the schema's parse rules like an HTML paste (rule 60), with the
 *   title rule and the dropped-options notice of §4.3 row 9. A YouTube
 *   iframe the server refuses (a wrong id, a user-uploads or search list)
 *   stays text, and staff are told why, as the lone-URL paste rule does.
 *   So does embed code pasted or dropped where media cannot go (a list
 *   item, quote, table cell or heading, `isMediaPlace`), where the slice
 *   would cut the list, quote or table in two (§4.2). Anything else stays
 *   text, Padlet's `div`-wrapped snippet included, and so does every paste
 *   that ProseMirror marks as plain (Shift held, as in Ctrl+Shift+V).
 *
 * `handleDOMEvents.paste` and `drop` (step 1) so far only forget the last
 * outcome, should a plugin before this one have consumed that paste; the
 * stash of the clipboard's types and file count belongs to the file rules
 * of §5.3 and joins with them.
 */
import { Extension } from "@tiptap/core"
import type { ResolvedPos, Slice } from "@tiptap/pm/model"
import { DOMParser as SchemaParser } from "@tiptap/pm/model"
import type { Selection } from "@tiptap/pm/state"
import { Plugin, PluginKey } from "@tiptap/pm/state"
import type { EditorView } from "@tiptap/pm/view"

import { padletRules, readPadletAddress } from "../padlet-input"
import type { PasteFilter } from "../paste/normalize-pasted-html"
import type { PasteOutcome } from "../paste/paste-filter"
import { filterPastedHtml } from "../paste/paste-filter"
import type { RichTextPolicy } from "../policy"
import {
  embedCodeIframe,
  readYoutubeIframe,
  youtubeRules,
  youtubeTitle,
} from "../youtube-input"
import type { EditorNotice, NotifyEditor } from "./editor-notice"
import { isMediaPlace } from "./embed-frame"
import { MEDIA_GROUP } from "./site-document"
import { pasteNotice, YOUTUBE_NOTICES } from "./youtube-embed"

/** The extension's name, and its storage's key. */
export const OFFICE_PASTE_EXTENSION = "officePaste"

/** The notices of the paste extension's own. */
export const PASTE_NOTICES = {
  /** The plain-text fallback (editor.md §5.5). */
  fallback: "editor.paste.fallback",
  /**
   * Padlet's embed code pasted or dropped as plain text where media cannot
   * go (a list item, quote, table cell or heading), so it stayed text. The
   * YouTube code there gets `editor.youtube.not_top_level`, the lone-URL
   * paste rule's notice.
   */
  padletNotTopLevel: "editor.padlet.not_top_level",
} as const satisfies Record<string, EditorNotice>

/**
 * The notices of one paste's outcome, in the order they are shown: the
 * fallback's alone, or the dropped-options notice (D134) and then the
 * message of each refused YouTube address (D129, D132).
 *
 * @param outcome - the outcome of `transformPastedHTML`, or `null` when no
 *   HTML was filtered (a text paste, or a paste a plugin before this one
 *   consumed).
 * @returns the notices, none twice; empty when there is nothing to say.
 */
export function outcomeNotices(outcome: PasteOutcome | null): EditorNotice[] {
  if (outcome === null) {
    return []
  }
  if (outcome.kind === "fallback") {
    return [PASTE_NOTICES.fallback]
  }
  const { dropped } = outcome.report
  const notices: EditorNotice[] =
    dropped.embedParams.length > 0 ? [YOUTUBE_NOTICES.optionsDropped] : []
  for (const reason of dropped.youtubeRefusals) {
    const notice = pasteNotice({ kind: "refused", reason })
    if (notice !== null && !notices.includes(notice)) {
      notices.push(notice)
    }
  }
  return notices
}

/**
 * Whether the plain-text route may put a media node where a paste or a
 * drop puts its slice (editor.md §4.2): the place itself, and for a paste
 * the other end of the selection it replaces, are places for media
 * (`isMediaPlace`).
 *
 * prosemirror-view passes a paste the selection's start as its context
 * (prosemirror-view@1.42.7:src/input.ts, `doPaste`) and replaces the whole
 * selection with the slice, so a selection that ends in a table would cut
 * it in two; a drop passes the drop point and replaces nothing
 * (`handleDrop`). A drop that lands exactly at the selection's start is
 * checked like a paste, which can only refuse more.
 *
 * @param $context - the position prosemirror-view parses the text at.
 * @param selection - the editor's selection when the text arrives.
 * @returns `true` when the embed may go there.
 */
export function isEmbedCodePlace(
  $context: ResolvedPos,
  selection: Selection
): boolean {
  return (
    isMediaPlace($context) &&
    ($context.pos !== selection.from || isMediaPlace(selection.$to))
  )
}

/** What {@link createOfficePaste} is built from. */
export interface OfficePasteOptions {
  /** The content-v1 policy: pass `CONTENT_V1_POLICY` (editor.md §1.2 D6). */
  readonly policy: RichTextPolicy
  /** Shows a notice of the editor to the staff member. */
  readonly notify: NotifyEditor
  /**
   * The parser of the clipboard HTML and of embed code. Default: a
   * `DOMParser` made at the first paste, so a headless schema needs none.
   */
  readonly parser?: DOMParser
  /**
   * The paste filter. Default: `normalizePastedHtml`; the tests replace it
   * with one that throws to check the fallback (editor.md §13.5).
   */
  readonly filter?: PasteFilter
  /**
   * Reports a fallback's error. Default: `console.error`, which the admin's
   * error reporting sees.
   *
   * @param message - what happened, with the detected source.
   * @param error - what the filter threw.
   */
  readonly reportError?: (message: string, error: unknown) => void
}

/** The extension's storage: the outcome of the paste in progress. */
export interface OfficePasteStorage {
  /**
   * The outcome of the last `transformPastedHTML`, until `handlePaste` or
   * `handleDrop` shows its notices; `null` between pastes.
   */
  pending: PasteOutcome | null
}

declare module "@tiptap/core" {
  interface Storage {
    officePaste: OfficePasteStorage
  }
}

/**
 * The value that tells prosemirror-view a `clipboardTextParser` declined:
 * its `someProp` takes the first truthy result, and on a falsy one it pastes
 * the text as paragraphs (prosemirror-view@1.42.7:src/clipboard.ts,
 * `parseFromClipboard`). The prop's type says `Slice`, so the falsy value is
 * cast once, here.
 */
const DECLINE = null as unknown as Slice

/**
 * The paste extension.
 *
 * @param options - the policy, the notice callback and the replaceable
 *   dependencies ({@link OfficePasteOptions}).
 * @returns the extension.
 * @throws {TypeError} when the options lack the policy or the notice
 *   callback.
 * @throws {PolicyError} when the policy lacks the YouTube or Padlet rules.
 */
export function createOfficePaste(options: OfficePasteOptions) {
  if (
    typeof options !== "object" ||
    options === null ||
    !options.policy ||
    typeof options.notify !== "function"
  ) {
    throw new TypeError(
      "createOfficePaste: options.policy and options.notify are required; siteExtensions passes both."
    )
  }
  const { policy, notify, filter } = options
  const reportError =
    options.reportError ??
    ((message: string, error: unknown) => console.error(message, error))
  const youtube = youtubeRules(policy)
  const padlet = padletRules(policy)
  let parser: DOMParser | null = options.parser ?? null
  const getParser = () => (parser ??= new DOMParser())

  /** Shows the notices of one paste's outcome. */
  const showNotices = (outcome: PasteOutcome | null) => {
    for (const notice of outcomeNotices(outcome)) {
      notify(notice)
    }
  }

  /**
   * The plain-text route (D128): a slice with the one embed of embed code,
   * or `null` to let ProseMirror paste the text.
   */
  const embedCodeSlice = (
    text: string,
    $context: ResolvedPos,
    plain: boolean,
    view: EditorView
  ): Slice | null => {
    if (plain) {
      return null
    }
    let iframe: Element | null
    try {
      iframe = embedCodeIframe(text, getParser())
    } catch {
      return null
    }
    if (iframe === null) {
      return null
    }
    const reading = readYoutubeIframe(iframe, youtube)
    if (reading.kind === "refused") {
      const notice = pasteNotice(reading)
      if (notice !== null) {
        notify(notice)
      }
      return null
    }
    if (
      reading.kind === "none" &&
      readPadletAddress(iframe.getAttribute("src") ?? "", padlet) === null
    ) {
      return null
    }
    // In a list item, quote, table cell or heading the parsed slice would be
    // fitted in by cutting the block in two (§4.2), so the code stays text
    // there, as the lone-URL paste rule keeps a link (§4.3 row 9). Where the
    // embed goes instead is §5.3's placement, which joins handlePaste with
    // the picture slice.
    if (!isEmbedCodePlace($context, view.state.selection)) {
      notify(
        reading.kind === "video"
          ? YOUTUBE_NOTICES.notTopLevel
          : PASTE_NOTICES.padletNotTopLevel
      )
      return null
    }
    if (reading.kind === "video") {
      iframe.setAttribute("title", youtubeTitle(reading.title))
    }
    const container = iframe.ownerDocument.createElement("div")
    container.append(iframe)
    const slice = SchemaParser.fromSchema(view.state.schema).parseSlice(
      container,
      { context: $context }
    )
    const node = slice.content.firstChild
    if (
      slice.content.childCount !== 1 ||
      node === null ||
      !node.type.isInGroup(MEDIA_GROUP)
    ) {
      return null
    }
    if (reading.kind === "video" && reading.dropped.length > 0) {
      notify(YOUTUBE_NOTICES.optionsDropped)
    }
    return slice
  }

  return Extension.create<Record<string, never>, OfficePasteStorage>({
    name: OFFICE_PASTE_EXTENSION,

    addStorage() {
      return { pending: null }
    },

    transformPastedHTML(html) {
      let outcome: PasteOutcome
      try {
        outcome = filterPastedHtml(
          html,
          { policy, parser: getParser() },
          filter
        )
      } catch (error) {
        // Only a missing DOMParser gets here; filterPastedHtml never throws.
        outcome = { kind: "fallback", html: "", source: null, error }
      }
      if (outcome.kind === "fallback") {
        reportError(
          `OfficePaste: the paste filter failed on ${outcome.source ?? "unrecognised"} HTML, so only its text was pasted (editor.md §5.5).`,
          outcome.error
        )
      }
      // Through the editor, not a captured `this.storage`: TipTap re-creates
      // the storage objects when it re-registers extensions.
      this.editor.storage.officePaste.pending = outcome
      return outcome.html
    },

    addProseMirrorPlugins() {
      const { editor } = this
      const settle = () => {
        const storage = editor.storage.officePaste
        const outcome = storage.pending
        storage.pending = null
        showNotices(outcome)
        // ProseMirror inserts the slice itself.
        return false
      }
      // A plugin that comes before this one may consume a paste in its
      // `handlePaste` (as prosemirror-tables' does with a paste into selected
      // cells, should an extension of higher priority install it), and then
      // `settle` never runs. The DOM event comes before
      // `transformPastedHTML`, so forgetting the outcome there keeps an
      // earlier paste's notices from showing on a later one.
      const forget = () => {
        editor.storage.officePaste.pending = null
        // ProseMirror goes on to read the clipboard.
        return false
      }
      return [
        new Plugin({
          key: new PluginKey(OFFICE_PASTE_EXTENSION),
          props: {
            handleDOMEvents: { paste: forget, drop: forget },
            handlePaste: settle,
            handleDrop: settle,
            clipboardTextParser: (text, $context, plain, view) =>
              embedCodeSlice(text, $context, plain, view) ?? DECLINE,
          },
        }),
      ]
    },
  })
}
