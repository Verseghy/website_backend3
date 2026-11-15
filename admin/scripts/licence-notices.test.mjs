// @vitest-environment node
/**
 * Unit tests of the licence-notices helpers and plugin (ADR 0038). The real
 * build is tested in `licence-notices.build.test.mjs`.
 */
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"

import fc from "fast-check"
import { afterEach, describe, expect, it } from "vitest"

import {
  COPIED_CODE_HEADING,
  COPIED_CODE_INTRO,
  LICENCE_NOTICES_BANNER,
  LICENCE_NOTICES_FILE,
  LICENCE_NOTICES_PATH,
  SOURCE_DIR,
  VITE_EMPTY,
  VITE_INTRO,
  addCopiedCode,
  addNotices,
  ancestorDirs,
  assetText,
  cssImportSpecifiers,
  findPackageDir,
  isCopiedLicenceFileName,
  isLicenceFileName,
  licenceNotices,
  listsPackage,
  noticeHeading,
  noticeSection,
  packageOfPath,
  packageOfSpecifier,
  parseNotices,
  readCopiedLicences,
  readNoticeEntry,
} from "./licence-notices.mjs"

/** Vite's file for one package, as `licenseEntryToMarkdown` writes it. */
const VITE_FILE = `# Licenses\n\n${VITE_INTRO}\n\n## react - 19.3.1 (MIT)\n\nMIT License\n`

/**
 * A package-name segment: what npm allows, without `/` or a leading dot, and
 * never `node_modules`, which would move the package boundary.
 */
const segment = fc
  .stringMatching(/^[a-z0-9][a-z0-9._-]{0,11}$/)
  .filter((value) => value !== "node_modules")

/** @type {string[]} the directories that `afterEach` removes */
const dirs = []

/**
 * Creates a throwaway directory that `afterEach` removes.
 *
 * @returns {string} its path.
 */
function tempDir() {
  const dir = mkdtempSync(join(tmpdir(), "licence-notices-"))
  dirs.push(dir)
  return dir
}

/**
 * Writes an installed package into `root/node_modules`.
 *
 * @param {string} root - the project directory.
 * @param {string} name - the package name.
 * @param {Record<string, string>} files - file name to contents; a
 *   `package.json` is written for the name unless `files` has one.
 * @returns {string} the package directory.
 */
function installPackage(root, name, files) {
  const dir = join(root, "node_modules", name)
  mkdirSync(dir, { recursive: true })
  const all = {
    "package.json": JSON.stringify({ name, version: "1.2.3", license: "MIT" }),
    ...files,
  }
  for (const [file, text] of Object.entries(all)) {
    writeFileSync(join(dir, file), text)
  }
  return dir
}

afterEach(() => {
  for (const dir of dirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true })
  }
})

describe("the constants", () => {
  it("serve the file from the dist root under /admin", () => {
    expect(LICENCE_NOTICES_FILE).toBe("licenses.txt")
    expect(LICENCE_NOTICES_PATH).toBe("/admin/licenses.txt")
  })

  it("make the banner a legal comment that names the URL", () => {
    expect(LICENCE_NOTICES_BANNER).toMatch(/^\/\*!.*\*\/$/)
    expect(LICENCE_NOTICES_BANNER).toContain(LICENCE_NOTICES_PATH)
    // Closing the comment early would break every chunk.
    expect(LICENCE_NOTICES_BANNER.slice(3, -2)).not.toContain("*/")
  })
})

