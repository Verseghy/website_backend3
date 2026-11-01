// @vitest-environment node
/**
 * ra-core's URL query parsing, run against the query-string it really loads:
 * in Node and in the production bundle the browser gets.
 *
 * Why this exists: ra-core 5.15.5 reads a list screen's `?filter=…&page=…`
 * (`parseQueryFromLocation`) and a create screen's `?source=…`
 * (`getRecordFromLocation`) with its own nested query-string 7.1.3, a
 * CommonJS package that `require()`s the CommonJS decode-uri-component 0.2.2.
 * That chain stays as ra-core ships it, and an npm override or a Vite alias of
 * either never points at a vendored or patched copy (admin/CLAUDE.md,
 * "ra-core's query-string stays on 7"). These tests stop an override or alias
 * to a published release that breaks ra-core, as the ESM-only releases do:
 *
 * - decode-uri-component 0.4 and later: query-string 7's `require()` gets a
 *   module namespace object, so every non-empty query throws a TypeError
 *   ("decodeComponent is not a function" in Node; the bundle names the
 *   minified variable instead). `vite build`, every other unit test and the
 *   E2E suite stay green, because no screen parses a query yet, so these
 *   tests are the only ones that fail.
 * - query-string 8 and later: they export only a default, so ra-core's
 *   `import { parse, stringify } from "query-string"` fails to link
 *   (`MISSING_EXPORT` in `vite build`, a `SyntaxError` in Node and in every
 *   test that imports ra-core, these ones included).
 *
 * Node and the bundler each resolve packages and wrap CommonJS in their own
 * way, and the browser runs only the bundle, so every case of
 * `src/test/ra-core-query-cases.ts` runs twice:
 *
 * 1. In the module graph Node builds from node_modules. Vitest does not
 *    transform dependencies, so this is a plain Node import of ra-core.
 * 2. In a production build of that module, made with the admin's own
 *    `vite.config.ts` (its plugins, aliases and resolve rules) and run in a
 *    child process. The build entry is that module rather than `index.html`:
 *    ra-core, query-string and the decoder go through the same resolver and
 *    the same CommonJS wrapping either way, and the small entry builds in
 *    well under a second. The child runs the bundle in Node, not a browser,
 *    which is enough here: the interop code under test is inside the bundle.
 *    A Vite alias of the decoder to an ESM-only release, which Node never
 *    sees, fails only this run.
 *
 * The cases also pin the lenient decoding ra-core relies on, so a decoder
 * change shows up here first.
 */
import { execFile } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { promisify } from "node:util"

import { afterAll, beforeAll, describe, expect, it } from "vitest"

import {
  QUERY_CASES,
  type QueryCaseName,
  type QueryOutcome,
} from "./test/ra-core-query-cases"

const ADMIN = resolve(import.meta.dirname, "..")
const CASES_MODULE = join(ADMIN, "src/test/ra-core-query-cases.ts")
const NAMES = Object.keys(QUERY_CASES) as QueryCaseName[]

/** The bundle's entry chunk, in the build's output directory. */
const ENTRY_CHUNK = "probe.js"
/** Where the child writes the outcomes of `runQueryCases`. */
const OUTCOMES_FILE = "outcomes.json"

/**
 * The child process: builds the cases module with Vite's JavaScript API, then
 * imports the built chunk and writes what every case returned or threw.
 * Arguments: the admin directory, the cases module, the output directory.
 *
 * `preserveEntrySignatures` keeps the entry's exports, which an application
 * build would otherwise drop. Everything else (mode, plugins, resolve rules,
 * minification) is what `npm run build` uses.
 */
const BUILD_AND_RUN = `
import { writeFile } from "node:fs/promises"
import { join } from "node:path"
import { pathToFileURL } from "node:url"
import { build } from "vite"

const [root, entry, outDir] = process.argv.slice(1)
await build({
  root,
  configFile: join(root, "vite.config.ts"),
  mode: "production",
  logLevel: "error",
  build: {
    outDir,
    emptyOutDir: true,
    copyPublicDir: false,
    rolldownOptions: {
      input: entry,
      preserveEntrySignatures: "exports-only",
      output: { entryFileNames: ${JSON.stringify(ENTRY_CHUNK)} },
    },
  },
})
const { runQueryCases } = await import(
  pathToFileURL(join(outDir, ${JSON.stringify(ENTRY_CHUNK)})).href
)
await writeFile(
  join(outDir, ${JSON.stringify(OUTCOMES_FILE)}),
  JSON.stringify(runQueryCases())
)
`

describe("ra-core's query parsing in Node's module graph", () => {
  it.each(NAMES)("%s", (name) => {
    expect(QUERY_CASES[name].run()).toEqual(QUERY_CASES[name].expected)
  })
})

describe("ra-core's query parsing in the production bundle", () => {
  let outDir = ""
  let outcomes: Record<QueryCaseName, QueryOutcome>

  beforeAll(async () => {
    outDir = await mkdtemp(join(tmpdir(), "admin-query-bundle-"))
    await promisify(execFile)(
      process.execPath,
      [
        "--input-type=module",
        "--eval",
        BUILD_AND_RUN,
        ADMIN,
        CASES_MODULE,
        outDir,
      ],
      {
        // `vite` resolves from here, as `npm run build` resolves it.
        cwd: ADMIN,
        // Not Vitest's environment: its NODE_ENV=test would make Vite build
        // in development mode. The `VITE_` variables are explicit (the
        // recorder off, as the image build pins it; no source commit and no
        // missing-key reports, as in a local build), so a local `.env*` file
        // or a CI variable cannot change what is tested (as in
        // src/recorder/production-bundle.test.ts).
        env: {
          PATH: process.env.PATH ?? "",
          VITE_CLIPBOARD_RECORDER: "0",
          VITE_SOURCE_COMMIT: "",
          VITE_REPORT_MISSING_KEYS: "",
        },
      }
    )
    outcomes = JSON.parse(
      await readFile(join(outDir, OUTCOMES_FILE), "utf8")
    ) as Record<QueryCaseName, QueryOutcome>
  }, 120_000)

  afterAll(async () => {
    if (outDir) {
      await rm(outDir, { recursive: true, force: true })
    }
  })

  it.each(NAMES)("%s", (name) => {
    expect(outcomes[name]).toEqual({ value: QUERY_CASES[name].expected })
  })
})
