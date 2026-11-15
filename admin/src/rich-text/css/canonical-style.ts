/**
 * `canonicalStyle`: the admin's half of content-v1's one spelling of every
 * style (editor.md §3.1). The server's grammar
 * (`src/domain/rich_text/css/`) is the other half; editor.md §3.1 calls them
 * "the same rules written twice", and both must write the output of every
 * line of `css-vectors.jsonl` (`canonical-style.test.ts` checks it here).
 *
 * The stored-HTML serialiser runs every `style` it writes through it
 * (editor.md §6 step 2.1), and the schema's parse rules read every `style`
 * through it (`style-decls.ts`), so a body pasted or loaded in another
 * spelling (`rgb()`, named colours, `pt`, shorthands) keeps what the server
 * would keep, in the spelling the server would write.
 *
 * The work mirrors the server's modules:
 *
 * - `declarations.ts` reads the attribute into declarations;
 * - {@link readValueWith} reads each value with the element's grammar from
 *   the policy (editor.md §3.3, the `style` lists of `policy.json`);
 * - `Cascade` resolves the accepted declarations in source order with CSS's
 *   cascade (§3.1 rule 1): the last valid declaration of a property wins, a
 *   rejected one leaves an earlier valid one in force, a default unsets the
 *   property, `background` sets `background-color`, and the cell border
 *   shorthands set the twelve longhands of the four sides, a part they leave
 *   out at CSS's initial value and a part no declaration sets at the page's
 *   `border: 0` (`css.border.initial` and `css.border.unset`);
 * - `Cascade.write` writes the stored declarations: every border side as a
 *   full triple or `none`/`hidden` once any border declaration was accepted,
 *   the locked declarations that are always present, `box-sizing` exactly
 *   when a cell has a size, sorted by property, `prop: value` joined with
 *   `; `, and nothing at all when nothing is left (§3.1 rules 2 and 5);
 * - the alphabet guard of §3.1 rule 6 drops a string outside
 *   `css.alphabet`, which the grammar never writes.
 *
 * Every rule of the grammar comes from the policy argument (editor.md §1.2
 * D6). What `policy.json` does not hold is written here as on the server:
 * the reader rules of `declarations.ts` and `values.ts`, the four side names
 * in CSS's top, right, bottom, left order, and which stored property a
 * scalar shorthand sets ({@link SHORTHAND_TARGETS}).
 */
import type { RichTextPolicy, StyleGrammar, StyleRule } from "../policy"
import { elementOf, PolicyError } from "../policy"
import type { Component } from "./declarations"
import { readDeclarations, readValue } from "./declarations"
import type { Colour } from "./values"
import {
  emSize,
  isOpaque,
  lengthPx,
  percentage,
  readColour,
  roundHundredths,
  unitlessNumber,
  writeColour,
  writeHundredths,
} from "./values"

/** The result of filtering one `style` attribute, as the server reports it. */
export interface CanonicalStyle {
  /**
   * The canonical style string to store, or `null` when nothing is left and
   * the attribute is removed (content-v1 never stores `style=""`).
   */
  readonly css: string | null
  /**
   * How many declarations were removed: unreadable, not allowed on the
   * element, outside their grammar, or after the first 32. A default that
   * is left out, a cell `box-sizing` without a size, an occurrence that a
   * later one replaces and an expanded shorthand are not counted.
   */
  readonly rejected: number
  /**
   * Whether the written string broke the alphabet guard (editor.md §3.1 rule
   * 6) and was dropped. The grammar never writes such a string, so this is
   * a bug.
   */
  readonly outsideAlphabet: boolean
}

/**
 * The stored property that each input-only shorthand with a scalar grammar
 * sets (editor.md §3.1 rule 1: "A single-colour `background` becomes
 * `background-color`"). `policy.json` lists a shorthand's grammar but not its
 * target, so the mapping is written here, as the server's `stored_name`
 * (`src/domain/rich_text/css/canonical.rs`) writes it. The cell border
 * shorthands are not listed: their values say which sides they set.
 */
export const SHORTHAND_TARGETS: Readonly<Record<string, string>> = {
  background: "background-color",
}

/** The four cell sides, in the order of CSS's one-to-four value shorthands. */
export const BORDER_SIDES = [
  "border-top",
  "border-right",
  "border-bottom",
  "border-left",
] as const

