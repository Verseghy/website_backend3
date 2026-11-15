/**
 * The value types of content-v1 CSS and their readers, as the server's
 * `src/domain/rich_text/css/values.rs` reads them (editor.md §3.1 rules 3
 * and 4, §7.3): numbers rounded to hundredths, lengths in px, sizes in
 * `em`, percentages and colours.
 *
 * Every reader takes one component that `declarations.ts` produced and
 * returns a typed value, or `null` when the component is not one. Each type
 * has exactly one spelling, written by {@link writeHundredths} and
 * {@link writeColour}, so a value that is read and written again is
 * unchanged.
 *
 * Numbers are held as whole hundredths and rounded before any bound is
 * checked, so a value that passes is exactly the value that is stored. The
 * server rounds with `round(x × 100)`, half away from zero; for the
 * non-negative numbers that content-v1 accepts that is what `Math.round`
 * gives, and every product and quotient here is computed in the same order
 * as on the server, so both sides round the same IEEE 754 doubles.
 */
import type { Argument, Component } from "./declarations"
import { NAMED_COLOURS } from "./named-colours"

/** The largest number of hundredths the server holds (`u32::MAX`). */
const MAX_HUNDREDTHS = 0xffff_ffff

/**
 * `value` in whole hundredths, rounded, or `null` when it is negative, not
 * finite, or too large for the server to hold.
 *
 * @param value - the number to round; `-0` counts as zero.
 */
export function roundHundredths(value: number): number | null {
  if (!Number.isFinite(value) || value < 0) {
    return null
  }
  const scaled = Math.round(value * 100)
  // `+ 0` turns a rounded `-0` into `0`.
  return scaled <= MAX_HUNDREDTHS ? scaled + 0 : null
}

/**
 * Writes hundredths with at most two decimals and no trailing zeros: `20`,
 * `0.67`, `1.5` (editor.md §7.3, "Numbers").
 *
 * @param hundredths - a whole, non-negative number of hundredths.
 */
export function writeHundredths(hundredths: number): string {
  const whole = Math.floor(hundredths / 100)
  const fraction = hundredths % 100
  if (fraction === 0) {
    return String(whole)
  }
  if (fraction % 10 === 0) {
    return `${whole}.${fraction / 10}`
  }
  return `${whole}.${String(fraction).padStart(2, "0")}`
}

/**
 * The absolute length units with how many of each make one inch (CSS Values
 * 4 §6.2). Each becomes px as `value × 96 / perInch` (editor.md §3.1 rule
 * 4); `px` itself is taken as it is.
 */
export const LENGTH_UNITS: ReadonlyMap<string, number> = new Map([
  ["px", 96],
  ["pt", 72],
  ["pc", 6],
  ["in", 1],
  ["cm", 2.54],
  ["mm", 25.4],
  ["q", 101.6],
])

/**
 * A length in px, as hundredths: a dimension in one of the
 * {@link LENGTH_UNITS}, or a unitless zero (`0 auto`).
 *
 * @param component - one value component.
 */
export function lengthPx(component: Component): number | null {
  if (component.kind === "number") {
    return component.value === 0 ? 0 : null
  }
  if (component.kind !== "dimension") {
    return null
  }
  const perInch = LENGTH_UNITS.get(component.unit)
  if (perInch === undefined) {
    return null
  }
  return roundHundredths(
    component.unit === "px" ? component.value : (component.value * 96) / perInch
  )
}

/**
 * A size in `em`, as hundredths (span `font-size`, editor.md §3.3).
 *
 * @param component - one value component.
 */
export function emSize(component: Component): number | null {
  return component.kind === "dimension" && component.unit === "em"
    ? roundHundredths(component.value)
    : null
}

/**
 * A percentage, as hundredths.
 *
 * @param component - one value component.
 */
export function percentage(component: Component): number | null {
  return component.kind === "percentage"
    ? roundHundredths(component.value)
    : null
}

/**
 * A number without a unit, as hundredths.
 *
 * @param component - one value component.
 */
export function unitlessNumber(component: Component): number | null {
  return component.kind === "number" ? roundHundredths(component.value) : null
}

/** A colour (editor.md §3.1 rule 3). */
export type Colour =
  | {
      readonly kind: "rgba"
      readonly red: number
      readonly green: number
      readonly blue: number
      /** The alpha channel, 0 to 255; 255 is opaque. */
      readonly alpha: number
    }
  | { readonly kind: "transparent" }
  | { readonly kind: "currentcolor" }

/** An opaque colour. */
const opaque = (red: number, green: number, blue: number): Colour => ({
  kind: "rgba",
  red,
  green,
  blue,
  alpha: 255,
})

/**
 * Whether a colour hides nothing behind it: an sRGB colour with full alpha.
 * `transparent` and `currentcolor` are not (span `color`, editor.md §3.3).
 *
 * @param colour - the colour.
 */
export function isOpaque(colour: Colour): boolean {
  return colour.kind === "rgba" && colour.alpha === 255
}

const hex2 = (value: number) => value.toString(16).padStart(2, "0")

