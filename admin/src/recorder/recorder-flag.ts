/**
 * The build-time switch for the development-only clipboard recorder
 * (editor.md §13.1).
 *
 * Editors never need the recorder, so the dev server always has it and a
 * build has it only with `VITE_CLIPBOARD_RECORDER=1` (the CI E2E build sets
 * it; the Containerfile pins `0`). `vite.config.ts` turns the decision into
 * the `__CLIPBOARD_RECORDER__` constant, the bundler drops the branch of
 * `main.tsx` that it guards, and `production-bundle.test.ts` checks the
 * image build for any trace of it.
 * Pure, so `vite.config.ts` can import it.
 */

/** The name of the build-time variable. */
export const CLIPBOARD_RECORDER_VARIABLE = "VITE_CLIPBOARD_RECORDER"

/** Thrown for a `VITE_CLIPBOARD_RECORDER` other than `1`, `0` or nothing. */
export class InvalidClipboardRecorderFlagError extends Error {
  /**
   * @param value - the rejected value, quoted in the message.
   */
  constructor(value: string) {
    super(
      `${CLIPBOARD_RECORDER_VARIABLE} must be "1" (include the development-only ` +
        `clipboard recorder in this build), "0" or unset, but it is ${JSON.stringify(value)}.`
    )
    this.name = "InvalidClipboardRecorderFlagError"
  }
}

/** The inputs of {@link includeClipboardRecorder}. */
export interface ClipboardRecorderInputs {
  /** Whether Vite is building (`vite build`) rather than serving (`vite`). */
  readonly build: boolean
  /**
   * The raw `VITE_CLIPBOARD_RECORDER` value; `undefined`, empty or blank mean
   * "not set".
   */
  readonly flag: string | undefined
}

/**
 * Decides whether this build or dev server includes the recorder.
 *
 * @param inputs - the Vite command and the raw switch; see
 *   {@link ClipboardRecorderInputs}.
 * @returns `true` on the dev server, and in a build when the switch is `1`.
 * @throws {InvalidClipboardRecorderFlagError} for any value other than `1`,
 *   `0` or nothing, in both modes, so a typo such as `true` fails instead of
 *   silently leaving the recorder out.
 */
export function includeClipboardRecorder(
  inputs: ClipboardRecorderInputs
): boolean {
  const flag = (inputs.flag ?? "").trim()
  if (flag !== "" && flag !== "0" && flag !== "1") {
    throw new InvalidClipboardRecorderFlagError(inputs.flag ?? "")
  }
  return !inputs.build || flag === "1"
}
