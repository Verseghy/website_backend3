import { describe, expect, it } from "vitest"

import { NAMED_COLOURS } from "./named-colours"

/**
 * The table against an independent reading: jsdom computes every named
 * colour of CSS Color 4 itself (through its CSS colour parser), so a
 * misspelt name or a wrong channel in the copy of cssparser's list fails
 * here, and so does a name that is not a CSS colour at all.
 */
describe("NAMED_COLOURS", () => {
  it("holds the 148 named colours of CSS Color 4, lower case, each once", () => {
    expect(NAMED_COLOURS.size).toBe(148)
    for (const name of NAMED_COLOURS.keys()) {
      expect(name).toMatch(/^[a-z]+$/)
    }
    for (const keyword of ["transparent", "currentcolor", "windowtext"]) {
      expect(NAMED_COLOURS.has(keyword), keyword).toBe(false)
    }
  })

  it.each([...NAMED_COLOURS].map(([name, rgb]) => [name, rgb]))(
    "gives %s the channels jsdom computes",
    (name, [red, green, blue]) => {
      const probe = document.createElement("span")
      document.body.append(probe)
      probe.style.color = name
      expect(getComputedStyle(probe).color).toBe(
        `rgb(${red}, ${green}, ${blue})`
      )
      probe.remove()
    }
  )
})