/** The indexes into {@link BORDER_SIDES} that a border-side shorthand sets. */
const SIDES_NAMED: Readonly<Record<string, readonly number[]>> = {
  border: [0, 1, 2, 3],
  "border-top": [0],
  "border-right": [1],
  "border-bottom": [2],
  "border-left": [3],
}

/** One border side as it was read: each part only when the input had it. */
interface BorderSideInput {
  width: number | null
  style: string | null
  colour: Colour | null
}

/** One cell side after the cascade. */
interface Side {
  /** The width in hundredths of a px. */
  readonly width: number
  /** The style keyword. */
  readonly style: string
  /** The colour. */
  readonly colour: Colour
}

/** An accepted value, typed; each kind has one spelling. */
type Value =
  | { readonly kind: "text"; readonly text: string }
  | { readonly kind: "px"; readonly hundredths: number }
  | { readonly kind: "em"; readonly hundredths: number }
  | { readonly kind: "colour"; readonly colour: Colour }
  | { readonly kind: "side"; readonly side: BorderSideInput }
  | { readonly kind: "widths"; readonly list: readonly number[] }
  | { readonly kind: "styles"; readonly list: readonly string[] }
  | { readonly kind: "colours"; readonly list: readonly Colour[] }

/** What a grammar made of a value. */
type Reading = Value | "default" | "rejected"

/** The parts of the policy the grammar reads, prepared once per policy. */
interface Grammar {
  readonly policy: RichTextPolicy
  readonly borderStyles: readonly string[]
  readonly widthKeywords: ReadonlyMap<string, number>
  readonly maxBorderWidth: number
  readonly initial: Side
  readonly unset: Side
}

/** The prepared grammar of each policy object. */
const prepared = new WeakMap<RichTextPolicy, Grammar>()

/**
 * A border part of the policy (`css.border.initial` or `.unset`) as a side.
 *
 * @throws {PolicyError} when its colour is not a colour.
 */
function sideOf(
  parts: RichTextPolicy["css"]["border"]["initial"],
  path: string
): Side {
  const colour = readColour({ kind: "ident", value: parts.colour })
  const width = roundHundredths(parts.width)
  if (colour === null || width === null) {
    throw new PolicyError(
      `policy.json: css.border.${path} must be a width in px and a colour keyword, not ${JSON.stringify(parts)} (editor.md §3.9).`
    )
  }
  return { width, style: parts.style, colour }
}

/**
 * Prepares the grammar of a policy, and checks the one thing this module
 * cannot follow on its own: every input-only shorthand with a scalar grammar
 * must have a target in {@link SHORTHAND_TARGETS}.
 *
 * @throws {PolicyError} naming the shorthand the admin cannot expand.
 */
function grammarOf(policy: RichTextPolicy): Grammar {
  const cached = prepared.get(policy)
  if (cached !== undefined) {
    return cached
  }
  for (const [tag, element] of Object.entries(policy.elements)) {
    for (const rule of element.style ?? []) {
      if (
        rule.role === "input_only" &&
        !rule.grammar.kind.startsWith("border_") &&
        !Object.hasOwn(SHORTHAND_TARGETS, rule.property)
      ) {
        throw new PolicyError(
          `${tag} ${rule.property} is an input-only shorthand whose stored property canonicalStyle does not know; add it to SHORTHAND_TARGETS as the server's cascade maps it (editor.md §3.1 rule 1).`
        )
      }
    }
  }
  const { border } = policy.css
  const toHundredths = (px: number) => roundHundredths(px) ?? 0
  const grammar: Grammar = {
    policy,
    borderStyles: border.styles,
    widthKeywords: new Map(
      Object.entries(border.width_keywords).map(([name, px]) => [
        name,
        toHundredths(px),
      ])
    ),
    maxBorderWidth: toHundredths(border.max_width),
    initial: sideOf(border.initial, "initial"),
    unset: sideOf(border.unset, "unset"),
  }
  prepared.set(policy, grammar)
  return grammar
}

/** The keyword of `values` that a component is. */
function keyword(
  component: Component,
  values: readonly string[]
): string | null {
  return component.kind === "ident" && values.includes(component.value)
    ? component.value
    : null
}