describe("cssImportSpecifiers", () => {
  it("reads every @import form", () => {
    const css = [
      '@import "tailwindcss";',
      "@import 'tw-animate-css';",
      '@import url("shadcn/tailwind.css");',
      "@import url('a/b.css') layer(base);",
      "@import url(plain.css) screen;",
      '@IMPORT "@fontsource-variable/inter" supports(display: grid);',
      '@import   "./local.css"  ;',
    ].join("\n")
    expect(cssImportSpecifiers(css)).toEqual([
      "tailwindcss",
      "tw-animate-css",
      "shadcn/tailwind.css",
      "a/b.css",
      "plain.css",
      "@fontsource-variable/inter",
      "./local.css",
    ])
  })

  it("skips commented-out imports and other at-rules", () => {
    const css = [
      '/* @import "commented"; */',
      "/*",
      '@import "also-commented";',
      "*/",
      '@plugin "a-plugin";',
      '@reference "x";',
      '.a { content: "x" }',
      '@import "kept";',
      '/* an unterminated comment @import "lost";',
    ].join("\n")
    expect(cssImportSpecifiers(css)).toEqual(["kept"])
  })

  it("finds every generated import, whatever surrounds it", () => {
    fc.assert(
      fc.property(
        fc.array(fc.tuple(segment, fc.constantFrom('"', "'", "url(")), {
          maxLength: 8,
        }),
        (imports) => {
          const css = imports
            .map(([name, quote]) =>
              quote === "url("
                ? `@import url(${name});`
                : `@import ${quote}${name}${quote};`
            )
            .join("\n.x { color: red }\n")
          expect(cssImportSpecifiers(css)).toEqual(
            imports.map(([name]) => name)
          )
        }
      )
    )
  })

  it("never throws", () => {
    fc.assert(
      fc.property(fc.string(), (css) => {
        expect(Array.isArray(cssImportSpecifiers(css))).toBe(true)
      })
    )
  })
})

describe("packageOfSpecifier", () => {
  it("names the package of bare specifiers", () => {
    expect(packageOfSpecifier("tailwindcss")).toBe("tailwindcss")
    expect(packageOfSpecifier("shadcn/tailwind.css")).toBe("shadcn")
    expect(packageOfSpecifier("@fontsource-variable/inter")).toBe(
      "@fontsource-variable/inter"
    )
    expect(packageOfSpecifier("@scope/name/sub/file.css")).toBe("@scope/name")
    expect(packageOfSpecifier(" tw-animate-css ")).toBe("tw-animate-css")
  })

  it("rejects what is not a bare specifier", () => {
    for (const specifier of [
      "",
      "   ",
      "./local.css",
      "../up.css",
      "/absolute.css",
      "\\windows.css",
      "#internal",
      "~webpack-style",
      "https://example.com/a.css",
      "data:text/css,a{}",
      "node:fs",
      "@scope",
      "@/alias",
      "@scope/",
    ]) {
      expect(packageOfSpecifier(specifier), specifier).toBeNull()
    }
  })

  it("returns the specifier's first segment, or its first two when scoped", () => {
    fc.assert(
      fc.property(
        fc.option(segment, { nil: undefined }),
        segment,
        fc.array(segment, { maxLength: 3 }),
        (scope, name, rest) => {
          const pkg = scope === undefined ? name : `@${scope}/${name}`
          expect(packageOfSpecifier([pkg, ...rest].join("/"))).toBe(pkg)
        }
      )
    )
  })
})

describe("packageOfPath", () => {
  it("finds the package of a file under node_modules", () => {
    expect(
      packageOfPath(
        "node_modules/@fontsource-variable/inter/files/inter-latin.woff2"
      )
    ).toEqual({
      name: "@fontsource-variable/inter",
      dir: "node_modules/@fontsource-variable/inter",
    })
    expect(packageOfPath("/app/node_modules/a/node_modules/b/x.js")).toEqual({
      name: "b",
      dir: "/app/node_modules/a/node_modules/b",
    })
    expect(packageOfPath("C:\\app\\node_modules\\pkg\\img.png")).toEqual({
      name: "pkg",
      dir: "C:/app/node_modules/pkg",
    })
  })

  it("rejects files outside node_modules and bare package directories", () => {
    for (const path of [
      "index.html",
      "src/assets/logo.svg",
      "my_node_modules/pkg/x.js",
      "node_modules/pkg",
      "node_modules/@scope/name",
      "node_modules/@scope",
      "node_modules//x.js",
    ]) {
      expect(packageOfPath(path), path).toBeNull()
    }
  })

  it("agrees with packageOfSpecifier on the path below node_modules", () => {
    fc.assert(
      fc.property(
        fc.array(segment, { maxLength: 3 }),
        fc.option(segment, { nil: undefined }),
        segment,
        fc.array(segment, { minLength: 1, maxLength: 3 }),
        (prefix, scope, name, rest) => {
          const pkg = scope === undefined ? name : `@${scope}/${name}`
          const below = [pkg, ...rest].join("/")
          const path = [...prefix, "node_modules", below].join("/")
          const owner = packageOfPath(path)
          expect(owner?.name).toBe(packageOfSpecifier(below))
          expect(path.startsWith(owner?.dir ?? "\0")).toBe(true)
        }
      )
    )
  })
})

