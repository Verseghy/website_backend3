// @vitest-environment node
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"

import { afterEach, describe, expect, it } from "vitest"

import { parseArgs, runLicenseCheck } from "./check-licenses.mjs"
import {
  evaluateLicenses,
  packageName,
  satisfies,
  validatePolicy,
} from "./license-policy.mjs"

/** @type {import("./license-policy.mjs").LicensePolicy} */
const POLICY = {
  allow: [
    { license: "MIT", reason: "r" },
    { license: "MPL-2.0", reason: "r" },
    { license: "Apache-2.0", reason: "r" },
  ],
  exceptions: [{ package: "@fonts/inter", license: "OFL-1.1", reason: "r" }],
}

/**
 * @param {Record<string, string | string[] | undefined>} licenses - id to licence.
 * @returns {Record<string, import("./license-policy.mjs").InstalledPackage>} a fake tree.
 */
const tree = (licenses) =>
  Object.fromEntries(
    Object.entries(licenses).map(([id, value]) => [id, { licenses: value }])
  )

describe("packageName", () => {
  it("strips the version of plain and scoped ids", () => {
    expect(packageName("react@19.3.1")).toBe("react")
    expect(packageName("@base-ui/react@1.9.0")).toBe("@base-ui/react")
  })
})

describe("evaluateLicenses", () => {
  it("passes allowed licences", () => {
    const result = evaluateLicenses(
      tree({ "a@1.0.0": "MIT", "b@2.0.0": "MPL-2.0" }),
      POLICY
    )
    expect(result.violations).toEqual([])
    expect(result.checked).toBe(2)
  })

  it("fails a licence that is not allowed", () => {
    const result = evaluateLicenses(
      tree({ "gpl@1.0.0": "GPL-2.0-only" }),
      POLICY
    )
    expect(result.violations).toEqual([
      expect.objectContaining({
        id: "gpl@1.0.0",
        license: "GPL-2.0-only",
        problem: expect.stringMatching(/not allowed/),
      }),
    ])
  })

  it("never lets a longer id through on a prefix (the --onlyAllow substring trap)", () => {
    const result = evaluateLicenses(
      tree({ "x@1.0.0": "MPL-2.0-no-copyleft-exception" }),
      POLICY
    )
    expect(result.violations).toHaveLength(1)
  })

  it("requires both sides of AND", () => {
    expect(
      evaluateLicenses(tree({ "x@1.0.0": "(MIT AND GPL-2.0-only)" }), POLICY)
        .violations
    ).toHaveLength(1)
    expect(
      evaluateLicenses(tree({ "x@1.0.0": "(MIT AND Apache-2.0)" }), POLICY)
        .violations
    ).toEqual([])
  })

  it("accepts OR when either side is allowed", () => {
    expect(
      evaluateLicenses(tree({ "x@1.0.0": "(GPL-3.0-only OR MIT)" }), POLICY)
        .violations
    ).toEqual([])
    expect(
      evaluateLicenses(tree({ "x@1.0.0": "(MIT OR CC0-1.0)" }), POLICY)
        .violations
    ).toEqual([])
  })

  it("accepts an or-later licence whose base version is allowed", () => {
    expect(
      evaluateLicenses(tree({ "x@1.0.0": "Apache-2.0+" }), POLICY).violations
    ).toEqual([])
  })

  it("rejects a licence exception (WITH), none of which is on the list", () => {
    expect(
      evaluateLicenses(
        tree({ "x@1.0.0": "Apache-2.0 WITH LLVM-exception" }),
        POLICY
      ).violations
    ).toHaveLength(1)
  })

  it.each([
    ["missing", undefined, /no licence found/],
    ["UNKNOWN", "UNKNOWN", /no licence found/],
    ["UNLICENSED", "UNLICENSED", /UNLICENSED/],
    ["guessed", "MIT*", /guessed/],
    ["not SPDX", "BSD", /not a valid SPDX expression/],
    ["custom text", "Custom: LICENSE.txt", /not a valid SPDX expression/],
  ])("fails a %s licence with advice", (_label, license, problem) => {
    const [violation] = evaluateLicenses(
      tree({ "x@1.0.0": license }),
      POLICY
    ).violations
    expect(violation.problem).toMatch(problem)
  })

  it("applies an exception only to its own package", () => {
    const result = evaluateLicenses(
      tree({ "@fonts/inter@5.0.0": "OFL-1.1", "other@1.0.0": "OFL-1.1" }),
      POLICY
    )
    expect(result.violations.map((violation) => violation.id)).toEqual([
      "other@1.0.0",
    ])
  })

  it("judges every entry of a legacy licences array", () => {
    expect(
      evaluateLicenses(tree({ "x@1.0.0": ["MIT", "GPL-2.0-only"] }), POLICY)
        .violations
    ).toHaveLength(1)
    expect(
      evaluateLicenses(tree({ "x@1.0.0": ["MIT", "Apache-2.0"] }), POLICY)
        .violations
    ).toEqual([])
  })

  it("reports policy entries that no package uses", () => {
    const result = evaluateLicenses(
      tree({ "a@1.0.0": "(MIT OR MPL-2.0)" }),
      POLICY
    )
    // MIT satisfied the OR first, so MPL-2.0 was not relied on.
    expect(result.unusedAllow).toEqual(["MPL-2.0", "Apache-2.0"])
    expect(result.unusedExceptions).toEqual(["@fonts/inter:OFL-1.1"])
  })
})