/** A length in px within `[min, max]` px, as hundredths. */
function pxWithin(
  component: Component,
  range: { readonly min: number; readonly max: number }
): number | null {
  const length = lengthPx(component)
  return length !== null && inRange(length, range) ? length : null
}

/** Whether hundredths lie within a range written in whole units. */
function inRange(
  hundredths: number,
  range: { readonly min: number; readonly max: number }
): boolean {
  return (
    hundredths >= Math.round(range.min * 100) &&
    hundredths <= Math.round(range.max * 100)
  )
}

/** A border width: a keyword of the policy, or a length up to the widest side. */
function borderWidth(grammar: Grammar, component: Component): number | null {
  if (component.kind === "ident") {
    return grammar.widthKeywords.get(component.value) ?? null
  }
  const length = lengthPx(component)
  return length !== null && length <= grammar.maxBorderWidth ? length : null
}

/**
 * One border side: at most one width, one style and one colour, in any
 * order, at least one of them; each component fills the first empty part it
 * can be.
 */
function borderSide(
  grammar: Grammar,
  value: readonly Component[]
): BorderSideInput | null {
  if (value.length === 0 || value.length > 3) {
    return null
  }
  const side: BorderSideInput = { width: null, style: null, colour: null }
  for (const component of value) {
    const width = side.width === null ? borderWidth(grammar, component) : null
    if (width !== null) {
      side.width = width
      continue
    }
    const style =
      side.style === null ? keyword(component, grammar.borderStyles) : null
    if (style !== null) {
      side.style = style
      continue
    }
    const colour = side.colour === null ? readColour(component) : null
    if (colour !== null) {
      side.colour = colour
      continue
    }
    return null
  }
  return side
}

/** One to four values, each read with `read`. */
function oneToFour<T>(
  value: readonly Component[],
  read: (component: Component) => T | null
): T[] | null {
  if (value.length < 1 || value.length > 4) {
    return null
  }
  const list: T[] = []
  for (const component of value) {
    const item = read(component)
    if (item === null) {
      return null
    }
    list.push(item)
  }
  return list
}

/** One component reduced to what a locked comparison looks at. */
function normal(component: Component): string | null {
  switch (component.kind) {
    case "ident":
      return `ident ${component.value}`
    case "percentage": {
      const value = percentage(component)
      return value === null ? null : `percentage ${value}`
    }
    case "slash":
      return "slash"
    case "number": {
      // A unitless zero is a length (`0 auto`); other numbers stay numbers
      // (`16 / 9`).
      const length = lengthPx(component)
      if (length !== null) {
        return `px ${length}`
      }
      const value = unitlessNumber(component)
      return value === null ? null : `number ${value}`
    }
    case "dimension": {
      const length = lengthPx(component)
      return length === null ? null : `px ${length}`
    }
    default:
      return null
  }
}

/**
 * Whether a value is a locked value, compared after each component is
 * normalised (`0px auto` matches `0 auto`).
 */
function matchesLocked(locked: string, value: readonly Component[]): boolean {
  const expected = readValue(locked)
  return (
    expected !== null &&
    expected.length === value.length &&
    expected.every((component, index) => {
      const want = normal(component)
      return want !== null && want === normal(value[index])
    })
  )
}

/**
 * Reads a declaration's value with a property's grammar, as the server's
 * `grammar::parse` does.
 *
 * @param grammar - the prepared policy grammar.
 * @param rule - the property's grammar from the policy.
 * @param value - the declaration's components.
 * @returns the typed value, `"default"` for the grammar's omitted default,
 *   or `"rejected"`.
 */
