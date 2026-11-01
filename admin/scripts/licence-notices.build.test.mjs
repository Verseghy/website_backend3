// @vitest-environment node
/**
 * The licence notices of a real admin build (ADR 0038): `dist/licenses.txt`
 * exists and lists every package the build bundles, each with its licence
 * text (research: DECISIONS R15, D88: "a test that the file exists and lists
 * every bundled package"), and the licence files of the code copied into
 * `src/` that the bundle holds.
 *
 * Why a real build: which packages end up in `dist/` is decided by the
 * bundler, so only its output can show that the file is complete. The build
 * runs Vite's CLI as the Containerfile's admin stage does, plus
 * `--sourcemap`: the source maps name the original file of every module in
 * every JavaScript chunk, independently of Vite's notices scan, so they are
 * the reference for "every bundled package" and for the bundled copied
 * code. Packages that arrive through a stylesheet `@import` have no source
 * map entry; they are read from the admin's own stylesheets, which is where
 * `licenceNotices` finds them. `THIRD_PARTY.md` at the repository root is
 * the reference for which licence file covers which copied path.
 */
import { execFile } from "node:child_process"
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, resolve, sep } from "node:path"
import { promisify } from "node:util"

import { afterAll, beforeAll, describe, expect, it } from "vitest"

import {
  LICENCE_NOTICES_BANNER,
  LICENCE_NOTICES_FILE,
  cssImportSpecifiers,
  findPackageDir,
  listsPackage,
  packageOfPath,
  packageOfSpecifier,
  parseNotices,
  readNoticeEntry,
} from "./licence-notices.mjs"

const ADMIN = resolve(import.meta.dirname, "..")
const REPOSITORY = resolve(ADMIN, "..")
const VITE = join(ADMIN, "node_modules/vite/bin/vite.js")

let outDir = ""
let notices = ""
/** @type {ReturnType<typeof parseNotices>} */
let parsed = { packages: [], copied: [] }
/** @type {string[]} output files, relative and `/`-separated */
let files = []
/** @type {Set<string>} the source files the JavaScript chunks' maps name */
const mappedSources = new Set()

/**
 * Lists the files below a directory.
 *
 * @param {string} dir - the directory.
 * @param {(path: string) => boolean} keep - which relative paths to return.
 * @returns {Promise<string[]>} the kept paths, relative and `/`-separated.
 */
async function listFiles(dir, keep) {
  const entries = await readdir(dir, { recursive: true })
  return entries.map((entry) => entry.replaceAll("\\", "/")).filter(keep)
}

/**
 * The packages whose files the JavaScript chunks' source maps name.
 *
 * @returns {Map<string, string>} package directory to package name.
 */
function chunkPackages() {
  /** @type {Map<string, string>} */
  const bundled = new Map()
  for (const source of mappedSources) {
    const owner = packageOfPath(source)
    if (owner) bundled.set(owner.dir, owner.name)
  }
  return bundled
}

/**
 * The packages the admin's own stylesheets import.
 *
 * @returns {Promise<Map<string, string>>} package directory to package name.
 */
async function stylesheetPackages() {
  const packages = new Map()
  for (const path of await ownStylesheets()) {
    for (const specifier of cssImportSpecifiers(await readFile(path, "utf8"))) {
      const name = packageOfSpecifier(specifier)
      if (name === null) continue
      const dir = findPackageDir(dirname(path), name)
      expect(dir, `${path} imports ${specifier}`).not.toBeNull()
      packages.set(dir, name)
    }
  }
  return packages
}

/**
 * The admin's own stylesheets, which the build compiles into its CSS.
 *
 * @returns {Promise<string[]>} their absolute paths.
 */
async function ownStylesheets() {
  const sheets = await listFiles(join(ADMIN, "src"), (path) =>
    path.endsWith(".css")
  )
  return sheets.map((sheet) => join(ADMIN, "src", sheet))
}

/**
 * The rows of `THIRD_PARTY.md` for code copied into the admin's source.
 *
 * @returns {Promise<{ paths: string[], licence: string }[]>} each row's
 *   copied paths and its licence file, relative to the repository root as
 *   the table writes them (a directory ends in `/`).
 */
