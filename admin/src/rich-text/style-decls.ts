/**
 * The style reader of the schema's parse rules (editor.md §2 stage 3,
 * `readStyleDecls`), and the checks of content-v1's one spelling that the
 * render rules use.
 *
 * Parse rules read the `style` attribute as written, never through
 * `element.style`: the CSSOM re-spells values (`rgb()` for hex, grouped
 * borders) and differs between jsdom and the browsers, so a parse through it
 * would not be the same everywhere (editor.md §6).
 *
 * The raw attribute goes through `canonicalStyle` (editor.md §3.1, §4.3 row
 * 4) before a parse rule sees it, so a parse rule reads exactly the
 * declarations the server stores for the element, in the server's
 * spelling: another spelling (`color: red`, `TEXT-ALIGN: CENTER`) is read
 * as the server writes it, a later `background` sets `background-color`
 * as the server's cascade does, and a declaration the server drops is not
 * there. Every body the editor loads is already content-v1 (`serve_content`,
 * editor.md §8.2), so for a stored body this changes nothing; for the
 * editor's own clipboard and any other HTML it keeps what the server would
 * keep.
 *
 * `canonicalStyle`'s output uses only the canonical alphabet of §3.1 rule 6,
 * with no quotes, parentheses, `*`, `!` or `\`, so a plain split of it is
 * exact ({@link readCanonicalDecls}): `;` always ends a declaration and its
 * first `:` always ends the name.
 */
import type { RichTextPolicy, StyleGrammar } from "./policy"
import { canonicalStyle } from "./css/canonical-style"
import { MAX_DECLARATIONS } from "./css/declarations"

export { MAX_DECLARATIONS } from "./css/declarations"

/** The style grammars whose values this module checks. */
export type ScalarGrammar = Exclude<
  StyleGrammar,
  {
    readonly kind:
      "border_side" | "border_widths" | "border_styles" | "border_colours"
  }
>

/**
 * Splits a style string in the canonical alphabet into its declarations, in
 * source order, every occurrence kept.
 *
 * Names and values are trimmed; a piece without `:` or with an empty name or
 * value is skipped, as the server skips a declaration it cannot read. Only
 * the first {@link MAX_DECLARATIONS} non-empty pieces are read.
 *
 * @param style - the `style` attribute's value.
 * @param alphabet - every character a canonical style may contain
 *   (`policy.css.alphabet`).
 * @returns the `[property, value]` pairs, or `null` when `style` holds a
 *   character outside `alphabet`, so it is not content-v1 and nothing is read
 *   from it.
 */
function splitCanonicalStyle(
  style: string,
  alphabet: string
): [string, string][] | null {
  for (const character of style) {
    if (!alphabet.includes(character)) {
      return null
    }
  }
  const pairs: [string, string][] = []
  let counted = 0
  for (const piece of style.split(";")) {
    // Whitespace between two `;` is no declaration; anything else counts
    // towards the limit, whether it can be read or not, as on the server.
    if (piece.trim() === "") {
      continue
    }
    counted += 1
    if (counted > MAX_DECLARATIONS) {
      break
    }
    const colon = piece.indexOf(":")
    if (colon === -1) {
      continue
    }
    const property = piece.slice(0, colon).trim()
    const value = piece.slice(colon + 1).trim()
    if (property !== "" && value !== "") {
      pairs.push([property, value])
    }
  }
  return pairs
}

/**
 * The declarations of a style string in the canonical alphabet
 * ({@link splitCanonicalStyle}), the last occurrence of a property winning.
 *
 * @param style - the `style` attribute's value.
 * @param alphabet - the canonical alphabet (`policy.css.alphabet`).
 * @returns the declarations by property, in first-occurrence order, or
 *   `null` when `style` is outside the alphabet.
 */
export function readCanonicalDecls(
  style: string,
  alphabet: string
): Map<string, string> | null {
  const pairs = splitCanonicalStyle(style, alphabet)
  return pairs === null ? null : new Map(pairs)
}

/**
 * Whether `value` is a number in content-v1's spelling (editor.md §7.3,
 * "Numbers"): no sign, at most two decimals, no trailing zero and no
 * superfluous leading zero (`0`, `0.5`, `1.25`, `20`).
 */
const CANONICAL_NUMBER = /^(?:0|[1-9][0-9]*)(?:\.[0-9]?[1-9])?$/

/**
 * Reads a number in content-v1's spelling as hundredths, so that bounds are
 * compared exactly and never through binary fractions.
 *
 * @param text - the number's text.
 * @returns the value in hundredths, or `null` when `text` is not one.
 */
function hundredths(text: string): number | null {
  if (!CANONICAL_NUMBER.test(text)) {
    return null
  }
  const [whole, fraction = ""] = text.split(".")
  return Number(whole) * 100 + Number(fraction.padEnd(2, "0"))
}

/**
 * Whether `text` is a canonical number with `unit` whose value lies in
 * `[min, max]`.
 */
