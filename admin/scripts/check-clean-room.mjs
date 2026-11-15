// @ts-check
/**
 * `npm run clean-room`: the CI check of the clean-room rule, which fails when
 * `admin/package.json` or `admin/package-lock.json` names any `@react-admin/`
 * package (ADR 0011 §8, ADR 0018 §3.4).
 *
 * Why it exists: react-admin's Enterprise Edition modules (the trash,
 * history, audit-log and tree screens among them) are published under the
 * private `@react-admin/` scope of a paid registry. backend3 builds those
 * screens itself, from the ADRs and public documentation only, and never
 * installs, reads or copies Enterprise code. A dependency from that scope
 * would bring non-free code into this AGPL-3.0-only repository and make the
 * build need a subscription. The free packages backend3 does use are named
 * `ra-core`, `ra-i18n-polyglot` and `ra-language-english`, without a scope.
 *
 * How: two searches of each file, so that no field can be forgotten and no
 * spelling hides the scope.
 *
 * - A plain text search, line by line, finds the scope wherever npm writes
 *   it: a dependency of any kind, an alias (`"x": "npm:@react-admin/..."`),
 *   an override, a nested package of the lockfile and a `resolved` URL. It
 *   reports the line, which is what an operator fixes.
 * - A walk over the parsed JSON looks at every key and every string value
 *   after JSON decoding, so it also finds a name written with escapes
 *   (`"\u0040react-admin/ra-tree"`, `"@react-admin\/ra-tree"`), which the text
 *   search cannot see. npm never writes those, but a hand-edited file can.
 *   It reports the JSON path of each name that the text search missed.
 *
 * A file that is not valid JSON fails the check: it cannot be searched
 * completely, and `npm ci` would refuse it anyway. The check needs no
 * `npm ci`. The `Admin licenses` CI job runs it beside the licence check.
 *
 * Usage: `node scripts/check-clean-room.mjs [--dir <admin dir>]`; the
 * default is this admin project. The option exists for tests.
 */
import { readFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

/** The npm scope of react-admin's Enterprise Edition packages. */
export const ENTERPRISE_SCOPE = "@react-admin/"

/** The files the check reads, relative to the admin project. */
export const CHECKED_FILES = ["package.json", "package-lock.json"]

const ADMIN_DIR = resolve(dirname(fileURLToPath(import.meta.url)), "..")

/**
 * @typedef {object} Finding
 * @property {string} file - the file name, relative to the admin project.
 * @property {number} line - the 1-based line number.
 * @property {string} text - the line, trimmed.
 */

/**
 * Finds every line of `text` that names the Enterprise scope.
 *
 * @param {string} file - the file name to report.
 * @param {string} text - the file's contents.
 * @returns {Finding[]} one finding per line that contains `@react-admin/`,
 *   in line order; empty when the file is clean.
 */
export function findEnterprisePackages(file, text) {
  /** @type {Finding[]} */
  const findings = []
  const lines = text.split(/\r?\n/)
  lines.forEach((line, index) => {
    if (line.includes(ENTERPRISE_SCOPE)) {
      findings.push({ file, line: index + 1, text: line.trim() })
    }
  })
  return findings
}

/**
 * @typedef {object} JsonFinding
 * @property {string} file - the file name, relative to the admin project.
 * @property {string} path - where the name sits, as a JSON path from the
 *   document's root, for example `$.dependencies["@react-admin/ra-tree"]`.
 * @property {"key" | "value"} kind - whether the name is the member's key or
 *   its string value.
 * @property {string} text - the key or value after JSON decoding.
 */

/**
 * One step of a JSON path: `.name` for a key that is a plain identifier,
 * `["..."]` for any other key, `[n]` for an array index.
 *
 * @param {string | number} step - a member key or an array index.
 * @returns {string} the step as it is printed.
 */
function pathStep(step) {
  if (typeof step === "number") {
    return `[${step}]`
  }
  return /^[A-Za-z_$][\w$]*$/.test(step)
    ? `.${step}`
    : `[${JSON.stringify(step)}]`
}

/**
 * Finds every key and string value of a parsed JSON document that names
 * the Enterprise scope once decoded.
 *
 * The walk uses an explicit stack, not recursion, so a deeply nested
 * hand-written file cannot overflow the call stack. A member's key goes on
 * the stack just before its value, so the findings come out in document
 * order, except where `JSON.parse` itself reorders: an object's
 * integer-like keys come first.
 *
 * @param {string} file - the file name to report.
 * @param {unknown} document - the value `JSON.parse` returned for the file.
 * @returns {JsonFinding[]} one finding per key or string value that contains
 *   `@react-admin/`, in document order; empty when there is none.
 */
export function findEnterpriseJsonNames(file, document) {
  /** @type {JsonFinding[]} */
  const findings = []
  /**
   * @typedef {{ kind: "key", key: string, path: string }
   *   | { kind: "value", value: unknown, path: string }} Entry
   */
  /** @type {Entry[]} */
  const stack = [{ kind: "value", value: document, path: "$" }]
  while (stack.length > 0) {
    const entry = /** @type {Entry} */ (stack.pop())
    if (entry.kind === "key") {
      if (entry.key.includes(ENTERPRISE_SCOPE)) {
        findings.push({ file, path: entry.path, kind: "key", text: entry.key })
      }
      continue
    }
    const { value, path } = entry
    if (typeof value === "string") {
      if (value.includes(ENTERPRISE_SCOPE)) {
        findings.push({ file, path, kind: "value", text: value })
      }
      continue
    }
    if (typeof value !== "object" || value === null) {
      continue
    }
    /** @type {Entry[]} */
    const children = []
    if (Array.isArray(value)) {
      value.forEach((item, index) => {
        children.push({
          kind: "value",
          value: item,
          path: `${path}${pathStep(index)}`,
        })
      })
    } else {
      for (const [key, child] of Object.entries(value)) {
        const childPath = `${path}${pathStep(key)}`
        children.push({ kind: "key", key, path: childPath })
        children.push({ kind: "value", value: child, path: childPath })
      }
    }
    // Pushed in reverse, so that the stack pops them in document order. A
    // loop, not a spread: a lockfile object has thousands of members.
    for (let index = children.length - 1; index >= 0; index -= 1) {
      stack.push(children[index])
    }
  }
  return findings
}

/**
 * @typedef {object} CleanRoomOptions
 * @property {string} adminDir - the admin project whose files are read.
 * @property {(path: string) => string} readFile - reads a file as UTF-8;
 *   tests pass a stub.
 * @property {(line: string) => void} log - normal output.
 * @property {(line: string) => void} logError - violation output.
 */

/**
 * Runs the check and reports the result.
 *
 * @param {CleanRoomOptions} options - see {@link CleanRoomOptions}.
 * @returns {number} the process exit code: 0 when neither file names the
 *   scope, 1 when one does, or when a file cannot be read or is not JSON.
 */
export function runCleanRoomCheck({ adminDir, readFile, log, logError }) {
  /** @type {Finding[]} */
  const findings = []
  /** @type {JsonFinding[]} */
  const escapedFindings = []
  for (const file of CHECKED_FILES) {
    let text
    try {
      text = readFile(join(adminDir, file))
    } catch (error) {
      logError(
        `Clean room: cannot read ${file} in ${adminDir} (${error instanceof Error ? error.message : String(error)}); ` +
          "the check needs both package.json and the committed package-lock.json."
      )
      return 1
    }
    let document
    try {
      document = JSON.parse(text)
    } catch (error) {
      logError(
        `Clean room: ${file} in ${adminDir} is not valid JSON (${error instanceof Error ? error.message : String(error)}), ` +
          "so the check cannot search all of it; fix the file (npm ci refuses it too)."
      )
      return 1
    }
    findings.push(...findEnterprisePackages(file, text))
    // Only the names the line search may not have seen. A name written as
    // npm writes it, in JSON.stringify's encoding, holds the scope literally,
    // so its line is already reported. Any other spelling is reported by its
    // path; for an odd spelling that still holds the scope literally (an
    // escaped quote elsewhere in the name, say), that repeats a reported line.
    escapedFindings.push(
      ...findEnterpriseJsonNames(file, document).filter(
        (finding) => !text.includes(JSON.stringify(finding.text))
      )
    )
  }
  if (findings.length + escapedFindings.length > 0) {
    logError(
      `Clean room: ${findings.length + escapedFindings.length} place(s) name the ${ENTERPRISE_SCOPE} scope of react-admin's ` +
        "Enterprise Edition, which backend3 never installs (ADR 0011 §8, ADR 0018 §3.4):"
    )
    for (const { file, line, text } of findings) {
      logError(`  ${file}:${line}: ${text}`)
    }
    for (const { file, path, kind, text } of escapedFindings) {
      logError(
        `  ${file}: the ${kind} at ${path}, written with JSON escapes, reads ${JSON.stringify(text)}`
      )
    }
    logError(
      "Remove the package and build the feature from the ADRs and public documentation instead."
    )
    return 1
  }
  log(
    `Clean room: neither ${CHECKED_FILES.join(" nor ")} names an ${ENTERPRISE_SCOPE} package.`
  )
  return 0
}

/**
 * Parses `--dir` from the command line.
 *
 * @param {string[]} argv - the arguments after the script name.
 * @returns {{ adminDir: string }} the absolute admin directory.
 * @throws {Error} on an unknown option or a missing value.
 */
export function parseArgs(argv) {
  let adminDir = ADMIN_DIR
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]]
    if (value === undefined || value.startsWith("--")) {
      throw new Error(
        `${flag} needs a value. Usage: check-clean-room.mjs [--dir <admin dir>]`
      )
    }
    if (flag === "--dir") adminDir = resolve(value)
    else
      throw new Error(
        `Unknown option ${flag}. Usage: check-clean-room.mjs [--dir <admin dir>]`
      )
  }
  return { adminDir }
}

if (
  process.argv[1] !== undefined &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const { adminDir } = parseArgs(process.argv.slice(2))
    process.exitCode = runCleanRoomCheck({
      adminDir,
      readFile: (path) => readFileSync(path, "utf8"),
      log: (line) => console.log(line),
      logError: (line) => console.error(line),
    })
  } catch (error) {
    console.error(
      `Clean room: ${error instanceof Error ? error.message : String(error)}`
    )
    process.exitCode = 1
  }
}
