// @ts-check
/**
 * Evaluates the admin's production npm dependencies against
 * `license-policy.json` (ADR 0022 §4, `Admin licenses`).
 *
 * Why this module exists instead of `license-checker-rseidelsohn --onlyAllow`:
 * that flag compares by substring (license-checker-rseidelsohn 5.0.1,
 * lib/index.js `checkForOnlyAllow`: `currentLicense.includes(allowedLicense)`).
 * Allowing `MPL-2.0` would then also pass `MPL-2.0-no-copyleft-exception`,
 * which ADR 0022 §4 denies, and `MIT` would pass `(MIT AND GPL-2.0-only)`. The
 * checker still reads the installed tree (it walks `node_modules` with
 * `@npmcli/arborist`, never the registry); this module only judges what it
 * found, with real SPDX expression semantics:
 *
 * - `A OR B` passes when either side passes; `A AND B` only when both do;
 * - `X+` ("or later") passes when `X` is allowed, since the licensee may pick X;
 * - `X WITH exception` never passes: no licence exception is on the list;
 * - an unknown licence, a guessed one (license-checker marks them `*`), a
 *   custom text or an unparsable expression fails, with a message that says
 *   what to do.
 *
 * Pure: no I/O. `check-licenses.mjs` does the reading and printing.
 */
import parseSpdx from "spdx-expression-parse"

/**
 * @typedef {object} AllowEntry
 * @property {string} license - an SPDX licence id allowed for every package.
 * @property {string} reason - why it may be combined with AGPL-3.0-only.
 */

/**
 * @typedef {object} ExceptionEntry
 * @property {string} package - the npm package name (any version) it applies to.
 * @property {string} license - the SPDX id allowed for that package only.
 * @property {string} reason - why, and why it is scoped.
 */

/**
 * @typedef {object} LicensePolicy
 * @property {AllowEntry[]} allow - licences allowed for every package.
 * @property {ExceptionEntry[]} exceptions - licences allowed for one package.
 */

/**
 * One installed package as license-checker reports it.
 *
 * @typedef {object} InstalledPackage
 * @property {string | string[] | undefined} [licenses] - the SPDX expression(s);
 *   absent when the package declares none.
 * @property {string | undefined} [path] - where it is installed.
 */

/**
 * @typedef {object} Violation
 * @property {string} id - `name@version` of the offending package.
 * @property {string} license - the licence text license-checker found.
 * @property {string} problem - what is wrong and how to fix it.
 */

/**
 * @typedef {object} Evaluation
 * @property {Violation[]} violations - packages that fail the policy.
 * @property {string[]} unusedAllow - allowed licences no package uses.
 * @property {string[]} unusedExceptions - exceptions no package needs.
 * @property {number} checked - how many packages were judged.
 */

/**
 * Splits `name@version` (and `@scope/name@version`) into its name.
 *
 * @param {string} id - the package id from license-checker.
 * @returns {string} the package name.
 */
export function packageName(id) {
  const at = id.lastIndexOf("@")
  return at > 0 ? id.slice(0, at) : id
}

/**
 * Checks the shape of a policy and returns it typed. Every entry must name a
 * licence and give a reason, so the file stays reviewable.
 *
 * An exception must also scope a licence that `allow` does not already admit
 * for every package. ADR 0027 keeps every licence beyond ADR 0022 §4's table
 * out of `allow` and admits it per package; an exception for a licence that
 * `allow` still lists would only look scoped. It would never be relied on
 * (`evaluateLicenses` credits the `allow` entry), so it is rejected here
 * rather than left to the "matches no installed package" warning.
 *
 * @param {unknown} raw - the parsed `license-policy.json`.
 * @returns {LicensePolicy} the same object, validated.
 * @throws {Error} naming the first malformed entry and what it lacks.
 */
export function validatePolicy(raw) {
  if (typeof raw !== "object" || raw === null) {
    throw new Error(
      "license-policy.json must contain a JSON object with `allow` and `exceptions` arrays."
    )
  }
  const { allow, exceptions } =
    /** @type {{ allow?: unknown, exceptions?: unknown }} */ (raw)
  if (!Array.isArray(allow) || !Array.isArray(exceptions)) {
    throw new Error(
      "license-policy.json needs both an `allow` array and an `exceptions` array."
    )
  }
  const nonEmpty = (/** @type {unknown} */ value) =>
    typeof value === "string" && value.trim() !== ""
  allow.forEach((entry, index) => {
    if (!nonEmpty(entry?.license) || !nonEmpty(entry?.reason)) {
      throw new Error(
        `license-policy.json allow[${index}] needs a non-empty "license" and a non-empty "reason".`
      )
    }
    assertSpdxId(entry.license, `allow[${index}]`)
  })
  exceptions.forEach((entry, index) => {
    if (
      !nonEmpty(entry?.package) ||
      !nonEmpty(entry?.license) ||
      !nonEmpty(entry?.reason)
    ) {
      throw new Error(
        `license-policy.json exceptions[${index}] needs a non-empty "package", "license" and "reason".`
      )
    }
    assertSpdxId(entry.license, `exceptions[${index}]`)
  })
  const allowedForAll = new Set(allow.map((entry) => entry.license))
  exceptions.forEach((entry, index) => {
    if (allowedForAll.has(entry.license)) {
      throw new Error(
        `license-policy.json exceptions[${index}] scopes "${entry.license}" to ${entry.package}, but allow already admits "${entry.license}" for every package, so the exception has no effect. Remove "${entry.license}" from allow to scope it, or remove the exception (ADR 0027).`
      )
    }
  })
  return /** @type {LicensePolicy} */ (raw)
}