describe("the sections", () => {
  it("are written as Vite writes them", () => {
    const entry = { name: "a", version: "1.0.0", identifier: "MIT", text: "T" }
    expect(noticeHeading(entry)).toBe("## a - 1.0.0 (MIT)")
    expect(noticeSection(entry)).toBe("\n## a - 1.0.0 (MIT)\n\nT\n")
    expect(noticeSection({ name: "b", version: "0.0.0" })).toBe(
      "\n## b - 0.0.0\n"
    )
  })

  it("are found again by listsPackage, with or without an identifier", () => {
    expect(listsPackage(VITE_FILE, { name: "react", version: "19.3.1" })).toBe(
      true
    )
    expect(
      listsPackage("## b - 2.0.0\n", { name: "b", version: "2.0.0" })
    ).toBe(true)
    expect(listsPackage(VITE_FILE, { name: "react", version: "19.3.2" })).toBe(
      false
    )
    expect(listsPackage(VITE_FILE, { name: "reac", version: "19.3.1" })).toBe(
      false
    )
    // A name inside a licence text is not a heading.
    expect(
      listsPackage("Uses react - 19.3.1 (MIT)\n", {
        name: "react",
        version: "19.3.1",
      })
    ).toBe(false)
  })
})

describe("addNotices", () => {
  const tailwind = {
    name: "tailwindcss",
    version: "4.3.3",
    identifier: "MIT",
    text: "MIT License\n\nCopyright (c) Tailwind Labs, Inc.",
  }
  const inter = { name: "@fontsource-variable/inter", version: "5.3.0" }

  it("appends missing packages after Vite's, in Vite's order", () => {
    const result = addNotices(VITE_FILE, [tailwind, inter])
    expect(result).toBe(
      `${VITE_FILE}\n## @fontsource-variable/inter - 5.3.0\n` +
        "\n## tailwindcss - 4.3.3 (MIT)\n\nMIT License\n\nCopyright (c) Tailwind Labs, Inc.\n"
    )
  })

  it("skips listed packages and duplicates", () => {
    const react = { name: "react", version: "19.3.1", identifier: "MIT" }
    expect(addNotices(VITE_FILE, [react])).toBe(VITE_FILE)
    const twice = addNotices(VITE_FILE, [tailwind, tailwind])
    expect(twice.split("## tailwindcss - ").length).toBe(2)
  })

  it("replaces Vite's empty-file sentence when it adds the first package", () => {
    const empty = `# Licenses\n\n${VITE_EMPTY}\n`
    const result = addNotices(empty, [inter])
    expect(result).toBe(
      `# Licenses\n\n${VITE_INTRO}\n\n## @fontsource-variable/inter - 5.3.0\n`
    )
    expect(addNotices(empty, [])).toBe(empty)
  })

  it("ends the text with a newline before the new sections", () => {
    expect(addNotices("# Licenses", [inter])).toBe(
      "# Licenses\n\n## @fontsource-variable/inter - 5.3.0\n"
    )
  })

  it("is idempotent and lists every entry once", () => {
    const entry = fc.record({
      name: segment,
      version: fc.constantFrom("1.0.0", "2.1.3", "0.0.0"),
      identifier: fc.option(fc.constantFrom("MIT", "ISC", "OFL-1.1"), {
        nil: undefined,
      }),
      text: fc.option(fc.constantFrom("MIT License", "ISC License"), {
        nil: undefined,
      }),
    })
    fc.assert(
      fc.property(fc.array(entry, { maxLength: 8 }), (entries) => {
        const once = addNotices(VITE_FILE, entries)
        expect(addNotices(once, entries)).toBe(once)
        expect(once.startsWith(VITE_FILE)).toBe(true)
        for (const item of entries) {
          expect(listsPackage(once, item)).toBe(true)
        }
      })
    )
  })
})