function readValueWith(
  grammar: Grammar,
  rule: StyleGrammar,
  value: readonly Component[]
): Reading {
  const [single] = value.length === 1 ? value : [undefined]
  let accepted: Value | null = null
  switch (rule.kind) {
    case "keyword": {
      if (
        rule.omitted !== null &&
        single?.kind === "ident" &&
        single.value === rule.omitted
      ) {
        return "default"
      }
      const text = single && keyword(single, rule.values)
      accepted = text ? { kind: "text", text } : null
      break
    }
    case "px": {
      const length = single && pxWithin(single, rule)
      accepted = length == null ? null : { kind: "px", hundredths: length }
      break
    }
    case "auto_or_px": {
      if (single && keyword(single, ["auto"]) !== null) {
        accepted = { kind: "text", text: "auto" }
        break
      }
      const length = single && pxWithin(single, rule)
      accepted = length == null ? null : { kind: "px", hundredths: length }
      break
    }
    case "em": {
      const size = single && emSize(single)
      accepted =
        size == null || !inRange(size, rule)
          ? null
          : { kind: "em", hundredths: size }
      break
    }
    case "colour": {
      const colour = single && readColour(single)
      accepted =
        colour == null || (rule.opaque_only && !isOpaque(colour))
          ? null
          : { kind: "colour", colour }
      break
    }
    case "border_side": {
      const side = borderSide(grammar, value)
      accepted = side === null ? null : { kind: "side", side }
      break
    }
    case "border_widths": {
      const list = oneToFour(value, (c) => borderWidth(grammar, c))
      accepted = list === null ? null : { kind: "widths", list }
      break
    }
    case "border_styles": {
      const list = oneToFour(value, (c) => keyword(c, grammar.borderStyles))
      accepted = list === null ? null : { kind: "styles", list }
      break
    }
    case "border_colours": {
      const list = oneToFour(value, readColour)
      accepted = list === null ? null : { kind: "colours", list }
      break
    }
    case "locked":
      accepted = matchesLocked(rule.value, value)
        ? { kind: "text", text: rule.value }
        : null
      break
  }
  return accepted ?? "rejected"
}

/** CSS's one-to-four value rule: the values for top, right, bottom and left. */
function four<T>(values: readonly T[]): [T, T, T, T] | null {
  switch (values.length) {
    case 1:
      return [values[0], values[0], values[0], values[0]]
    case 2:
      return [values[0], values[1], values[0], values[1]]
    case 3:
      return [values[0], values[1], values[2], values[1]]
    case 4:
      return [values[0], values[1], values[2], values[3]]
    default:
      return null
  }
}

/** Writes a side: `none` or `hidden` alone, otherwise its full triple. */
function writeSide(side: Side): string {
  return side.style === "none" || side.style === "hidden"
    ? side.style
    : `${writeHundredths(side.width)}px ${side.style} ${writeColour(side.colour)}`
}

/** Writes a stored value in its one spelling. */
function writeValue(value: Value): string {
  switch (value.kind) {
    case "text":
      return value.text
    case "px":
      return `${writeHundredths(value.hundredths)}px`
    case "em":
      return `${writeHundredths(value.hundredths)}em`
    case "colour":
      return writeColour(value.colour)
    default:
      // The border values set the sides, which are written on their own.
      throw new Error(`canonicalStyle: a ${value.kind} value is not stored`)
  }
}

/**
 * The accepted declarations of one `style` attribute, resolved with CSS's
 * cascade (see the module documentation).
 */
class Cascade {
  private readonly values = new Map<string, Value>()
  private sides: Side[] | null = null

  private readonly grammar: Grammar

  constructor(grammar: Grammar) {
    this.grammar = grammar
  }

  /** Applies one accepted declaration, after every earlier one. */
  set(property: string, value: Value): void {
    switch (value.kind) {
      case "side": {
        const { initial } = this.grammar
        const side: Side = {
          width: value.side.width ?? initial.width,
          style: value.side.style ?? initial.style,
          colour: value.side.colour ?? initial.colour,
        }
        // The sides a shorthand does not name stay as the page has them.
        const sides = this.allSides()
        for (const index of SIDES_NAMED[property] ?? []) {
          sides[index] = side
        }
        break
      }
      case "widths":
        this.setEach(value.list, (side, width) => ({ ...side, width }))
        break
      case "styles":
        this.setEach(value.list, (side, style) => ({ ...side, style }))
        break
      case "colours":
        this.setEach(value.list, (side, colour) => ({ ...side, colour }))
        break
      default:
        this.values.set(storedName(property), value)
    }
  }

  /** Unsets a property whose default came last (`list-style-type: disc`). */
  unset(property: string): void {
    this.values.delete(storedName(property))
  }

  /** The four sides, at the page's values until a declaration sets them. */
  private allSides(): Side[] {
    this.sides ??= [0, 1, 2, 3].map(() => this.grammar.unset)
    return this.sides
  }

