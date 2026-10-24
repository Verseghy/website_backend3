/**
 * The build-time switch that keeps missing Hungarian strings loud in a
 * production build (ADR 0021 §7, "Smoke test").
 *
 * The i18n provider falls back to English for a key that `hu.ts` lacks, and
 * reports the key only when asked to (see `provider.ts`). Development and unit
 * tests always ask. A production build stays silent, because editors must
 * never see a console full of warnings. The end-to-end suite runs against a
 * production build, though, and ADR 0021 §7 requires it to run "in a mode where
 * any fallback to English fails the run". `VITE_REPORT_MISSING_KEYS=1` is that
 * mode: the CI E2E job sets it when it builds `admin/dist`, every missing key
 * then reaches `console.error`, and the E2E fixture fails the test on any
 * console error. The Containerfile never sets it.
 *
 * This module is pure and has no DOM or Vite dependency, so `vite.config.ts`
 * validates the variable at build time with the same function that
 * `main.tsx` uses at run time.
 */

/** The name of the build-time variable. */
export const REPORT_MISSING_KEYS_VARIABLE = "VITE_REPORT_MISSING_KEYS"

/**
 * Thrown when `VITE_REPORT_MISSING_KEYS` has a value other than `1`, `0` or
 * nothing. The message names the variable and the accepted values.
 */
export class InvalidReportMissingKeysError extends Error {
  /**
   * @param value - the rejected value, quoted in the message.
   */
  constructor(value: string) {
    super(
      `${REPORT_MISSING_KEYS_VARIABLE} must be "1" (report missing Hungarian strings, ` +
        `for the E2E build), "0" or unset, but it is ${JSON.stringify(value)}.`
    )
    this.name = "InvalidReportMissingKeysError"
  }
}

/**
 * Reads the switch.
 *
 * @param value - the raw `VITE_REPORT_MISSING_KEYS` value. `undefined`, an
 *   empty string or only whitespace mean "not set".
 * @returns `true` for `1`, `false` for `0` or no value.
 * @throws {InvalidReportMissingKeysError} for any other value, so a typo such
 *   as `true` or `yes` fails the build instead of silently turning the check
 *   off.
 */
export function parseReportMissingKeys(value: string | undefined): boolean {
  const trimmed = (value ?? "").trim()
  if (trimmed === "" || trimmed === "0") {
    return false
  }
  if (trimmed === "1") {
    return true
  }
  throw new InvalidReportMissingKeysError(value ?? "")
}

/** The inputs of {@link shouldReportMissingKeys}. */
export interface ReportMissingKeysInputs {
  /** Whether this is a production build (`import.meta.env.PROD`). */
  readonly production: boolean
  /** The raw `VITE_REPORT_MISSING_KEYS` value. */
  readonly flag: string | undefined
}

/**
 * Decides whether the i18n provider reports missing keys.
 *
 * @param inputs - the build mode and the raw switch value; see
 *   {@link ReportMissingKeysInputs}.
 * @returns `true` outside production builds, and in a production build when
 *   the switch is `1`.
 * @throws {InvalidReportMissingKeysError} when the switch is malformed.
 */
export function shouldReportMissingKeys(
  inputs: ReportMissingKeysInputs
): boolean {
  const requested = parseReportMissingKeys(inputs.flag)
  return !inputs.production || requested
}
