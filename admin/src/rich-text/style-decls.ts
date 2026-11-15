/**
 * The raw style reader of the schema's parse rules (editor.md §2 stage 3,
 * `readStyleDecls`).
 *
 * Parse rules read the `style` attribute as written, never through
 * `element.style`: the CSSOM re-spells values (`rgb()` for hex, grouped
 * borders) and differs between jsdom and the browsers, so a parse through it
 * would not be the same everywhere (editor.md §6).
 *
 * This reader accepts content-v1's own spelling only, the output of the
 * server's grammar (editor.md §3.1). Every body the editor loads is
 * sanitised content-v1 (`serve_content`, editor.md §8.2), and the paste
 * filter writes content-v1 before the parse (§5.2 step 8), so that is the
 * whole input of stage 3. Restricting the reader to the canonical alphabet
 * of §3.1 rule 6 is also what makes a plain split exact: without quotes,
 * parentheses, `*`, `!` or `\`, a style string holds no CSS string, comment,
 * function, escape or `!important`, so `;` always ends a declaration and its
 * first `:` always ends the name. Values are then checked against the
 * policy's grammar in its one spelling ({@link isCanonicalValue}), so a
 * declaration the server would rewrite or drop never becomes a schema
 * attribute. Reading other spellings is `canonicalStyle`'s job (editor.md
 * §3.1, §6), which turns them into this one before a parse rule sees them.
 *
 * The declarations are resolved in source order, as the server's cascade
 * resolves them: an input-only shorthand sets the stored property it
 * expands into (`background` sets `background-color`, §3.1 rule 1), so a
 * later shorthand overrides an earlier stored declaration. This reader does
 * not read shorthands, so it then keeps nothing of that property.
 */
import type { RichTextPolicy, StyleGrammar } from "./policy"
import { styleRuleOf } from "./policy"

/** The style grammars whose values this module checks. */
export type ScalarGrammar = Exclude<
  StyleGrammar,
  {
    readonly kind:
      "border_side" | "border_widths" | "border_styles" | "border_colours"
  }
>

/**
 * The most declarations one `style` attribute may hold; later ones are not
 * read (editor.md §7.3, "Limits"). Like the server's other reader limits it
 * is not in `policy.json`; `css-vectors.jsonl` pins it.
 */
export const MAX_DECLARATIONS = 32

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
 * Reads an element's raw `style` attribute ({@link readCanonicalDecls}).
 *
 * @param element - the element a parse rule matched.
 * @param alphabet - the canonical alphabet (`policy.css.alphabet`).
 * @returns the declarations; empty when the element has no `style` or one
 *   outside the alphabet.
 */
