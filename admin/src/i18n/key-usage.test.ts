// @vitest-environment node
/**
 * The "Key usage" check of ADR 0021 §7.
 *
 * backend3's own code is covered by the compiler (`useT` accepts only keys of
 * `hu.ts`). The copied kit files call ra-core's untyped `translate` and pass
 * keys through props such as `label="ra.action.save"`, so this test scans
 * every source file for string literals shaped like a key of one of the
 * admin's namespaces and fails on each one `hu.ts` lacks.
 */
import { readdirSync, readFileSync } from "node:fs"
import { join, relative, resolve } from "node:path"

import { describe, expect, it } from "vitest"

import { hu } from "./hu"
import { lookupMessage } from "./messages"

const SRC = resolve(import.meta.dirname, "..")

/** The top-level namespaces of `hu.ts` and of ADR 0021 §2's table. */
const NAMESPACES = [
  "ra",
  "app",
  "kit",
  "dashboard",
  "recorder",
  "resources",
  "editor",
  "tree",
  "canteen",
  "media",
  "trash",
  "history",
  "activity",
  "auth",
  "enums",
  "errors",
]

/**
 * A quoted string that is a complete key: a namespace, then dotted segments.
 * Template literals with `${…}` are dynamic and are not matched, because the
 * backtick alternative stops at `$`.
 */
const KEY_LITERAL = new RegExp(
  String.raw`(["'\`])((?:${NAMESPACES.join("|")})(?:\.[A-Za-z0-9_-]+)+)\1`,
  "g"
)

/** File names such as "app.css" match the shape but are not keys. */
const FILE_EXTENSION = /\.(css|ts|tsx|js|mjs|json|svg|png|html)$/

/**
 * Lists the source files to scan: TypeScript under `src/`, without tests and
 * without the generated bindings.
 *
 * @param dir - the directory to walk.
 * @returns absolute file paths.
 */
function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) {
      return entry.name === "bindings" || entry.name === "test"
        ? []
        : sourceFiles(path)
    }
    return /\.(ts|tsx)$/.test(entry.name) &&
      !/\.test\.(ts|tsx)$/.test(entry.name)
      ? [path]
      : []
  })
}

/**
 * Removes comments, so JSDoc examples (`label="resources.users.fields.name"`)
 * are not taken for real lookups. A `//` counts as a comment only at the start
 * of a line or after whitespace, so the `//` of a URL inside a string stays.
 *
 * @param text - TypeScript source.
 * @returns the source without block and line comments.
 */
function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|\s)\/\/.*$/gm, "$1")
}

/**
 * Finds key-shaped literals in one file's text.
 *
 * @param text - the file's contents.
 * @returns the keys, in order of appearance.
 */
export function keyLiterals(text: string): string[] {
  return [...stripComments(text).matchAll(KEY_LITERAL)]
    .map((match) => match[2])
    .filter((key) => !FILE_EXTENSION.test(key))
}

describe("keyLiterals", () => {
  it("finds keys in translate calls, i18nKey and label props", () => {
    expect(
      keyLiterals(
        `translate("ra.action.save"); <Translate i18nKey='ra.page.dashboard' />; label = "ra.action.select_all"`
      )
    ).toEqual(["ra.action.save", "ra.page.dashboard", "ra.action.select_all"])
  })

  it("skips keys in comments but keeps strings that contain a URL", () => {
    expect(
      keyLiterals(
        `/** label="ra.example.one" */\n// translate("ra.example.two")\nconst u = "https://x/y"; translate("ra.action.save") // ra.example.three`
      )
    ).toEqual(["ra.action.save"])
  })

  it("skips dynamic template literals, other packages and file names", () => {
    expect(
      keyLiterals(
        "translate(`ra.sort.${order}`); import 'ra-core'; import './app.css'"
      )
    ).toEqual([])
  })
})

describe("translation keys used in admin/src", () => {
  it("all exist in hu.ts", () => {
    const missing = sourceFiles(SRC).flatMap((file) =>
      keyLiterals(readFileSync(file, "utf8"))
        .filter((key) => lookupMessage(hu, key) === undefined)
        .map((key) => `${relative(SRC, file)}: ${key}`)
    )
    expect(
      missing,
      `Add these keys to admin/src/i18n/hu.ts:\n${missing.join("\n")}`
    ).toEqual([])
  })

  it("scans a meaningful number of files (guards against a broken walk)", () => {
    expect(sourceFiles(SRC).length).toBeGreaterThan(100)
  })
})