describe("the licence file names", () => {
  it("of a package are the ones Vite accepts, in any case", () => {
    for (const name of ["LICENSE", "license.md", "Licence", "COPYING.txt"]) {
      expect(isLicenceFileName(name), name).toBe(true)
    }
    for (const name of ["README.md", "package.json", "MIT-LICENSE", ""]) {
      expect(isLicenceFileName(name), name).toBe(false)
    }
  })

  it("of copied code are the capitalised ones, so no module of src/ counts", () => {
    for (const name of [
      "LICENSE",
      "LICENCE",
      "COPYING",
      "LICENSE.md",
      "LICENSE-MIT",
      "LICENSE.ra-language-hungarian",
    ]) {
      expect(isCopiedLicenceFileName(name), name).toBe(true)
    }
    for (const name of [
      "license",
      "License.md",
      "licence-link.tsx",
      "license-policy.ts",
      "LICENSES",
      "LICENSEE.md",
      "LOCAL_CHANGES.md",
      "",
    ]) {
      expect(isCopiedLicenceFileName(name), name).toBe(false)
    }
  })

  it("of copied code are always licence file names of a package too", () => {
    fc.assert(
      fc.property(fc.string(), (name) => {
        if (isCopiedLicenceFileName(name)) {
          expect(isLicenceFileName(name)).toBe(true)
        }
      })
    )
  })
})

describe("ancestorDirs", () => {
  it("lists the directories from the file's up to the top", () => {
    expect(ancestorDirs("/a/src/components/admin/x.tsx", "/a/src")).toEqual([
      "/a/src/components/admin",
      "/a/src/components",
      "/a/src",
    ])
    expect(ancestorDirs("/a/src/main.tsx", "/a/src/")).toEqual(["/a/src"])
  })

  it("lists nothing for a file outside the top, or the top itself", () => {
    for (const file of [
      "/a/main.tsx",
      "/a/srcx/y.ts",
      "/b/src/y.ts",
      "/a/src",
    ]) {
      expect(ancestorDirs(file, "/a/src"), file).toEqual([])
    }
    // A name that only starts with two dots is inside.
    expect(ancestorDirs("/a/src/..x/y.ts", "/a/src")).toEqual([
      "/a/src/..x",
      "/a/src",
    ])
  })

  it("walks one directory per path segment, each the parent of the one before", () => {
    fc.assert(
      fc.property(
        fc.array(segment, { minLength: 1, maxLength: 3 }),
        fc.array(segment, { maxLength: 4 }),
        segment,
        (topSegments, below, name) => {
          const top = `/${topSegments.join("/")}`
          const dirs = ancestorDirs(`${top}/${[...below, name].join("/")}`, top)
          expect(dirs.length).toBe(below.length + 1)
          expect(dirs.at(-1)).toBe(top)
          dirs.slice(1).forEach((dir, index) => {
            expect(resolve(dirs[index], "..")).toBe(dir)
          })
        }
      )
    )
  })
})

describe("addCopiedCode", () => {
  const kit = { path: "src/components/admin/LICENSE", text: "MIT License kit" }
  const ui = { path: "src/components/ui/LICENSE", text: "MIT License ui" }

  it("appends a section after the packages, sorted by path", () => {
    expect(addCopiedCode(VITE_FILE, [ui, kit, ui])).toBe(
      `${VITE_FILE}\n${COPIED_CODE_HEADING}\n\n${COPIED_CODE_INTRO}\n` +
        "\n## src/components/admin/LICENSE\n\nMIT License kit\n" +
        "\n## src/components/ui/LICENSE\n\nMIT License ui\n"
    )
  })

  it("leaves the file alone without copied code", () => {
    expect(addCopiedCode(VITE_FILE, [])).toBe(VITE_FILE)
  })

  it("wraps its intro like the licence texts", () => {
    for (const line of COPIED_CODE_INTRO.split("\n")) {
      expect(line.length).toBeLessThanOrEqual(80)
    }
  })
})

