import fc from "fast-check"
import { describe, expect, it } from "vitest"

import type { Argument, Component } from "./declarations"
import { readValue } from "./declarations"
import type { Colour } from "./values"
import {
  emSize,
  isOpaque,
  lengthPx,
  LENGTH_UNITS,
  percentage,
  readColour,
  rgbColour,
  roundHundredths,
  unitlessNumber,
  writeColour,
  writeHundredths,
} from "./values"

/** The one component of a bare value. */
function component(value: string): Component {
  const read = readValue(value)
  if (read?.length !== 1) {
    throw new Error(`not one component: ${value}`)
  }
  return read[0]
}

describe("roundHundredths and writeHundredths", () => {
  it.each<[number, number | null]>([
    [0, 0],
    [-0, 0],
    [1.005, 100],
    [10.005, 1001],
    [0.015, 2],
    [0.67, 67],
    [42_949_672.95, 4_294_967_295],
    [42_949_672.96, null],
    [-0.01, null],
    [Number.POSITIVE_INFINITY, null],
    [Number.NaN, null],
  ])("rounds %d to %s hundredths", (value, expected) => {
    expect(roundHundredths(value)).toBe(expected)
    if (expected === 0) {
      expect(Object.is(roundHundredths(value), 0)).toBe(true)
    }
  })

  it.each<[number, string]>([
    [0, "0"],
    [2000, "20"],
    [67, "0.67"],
    [150, "1.5"],
    [101, "1.01"],
    [3780, "37.8"],
  ])("writes %d hundredths as %s", (hundredths, expected) => {
    expect(writeHundredths(hundredths)).toBe(expected)
  })

  it("writes every number in a spelling that reads back to it", () => {
    fc.assert(
      fc.property(fc.integer({ min: 0, max: 4_294_967_295 }), (hundredths) => {
        const written = writeHundredths(hundredths)
        expect(written).toMatch(/^(?:0|[1-9][0-9]*)(?:\.[0-9]?[1-9])?$/)
        expect(roundHundredths(Number(written))).toBe(hundredths)
      })
    )
  })
})

describe("lengths, sizes and numbers", () => {
  it.each<[string, number | null]>([
    ["10px", 1000],
    ["96pt", 12800],
    ["1.0cm", 3780],
    ["10mm", 3780],
    ["1in", 9600],
    ["6pc", 9600],
    ["40q", 3780],
    ["0", 0],
    ["-0", 0],
    ["1", null],
    ["1em", null],
    ["-1px", null],
    ["100%", null],
    ["auto", null],
  ])("reads the length %s as %s hundredths of a px", (value, expected) => {
    expect(lengthPx(component(value))).toBe(expected)
  })

  it("knows the seven absolute units of CSS Values 4", () => {
    expect([...LENGTH_UNITS.keys()]).toEqual([
      "px",
      "pt",
      "pc",
      "in",
      "cm",
      "mm",
      "q",
    ])
  })

  it("reads em, percentages and unitless numbers each on their own", () => {
    // 1.255 × 100 is 125.4999… in binary, as on the server (`1.255em`
    // becomes `1.25em` in css-vectors.jsonl).
    expect(emSize(component("1.255em"))).toBe(125)
    expect(emSize(component("1px"))).toBeNull()
    expect(percentage(component("100.00%"))).toBe(10000)
    expect(percentage(component("1"))).toBeNull()
    expect(unitlessNumber(component("16"))).toBe(1600)
    expect(unitlessNumber(component("16px"))).toBeNull()
  })
})

const opaque = (red: number, green: number, blue: number): Colour => ({
  kind: "rgba",
  red,
  green,
  blue,
  alpha: 255,
})

describe("readColour", () => {
  it.each<[string, Colour | null]>([
    ["#c00", opaque(204, 0, 0)],
    ["#C00000", opaque(192, 0, 0)],
    ["#c008", { kind: "rgba", red: 204, green: 0, blue: 0, alpha: 136 }],
    ["#c0000080", { kind: "rgba", red: 192, green: 0, blue: 0, alpha: 128 }],
    ["#12345", null],
    ["#ggg", null],
    ["red", opaque(255, 0, 0)],
    ["RebeccaPurple", opaque(102, 51, 153)],
    ["windowtext", opaque(0, 0, 0)],
    ["buttontext", null],
    ["transparent", { kind: "transparent" }],
    ["currentColor", { kind: "currentcolor" }],
    ["rgb(192, 0, 0)", opaque(192, 0, 0)],
    ["rgb(100%, 0%, 0%)", opaque(255, 0, 0)],
    ["rgb(50.5%, 0%, 0%)", opaque(129, 0, 0)],
    ["rgb(127.5, 0, 0)", opaque(128, 0, 0)],
    ["rgb(192 0 0 / 100%)", opaque(192, 0, 0)],
    [
      "rgba(0, 0, 0, 0.502)",
      { kind: "rgba", red: 0, green: 0, blue: 0, alpha: 128 },
    ],
    ["rgb(1, 2%, 3)", null],
    ["rgb(1 2% 3)", opaque(1, 5, 3)],
    ["rgb(256, 0, 0)", null],
    ["rgb(1 2 3 / 2)", null],
    ["rgb(1, 2, 3 / 0.5)", null],
    ["rgb(1 2)", null],
    ["1px", null],
  ])("reads %s", (value, expected) => {
    expect(readColour(component(value))).toEqual(expected)
  })

  it("writes a colour in its one spelling, which reads back to it", () => {
    const byte = fc.integer({ min: 0, max: 255 })
    const colour = fc.oneof(
      fc
        .tuple(byte, byte, byte, byte)
        .map(([red, green, blue, alpha]): Colour => ({
          kind: "rgba",
          red,
          green,
          blue,
          alpha,
        })),
      fc.constant<Colour>({ kind: "transparent" }),
      fc.constant<Colour>({ kind: "currentcolor" })
    )
    fc.assert(
      fc.property(colour, (value) => {
        const written = writeColour(value)
        expect(written).toMatch(
          /^(?:#[0-9a-f]{6}(?:[0-9a-f]{2})?|transparent|currentcolor)$/
        )
        expect(written.endsWith("ff") && written.length === 9).toBe(false)
        expect(readColour(component(written))).toEqual(value)
        expect(isOpaque(value)).toBe(/^#[0-9a-f]{6}$/.test(written))
      })
    )
  })
})

describe("rgbColour", () => {
  it("reads only the four argument shapes of CSS Color 4", () => {
    const n = (value: number): Argument => ({ kind: "number", value })
    const comma: Argument = { kind: "comma" }
    const slash: Argument = { kind: "slash" }
    expect(rgbColour([n(1), comma, n(2), comma, n(3)])).toEqual(opaque(1, 2, 3))
    expect(rgbColour([n(1), comma, n(2), comma, n(3), comma, n(0)])).toEqual({
      kind: "rgba",
      red: 1,
      green: 2,
      blue: 3,
      alpha: 0,
    })
    expect(rgbColour([n(1), n(2), n(3)])).toEqual(opaque(1, 2, 3))
    expect(rgbColour([n(1), n(2), n(3), slash, n(1)])).toEqual(opaque(1, 2, 3))
    expect(rgbColour([n(1), comma, n(2), n(3)])).toBeNull()
    expect(rgbColour([slash, slash, slash])).toBeNull()
    expect(rgbColour([comma, comma, comma, comma, comma])).toBeNull()
    expect(rgbColour([])).toBeNull()
  })
})
