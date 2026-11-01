// @vitest-environment node
import { execFileSync } from "node:child_process"
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"

import fc from "fast-check"
import { afterEach, describe, expect, it } from "vitest"

import {
  CHECKED_FILES,
  ENTERPRISE_SCOPE,
  findEnterpriseJsonNames,
  findEnterprisePackages,
  parseArgs,
  runCleanRoomCheck,
} from "./check-clean-room.mjs"

const ADMIN_DIR = resolve(import.meta.dirname, "..")
const SCRIPT = resolve(import.meta.dirname, "check-clean-room.mjs")

/**
 * Runs the check over in-memory files.
 *
 * @param {Record<string, string>} files - contents by file name.
 * @returns {{ code: number, out: string[], err: string[] }} the result.
 */
function run(files) {
  /** @type {string[]} */
  const out = []
  /** @type {string[]} */
  const err = []
  const code = runCleanRoomCheck({
    adminDir: "/admin",
    readFile: (path) => {
      const name = path.replace(/^\/admin\//, "")
      if (!(name in files)) {
        throw new Error(`ENOENT: ${name}`)
      }
      return files[name]
    },
    log: (line) => out.push(line),
    logError: (line) => err.push(line),
  })
  return { code, out, err }
}

const CLEAN_PACKAGE = JSON.stringify(
  { dependencies: { "ra-core": "^5.15.5", "@base-ui/react": "^1.9.0" } },
  null,
  2
)
const CLEAN_LOCK = JSON.stringify(
  {
    packages: {
      "node_modules/ra-core": { version: "5.15.5" },
      "node_modules/@react-aria/utils": { version: "1.0.0" },
    },
  },
  null,
  2
)

describe("findEnterprisePackages", () => {
  it("finds nothing in a file without the scope", () => {
    expect(findEnterprisePackages("package.json", CLEAN_PACKAGE)).toEqual([])
  })

  it("does not mistake look-alike names for the scope", () => {
    const text = [
      '"react-admin": "^5"',
      '"@react-aria/utils": "1"',
      '"ra-core": "^5"',
      '"@react-admin": "x"',
      '"react-admin/x": "y"',
    ].join("\n")
    expect(findEnterprisePackages("package.json", text)).toEqual([])
  })

  it.each([
    ['"@react-admin/ra-tree": "^5.0.0"', "a dependency"],
    ['"tree": "npm:@react-admin/ra-tree@5"', "an alias"],
    ['"node_modules/@react-admin/ra-history": {', "a lockfile package"],
    [
      '"resolved": "https://registry.marmelab.com/@react-admin/ra-audit-log/-/x.tgz"',
      "a resolved URL",
    ],
  ])("finds %s (%s) with its line number", (line) => {
    const text = ["{", `  ${line}`, "}"].join("\r\n")
    expect(findEnterprisePackages("package-lock.json", text)).toEqual([
      { file: "package-lock.json", line: 2, text: line },
    ])
  })

  it("reports one finding per line that names the scope, in order", () => {
    fc.assert(
      fc.property(
        fc.array(
          fc.oneof(
            fc
              .string()
              .filter(
                (line) =>
                  !line.includes(ENTERPRISE_SCOPE) && !/[\r\n]/.test(line)
              ),
            fc.constant(`"${ENTERPRISE_SCOPE}ra-tree": "1"`)
          ),
          { maxLength: 20 }
        ),
        (lines) => {
          const findings = findEnterprisePackages("f", lines.join("\n"))
          const expected = lines
            .map((line, index) => ({ line, number: index + 1 }))
            .filter(({ line }) => line.includes(ENTERPRISE_SCOPE))
            .map(({ number }) => number)
          expect(findings.map((finding) => finding.line)).toEqual(expected)
        }
      )
    )
  })
})

describe("findEnterpriseJsonNames", () => {
  it("finds nothing in a document without the scope", () => {
    expect(
      findEnterpriseJsonNames("package.json", JSON.parse(CLEAN_PACKAGE))
    ).toEqual([])
    expect(
      findEnterpriseJsonNames("package-lock.json", JSON.parse(CLEAN_LOCK))
    ).toEqual([])
  })

  it("finds a key and a value written with JSON escapes, with their paths", () => {
    // \u0040 is "@", and \/ is "/": both decode to the scope, but neither
    // file's text holds it literally.
    const text = String.raw`{
      "dependencies": { "\u0040react-admin/ra-tree": "^5" },
      "overrides": { "tree": "npm:@react-admin\/ra-tree@5" },
      "bundled": ["x", "\u0040react-admin/ra-history"]
    }`
    expect(text).not.toContain(ENTERPRISE_SCOPE + "ra-history")
    expect(findEnterpriseJsonNames("package.json", JSON.parse(text))).toEqual([
      {
        file: "package.json",
        path: '$.dependencies["@react-admin/ra-tree"]',
        kind: "key",
        text: "@react-admin/ra-tree",
      },
      {
        file: "package.json",
        path: "$.overrides.tree",
        kind: "value",
        text: "npm:@react-admin/ra-tree@5",
      },
      {
        file: "package.json",
        path: "$.bundled[1]",
        kind: "value",
        text: "@react-admin/ra-history",
      },
    ])
  })

  it("reports a key before the values below it, in document order", () => {
    const document = {
      a: "@react-admin/first",
      "@react-admin/second": { b: "@react-admin/third" },
      c: ["@react-admin/fourth"],
    }
    expect(
      findEnterpriseJsonNames("f", document).map((finding) => finding.text)
    ).toEqual([
      "@react-admin/first",
      "@react-admin/second",
      "@react-admin/third",
      "@react-admin/fourth",
    ])
  })

  it("ignores numbers, booleans and null, and a bare scope-like key", () => {
    expect(
      findEnterpriseJsonNames("f", {
        n: 1,
        t: true,
        z: null,
        "@react-admin": "x",
      })
    ).toEqual([])
  })

  it("walks a deeply nested document without overflowing the stack", () => {
    /** @type {unknown} */
    let document = "@react-admin/deep"
    for (let depth = 0; depth < 100_000; depth += 1) {
      document = [document]
    }
    const findings = findEnterpriseJsonNames("f", document)
    expect(findings).toHaveLength(1)
    expect(findings[0].text).toBe("@react-admin/deep")
  })

  it("finds a name exactly when the canonical encoding holds the scope", () => {
    const name = fc.oneof(
      fc.string(),
      fc.string().map((tail) => `${ENTERPRISE_SCOPE}${tail}`)
    )
    const document = fc.letrec((tie) => ({
      value: fc.oneof(
        { depthSize: "small" },
        name,
        fc.integer(),
        fc.boolean(),
        fc.constant(null),
        fc.array(tie("value"), { maxLength: 4 }),
        fc.dictionary(name, tie("value"), { maxKeys: 4 })
      ),
    })).value
    fc.assert(
      fc.property(document, (value) => {
        // JSON.stringify never escapes "@" or "/", and the scope holds no
        // quote, so the scope appears in the encoding only inside one key or
        // string, as that key or string's own text.
        expect(findEnterpriseJsonNames("f", value).length > 0).toBe(
          JSON.stringify(value).includes(ENTERPRISE_SCOPE)
        )
      })
    )
  })
})

describe("runCleanRoomCheck", () => {
  it("passes when neither file names the scope", () => {
    const result = run({
      "package.json": CLEAN_PACKAGE,
      "package-lock.json": CLEAN_LOCK,
    })
    expect(result.code).toBe(0)
    expect(result.err).toEqual([])
    expect(result.out.join("\n")).toContain("names an @react-admin/ package")
  })

  it("fails on a dependency in package.json and names the file, the line and the fix", () => {
    const result = run({
      "package.json":
        '{\n  "dependencies": {\n    "@react-admin/ra-tree": "^5"\n  }\n}',
      "package-lock.json": CLEAN_LOCK,
    })
    expect(result.code).toBe(1)
    const message = result.err.join("\n")
    expect(message).toContain("ADR 0011 §8, ADR 0018 §3.4")
    expect(message).toContain('package.json:3: "@react-admin/ra-tree": "^5"')
    expect(message).toContain("build the feature from the ADRs")
  })

  it("fails on a package that only the lockfile names", () => {
    const result = run({
      "package.json": CLEAN_PACKAGE,
      "package-lock.json":
        '{\n  "node_modules/@react-admin/ra-soft-delete": {}\n}',
    })
    expect(result.code).toBe(1)
    expect(result.err.join("\n")).toContain("package-lock.json:2:")
  })

  it("reports a name that npm writes as it reads once, by its line", () => {
    const result = run({
      "package.json":
        '{\n  "dependencies": {\n    "@react-admin/ra-tree": "^5"\n  }\n}',
      "package-lock.json": CLEAN_LOCK,
    })
    expect(result.err[0]).toContain("1 place(s)")
    expect(result.err.join("\n")).not.toContain("JSON escapes")
  })

  it("fails on a name written with JSON escapes and names its path", () => {
    const result = run({
      "package.json": String.raw`{"dependencies": {"\u0040react-admin/ra-tree": "^5"}}`,
      "package-lock.json": String.raw`{"packages": {"x": {"resolved": "https://r.example/@react-admin\/ra-tree.tgz"}}}`,
    })
    expect(result.code).toBe(1)
    const message = result.err.join("\n")
    expect(message).toContain("2 place(s)")
    expect(message).toContain(
      'package.json: the key at $.dependencies["@react-admin/ra-tree"], written with JSON escapes, reads "@react-admin/ra-tree"'
    )
    expect(message).toContain(
      'package-lock.json: the value at $.packages.x.resolved, written with JSON escapes, reads "https://r.example/@react-admin/ra-tree.tgz"'
    )
  })

  it.each(["package.json", "package-lock.json"])(
    "fails when %s is not JSON, because it cannot be searched completely",
    (broken) => {
      const files = {
        "package.json": CLEAN_PACKAGE,
        "package-lock.json": CLEAN_LOCK,
        [broken]: '{ "dependencies": ',
      }
      const result = run(files)
      expect(result.code).toBe(1)
      expect(result.err.join("\n")).toContain(
        `${broken} in /admin is not valid JSON`
      )
    }
  )

  it("fails exactly when a key or string value of either file names the scope", () => {
    const name = fc.oneof(
      fc.string(),
      fc.string().map((tail) => `${ENTERPRISE_SCOPE}${tail}`)
    )
    const manifest = fc.dictionary(
      name,
      fc.oneof(name, fc.dictionary(name, name, { maxKeys: 3 })),
      { maxKeys: 4 }
    )
    fc.assert(
      fc.property(manifest, fc.boolean(), (document, escape) => {
        let text = JSON.stringify(document, null, 2)
        if (escape) {
          // The spelling a hand edit could use: every "@" as \u0040.
          text = text.replaceAll("@", "\\u0040")
        }
        const result = run({
          "package.json": text,
          "package-lock.json": CLEAN_LOCK,
        })
        const named =
          findEnterpriseJsonNames("package.json", document).length > 0
        expect(result.code).toBe(named ? 1 : 0)
      })
    )
  })

  it("fails when a file is missing, instead of passing a check it could not run", () => {
    const result = run({ "package.json": CLEAN_PACKAGE })
    expect(result.code).toBe(1)
    expect(result.err.join("\n")).toContain("cannot read package-lock.json")
  })

  it("checks exactly the manifest and the lockfile", () => {
    expect(CHECKED_FILES).toEqual(["package.json", "package-lock.json"])
  })

  it("passes on this admin project's committed files", () => {
    const result = runCleanRoomCheck({
      adminDir: ADMIN_DIR,
      readFile: (path) => readFileSync(path, "utf8"),
      log: () => {},
      logError: () => {},
    })
    expect(result).toBe(0)
  })
})

describe("parseArgs", () => {
  it("defaults to the admin project", () => {
    expect(parseArgs([])).toEqual({ adminDir: ADMIN_DIR })
  })

  it("reads --dir", () => {
    expect(parseArgs(["--dir", "/srv/admin"])).toEqual({
      adminDir: "/srv/admin",
    })
  })

  it("refuses an unknown option and a missing value", () => {
    expect(() => parseArgs(["--start", "x"])).toThrow(/Unknown option --start/)
    expect(() => parseArgs(["--dir"])).toThrow(/--dir needs a value/)
  })
})

describe("the command line", () => {
  /** @type {string[]} */
  const directories = []
  afterEach(() => {
    for (const directory of directories.splice(0)) {
      rmSync(directory, { recursive: true, force: true })
    }
  })

  it("exits 1 for a project with an Enterprise package and 0 for a clean one", () => {
    const directory = mkdtempSync(join(tmpdir(), "clean-room-"))
    directories.push(directory)
    writeFileSync(join(directory, "package.json"), CLEAN_PACKAGE)
    writeFileSync(join(directory, "package-lock.json"), CLEAN_LOCK)
    expect(() =>
      execFileSync(process.execPath, [SCRIPT, "--dir", directory], {
        stdio: "pipe",
      })
    ).not.toThrow()

    writeFileSync(
      join(directory, "package-lock.json"),
      '{\n  "node_modules/@react-admin/ra-tree": {}\n}'
    )
    let status = 0
    try {
      execFileSync(process.execPath, [SCRIPT, "--dir", directory], {
        stdio: "pipe",
      })
    } catch (error) {
      status = /** @type {{ status: number }} */ (error).status
    }
    expect(status).toBe(1)
  })
})