/** A licence text line that `parseNotices` could not take for a heading. */
const textLine = fc.oneof(
  fc.string({ maxLength: 20 }).filter((line) => !line.startsWith("#")),
  // Markdown headings of licence texts.
  fc.constantFrom("## Terms", "## Section 1 - Definitions", "### 2.1", "#")
)

/** A trimmed licence text of some lines, possibly with blank ones. */
const licenceText = fc
  .array(fc.oneof(textLine, fc.constant("")), { maxLength: 6 })
  .map((lines) => lines.join("\n").trim())

/** A package entry with an npm-like name and a semver version. */
const packageEntry = fc.record(
  {
    name: fc.oneof(
      segment,
      fc.tuple(segment, segment).map(([scope, name]) => `@${scope}/${name}`)
    ),
    version: fc.constantFrom("1.0.0", "0.0.0", "19.3.1", "2.0.0-beta.1"),
    identifier: fc.constantFrom("MIT", "ISC", "MIT OR Apache-2.0"),
    text: licenceText,
  },
  { requiredKeys: ["name", "version"] }
)

/**
 * The first item of each key, in order: how addNotices and addCopiedCode
 * treat duplicates.
 *
 * @template T
 * @param {T[]} items - the items.
 * @param {(item: T) => string} key - the key of an item.
 * @returns {T[]} the first item of every key.
 */
function firstOfEach(items, key) {
  /** @type {Map<string, T>} */
  const first = new Map()
  for (const item of items) {
    if (!first.has(key(item))) first.set(key(item), item)
  }
  return [...first.values()]
}

describe("parseNotices", () => {
  it("reads Vite's entries and the copied code back", () => {
    const file = addCopiedCode(
      addNotices(VITE_FILE, [{ name: "b", version: "2.0.0" }]),
      [{ path: "src/i18n/LICENSE.x", text: "MIT License\n\n## Terms\n\nT" }]
    )
    expect(parseNotices(file)).toEqual({
      packages: [
        {
          name: "react",
          version: "19.3.1",
          identifier: "MIT",
          body: "MIT License",
        },
        { name: "b", version: "2.0.0", body: "" },
      ],
      copied: [
        { path: "src/i18n/LICENSE.x", text: "MIT License\n\n## Terms\n\nT" },
      ],
    })
  })

  it("keeps a licence text's Markdown headings in its entry", () => {
    const file = `${VITE_FILE}\n## Section 1 - Definitions\n\n## Terms\n`
    const { packages } = parseNotices(file)
    expect(packages.map(({ name }) => name)).toEqual(["react"])
    expect(packages[0].body).toBe(
      "MIT License\n\n## Section 1 - Definitions\n\n## Terms"
    )
  })

  it("takes a heading only after a blank line", () => {
    const file = "# Licenses\n\nintro:\n\n## a - 1.0.0\nT\n## b - 1.0.0\n"
    expect(parseNotices(file).packages).toEqual([
      { name: "a", version: "1.0.0", body: "T\n## b - 1.0.0" },
    ])
  })

  it("finds nothing in a file without headings", () => {
    expect(parseNotices(`# Licenses\n\n${VITE_EMPTY}\n`)).toEqual({
      packages: [],
      copied: [],
    })
  })

  it("reads back what addNotices and addCopiedCode write", () => {
    fc.assert(
      fc.property(
        fc.array(packageEntry, { maxLength: 6 }),
        fc.array(
          fc.record({
            path: fc
              .array(segment, { minLength: 1, maxLength: 3 })
              .map((parts) => `${SOURCE_DIR}/${parts.join("/")}/LICENSE`),
            text: licenceText,
          }),
          { maxLength: 4 }
        ),
        (entries, copied) => {
          const file = addCopiedCode(
            addNotices(`# Licenses\n\n${VITE_EMPTY}\n`, entries),
            copied
          )
          const parsed = parseNotices(file)
          const unique = firstOfEach(
            entries,
            (entry) => `${entry.name}@${entry.version}`
          )
          expect(
            parsed.packages.map(({ name, version, identifier, body }) => ({
              id: `${name}@${version}`,
              identifier,
              body,
            }))
          ).toEqual(
            unique
              .sort((a, b) => {
                const [x, y] = [
                  `${a.name}@${a.version}`,
                  `${b.name}@${b.version}`,
                ]
                return x < y ? -1 : x > y ? 1 : 0
              })
              .map((entry) => ({
                id: `${entry.name}@${entry.version}`,
                identifier: entry.identifier,
                body: entry.text ?? "",
              }))
          )
          const paths = firstOfEach(copied, (licence) => licence.path)
          expect(parsed.copied).toEqual(
            paths.sort((a, b) =>
              a.path < b.path ? -1 : a.path > b.path ? 1 : 0
            )
          )
        }
      )
    )
  })

  it("never throws", () => {
    fc.assert(
      fc.property(fc.string(), (text) => {
        const parsed = parseNotices(text)
        expect(Array.isArray(parsed.packages)).toBe(true)
        expect(Array.isArray(parsed.copied)).toBe(true)
      })
    )
  })
})

