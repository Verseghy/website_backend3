// @ts-check
/**
 * `npm run licenses`: the `Admin licenses` check of ADR 0022 §4.
 *
 * It reads the licence of every production dependency in the installed
 * `admin/node_modules` tree (everything that can end up in `admin/dist`) with
 * license-checker-rseidelsohn, and judges them with `license-policy.mjs`
 * against `admin/license-policy.json`. It fails on any licence outside the
 * policy and on any package without one. Development-only tools are not
 * gated: they never ship in the image. backend3-admin itself is private and
 * AGPL-3.0-only, so `excludePrivatePackages` leaves it out.
 *
 * Usage: `node scripts/check-licenses.mjs [--start <dir>] [--policy <file>]`;
 * both default to this admin project. The options exist for tests.
 */
import { readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import { evaluateLicenses, validatePolicy } from "./license-policy.mjs"

/** @typedef {import("./license-policy.mjs").InstalledPackage} InstalledPackage */

const ADMIN_DIR = resolve(dirname(fileURLToPath(import.meta.url)), "..")

/**
 * Reads the production dependency tree installed under `start` with
 * license-checker-rseidelsohn (which walks `node_modules`, not the registry).
 *
 * @param {string} start - the npm project directory whose tree is read.
 * @returns {Promise<Record<string, InstalledPackage>>} packages by `name@version`.
 */
export async function readInstalledProductionTree(start) {
  const { init } = await import("license-checker-rseidelsohn")
  return new Promise((resolvePackages, reject) => {
    init(
      {
        start,
        production: true,
        excludePrivatePackages: true,
        // Report licences guessed from a file as UNKNOWN, so they fail.
        unknown: true,
        color: false,
      },
      (error, packages) => (error ? reject(error) : resolvePackages(packages))
    )
  })
}

/**
 * @typedef {object} CheckOptions
 * @property {string} start - the npm project whose installed tree is checked.
 * @property {string} policyPath - the policy file.
 * @property {(start: string) => Promise<Record<string, InstalledPackage>>} readTree -
 *   how the installed tree is read; tests pass a stub.
 * @property {(line: string) => void} log - normal output.
 * @property {(line: string) => void} logError - violation output.
 */

/**
 * Runs the check and reports the result.
 *
 * @param {CheckOptions} options - see {@link CheckOptions}.
 * @returns {Promise<number>} the process exit code: 0 when every production
 *   package passes, 1 otherwise.
 */
export async function runLicenseCheck({
  start,
  policyPath,
  readTree,
  log,
  logError,
}) {
  const policy = validatePolicy(JSON.parse(readFileSync(policyPath, "utf8")))
  const packages = await readTree(start)
  if (Object.keys(packages).length === 0) {
    logError(
      `No production packages found under ${start}. Run \`npm ci\` in admin/ first.`
    )
    return 1
  }
  const result = evaluateLicenses(packages, policy)

  for (const license of result.unusedAllow) {
    log(
      `warning: "${license}" is allowed but no production package uses it; consider removing it from the policy.`
    )
  }
  for (const exception of result.unusedExceptions) {
    log(
      `warning: exception ${exception} matches no installed package; consider removing it.`
    )
  }
  if (result.violations.length > 0) {
    logError(
      `Admin licenses: ${result.violations.length} of ${result.checked} production packages fail the policy:`
    )
    for (const { id, license, problem } of result.violations) {
      logError(`  ${id} (${license}): ${problem}`)
    }
    return 1
  }
  log(
    `Admin licenses: all ${result.checked} production packages pass admin/license-policy.json.`
  )
  return 0
}

/**
 * Parses `--start` and `--policy` from the command line.
 *
 * @param {string[]} argv - the arguments after the script name.
 * @returns {{ start: string, policyPath: string }} absolute paths.
 * @throws {Error} on an unknown option or a missing value.
 */
export function parseArgs(argv) {
  let start = ADMIN_DIR
  let policyPath = resolve(ADMIN_DIR, "license-policy.json")
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]]
    if (value === undefined || value.startsWith("--")) {
      throw new Error(
        `${flag} needs a value. Usage: check-licenses.mjs [--start <dir>] [--policy <file>]`
      )
    }
    if (flag === "--start") start = resolve(value)
    else if (flag === "--policy") policyPath = resolve(value)
    else
      throw new Error(
        `Unknown option ${flag}. Usage: check-licenses.mjs [--start <dir>] [--policy <file>]`
      )
  }
  return { start, policyPath }
}

if (
  process.argv[1] !== undefined &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const { start, policyPath } = parseArgs(process.argv.slice(2))
    process.exitCode = await runLicenseCheck({
      start,
      policyPath,
      readTree: readInstalledProductionTree,
      log: (line) => console.log(line),
      logError: (line) => console.error(line),
    })
  } catch (error) {
    console.error(
      `Admin licenses: ${error instanceof Error ? error.message : String(error)}`
    )
    process.exitCode = 1
  }
}