  /** Sets one longhand of every side from a one-to-four value list. */
  private setEach<T>(list: readonly T[], set: (side: Side, value: T) => Side) {
    const values = four(list)
    if (values !== null) {
      const sides = this.allSides()
      values.forEach((value, index) => {
        sides[index] = set(sides[index], value)
      })
    }
  }

  /**
   * Writes the canonical style string, or `null` when nothing is left.
   *
   * @param rules - the element's style rules; they give the locked
   *   declarations that are always present or depend on a size.
   */
  write(rules: readonly StyleRule[]): string | null {
    const sized = this.values.has("width") || this.values.has("height")
    for (const rule of rules) {
      if (rule.grammar.kind !== "locked") {
        continue
      }
      const locked: Value = { kind: "text", text: rule.grammar.value }
      if (rule.presence === "always" && !this.values.has(rule.property)) {
        this.values.set(rule.property, locked)
      } else if (rule.presence === "when_sized") {
        if (sized) {
          this.values.set(rule.property, locked)
        } else {
          this.values.delete(rule.property)
        }
      }
    }
    const declarations: [string, string][] = [...this.values].map(
      ([property, value]) => [property, writeValue(value)]
    )
    this.sides?.forEach((side, index) => {
      declarations.push([BORDER_SIDES[index], writeSide(side)])
    })
    if (declarations.length === 0) {
      return null
    }
    // Property names are ASCII, so code-unit order is the server's byte
    // order, and each name occurs once.
    declarations.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    return declarations
      .map(([property, value]) => `${property}: ${value}`)
      .join("; ")
  }
}

/** The stored property that a property sets. */
function storedName(property: string): string {
  return Object.hasOwn(SHORTHAND_TARGETS, property)
    ? SHORTHAND_TARGETS[property]
    : property
}

/**
 * Filters the `style` attribute of one element as the server's
 * `sanitize_style` does: the canonical string, and what was removed.
 *
 * @param policy - the content-v1 policy (pass `CONTENT_V1_POLICY`).
 * @param element - the element's tag name, lower case. An element without a
 *   `style` grammar keeps nothing.
 * @param style - the attribute value, as the DOM holds it.
 * @returns the canonical string and the removal counts.
 * @throws {PolicyError} when the policy has an input-only shorthand this
 *   module cannot expand (see {@link SHORTHAND_TARGETS}).
 */
export function sanitizeStyle(
  policy: RichTextPolicy,
  element: string,
  style: string
): CanonicalStyle {
  const grammar = grammarOf(policy)
  const read = readDeclarations(style)
  const rules = elementOf(policy, element)?.style ?? null
  if (rules === null) {
    return {
      css: null,
      rejected: read.unreadable + read.read.length,
      outsideAlphabet: false,
    }
  }
  let rejected = read.unreadable
  const cascade = new Cascade(grammar)
  for (const declaration of read.read) {
    const rule = rules.find((entry) => entry.property === declaration.name)
    if (rule === undefined) {
      rejected += 1
      continue
    }
    const reading = readValueWith(grammar, rule.grammar, declaration.value)
    if (reading === "rejected") {
      rejected += 1
    } else if (reading === "default") {
      cascade.unset(rule.property)
    } else {
      cascade.set(rule.property, reading)
    }
  }
  const css = cascade.write(rules)
  if (css !== null && [...css].some((c) => !policy.css.alphabet.includes(c))) {
    return { css: null, rejected, outsideAlphabet: true }
  }
  return { css, rejected, outsideAlphabet: false }
}

/**
 * The canonical style string of one element's `style` attribute (editor.md
 * §3.1): what the server stores for it, byte for byte.
 *
 * @param policy - the content-v1 policy (pass `CONTENT_V1_POLICY`).
 * @param element - the element's tag name, lower case.
 * @param style - the attribute value, as the DOM holds it.
 * @returns the string to store, or `null` when the attribute is removed.
 * @throws {PolicyError} as {@link sanitizeStyle} does.
 */
export function canonicalStyle(
  policy: RichTextPolicy,
  element: string,
  style: string
): string | null {
  return sanitizeStyle(policy, element, style).css
}
