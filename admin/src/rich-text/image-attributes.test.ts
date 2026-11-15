import fc from "fast-check"
import { describe, expect, it } from "vitest"

import {
  IMAGE_COLUMN_WIDTH,
  IMAGE_WIDTH_PRESETS,
  imageSize,
  isAltMissing,
} from "./image-attributes"

const RANGE = { min: 1, max: 5000 }

describe("the width presets", () => {
  it("are a quarter to the whole of the 700 px column (editor.md §4.3 row 8)", () => {
    expect(IMAGE_WIDTH_PRESETS).toEqual([175, 350, 525, 700])
    expect(IMAGE_COLUMN_WIDTH).toBe(700)
  })
})

describe("isAltMissing", () => {
  it.each<[string | null, boolean]>([
    [null, true],
    ["", true],
    ["  \n\t", true],
    [" ", true],
    ["kép", false],
    [" A díjazottak ", false],
  ])("judges %j as missing: %s", (alt, missing) => {
    expect(isAltMissing(alt)).toBe(missing)
  })

  it("is missing exactly when nothing but whitespace is left", () => {
    fc.assert(
      fc.property(fc.option(fc.string(), { nil: null }), (alt) => {
        expect(isAltMissing(alt)).toBe(alt === null || alt.trim() === "")
      })
    )
  })
})

describe("imageSize", () => {
  it.each<[number | null, number | null, number | null, number | null]>([
    [350, 495 / 350, 350, 495],
    [350.5, 495 / 350, 351, 496],
    [175, 0.5, 175, 88],
    [350, null, 350, null],
    [null, 1.5, null, null],
    [0.2, 1, null, null],
    [700, 100, 700, null],
    [700, Number.NaN, 700, null],
  ])("sizes %s px at ratio %s as %s × %s", (width, ratio, w, h) => {
    expect(imageSize(width, ratio, RANGE)).toEqual({ width: w, height: h })
  })

  it("loads back the size it writes (a fixed point)", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 2000 }),
        fc.integer({ min: 1, max: 5000 }),
        fc.integer({ min: 1, max: 5000 }),
        (displayWidth, naturalWidth, naturalHeight) => {
          const first = imageSize(
            displayWidth,
            naturalHeight / naturalWidth,
            RANGE
          )
          // Loading reads the ratio back from the written attributes.
          const ratio =
            first.width !== null && first.height !== null
              ? first.height / first.width
              : null
          expect(imageSize(displayWidth, ratio, RANGE)).toEqual(
            ratio === null ? { width: first.width, height: null } : first
          )
        }
      )
    )
  })
})