function inRange(
  text: string,
  unit: string,
  grammar: { readonly min: number; readonly max: number }
): boolean {
  if (!text.endsWith(unit)) {
    return false
  }
  const value = hundredths(text.slice(0, text.length - unit.length))
  return (
    value !== null &&
    value >= Math.round(grammar.min * 100) &&
    value <= Math.round(grammar.max * 100)
  )
}

/**
 * Whether `value` is a colour in its one spelling (editor.md §3.1 rule 3):
 * lower-case `#rrggbb`, `#rrggbbaa` when the alpha is below 1 (an `ff` alpha
 * is written as `#rrggbb`), or the keywords `transparent` and
 * `currentcolor`. An opaque-only colour (`span` `color`, §3.3) is `#rrggbb`
 * alone.
 */
function isCanonicalColour(value: string, opaqueOnly: boolean): boolean {
  if (/^#[0-9a-f]{6}$/.test(value)) {
    return true
  }
  if (opaqueOnly) {
    return false
  }
  return (
    (/^#[0-9a-f]{8}$/.test(value) && !value.endsWith("ff")) ||
    value === "transparent" ||
    value === "currentcolor"
  )
}

/**
 * Whether a grammar is one this module checks (every kind except the cell
 * border sides and their input-only shorthands).
 *
 * @param grammar - a property's grammar from the policy.
 * @returns `true` for a {@link ScalarGrammar}.
 */
export function isScalarGrammar(
  grammar: StyleGrammar
): grammar is ScalarGrammar {
  return !grammar.kind.startsWith("border_")
}

/**
 * Whether a value is accepted by a grammar and already written in the one
 * spelling content-v1 stores it in. A keyword that the grammar omits as the
 * default (`list-style-type: disc`) is not a stored value.
 *
 * @param grammar - the property's grammar from the policy.
 * @param value - the declaration's value, trimmed.
 * @returns `true` when the server would store exactly this value.
 */
export function isCanonicalValue(
  grammar: ScalarGrammar,
  value: string
): boolean {
  switch (grammar.kind) {
    case "keyword":
      return grammar.values.includes(value) && value !== grammar.omitted
    case "px":
      return inRange(value, "px", grammar)
    case "auto_or_px":
      return value === "auto" || inRange(value, "px", grammar)
    case "em":
      return inRange(value, "em", grammar)
    case "colour":
      return isCanonicalColour(value, grammar.opaque_only)
    case "locked":
      return value === grammar.value
  }
}

/**
 * The most style strings whose declarations {@link readStoredDeclarations}
 * keeps per policy. A parse reads each `span` style once for every span it
 * is nested in, and a body repeats few distinct styles, so a small memo
 * keeps deep nesting linear; when it is full it starts over.
 */
const MEMO_SIZE = 1024

/** The memo of {@link readStoredDeclarations}, per policy object. */
const memo = new WeakMap<
  RichTextPolicy,
  Map<string, ReadonlyMap<string, string>>
>()

/**
 * The declarations that the server stores for one element's style: the
 * output of `canonicalStyle`, by property, in its sorted order. Each value
 * is in content-v1's one spelling; a cell's border sides are full triples
 * or `none`/`hidden` (editor.md §3.1 rule 2).
 *
 * @param policy - the content-v1 policy.
 * @param tag - the element's tag name, lower case.
 * @param style - the `style` attribute's value, or `null` when there is none.
 * @returns the stored declarations; empty when the server keeps none. The
 *   map is shared between calls with the same style and must not be
 *   changed.
 * @throws {PolicyError} as `canonicalStyle` does.
 */
export function readStoredDeclarations(
  policy: RichTextPolicy,
  tag: string,
  style: string | null
): ReadonlyMap<string, string> {
  if (style === null) {
    return new Map()
  }
  let cache = memo.get(policy)
  if (cache === undefined) {
    cache = new Map()
    memo.set(policy, cache)
  }
  // A tag name holds no NUL, so the key is unambiguous.
  const key = `${tag}\u0000${style}`
  const cached = cache.get(key)
  if (cached !== undefined) {
    return cached
  }
  const css = canonicalStyle(policy, tag, style)
  const declarations: ReadonlyMap<string, string> =
    css === null
      ? new Map()
      : (readCanonicalDecls(css, policy.css.alphabet) ?? new Map())
  if (cache.size >= MEMO_SIZE) {
    cache.clear()
  }
  cache.set(key, declarations)
  return declarations
}

/**
 * Reads an element's raw `style` attribute ({@link readStoredDeclarations}).
 *
 * @param policy - the content-v1 policy.
 * @param element - the element a parse rule matched.
 * @returns the declarations the server stores for it.
 */
export function readStyleDecls(
  policy: RichTextPolicy,
  element: Element
): ReadonlyMap<string, string> {
  return readStoredDeclarations(
    policy,
    element.localName,
    element.getAttribute("style")
  )
}