/**
 * @param {string} license - the value to check.
 * @param {string} where - the policy entry, for the message.
 * @throws {Error} when `license` is not a single SPDX id.
 */
function assertSpdxId(license, where) {
  let parsed
  try {
    parsed = parseSpdx(license)
  } catch {
    throw new Error(
      `license-policy.json ${where}: "${license}" is not an SPDX licence id.`
    )
  }
  if (!("license" in parsed) || parsed.plus || parsed.exception) {
    throw new Error(
      `license-policy.json ${where}: "${license}" must be one plain SPDX id, not an expression.`
    )
  }
}

/**
 * Decides whether a parsed SPDX expression is acceptable for one package.
 *
 * @param {parseSpdx.Info} node - the parsed expression.
 * @param {(license: string) => boolean} isAllowed - the policy for this package.
 * @returns {string[] | null} the licences the package is used under when the
 *   expression passes (for `OR`, only the first passing branch), or `null`.
 */
export function satisfies(node, isAllowed) {
  if ("conjunction" in node) {
    const left = satisfies(node.left, isAllowed)
    if (node.conjunction === "or") {
      return left ?? satisfies(node.right, isAllowed)
    }
    const right = satisfies(node.right, isAllowed)
    return left !== null && right !== null ? [...left, ...right] : null
  }
  if (node.exception !== undefined) {
    return null
  }
  return isAllowed(node.license) ? [node.license] : null
}

/**
 * Judges every installed production package against the policy.
 *
 * @param {Record<string, InstalledPackage>} packages - license-checker's
 *   result, keyed by `name@version`, private packages already excluded.
 * @param {LicensePolicy} policy - the validated policy.
 * @returns {Evaluation} the violations and the unused policy entries.
 */
export function evaluateLicenses(packages, policy) {
  const allowed = new Set(policy.allow.map((entry) => entry.license))
  /** @type {Violation[]} */
  const violations = []
  /** @type {Set<string>} */
  const usedAllow = new Set()
  /** @type {Set<string>} */
  const usedExceptions = new Set()

  for (const [id, info] of Object.entries(packages)) {
    const name = packageName(id)
    const scoped = new Set(
      policy.exceptions
        .filter((entry) => entry.package === name)
        .map((entry) => entry.license)
    )
    const isAllowed = (/** @type {string} */ license) =>
      allowed.has(license) || scoped.has(license)
    // A legacy `licenses` array is judged conservatively: every entry must pass.
    const expressions = Array.isArray(info.licenses)
      ? info.licenses
      : [info.licenses]

    for (const expression of expressions) {
      const problem = problemWith(name, expression, isAllowed, (license) => {
        if (scoped.has(license) && !allowed.has(license)) {
          usedExceptions.add(`${name}:${license}`)
        } else {
          usedAllow.add(license)
        }
      })
      if (problem !== null) {
        violations.push({ id, license: String(expression ?? ""), problem })
      }
    }
  }

  return {
    violations,
    unusedAllow: [...allowed].filter((license) => !usedAllow.has(license)),
    unusedExceptions: policy.exceptions
      .map((entry) => `${entry.package}:${entry.license}`)
      .filter((key) => !usedExceptions.has(key)),
    checked: Object.keys(packages).length,
  }
}

/**
 * Returns what is wrong with one licence expression, or `null` when it passes.
 *
 * @param {string} name - the package name, so the advice can say which
 *   `exceptions` entry would admit it.
 * @param {string | undefined} expression - the licence license-checker found.
 * @param {(license: string) => boolean} isAllowed - the policy for this package.
 * @param {(license: string) => void} markUsed - told which licences it relied on.
 * @returns {string | null} the problem, phrased as what to do.
 */
function problemWith(name, expression, isAllowed, markUsed) {
  if (
    expression === undefined ||
    expression.trim() === "" ||
    expression.includes("UNKNOWN")
  ) {
    return "no licence found. Check the package; if it is free software, add a clarifications entry or replace the dependency."
  }
  if (expression.includes("UNLICENSED")) {
    return "the package says UNLICENSED (no permission to use it). Remove the dependency."
  }
  if (expression.endsWith("*")) {
    return "the licence was guessed from a file, not declared. Confirm it upstream and declare it explicitly (clarifications), or replace the dependency."
  }
  /** @type {parseSpdx.Info} */
  let parsed
  try {
    parsed = parseSpdx(expression)
  } catch {
    return "not a valid SPDX expression. Confirm the licence upstream and record it as a clarification, or replace the dependency."
  }
  const used = satisfies(parsed, isAllowed)
  if (used === null) {
    // ADR 0027 §3: a licence beyond ADR 0022 §4's table enters for one named
    // package, never on `allow`, so the advice names the per-package route.
    // A licence exception (`WITH`) cannot be admitted by any entry, because
    // validatePolicy accepts only plain SPDX ids.
    return `licence not allowed by admin/license-policy.json. Replace the dependency, or add an \`exceptions\` entry for "${name}" with the licence it is used under, in a PR that states why that licence may be combined with AGPL-3.0-only (ADR 0022 §4, ADR 0027). \`allow\` takes only the licences of ADR 0022 §4's allow-list, and no entry can admit a licence exception (WITH).`
  }
  used.forEach(markUsed)
  return null
}
