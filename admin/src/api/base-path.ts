/**
 * The one rule for the admin API's base path, shared by every module that
 * talks to `/api/admin` (the data provider and the auth provider).
 *
 * Why it exists: the admin is served from the same origin as its API
 * (ADR 0004 §3), and the session cookie is `SameSite=Strict` and `__Host-`
 * prefixed (ADR 0006 §1), so every request must stay on the admin's own
 * origin. An absolute URL, or any spelling that a URL parser resolves to
 * another host, would send requests elsewhere without the cookie or, worse,
 * send the sign-in body to a foreign host. Both providers take the base path
 * as an explicit option and refuse a bad one at start-up with the same
 * actionable message.
 */

/**
 * A same-origin API base path: `/` and an unreserved segment, repeated, with
 * no empty, `.` or `..` segment. A browser resolves it against the admin's own
 * origin, never against another host. The narrow character set also rules out
 * every spelling that a URL parser reads as another host, such as `//host` or
 * `/\host` (WHATWG URL parsing treats `\` like `/`).
 */
const SAME_ORIGIN_BASE_PATH = /^(?:\/(?!\.{1,2}(?:\/|$))[A-Za-z0-9._~-]+)+$/

/**
 * Whether `value` is a root-relative base path that keeps requests on the
 * admin's own origin.
 *
 * @param value - the configured base path, of any type (it is an option at a
 *   module boundary).
 * @returns `true` for a path such as `/api/admin`.
 */
export function isSameOriginBasePath(value: unknown): value is string {
  return typeof value === "string" && SAME_ORIGIN_BASE_PATH.test(value)
}

/**
 * Describes what is wrong with a configured base path.
 *
 * @param option - the option's name as its caller documents it, for example
 *   `apiBaseUrl`; the message names it so the developer knows what to fix.
 * @param value - the configured value.
 * @returns `undefined` when the value is a same-origin base path, otherwise
 *   a message that says what the value must look like and what it is.
 */
export function basePathProblem(
  option: string,
  value: unknown
): string | undefined {
  if (isSameOriginBasePath(value)) {
    return undefined
  }
  return (
    `${option} must be a root-relative path such as "/api/admin" (leading "/", no trailing "/", ` +
    `no scheme or host, only letters, digits and "-._~" in "/"-separated segments, so requests ` +
    `stay same-origin), but it is ${JSON.stringify(value)}.`
  )
}
