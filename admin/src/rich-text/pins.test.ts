// @vitest-environment node
import { readFileSync } from "node:fs"

import { describe, expect, it } from "vitest"

/**
 * The editor's package pins (editor.md §4.1, ADR 0013 §2): every `@tiptap/*`
 * package is pinned exactly, all at one version, and prosemirror-view,
 * prosemirror-model and prosemirror-tables, which `@tiptap/pm` requires with
 * caret ranges only, are pinned exactly through npm's `overrides`. The
 * serialiser relies on a prosemirror-model internal and the paste filter on
 * prosemirror-view's paste order, so neither may move without the corpus
 * job.
 *
 * The lockfile must also hold one copy of each ProseMirror package: a second
 * copy, even of the same version, makes `instanceof` checks between the two
 * fail (a `Decoration` or `Plugin` from one copy is not one of the other), and
 * npm can install such copies when an override is added to an existing tree.
 */
interface PackageJson {
  dependencies: Record<string, string>
  overrides?: Record<string, string>
}
interface PackageLock {
  packages: Record<string, { version?: string }>
}

const read = <T>(file: string): T =>
  JSON.parse(
    readFileSync(new URL(`../../${file}`, import.meta.url), "utf8")
  ) as T

const manifest = read<PackageJson>("package.json")
const lock = read<PackageLock>("package-lock.json")
const EXACT = /^[0-9]+\.[0-9]+\.[0-9]+$/
const PINNED_PROSEMIRROR = [
  "prosemirror-model",
  "prosemirror-tables",
  "prosemirror-view",
]

/** The installed copies of a package in the lockfile, by path. */
function copiesOf(name: string): [string, string | undefined][] {
  return Object.entries(lock.packages)
    .filter(
      ([path]) =>
        path === `node_modules/${name}` ||
        path.endsWith(`/node_modules/${name}`)
    )
    .map(([path, entry]) => [path, entry.version])
}

describe("the editor's package pins", () => {
  const tiptap = Object.entries(manifest.dependencies).filter(([name]) =>
    name.startsWith("@tiptap/")
  )

  it("pins every @tiptap/* dependency exactly, all at one version", () => {
    expect(tiptap.length).toBeGreaterThan(0)
    for (const [name, requirement] of tiptap) {
      expect(requirement, name).toMatch(EXACT)
    }
    expect(new Set(tiptap.map(([, requirement]) => requirement)).size).toBe(1)
  })

  it("installs no other version of any @tiptap/* package", () => {
    const [[, version]] = tiptap
    const installed = Object.entries(lock.packages).filter(([path]) =>
      /(^|\/)node_modules\/@tiptap\//.test(path)
    )
    expect(installed.length).toBeGreaterThanOrEqual(tiptap.length)
    for (const [path, entry] of installed) {
      expect(entry.version, path).toBe(version)
    }
  })

  it.each(PINNED_PROSEMIRROR)("pins %s exactly through overrides", (name) => {
    const pinned = manifest.overrides?.[name]
    expect(pinned).toMatch(EXACT)
    expect(copiesOf(name)).toEqual([[`node_modules/${name}`, pinned]])
  })

  it("installs one copy of every ProseMirror package", () => {
    const names = new Set(
      Object.keys(lock.packages)
        .map((path) => /node_modules\/(prosemirror-[a-z-]+)$/.exec(path)?.[1])
        .filter((name): name is string => name !== undefined)
    )
    expect(names.size).toBeGreaterThan(PINNED_PROSEMIRROR.length)
    for (const name of names) {
      expect(
        copiesOf(name).map(([path]) => path),
        name
      ).toEqual([`node_modules/${name}`])
    }
  })
})
