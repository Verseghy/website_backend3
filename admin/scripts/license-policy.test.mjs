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

  it("tells the operator to add an exception for the failing package, not to widen allow (ADR 0027)", () => {
    const [violation] = evaluateLicenses(
      tree({ "@scope/other@1.0.0": "OFL-1.1" }),
      POLICY
    ).violations
    expect(violation.problem).toBe(
      'licence not allowed by admin/license-policy.json. Replace the dependency, or add an `exceptions` entry for "@scope/other" with the licence it is used under, in a PR that states why that licence may be combined with AGPL-3.0-only (ADR 0022 §4, ADR 0027). `allow` takes only the licences of ADR 0022 §4\'s allow-list, and no entry can admit a licence exception (WITH).'
    )
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
    [
      "an exception for a licence that allow admits for every package",
      {
        allow: [
          { license: "MIT", reason: "r" },
          { license: "0BSD", reason: "r" },
        ],
        exceptions: [
          { package: "@fonts/inter", license: "OFL-1.1", reason: "r" },
          { package: "tslib", license: "0BSD", reason: "r" },
        ],
      },
      /exceptions\[1\] scopes "0BSD" to tslib, but allow already admits "0BSD" for every package, so the exception has no effect\. Remove "0BSD" from allow to scope it, or remove the exception \(ADR 0027\)\./,
    ],
  ])("rejects %s", (_label, raw, message) => {
    expect(() => validatePolicy(raw)).toThrow(message)
  })
})

/**
 * The committed policy itself, as ADR 0027 decides it: a licence beyond ADR
 * 0022 §4's table enters for named packages only, like deny.toml's crate
 * exceptions.
 */
describe("the committed license-policy.json (ADR 0027)", () => {
  // Read inside each test, never while the file is collected, so that a
  // syntax error in either file fails these tests with its message instead of
  // the whole test file. Not validated here: "accepts the committed policy"
  // above reports a malformed policy.
  /** @returns {import("./license-policy.mjs").LicensePolicy} the policy. */
  const readCommitted = () =>
    JSON.parse(
      readFileSync(
        resolve(import.meta.dirname, "../license-policy.json"),
        "utf8"
      )
    )

  // ADR 0022 §4's `[licenses] allow` row. The test below ties this copy to
  // deny.toml, so the ADR, deny.toml and the npm policy cannot drift apart.
  const ADR_0022_TABLE = [
    "MIT",
    "Apache-2.0",
    "ISC",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "Zlib",
    "BSL-1.0",
    "MPL-2.0",
    "Unicode-3.0",
  ]

  /**
   * The licence ids of deny.toml's `[licenses] allow` array. A small reader,
   * not a TOML parser: the admin has no TOML dependency, and this one array
   * of quoted ids is all it needs. Comments go first, so an id or a bracket
   * in a comment is never read. The Admin test job checks out the whole
   * repository, so `../../deny.toml` is there.
   *
   * @returns {string[]} the ids, in file order.
   * @throws {Error} when the table or the array is missing.
   */
  const readDenyTomlAllow = () => {
    const text = readFileSync(
      resolve(import.meta.dirname, "../../deny.toml"),
      "utf8"
    )
      .split("\n")
      .map((line) => line.replace(/#.*$/, ""))
      .join("\n")
    const table = /^\[licenses\][ \t]*$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(text)
    if (table === null) {
      throw new Error("deny.toml has no [licenses] table.")
    }
    const array = /^allow\s*=\s*\[([^\]]*)\]/m.exec(table[1])
    if (array === null) {
      throw new Error("deny.toml's [licenses] table has no `allow = [...]`.")
    }
    return [...array[1].matchAll(/"([^"]*)"/g)].map((match) => match[1])
  }

  it("matches ADR 0022 §4's table with deny.toml's allow", () => {
    expect(readDenyTomlAllow()).toEqual(ADR_0022_TABLE)
  })

  it("allows for every package only licences of ADR 0022 §4's table", () => {
    const outside = readCommitted()
      .allow.map((entry) => entry.license)
      .filter((license) => !ADR_0022_TABLE.includes(license))
    expect(outside).toEqual([])
  })

  it("passes each licence beyond the table for its named packages and fails it for any other", () => {
    const committed = readCommitted()
    const result = evaluateLicenses(
      tree({
        // The packages ADR 0027 names, at the versions installed today.
        "tslib@2.8.1": "0BSD",
        "isexe@3.1.5": "BlueOak-1.0.0",
        "minimatch@10.2.6": "BlueOak-1.0.0",
        "@fontsource-variable/inter@5.3.0": "OFL-1.1",
        "caniuse-lite@1.0.30001819": "CC-BY-4.0",
        // glob 13 is BlueOak-1.0.0 and already in the installed tree as a
        // development dependency; it must not reach production unreviewed.
        "glob@13.0.6": "BlueOak-1.0.0",
        "some-helper@1.0.0": "0BSD",
        "some-font@1.0.0": "OFL-1.1",
        "some-data@1.0.0": "CC-BY-4.0",
      }),
      committed
    )
    expect(result.violations.map((violation) => violation.id)).toEqual([
      "glob@13.0.6",
      "some-helper@1.0.0",
      "some-font@1.0.0",
      "some-data@1.0.0",
    ])
    // ADR 0027's case: a dependency update brings another BlueOak-1.0.0
    // package into production. The advice is the one-line exception PR.
    expect(result.violations[0].problem).toMatch(
      /add an `exceptions` entry for "glob" with the licence it is used under/
    )
    expect(result.violations[0].problem).toMatch(
      /`allow` takes only the licences of ADR 0022 §4's allow-list/
    )
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
