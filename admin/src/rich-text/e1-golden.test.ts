import {
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { join, resolve } from "node:path"
import { afterAll, describe, expect, it } from "vitest"

import {
  COMMAND_START,
  editorGoldenFiles,
  GOLDEN_DIRECTORY,
  goldenChanges,
  goldenFiles,
} from "@/test/golden-e1"
import { storedHtmlHarness } from "@/test/stored-html"

/**
 * Writes the golden files of `tests/fixtures/e1/` (editor.md §13.6, P2 and
 * P3; `src/test/golden-e1.ts` says what they are).
 *
 * Like the ts-rs export tests, which write `src/bindings/`, this test
 * writes rather than compares: it brings the directory to what the editor
 * stores now, and the Contract CI job, which runs it with
 * `npm run test:e1`, fails on any difference from the committed files
 * (`scripts/ci/check-contract-drift.sh`). A change to the editor that
 * changes a stored body therefore shows up as a changed golden file in the
 * PR, and the server's `tests/rich_text/golden.rs` checks the new bytes.
 * Commit the files this test writes.
 */
const E1 = resolve(import.meta.dirname, "../../..", GOLDEN_DIRECTORY)

const harness = storedHtmlHarness()
afterAll(() => harness.destroy())
const FILES = editorGoldenFiles(harness)

describe("the golden files of tests/fixtures/e1", () => {
  it("cover every group of editor fixtures", () => {
    const groups = new Set([...FILES.keys()].map((file) => file.split("-")[0]))
    expect([...groups].sort()).toEqual([
      "canonical",
      "command",
      "regression",
      "table",
    ])
    expect(FILES.size).toBeGreaterThanOrEqual(100)
  })

  it.each([...FILES])("%s is a fixed point of J", (_file, html) => {
    expect(harness.J(html)).toBe(html)
  })

  it.each([...FILES])("%s holds no CSSOM spelling", (_file, html) => {
    expect(html).not.toMatch(/rgb\(|border-width:/)
  })

  it("start the command fixtures from a body with a paragraph", () => {
    expect(harness.J(COMMAND_START)).toBe(COMMAND_START)
  })

  it("are written to the directory", () => {
    mkdirSync(E1, { recursive: true })
    const existing = new Map(
      readdirSync(E1).map((file) => [
        file,
        readFileSync(join(E1, file), "utf8"),
      ])
    )
    const { write, remove } = goldenChanges(existing, FILES)
    for (const file of write) {
      writeFileSync(join(E1, file), FILES.get(file)!, "utf8")
    }
    for (const file of remove) {
      rmSync(join(E1, file))
    }
    const written = readdirSync(E1).filter((file) => file.endsWith(".html"))
    expect(written.sort()).toEqual([...FILES.keys()])
    for (const file of written) {
      expect(readFileSync(join(E1, file), "utf8"), file).toBe(FILES.get(file))
    }
  })
})

describe("goldenFiles", () => {
  it("names each file after its fixture, in name order", () => {
    expect([
      ...goldenFiles([
        { name: "table-b", html: "<p>b</p>" },
        { name: "canonical-a", html: "<p>a</p>" },
      ]),
    ]).toEqual([
      ["canonical-a.html", "<p>a</p>"],
      ["table-b.html", "<p>b</p>"],
    ])
  })

  it.each(["", "Table-a", "table_a", "table--a", "-a", "a-", "../a", "a.b"])(
    "refuses the name %j",
    (name) => {
      expect(() => goldenFiles([{ name, html: "" }])).toThrow(
        "is not a golden file name"
      )
    }
  )

  it("refuses two fixtures with one name", () => {
    expect(() =>
      goldenFiles([
        { name: "table-a", html: "<p>1</p>" },
        { name: "table-a", html: "<p>2</p>" },
      ])
    ).toThrow('two fixtures are named "table-a"')
  })
})

describe("goldenChanges", () => {
  const planned = new Map([
    ["a.html", "<p>a</p>"],
    ["b.html", "<p>b</p>"],
  ])

  it("writes what is missing or changed and removes what the plan lacks", () => {
    expect(
      goldenChanges(
        new Map([
          ["README.md", "# e1"],
          ["b.html", "<p>old</p>"],
          ["z.html", "<p>z</p>"],
          ["c.html", "<p>c</p>"],
        ]),
        planned
      )
    ).toEqual({ write: ["a.html", "b.html"], remove: ["c.html", "z.html"] })
  })

  it("changes nothing in a directory that matches", () => {
    expect(
      goldenChanges(new Map([...planned, ["README.md", "# e1"]]), planned)
    ).toEqual({ write: [], remove: [] })
  })
})