describe("assetText", () => {
  it("returns a string as it is and decodes bytes as UTF-8", () => {
    expect(assetText("© a")).toBe("© a")
    expect(assetText(new TextEncoder().encode("© Árvíztűrő"))).toBe(
      "© Árvíztűrő"
    )
  })

  it("gives the same text for a string and its UTF-8 bytes", () => {
    fc.assert(
      fc.property(fc.string({ unit: "grapheme" }), (text) => {
        expect(assetText(new TextEncoder().encode(text))).toBe(assetText(text))
      })
    )
  })
})

describe("findPackageDir", () => {
  it("looks in node_modules of the directory and of every parent", () => {
    const root = tempDir()
    const top = installPackage(root, "top", {})
    const nested = join(root, "node_modules", "top", "deep", "dir")
    mkdirSync(nested, { recursive: true })
    const inner = installPackage(join(root, "node_modules", "top"), "inner", {})
    expect(findPackageDir(nested, "top")).toBe(top)
    expect(findPackageDir(nested, "inner")).toBe(inner)
    expect(findPackageDir(root, "inner")).toBeNull()
    expect(findPackageDir(root, "missing")).toBeNull()
  })

  it("finds scoped packages", () => {
    const root = tempDir()
    const dir = installPackage(root, "@scope/pkg", {})
    expect(findPackageDir(join(root, "src"), "@scope/pkg")).toBe(dir)
  })
})

describe("readNoticeEntry", () => {
  it("reads the name, version, licence and licence text", () => {
    const root = tempDir()
    const dir = installPackage(root, "pkg", {
      "README.md": "not a licence",
      "LICENSE.md": "\n  MIT License\n\n",
    })
    expect(readNoticeEntry(dir)).toEqual({
      name: "pkg",
      version: "1.2.3",
      identifier: "MIT",
      text: "MIT License",
    })
  })

  it("accepts the licence file names Vite accepts, and none", () => {
    const root = tempDir()
    for (const [name, file] of [
      ["a", "licence"],
      ["b", "COPYING.txt"],
      ["c", "License-MIT"],
    ]) {
      expect(
        readNoticeEntry(installPackage(root, name, { [file]: "T" })).text
      ).toBe("T")
    }
    const bare = installPackage(root, "d", {
      "package.json": JSON.stringify({ name: "d", license: " ISC " }),
    })
    expect(readNoticeEntry(bare)).toEqual({
      name: "d",
      version: "0.0.0",
      identifier: "ISC",
    })
  })

  it("names the manifest that has no name", () => {
    const root = tempDir()
    const dir = installPackage(root, "e", { "package.json": "{}" })
    expect(() => readNoticeEntry(dir)).toThrow(/package\.json has no "name"/)
  })

  it("ignores a byte-order mark before the manifest", () => {
    const root = tempDir()
    const dir = installPackage(root, "f", {
      "package.json": `\uFEFF${JSON.stringify({ name: "f", version: "2.0.0" })}`,
    })
    expect(readNoticeEntry(dir)).toEqual({ name: "f", version: "2.0.0" })
  })

  it("takes the first licence file in name order, and only a file", () => {
    const root = tempDir()
    const dir = installPackage(root, "g", {
      "LICENSE-MIT": "second",
      LICENSE: "first",
    })
    expect(readNoticeEntry(dir).text).toBe("first")
    const withDir = installPackage(root, "h", { "LICENSE.txt": "the file" })
    mkdirSync(join(withDir, "LICENSES"))
    expect(readNoticeEntry(withDir).text).toBe("the file")
  })
})

