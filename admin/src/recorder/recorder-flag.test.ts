// @vitest-environment node
import { describe, expect, it } from "vitest"

import {
  CLIPBOARD_RECORDER_VARIABLE,
  includeClipboardRecorder,
  InvalidClipboardRecorderFlagError,
} from "./recorder-flag"

describe("includeClipboardRecorder", () => {
  it.each([undefined, "", "  ", "0", " 0\n", "1"])(
    "includes the recorder on the dev server when the switch is %j",
    (flag) => {
      expect(includeClipboardRecorder({ build: false, flag })).toBe(true)
    }
  )

  it.each([undefined, "", "  ", "0", " 0\n"])(
    "leaves it out of a build when the switch is %j",
    (flag) => {
      expect(includeClipboardRecorder({ build: true, flag })).toBe(false)
    }
  )

  it.each(["1", " 1\n"])(
    "includes it in a build when the switch is %j",
    (flag) => {
      expect(includeClipboardRecorder({ build: true, flag })).toBe(true)
    }
  )

  it.each(["true", "yes", "on", "2", "01", "-1"])(
    "refuses %j in both modes, naming the variable",
    (flag) => {
      for (const build of [false, true]) {
        expect(() => includeClipboardRecorder({ build, flag })).toThrow(
          InvalidClipboardRecorderFlagError
        )
        expect(() => includeClipboardRecorder({ build, flag })).toThrow(
          new RegExp(`^${CLIPBOARD_RECORDER_VARIABLE} must be "1"`)
        )
      }
    }
  )
})