describe("satisfies", () => {
  it("returns the licences relied on", () => {
    const allowed = (/** @type {string} */ id) => id === "MIT" || id === "ISC"
    expect(
      satisfies(
        {
          conjunction: "and",
          left: { license: "MIT" },
          right: { license: "ISC" },
        },
        allowed
      )
    ).toEqual(["MIT", "ISC"])
    expect(satisfies({ license: "GPL-3.0-only" }, allowed)).toBeNull()
  })
})

describe("validatePolicy", () => {
  it("accepts the committed policy, where every entry has a reason", () => {
    const committed = JSON.parse(
      readFileSync(
        resolve(import.meta.dirname, "../license-policy.json"),
        "utf8"
      )
    )
    expect(() => validatePolicy(committed)).not.toThrow()
  })

  it.each([
    ["a non-object", 42, /JSON object/],
    [
      "missing arrays",
      { allow: [] },
      /`allow` array and an `exceptions` array/,
    ],
    [
      "an entry without a reason",
      { allow: [{ license: "MIT" }], exceptions: [] },
      /allow\[0\] needs a non-empty "license" and a non-empty "reason"/,
    ],
    [
      "an exception without a package",
      { allow: [], exceptions: [{ license: "OFL-1.1", reason: "r" }] },
      /exceptions\[0\]/,
    ],
    [
      "an invalid id",
      { allow: [{ license: "BSD", reason: "r" }], exceptions: [] },
      /not an SPDX licence id/,
    ],
    [
      "an expression",
      { allow: [{ license: "MIT OR ISC", reason: "r" }], exceptions: [] },
      /one plain SPDX id/,
    ],
  ])("rejects %s", (_label, raw, message) => {
    expect(() => validatePolicy(raw)).toThrow(message)
  })
})

describe("runLicenseCheck", () => {
  /** @type {string[]} */
  const dirs = []
  afterEach(() => {
    for (const dir of dirs.splice(0))
      rmSync(dir, { recursive: true, force: true })
  })

  /**
   * @param {object} policy - the policy to write.
   * @returns {string} the path of a temporary policy file.
   */
  const writePolicy = (policy) => {
    const dir = mkdtempSync(join(tmpdir(), "admin-licenses-"))
    dirs.push(dir)
    const path = join(dir, "policy.json")
    writeFileSync(path, JSON.stringify(policy))
    return path
  }

  /**
   * @param {Record<string, string | undefined>} licenses - the fake tree.
   * @param {object} [policy] - the policy.
   */
  const run = async (licenses, policy = POLICY) => {
    /** @type {string[]} */
    const out = []
    /** @type {string[]} */
    const err = []
    const code = await runLicenseCheck({
      start: "/project",
      policyPath: writePolicy(policy),
      readTree: async () => tree(licenses),
      log: (line) => out.push(line),
      logError: (line) => err.push(line),
    })
    return { code, out, err }
  }

  it("exits 0 and says so when every package passes", async () => {
    const { code, out } = await run({
      "a@1.0.0": "MIT",
      "b@1.0.0": "MPL-2.0",
      "c@1.0.0": "Apache-2.0",
      "@fonts/inter@5.0.0": "OFL-1.1",
    })
    expect(code).toBe(0)
    expect(out).toContain(
      "Admin licenses: all 4 production packages pass admin/license-policy.json."
    )
  })

  it("exits 1 and lists every violation, not only the first", async () => {
    const { code, err } = await run({
      "a@1.0.0": "GPL-2.0-only",
      "b@1.0.0": undefined,
    })
    expect(code).toBe(1)
    expect(err[0]).toBe(
      "Admin licenses: 2 of 2 production packages fail the policy:"
    )
    expect(err).toHaveLength(3)
  })

  it("warns about unused policy entries without failing", async () => {
    const { code, out } = await run({ "a@1.0.0": "MIT" })
    expect(code).toBe(0)
    expect(
      out.some((line) =>
        line.includes('"MPL-2.0" is allowed but no production package uses it')
      )
    ).toBe(true)
  })

  it("fails when the tree is empty, which means npm ci did not run", async () => {
    const { code, err } = await run({})
    expect(code).toBe(1)
    expect(err[0]).toMatch(/Run `npm ci` in admin\/ first/)
  })
})

describe("parseArgs", () => {
  it("defaults to this admin project", () => {
    const { start, policyPath } = parseArgs([])
    expect(policyPath).toBe(resolve(start, "license-policy.json"))
  })

  it("takes --start and --policy", () => {
    expect(parseArgs(["--start", "/a", "--policy", "/b/p.json"])).toEqual({
      start: "/a",
      policyPath: "/b/p.json",
    })
  })

  it.each([[["--start"]], [["--start", "--policy"]], [["--frobnicate", "x"]]])(
    "rejects %j",
    (argv) => {
      expect(() => parseArgs(argv)).toThrow(/Usage: check-licenses.mjs/)
    }
  )
})