describe("readCopiedLicences", () => {
  /**
   * Writes an admin source tree with copied code under `root/src`.
   *
   * @param {string} root - the Vite root.
   * @returns {{ kit: string, ui: string, strings: string, own: string }}
   *   one module of each part of the tree.
   */
  function sourceTree(root) {
    const files = {
      "src/components/admin/LICENSE": "\nMIT License kit\n",
      "src/components/admin/LOCAL_CHANGES.md": "changes",
      "src/components/admin/list.tsx": "",
      "src/components/ui/LICENSE": "MIT License ui",
      "src/components/ui/button.tsx": "",
      "src/i18n/LICENSE.ra-language-hungarian": "MIT License © hu",
      "src/i18n/hu.ts": "",
      "src/about/licence-link.tsx": "",
      LICENSE: "not the copied code's",
    }
    for (const [path, text] of Object.entries(files)) {
      mkdirSync(join(root, path, ".."), { recursive: true })
      writeFileSync(join(root, path), text)
    }
    return {
      kit: join(root, "src/components/admin/list.tsx"),
      ui: join(root, "src/components/ui/button.tsx"),
      strings: join(root, "src/i18n/hu.ts"),
      own: join(root, "src/about/licence-link.tsx"),
    }
  }

  it("reads the licence files beside the bundled modules, each once", () => {
    const root = tempDir()
    const { kit, ui, strings, own } = sourceTree(root)
    expect(
      readCopiedLicences(root, [ui, kit, strings, own, kit, join(root, "x.ts")])
    ).toEqual([
      { path: "src/components/admin/LICENSE", text: "MIT License kit" },
      { path: "src/components/ui/LICENSE", text: "MIT License ui" },
      {
        path: "src/i18n/LICENSE.ra-language-hungarian",
        text: "MIT License © hu",
      },
    ])
  })

  it("leaves out code that is not bundled, and files outside src/", () => {
    const root = tempDir()
    const { ui, own } = sourceTree(root)
    expect(readCopiedLicences(root, [ui, own]).map(({ path }) => path)).toEqual(
      ["src/components/ui/LICENSE"]
    )
    expect(readCopiedLicences(root, [join(root, "main.tsx")])).toEqual([])
  })

  it("skips modules whose directory does not exist", () => {
    const root = tempDir()
    expect(
      readCopiedLicences(root, [join(root, SOURCE_DIR, "gone", "virtual.ts")])
    ).toEqual([])
  })
})

/**
 * A fake bundle output: only the fields the plugin reads.
 *
 * @typedef {{ type: string, source?: string | Uint8Array, originalFileNames?: string[], moduleIds?: string[] }} FakeOutput
 */

/**
 * Runs the plugin's `generateBundle` over a fake bundle.
 *
 * @param {string} root - the Vite root.
 * @param {Record<string, FakeOutput>} bundle - output file name to output.
 * @returns {Record<string, FakeOutput>} the bundle after the hook.
 */
function runPlugin(root, bundle) {
  const plugin = licenceNotices({ fileName: LICENCE_NOTICES_FILE })
  expect(plugin.apply).toBe("build")
  // @ts-expect-error: only `root` is read.
  plugin.configResolved({ root })
  const hook = /** @type {{ order: string, handler: Function }} */ (
    plugin.generateBundle
  )
  expect(hook.order).toBe("post")
  const context = {
    /** @param {string} message - the error. */
    error(message) {
      throw new Error(message)
    },
  }
  hook.handler.call(context, {}, bundle)
  return bundle
}

