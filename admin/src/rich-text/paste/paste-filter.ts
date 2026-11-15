/**
 * The paste filter as the paste extension calls it: `normalizePastedHtml`
 * inside a `try`, with the plain-text fallback of editor.md §5.5 ("Last
 * line of defence", ADR 0013 §3).
 *
 * The filter is built never to throw, and the degrade-safely suite checks
 * that on every kind of input (§13.5). Should it throw anyway, on an input
 * nobody foresaw, the paste must not break and the editor must keep
 * running: {@link filterPastedHtml} then returns the parsed input's text
 * content, escaped, as plain paragraphs, which holds no tag, attribute or
 * style and so is content-v1 by construction. The caller logs the error
 * with the detected source and shows the Hungarian notice
 * `editor.paste.fallback`.
 */
import type { PasteReport } from "./paste-context"
import type { PasteFilter, PasteFilterOptions } from "./normalize-pasted-html"
import {
  normalizePastedHtml,
  preCleanPastedHtml,
} from "./normalize-pasted-html"
import type { PasteSource } from "./paste-source"
import { detectPasteSource } from "./paste-source"

/** What one paste's HTML became. */
export type PasteOutcome =
  | {
      /** The filter ran. */
      readonly kind: "filtered"
      /** The content-v1 HTML. */
      readonly html: string
      /** The detected source. */
      readonly source: PasteSource
      /** What the filter removed or moved. */
      readonly report: PasteReport
    }
  | {
      /** The filter threw; the text was kept. */
      readonly kind: "fallback"
      /** The text as plain paragraphs. */
      readonly html: string
      /** The detected source, or `null` when even that failed. */
      readonly source: PasteSource | null
      /** What the filter threw. */
      readonly error: unknown
    }

/** The elements whose text is not shown, left out of the fallback's text. */
const TEXTLESS =
  "head, script, style, template, noscript, noembed, noframes, title, xml, textarea, select, svg, math, object, embed, applet"

/** The elements after which the fallback starts a new line. */
const LINE_ENDS =
  "p, div, li, tr, h1, h2, h3, h4, h5, h6, blockquote, table, ul, ol, dt, dd, pre, br, hr"

/**
 * Escapes text for HTML: `&`, `<` and `>`.
 *
 * @param text - plain text.
 */
export function escapeText(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
}

/**
 * The plain-text fallback (editor.md §5.5): the text content of the parsed
 * input, one paragraph per line, escaped.
 *
 * @param html - the clipboard's `text/html`.
 * @param parser - an inert `DOMParser`.
 * @returns `<p>` elements holding only escaped text, or `""` when even
 *   parsing fails, so the paste inserts nothing rather than breaking.
 */
export function plainTextFallback(html: string, parser: DOMParser): string {
  try {
    const document = parser.parseFromString(
      preCleanPastedHtml(html),
      "text/html"
    )
    const body = document.body
    for (const element of [...body.querySelectorAll(TEXTLESS)]) {
      element.remove()
    }
    for (const element of [...body.querySelectorAll(LINE_ENDS)]) {
      element.after("\n")
    }
    return (body.textContent ?? "")
      .split(/\r\n?|\n/)
      .map((line) => line.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, ""))
      .filter((line) => line !== "")
      .map((line) => `<p>${escapeText(line)}</p>`)
      .join("")
  } catch {
    return ""
  }
}

/** The source of the input, or `null` when it cannot be read. */
function sourceOf(html: string, parser: DOMParser): PasteSource | null {
  try {
    return detectPasteSource(
      parser.parseFromString(preCleanPastedHtml(html), "text/html")
    )
  } catch {
    return null
  }
}

/**
 * Runs the paste filter, falling back to the text when it throws.
 *
 * @param html - the clipboard's `text/html`.
 * @param options - the filter's options ({@link PasteFilterOptions}).
 * @param filter - the filter; `normalizePastedHtml` unless a test replaces
 *   it to check the fallback.
 * @returns the outcome; never throws.
 */
export function filterPastedHtml(
  html: string,
  options: PasteFilterOptions,
  filter: PasteFilter = normalizePastedHtml
): PasteOutcome {
  try {
    const { html: filtered, source, report } = filter(html, options)
    return { kind: "filtered", html: filtered, source, report }
  } catch (error) {
    const text = typeof html === "string" ? html : ""
    // A missing parser makes both helpers give up inside their own `try`,
    // so the paste inserts nothing instead of throwing.
    const parser = options?.parser
    return {
      kind: "fallback",
      html: plainTextFallback(text, parser),
      source: sourceOf(text, parser),
      error,
    }
  }
}
