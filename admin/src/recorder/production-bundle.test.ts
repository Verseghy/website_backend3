// @vitest-environment node
/**
 * No image carries any trace of the clipboard recorder (editor.md §13.1,
 * `recorder-flag.ts`).
 *
 * Why a real build: the recorder is left out by dead-code elimination, which
 * only the bundler performs, so no unit test of `main.tsx` could show it. This
 * test runs Vite's CLI twice in a clean environment: once as the
 * Containerfile's admin stage builds, with `VITE_CLIPBOARD_RECORDER=0`, and
 * once with `1`, the positive control which proves that the markers searched
 * for are the right ones. A third test checks that the Containerfile pins the
 * switch to `0`: an explicit variable wins over the `.env*` files Vite also
 * reads, so no such file in the build context can switch the recorder on in
 * an image. The Hungarian `recorder.*` strings are not markers: `hu.ts` is
 * one module, so they ship as inert text in every build.
 */
import { execFile } from "node:child_process"
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { promisify } from "node:util"

import { afterAll, beforeAll, describe, expect, it } from "vitest"

import { CAPTURE_FORMAT } from "./capture"

const ADMIN = resolve(import.meta.dirname, "../..")
const VITE = join(ADMIN, "node_modules/vite/bin/vite.js")

/**
 * The recorder's route path, as `main.tsx` registers it. Unquoted, because the
 * minifier may write it as a template literal.
 */
const ROUTE_PATH = "/clipboard-recorder"

/** Output files whose contents are searched; fonts are binary. */
const TEXT_FILE = /\.(html|js|css|json|svg|txt)$/

const outDirs: string[] = []

/**
 * Builds the admin into a fresh temporary directory.
 *
 * @param flag - the `VITE_CLIPBOARD_RECORDER` value for this build.
 * @returns every output file's path (relative, `/`-separated) and, for text
 *   files, its contents.
 */
async function build(flag: string): Promise<Map<string, string>> {
  const outDir = await mkdtemp(join(tmpdir(), "admin-bundle-"))
  outDirs.push(outDir)
  await promisify(execFile)(
    process.execPath,
    [VITE, "build", "--outDir", outDir, "--emptyOutDir", "--logLevel", "error"],
    {
      cwd: ADMIN,
      // Not Vitest's environment: its NODE_ENV=test would make Vite build in
      // development mode. The switch is explicit, as in the Containerfile, and
      // so are the two other variables (empty: a local build with no missing-
      // key reports), so a local `.env*` file cannot change what is tested.
      env: {
        PATH: process.env.PATH ?? "",
        VITE_CLIPBOARD_RECORDER: flag,
        VITE_SOURCE_COMMIT: "",
        VITE_REPORT_MISSING_KEYS: "",
      },
    }
  )
  const files = new Map<string, string>()
  for (const entry of await readdir(outDir, { recursive: true })) {
    const name = entry.replaceAll("\\", "/")
    files.set(
      name,
      TEXT_FILE.test(name) ? await readFile(join(outDir, entry), "utf8") : ""
    )
  }
  return files
}

/**
 * Lists the output files whose path or contents contain a marker.
 *
 * @param files - a build's output, from {@link build}.
 * @param marker - the text to look for.
 * @returns the matching file paths.
 */
function filesWith(files: Map<string, string>, marker: string): string[] {
  return [...files].flatMap(([name, text]) =>
    text.includes(marker) ? [name] : []
  )
}

let production: Map<string, string>
let withRecorder: Map<string, string>

beforeAll(async () => {
  ;[production, withRecorder] = await Promise.all([build("0"), build("1")])
}, 180_000)

afterAll(async () => {
  await Promise.all(
    outDirs.map((dir) => rm(dir, { recursive: true, force: true }))
  )
})

describe("the image build", () => {
  it("pins VITE_CLIPBOARD_RECORDER=0 in the Containerfile's admin build", async () => {
    const containerfile = await readFile(
      join(ADMIN, "../Containerfile"),
      "utf8"
    )
    const builds = containerfile
      .split("\n")
      .filter((line) => /^RUN\b.*\bnpm run build\b/.test(line))
    expect(builds).toHaveLength(1)
    expect(builds[0]).toMatch(/^RUN (\w+=\S+ )*VITE_CLIPBOARD_RECORDER=0 /)
  })

  it("has no recorder chunk", () => {
    expect(
      [...production.keys()].filter((name) => /recorder/i.test(name))
    ).toEqual([])
  })

  it("registers no recorder route and holds none of its capture code", () => {
    expect(filesWith(production, ROUTE_PATH)).toEqual([])
    expect(filesWith(production, CAPTURE_FORMAT)).toEqual([])
  })
})

describe("a build with VITE_CLIPBOARD_RECORDER=1 (the positive control)", () => {
  it("registers the route in the entry chunk and loads the page as a chunk of its own", () => {
    const index = withRecorder.get("index.html") ?? ""
    const entry = [...withRecorder.keys()].filter(
      (name) => name.endsWith(".js") && index.includes(name)
    )
    expect(entry).toHaveLength(1)
    expect(filesWith(withRecorder, ROUTE_PATH)).toContain(entry[0])

    const capture = filesWith(withRecorder, CAPTURE_FORMAT)
    expect(capture).toHaveLength(1)
    expect(capture[0]).toMatch(/^assets\/ClipboardRecorderPage-[\w-]+\.js$/)
  })
})
