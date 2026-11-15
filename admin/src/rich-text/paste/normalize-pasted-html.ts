/**
 * `normalizePastedHtml`, stage 2 of the editor's pipeline (editor.md §2,
 * §5.2): clipboard HTML in, content-v1 HTML out, before ProseMirror parses
 * it with the site schema.
 *
 * It is the paste filter of ADR 0013 §3: pure apart from the `DOMParser`
 * its caller passes in (whose documents run no script and load no image),
 * synchronous, and built so that every source gives sanitised content-v1
 * (P-paste: only content-v1 tags, attributes and properties, and no
 * `<style>`). The steps, in order, as this module runs them:
 *
 * | Step | Module | Runs for |
 * |---|---|---|
 * | 2 Pre-clean: cut everything after `</html>` | here | every source |
 * | 3 Parse the whole string | here | every source |
 * | 1 Detect the source | `paste-source.ts` | every source |
 * | 6 Inline every `<style>` rule, then remove every `<style>` | `inline-styles.ts` | all but the editor's own clipboard |
 * | 7 Remove hidden content | `hidden-content.ts` | all but the editor's own clipboard |
 * | 10, 8, 9 Structure, declarations, inline marks | `paste-structure.ts`, `paste-declarations.ts`, `inline-marks.ts` | every source |
 * | 11a Media placement | `media-placement.ts` | every source |
 * | 12 Return the HTML, the source and the report | here | every source |
 *
 * Step 1 reads the parsed document, so it follows the parse (§5.2 step 1
 * names attributes of the root element and of `<meta>`, which the parser
 * reads more reliably than a regular expression). Step 2 is a no-op on the
 * editor's own clipboard, which has no `</html>`.
 *
 * **What the first slice of the filter leaves out.** This is the generic
 * path (§5.5). The source detection already tells Word and Excel desktop
 * apart, and they run the generic steps until their own rules land: the
 * stash of Office-only CSS and Word's lists (steps 4 and 5), Word's `%`
 * widths, Excel's border weights, base-rule values and General alignment
 * (step 8), marks per text run relative to the source's base size (step 9;
 * `inline-marks.ts` maps marks element by element meanwhile), Word's
 * `mso-spacerun` (step 10) and the picture rules (step 11, §5.3). None of
 * them changes what this module returns for a generic source.
 *
 * Never call it directly from a paste handler: `paste-filter.ts` wraps it
 * so that no clipboard content can break a paste (§5.5, "Last line of
 * defence").
 */
import type { RichTextPolicy } from "../policy"
import { padletRules } from "../padlet-input"
import { youtubeRules } from "../youtube-input"
import { removeHiddenContent } from "./hidden-content"
import { inlineStyleSheets } from "./inline-styles"
import { placeMedia } from "./media-placement"
import type { PasteContext, PasteReport } from "./paste-context"
import { emptyReportDraft, finishReport } from "./paste-context"
import type { PasteSource } from "./paste-source"
import { detectPasteSource } from "./paste-source"
import { normalizeStructure } from "./paste-structure"

/** What the paste filter is built from (editor.md §1.2 D6). */
export interface PasteFilterOptions {
  /** The content-v1 policy: pass `CONTENT_V1_POLICY` from `../policy`. */
  readonly policy: RichTextPolicy
  /**
   * The parser of the clipboard HTML. Its documents have no browsing
   * context, so nothing in them loads or runs; pass `new DOMParser()`.
   */
  readonly parser: DOMParser
}

/** What the paste filter returns (editor.md §5.2 step 12). */
export interface NormalizedPaste {
  /** The content-v1 HTML that ProseMirror parses. */
  readonly html: string
  /** The source step 1 detected. */
  readonly source: PasteSource
  /** What the steps removed or moved. */
  readonly report: PasteReport
}

/** The signature of the paste filter, which the paste extension takes. */
export type PasteFilter = (
  html: string,
  options: PasteFilterOptions
) => NormalizedPaste

/**
 * Step 2: cuts everything after the first `</html>` end tag. Windows
 * clipboard HTML can carry bytes after the document, which the HTML parser
 * would otherwise put into the body. The string is never sliced on
 * `<!--StartFragment-->`: Excel desktop places that comment inside
 * `<table>` (§5.2 step 2).
 *
 * @param html - the clipboard's `text/html`.
 * @returns the HTML up to and including `</html>`, or all of it.
 */
export function preCleanPastedHtml(html: string): string {
  const end = /<\/html\s*>/i.exec(html)
  return end === null ? html : html.slice(0, end.index + end[0].length)
}

/**
 * Checks the options at the module's boundary.
 *
 * @throws {TypeError} naming what is missing.
 */
function checkOptions(options: PasteFilterOptions): void {
  if (typeof options !== "object" || options === null || !options.policy) {
    throw new TypeError(
      "normalizePastedHtml: options.policy is required; pass CONTENT_V1_POLICY from admin/src/rich-text/policy.ts."
    )
  }
  if (typeof options.parser?.parseFromString !== "function") {
    throw new TypeError(
      "normalizePastedHtml: options.parser must be a DOMParser; pass new DOMParser()."
    )
  }
}

/**
 * The paste filter (editor.md §5.2): clipboard HTML to content-v1.
 *
 * @param html - the clipboard's `text/html`, as prosemirror-view passes it
 *   to `transformPastedHTML`.
 * @param options - the policy and the parser ({@link PasteFilterOptions}).
 * @returns the content-v1 HTML, the detected source and the report.
 * @throws {TypeError} when `html` is not a string or an option is missing.
 * @throws {PolicyError} when the policy lacks a rule the filter reads.
 */
export function normalizePastedHtml(
  html: string,
  options: PasteFilterOptions
): NormalizedPaste {
  if (typeof html !== "string") {
    throw new TypeError(
      `normalizePastedHtml: html must be the clipboard's text/html as a string, not ${typeof html}.`
    )
  }
  checkOptions(options)
  const { policy, parser } = options
  const document = parser.parseFromString(preCleanPastedHtml(html), "text/html")
  const source = detectPasteSource(document)
  const context: PasteContext = {
    policy,
    source,
    youtube: youtubeRules(policy),
    padlet: padletRules(policy),
    report: emptyReportDraft(),
  }
  const body = document.body
  if (source !== "internal") {
    inlineStyleSheets(document)
    const hidden = removeHiddenContent(body)
    context.report.hiddenRows = hidden.rows
    context.report.hiddenCells = hidden.cells
    context.report.hiddenOther = hidden.other
  }
  normalizeStructure(body, context)
  placeMedia(body, context)
  return { html: body.innerHTML, source, report: finishReport(context.report) }
}