async function copiedCodeRows() {
  const table = await readFile(join(REPOSITORY, "THIRD_PARTY.md"), "utf8")
  /** @type {{ paths: string[], licence: string }[]} */
  const rows = []
  for (const line of table.split("\n")) {
    const cells = line.split("|").map((cell) => cell.trim())
    if (!line.startsWith("|") || cells.length < 7) continue
    const inSource = (/** @type {string} */ cell) =>
      [...cell.matchAll(/`(admin\/src\/[^`]+)`/g)].map((match) => match[1])
    const paths = inSource(cells[1])
    const [licence] = inSource(cells[5])
    if (paths.length > 0 && licence) rows.push({ paths, licence })
  }
  return rows
}

beforeAll(async () => {
  outDir = await mkdtemp(join(tmpdir(), "admin-licences-"))
  await promisify(execFile)(
    process.execPath,
    [
      VITE,
      "build",
      "--outDir",
      outDir,
      "--emptyOutDir",
      "--sourcemap",
      "--logLevel",
      "error",
    ],
    {
      cwd: ADMIN,
      // As in production-bundle.test.ts: not Vitest's NODE_ENV=test, and the
      // Containerfile's explicit switches, so no local `.env*` file counts.
      env: {
        PATH: process.env.PATH ?? "",
        VITE_CLIPBOARD_RECORDER: "0",
        VITE_SOURCE_COMMIT: "",
        VITE_REPORT_MISSING_KEYS: "",
      },
    }
  )
  files = await listFiles(outDir, () => true)
  notices = await readFile(join(outDir, LICENCE_NOTICES_FILE), "utf8")
  parsed = parseNotices(notices)
  for (const map of files.filter((file) => file.endsWith(".js.map"))) {
    /** @type {{ sources: string[] }} */
    const { sources } = JSON.parse(await readFile(join(outDir, map), "utf8"))
    for (const source of sources) {
      mappedSources.add(resolve(dirname(join(outDir, map)), source))
    }
  }
}, 180_000)

afterAll(async () => {
  if (outDir !== "") {
    await rm(outDir, { recursive: true, force: true })
  }
})

describe("the notices file of the image build", () => {
  it("is written at the dist root, where backend3 serves it, and not under .vite/", () => {
    expect(files).toContain(LICENCE_NOTICES_FILE)
    expect(files.filter((file) => file.startsWith(".vite"))).toEqual([])
    expect(notices.startsWith("# Licenses\n")).toBe(true)
  })

  it("lists every package whose code is in a JavaScript chunk, at its installed version", () => {
    const bundled = chunkPackages()
    // React and ra-core are in every build: a positive control.
    expect([...bundled.values()]).toEqual(
      expect.arrayContaining(["react", "ra-core"])
    )
    const missing = [...bundled.keys()]
      .map(readNoticeEntry)
      .filter((entry) => !listsPackage(notices, entry))
      .map((entry) => `${entry.name}@${entry.version}`)
    expect(missing, "bundled packages that the notices leave out").toEqual([])
  })

  it("lists every package that the admin's stylesheets import", async () => {
    const packages = await stylesheetPackages()
    expect([...packages.values()]).toContain("tailwindcss")
    const missing = [...packages.keys()]
      .map(readNoticeEntry)
      .filter((entry) => !listsPackage(notices, entry))
      .map((entry) => `${entry.name}@${entry.version}`)
    expect(
      missing,
      "imported stylesheet packages that the notices leave out"
    ).toEqual([])
  })

  it("lists the package of the font files it copies", () => {
    expect(files.some((file) => /^assets\/inter-.*\.woff2$/.test(file))).toBe(
      true
    )
    expect(notices).toMatch(
      /^## @fontsource-variable\/inter - \S+ \(OFL-1\.1\)$/m
    )
  })

  it("lists only installed packages, each once", async () => {
    /** @type {{ packages: Record<string, { version?: string }> }} */
    const lock = JSON.parse(
      await readFile(join(ADMIN, "package-lock.json"), "utf8")
    )
    const installed = new Set(
      Object.entries(lock.packages).flatMap(([path, { version }]) => {
        const owner = packageOfPath(`${path}/package.json`)
        return owner && version ? [`${owner.name}@${version}`] : []
      })
    )
    const ids = parsed.packages.map(({ name, version }) => `${name}@${version}`)
    expect(ids.length).toBeGreaterThan(0)
    expect(ids.filter((id) => !installed.has(id))).toEqual([])
    expect(new Set(ids).size).toBe(ids.length)
  })

  it("carries a licence text for every package", () => {
    const withoutText = parsed.packages
      .filter(({ body }) => body === "")
      .map(({ name, version }) => `${name}@${version}`)
    // A package without a licence file gets only its identifier, which is not
    // the notice its licence may require. Check that licence's notice duty
    // and record how it is met (ADR 0038) before accepting the package.
    expect(withoutText, "packages listed without their licence text").toEqual(
      []
    )
  })

  it("leaves out no NOTICE file of the bundled code", async () => {
    const dirs = [
      ...chunkPackages().keys(),
      ...(await stylesheetPackages()).keys(),
    ]
    const notice = (/** @type {string} */ name) => /^notice/i.test(name)
    const found = []
    for (const dir of new Set(dirs)) {
      for (const name of (await readdir(dir)).filter(notice)) {
        found.push(`${dir}/${name}`)
      }
    }
    found.push(
      ...(
        await listFiles(join(ADMIN, "src"), (path) =>
          notice(path.split("/").at(-1) ?? "")
        )
      ).map((path) => `src/${path}`)
    )
    // Vite and licenceNotices read licence files only. Apache-2.0 §4 asks
    // that a NOTICE file travel with the code as well, so a bundled package
    // or copied code that brings one needs licenceNotices to add it first
    // (ADR 0038 §1).
    expect(found, "NOTICE files that the notices would leave out").toEqual([])
  })
})

describe("the copied code", () => {
  it("is listed with the licence file of every bundled path of THIRD_PARTY.md", async () => {
    const bundled = [...mappedSources, ...(await ownStylesheets())]
    const rows = await copiedCodeRows()
    // The kit, shadcn/ui and the Hungarian strings: a positive control.
    expect(rows.length).toBeGreaterThanOrEqual(3)
    const listed = new Map(parsed.copied.map(({ path, text }) => [path, text]))
    const missing = []
    for (const { paths, licence } of rows) {
      const isBundled = paths.some((path) => {
        const absolute = resolve(REPOSITORY, path)
        return path.endsWith("/")
          ? bundled.some((source) => source.startsWith(`${absolute}${sep}`))
          : bundled.includes(absolute)
      })
      const below = licence.replace(/^admin\//, "")
      if (isBundled && listed.get(below) === undefined) {
        missing.push(`${licence} (for ${paths.join(", ")})`)
      }
    }
    // licenceNotices lists a licence file when bundled code sits in its
    // directory or below it. Copied code whose licence file lies elsewhere
    // is covered only if code beside that file is bundled too; if this
    // fails, move the licence file next to the code or extend the plugin.
    expect(missing, "licence files of bundled copied code").toEqual([])
  })

  it("lists each licence file once, with its text as committed", async () => {
    expect(parsed.copied.length).toBeGreaterThan(0)
    const paths = parsed.copied.map(({ path }) => path)
    expect(new Set(paths).size).toBe(paths.length)
    for (const { path, text } of parsed.copied) {
      const committed = await readFile(join(ADMIN, path), "utf8")
      expect(text, path).toBe(committed.trim())
    }
  })
})

describe("the limit of the stylesheet scan", () => {
  it("holds: no imported package's stylesheets import another package", async () => {
    const nested = []
    for (const [dir, name] of await stylesheetPackages()) {
      const sheets = await listFiles(
        dir,
        (path) => path.endsWith(".css") && !path.includes("node_modules/")
      )
      for (const sheet of sheets) {
        const css = await readFile(join(dir, sheet), "utf8")
        for (const specifier of cssImportSpecifiers(css)) {
          const other = packageOfSpecifier(specifier)
          if (other !== null && other !== name) {
            nested.push(`${name}/${sheet} imports ${specifier}`)
          }
        }
      }
    }
    // licenceNotices reads only the stylesheets that are modules of the
    // bundle. If this fails, extend it to follow the nested import, so the
    // nested package is listed too.
    expect(nested).toEqual([])
  })
})

describe("the JavaScript chunks", () => {
  it("each start with the comment that names the notices file", async () => {
    const chunks = files.filter((file) => file.endsWith(".js"))
    expect(chunks.length).toBeGreaterThan(0)
    for (const chunk of chunks) {
      const code = await readFile(join(outDir, chunk), "utf8")
      expect(code.startsWith(`${LICENCE_NOTICES_BANNER}\n`), chunk).toBe(true)
    }
  })
})