/**
 * Writes a colour: lower-case `#rrggbb`, `#rrggbbaa` when the alpha is below
 * 1, or the keyword.
 *
 * @param colour - the colour.
 */
export function writeColour(colour: Colour): string {
  if (colour.kind !== "rgba") {
    return colour.kind
  }
  const rgb = `#${hex2(colour.red)}${hex2(colour.green)}${hex2(colour.blue)}`
  return colour.alpha === 255 ? rgb : `${rgb}${hex2(colour.alpha)}`
}

/**
 * A colour: a hex colour of 3, 4, 6 or 8 digits, a CSS named colour,
 * `transparent`, `currentcolor`, `windowtext` (the system colour Office
 * writes for black text, read as `#000000`), or an `rgb()` or `rgba()`
 * call. Other system colours and every other function are rejected.
 *
 * @param component - one value component.
 */
export function readColour(component: Component): Colour | null {
  switch (component.kind) {
    case "hash":
      return hexColour(component.value)
    case "ident": {
      if (component.value === "transparent") {
        return { kind: "transparent" }
      }
      if (component.value === "currentcolor") {
        return { kind: "currentcolor" }
      }
      if (component.value === "windowtext") {
        return opaque(0, 0, 0)
      }
      const named = NAMED_COLOURS.get(component.value)
      return named === undefined ? null : opaque(...named)
    }
    case "rgb":
      return rgbColour(component.arguments)
    default:
      return null
  }
}

/** The colour of a hex token without its `#`. */
function hexColour(hex: string): Colour | null {
  if (!/^[0-9a-fA-F]*$/.test(hex)) {
    return null
  }
  const digit = (index: number) => Number.parseInt(hex[index], 16)
  const short = (index: number) => digit(index) * 17
  const long = (index: number) => digit(index) * 16 + digit(index + 1)
  switch (hex.length) {
    case 3:
      return opaque(short(0), short(1), short(2))
    case 4:
      return {
        kind: "rgba",
        red: short(0),
        green: short(1),
        blue: short(2),
        alpha: short(3),
      }
    case 6:
      return opaque(long(0), long(2), long(4))
    case 8:
      return {
        kind: "rgba",
        red: long(0),
        green: long(2),
        blue: long(4),
        alpha: long(6),
      }
    default:
      return null
  }
}

/** `value` rounded to a whole number when it lies from 0 to 255. */
function toByte(value: number): number | null {
  const rounded = Math.round(value) + 0
  return rounded >= 0 && rounded <= 255 ? rounded : null
}

/**
 * The colour of an `rgb()` or `rgba()` call, in its legacy syntax (three
 * numbers or three percentages separated by commas, and an optional alpha)
 * or its modern syntax (`rgb(255 0 0 / 50%)`). Channels are numbers from 0
 * to 255 or percentages from 0 to 100, the alpha a number from 0 to 1 or a
 * percentage; a value out of range is rejected, not clamped, as on the
 * server.
 *
 * @param list - the call's arguments.
 */
export function rgbColour(list: readonly Argument[]): Colour | null {
  const kinds = list.map((argument) => argument.kind).join(",")
  const hasComma = list.some((argument) => argument.kind === "comma")
  let channels: readonly Argument[]
  let alpha: Argument | undefined
  if (/^[^,]+,comma,[^,]+,comma,[^,]+$/.test(kinds)) {
    channels = [list[0], list[2], list[4]]
  } else if (/^[^,]+,comma,[^,]+,comma,[^,]+,comma,[^,]+$/.test(kinds)) {
    channels = [list[0], list[2], list[4]]
    alpha = list[6]
  } else if (list.length === 3 && !hasComma) {
    channels = list
  } else if (list.length === 5 && !hasComma && list[3].kind === "slash") {
    channels = list.slice(0, 3)
    alpha = list[4]
  } else {
    return null
  }
  if (
    hasComma &&
    !channels.every((channel) => channel.kind === "number") &&
    !channels.every((channel) => channel.kind === "percentage")
  ) {
    return null
  }
  const channel = (argument: Argument): number | null => {
    if (
      argument.kind === "number" &&
      argument.value >= 0 &&
      argument.value <= 255
    ) {
      return toByte(argument.value)
    }
    if (
      argument.kind === "percentage" &&
      argument.value >= 0 &&
      argument.value <= 100
    ) {
      return toByte((argument.value * 255) / 100)
    }
    return null
  }
  let alphaByte: number | null = 255
  if (alpha !== undefined) {
    if (alpha.kind === "number" && alpha.value >= 0 && alpha.value <= 1) {
      alphaByte = toByte(alpha.value * 255)
    } else if (
      alpha.kind === "percentage" &&
      alpha.value >= 0 &&
      alpha.value <= 100
    ) {
      alphaByte = toByte((alpha.value * 255) / 100)
    } else {
      return null
    }
  }
  const [red, green, blue] = channels.map(channel)
  if (red === null || green === null || blue === null || alphaByte === null) {
    return null
  }
  return { kind: "rgba", red, green, blue, alpha: alphaByte }
}