export function readStyleDecls(
  element: Element,
  alphabet: string
): Map<string, string> {
  return (
    readCanonicalDecls(element.getAttribute("style") ?? "", alphabet) ??
    new Map()
  )
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
 * The stored property that each input-only shorthand with a scalar grammar
 * sets (editor.md §3.1 rule 1: "A single-colour `background` becomes
 * `background-color`"). `policy.json` lists a shorthand's grammar but not
 * its target, so the mapping is written here, as the server's
 * `stored_name` (`src/domain/rich_text/css/canonical.rs`) writes it. The
 * border shorthands are not listed: they set the cell border sides, which
 * this reader never returns ({@link resolveStoredDeclarations}).
 */
export const SHORTHAND_TARGETS: Readonly<Record<string, string>> = {
  background: "background-color",
}

/**
 * Resolves the declarations of a style string for one element in source
 * order, as the server's cascade does, and says for each stored property
 * whether its winning declaration is one this reader keeps.
 *
 * For every stored property the style sets, the last declaration that sets
 * it decides:
 *
 * - the property itself with a canonical value: that value;
 * - the property with a value outside its grammar or in another spelling,
 *   or an input-only shorthand that sets it (`background`): `null`. The
 *   server may keep an earlier declaration, rewrite this one or expand the
 *   shorthand, so this reader keeps nothing rather than something else.
 *   Keeping the last valid occurrence instead would keep `#c00000` of
 *   `color: #c00000; color: red`, which the server stores as `#ff0000`, and
 *   `#00ff00` of `background-color: #00ff00; background: #ff0000`, which it
 *   stores as `#ff0000`.
 *
 * Properties the element does not store, and the cell border shorthands
 * (whose sides this reader never returns), set nothing here.
 *
 * @param policy - the content-v1 policy.
 * @param tag - the element's tag name, lower case.
 * @param style - the `style` attribute's value, or `null` when there is none.
 * @returns the stored properties the style sets, in first-setting order,
 *   each with its kept value or `null`. Empty when there is no style or it
 *   is outside the canonical alphabet.
 * @throws {Error} when the element stores a property whose grammar this
 *   module does not check (a cell border side), or the style holds an
 *   input-only shorthand with a scalar grammar that {@link SHORTHAND_TARGETS}
 *   does not map: either would silently misread a declaration the server
 *   keeps.
 */
export function resolveStoredDeclarations(
  policy: RichTextPolicy,
  tag: string,
  style: string | null
): Map<string, string | null> {
  const resolved = new Map<string, string | null>()
  const pairs =
    style === null ? null : splitCanonicalStyle(style, policy.css.alphabet)
  for (const [property, value] of pairs ?? []) {
    const rule = styleRuleOf(policy, tag, property)
    if (rule === undefined) {
      continue
    }
    if (rule.role === "input_only") {
      if (!isScalarGrammar(rule.grammar)) {
        // `border`, `border-width`, `border-style`, `border-color`: they
        // set the cell sides, which a stored border side below refuses to
        // read, so they cannot change anything this reader returns.
        continue
      }
      // Own keys only, so a property named like an Object method is unknown.
      const target = Object.hasOwn(SHORTHAND_TARGETS, property)
        ? SHORTHAND_TARGETS[property]
        : undefined
      if (target === undefined) {
        throw new Error(
          `resolveStoredDeclarations: ${tag} ${property} is an input-only shorthand whose stored property is unknown; add it to SHORTHAND_TARGETS as the server's cascade maps it (editor.md §3.1 rule 1).`
        )
      }
      // Re-inserting moves nothing: a Map keeps first-insertion order.
      resolved.set(target, null)
      continue
    }
    if (!isScalarGrammar(rule.grammar)) {
      throw new Error(
        `resolveStoredDeclarations: ${tag} ${property} has the grammar ${rule.grammar.kind}, which this reader does not check; read cell borders with a reader that implements editor.md §3.1 rule 2.`
      )
    }
    resolved.set(property, isCanonicalValue(rule.grammar, value) ? value : null)
  }
  return resolved
}

/**
 * The stored declarations of a style string for one element: those whose
 * property the policy stores on that element, whose winning declaration
 * sets that property itself and whose value is canonical
 * ({@link resolveStoredDeclarations}). Anything else (another property, an
 * input-only shorthand, a value outside the grammar or in another spelling)
 * is left out.
 *
 * Every declaration it keeps is one the server stores with the same value
 * (`css-vectors.jsonl` checks it); it may keep less than the server, never
 * something else.
 *
 * @param policy - the content-v1 policy.
 * @param tag - the element's tag name, lower case.
 * @param style - the `style` attribute's value, or `null` when there is none.
 * @returns the declarations by property, in first-setting order.
 * @throws {Error} as {@link resolveStoredDeclarations} does.
 */
export function readStoredDeclarations(
  policy: RichTextPolicy,
  tag: string,
  style: string | null
): Map<string, string> {
  return keptDeclarations(resolveStoredDeclarations(policy, tag, style))
}

/**
 * The declarations of a resolution that are kept, without the refused ones.
 *
 * @param resolved - a resolution from {@link resolveStoredDeclarations}, or
 *   several folded together.
 * @returns the properties whose value is not `null`, in the same order.
 */
export function keptDeclarations(
  resolved: ReadonlyMap<string, string | null>
): Map<string, string> {
  const kept = new Map<string, string>()
  for (const [property, value] of resolved) {
    if (value !== null) {
      kept.set(property, value)
    }
  }
  return kept
}
