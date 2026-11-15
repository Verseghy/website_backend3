import { afterAll, describe, expect, it } from "vitest"

import { normaliserOutputs, storedHtmlHarness } from "@/test/stored-html"

/**
 * The editor round trip of the import normaliser's outputs
 * (data-migration.md §12, "Admin editor round trip"; editor.md §13.4, "The
 * importer's fixtures"; ADR 0013 §10): loading an output into the editor
 * and saving it returns it byte for byte, the third check of N9.
 *
 * It covers the outputs of N5b's image rules (`n5b-`) and the body with
 * images without alt text (`alt-`), whose missing and blank alts the editor
 * keeps on load (ADR 0013 §2). The other fixture outputs still hold shapes
 * that only N5b's structure rules bring into the editor's stored form
 * (`tests/fixtures/normalise/README.md`), and join this test with them.
 * An image wider than content-v1's 2000 px bound is covered too: N7 drops
 * its width for review, and what is left is the editor's own form.
 */
const COVERED = /^(n5b|alt)-/

const harness = storedHtmlHarness()
afterAll(() => harness.destroy())

const OUTPUTS = normaliserOutputs()
const ROUND_TRIP = OUTPUTS.filter(({ name }) => COVERED.test(name)).map(
  ({ name, html }): [string, string] => [name, html]
)

/**
 * The output of the fixture `name`.
 *
 * @param name - a fixture name.
 * @returns its output.
 * @throws {Error} when no fixture has that name.
 */
function output(name: string): string {
  const found = OUTPUTS.find((candidate) => candidate.name === name)
  if (found === undefined) {
    throw new Error(
      `output: no fixture "${name}" under tests/fixtures/normalise/.`
    )
  }
  return found.html
}

describe("the editor round trip of the normaliser's image outputs", () => {
  it("covers every image fixture and the body without alt text", () => {
    const names = ROUND_TRIP.map(([name]) => name)
    expect(names).toContain("alt-missing-and-blank")
    expect(names).toContain("n5b-image-without-alt")
    expect(names).toContain("n5b-image-width-over-bound")
    expect(names.filter((name) => name.startsWith("n5b-")).length).toBe(
      OUTPUTS.filter(({ name }) => name.startsWith("n5b-")).length
    )
  })

  it.each(ROUND_TRIP)("%s loads and saves byte for byte", (_name, html) => {
    expect(harness.J(html)).toBe(html)
  })

  it("keeps a missing and a blank alt as they are", () => {
    const stored = harness.J(output("alt-missing-and-blank"))
    expect(stored).toContain('alt=""')
    expect(stored).toContain('alt=" &nbsp;"')
    expect(harness.J(output("n5b-image-without-alt"))).not.toContain("alt=")
  })

  it("stores an image over the 2000 px bound without the width that N7 drops", () => {
    // editor.md §16 item 5: the import keeps and reports the width, and
    // N7 drops it, which stops the import until the bound is raised or an
    // N7-accept entry lets the width go. N5b writes no width or height
    // attribute for it, as the editor writes none for such a width, so
    // what N7 leaves is the editor's own form.
    const wide = output("n5b-image-width-over-bound")
    expect(wide).not.toContain("2400")
    expect(wide).not.toContain(' width="')
    expect(harness.J(wide)).toBe(wide)
  })
})