describe("the licenceNotices plugin", () => {
  it("adds stylesheet and asset packages to Vite's file", () => {
    const root = tempDir()
    installPackage(root, "css-pkg", { LICENSE: "CSS licence" })
    installPackage(root, "@fonts/face", { LICENSE: "OFL text" })
    installPackage(root, "via-asset", {})
    mkdirSync(join(root, "src"))
    const css = join(root, "src", "index.css")
    writeFileSync(
      css,
      '@import "css-pkg/x.css";\n@import "./local.css";\n@import "@fonts/face";\n'
    )
    const bundle = runPlugin(root, {
      [LICENCE_NOTICES_FILE]: {
        type: "asset",
        source: VITE_FILE,
        originalFileNames: [],
      },
      "assets/index.js": {
        type: "chunk",
        moduleIds: [
          css,
          `${css}?inline`,
          "\0virtual.css",
          join(root, "src", "main.tsx"),
        ],
      },
      "assets/font.woff2": {
        type: "asset",
        source: "",
        originalFileNames: ["node_modules/via-asset/files/font.woff2"],
      },
      "assets/logo.svg": {
        type: "asset",
        source: "",
        originalFileNames: ["src/logo.svg"],
      },
    })
    const notices = String(bundle[LICENCE_NOTICES_FILE].source)
    expect(notices.startsWith(VITE_FILE)).toBe(true)
    expect(notices).toContain("\n## css-pkg - 1.2.3 (MIT)\n\nCSS licence\n")
    expect(notices).toContain("\n## @fonts/face - 1.2.3 (MIT)\n\nOFL text\n")
    expect(notices).toContain("\n## via-asset - 1.2.3 (MIT)\n")
    expect(notices.split("## css-pkg").length).toBe(2)
  })

  it("adds the licence files of the copied code after the packages", () => {
    const root = tempDir()
    installPackage(root, "css-pkg", { LICENSE: "CSS licence" })
    mkdirSync(join(root, "src", "components", "admin"), { recursive: true })
    const css = join(root, "src", "index.css")
    writeFileSync(css, '@import "css-pkg";\n')
    const kit = join(root, "src", "components", "admin", "list.tsx")
    writeFileSync(kit, "")
    writeFileSync(
      join(root, "src", "components", "admin", "LICENSE"),
      "Kit MIT"
    )
    const bundle = runPlugin(root, {
      [LICENCE_NOTICES_FILE]: {
        type: "asset",
        // Rolldown may hand the asset over as bytes.
        source: new TextEncoder().encode(VITE_FILE),
        originalFileNames: [],
      },
      "assets/index.js": { type: "chunk", moduleIds: [`${kit}?x`, css] },
    })
    const notices = /** @type {string} */ (bundle[LICENCE_NOTICES_FILE].source)
    expect(typeof notices).toBe("string")
    expect(parseNotices(notices)).toEqual({
      packages: [
        {
          name: "react",
          version: "19.3.1",
          identifier: "MIT",
          body: "MIT License",
        },
        {
          name: "css-pkg",
          version: "1.2.3",
          identifier: "MIT",
          body: "CSS licence",
        },
      ],
      copied: [{ path: "src/components/admin/LICENSE", text: "Kit MIT" }],
    })
  })

  it("leaves Vite's file alone when nothing is missing", () => {
    const root = tempDir()
    const bundle = runPlugin(root, {
      [LICENCE_NOTICES_FILE]: {
        type: "asset",
        source: VITE_FILE,
        originalFileNames: [],
      },
      "assets/index.js": { type: "chunk", moduleIds: [join(root, "gone.css")] },
    })
    expect(bundle[LICENCE_NOTICES_FILE].source).toBe(VITE_FILE)
  })

  it("fails the build when Vite wrote no notices file", () => {
    expect(() => runPlugin(tempDir(), {})).toThrow(
      /Vite wrote no licenses\.txt\. Set build\.license to \{ fileName: "licenses\.txt" \}/
    )
  })

  it("fails the build when an imported package is not installed", () => {
    const root = tempDir()
    const css = join(root, "index.css")
    writeFileSync(css, '@import "not-installed/a.css";')
    expect(() =>
      runPlugin(root, {
        [LICENCE_NOTICES_FILE]: {
          type: "asset",
          source: VITE_FILE,
          originalFileNames: [],
        },
        "assets/index.js": { type: "chunk", moduleIds: [css] },
      })
    ).toThrow(
      /imports "not-installed\/a\.css", but no node_modules\/not-installed\/package\.json/
    )
  })
})
