/**
 * The AGPL-3.0 §13 source offer of the admin UI (ADR 0012 §4).
 *
 * Editors use backend3 over a network, so the admin footer links to the public
 * repository at the commit that is running. Production images are built only
 * from public `master` commits by PAC (ADR 0022), and the Containerfile passes
 * that commit's full SHA to the admin build as `VITE_SOURCE_COMMIT`. A local
 * build has no commit and links to `master` instead.
 *
 * This module is pure and has no DOM or Vite dependency, so `vite.config.ts`
 * uses the same validation at build time that the footer relies on at run
 * time.
 */

/** The public repository whose source the running admin was built from. */
export const SOURCE_REPOSITORY_URL =
  "https://github.com/Verseghy/website_backend3"

/** The branch the link falls back to when the build carries no commit. */
export const FALLBACK_REF = "master"

/** A full, lowercase git commit SHA, as PAC passes `{{ revision }}`. */
const FULL_SHA = /^[0-9a-f]{40}$/

/**
 * Thrown when `VITE_SOURCE_COMMIT` is set but is not a full commit SHA.
 *
 * The message names the variable and the expected shape, so whoever runs the
 * build knows exactly what to fix.
 */
export class InvalidSourceCommitError extends Error {
  /**
   * @param value - the rejected value, quoted in the message.
   */
  constructor(value: string) {
    super(
      `VITE_SOURCE_COMMIT must be empty or a full 40-character lowercase git commit SHA ` +
        `(the Containerfile passes ARG SOURCE_COMMIT), but it is ${JSON.stringify(value)}.`
    )
    this.name = "InvalidSourceCommitError"
  }
}

/**
 * Normalises and validates the build's source commit.
 *
 * @param value - the raw `VITE_SOURCE_COMMIT` value. `undefined`, an empty
 *   string or only whitespace mean "no commit" (a local build).
 * @returns the commit SHA, or `null` when the build carries none.
 * @throws {InvalidSourceCommitError} when the value is set but is not a full,
 *   lowercase 40-character hexadecimal SHA. Abbreviated SHAs are rejected on
 *   purpose: they can become ambiguous as the repository grows.
 */
export function parseSourceCommit(value: string | undefined): string | null {
  const trimmed = (value ?? "").trim()
  if (trimmed === "") {
    return null
  }
  if (!FULL_SHA.test(trimmed)) {
    throw new InvalidSourceCommitError(value ?? "")
  }
  return trimmed
}

/**
 * Builds the URL of the source tree the admin was built from.
 *
 * @param commit - the raw `VITE_SOURCE_COMMIT` value (see
 *   {@link parseSourceCommit}).
 * @returns `https://github.com/Verseghy/website_backend3/tree/<commit>`, or
 *   `.../tree/master` when there is no commit.
 * @throws {InvalidSourceCommitError} when `commit` is malformed.
 */
export function sourceTreeUrl(commit: string | undefined): string {
  const sha = parseSourceCommit(commit)
  return `${SOURCE_REPOSITORY_URL}/tree/${sha ?? FALLBACK_REF}`
}
