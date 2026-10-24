import { describe, expect, it } from "vitest"

import {
  InvalidReportMissingKeysError,
  parseReportMissingKeys,
  REPORT_MISSING_KEYS_VARIABLE,
  shouldReportMissingKeys,
} from "./report-missing-keys"

describe("parseReportMissingKeys", () => {
  it.each([undefined, "", "  ", "0", " 0\n"])("reads %j as off", (value) => {
    expect(parseReportMissingKeys(value)).toBe(false)
  })

  it.each(["1", " 1\n"])("reads %j as on", (value) => {
    expect(parseReportMissingKeys(value)).toBe(true)
  })

  it.each(["true", "yes", "on", "2", "01", "-1"])(
    "refuses %j with a message that names the variable",
    (value) => {
      expect(() => parseReportMissingKeys(value)).toThrow(
        InvalidReportMissingKeysError
      )
      expect(() => parseReportMissingKeys(value)).toThrow(
        new RegExp(`^${REPORT_MISSING_KEYS_VARIABLE} must be "1"`)
      )
    }
  )
})

describe("shouldReportMissingKeys", () => {
  it("reports outside production builds whatever the switch says", () => {
    expect(
      shouldReportMissingKeys({ production: false, flag: undefined })
    ).toBe(true)
    expect(shouldReportMissingKeys({ production: false, flag: "0" })).toBe(true)
  })

  it("stays silent in a production build unless the switch is on", () => {
    expect(shouldReportMissingKeys({ production: true, flag: undefined })).toBe(
      false
    )
    expect(shouldReportMissingKeys({ production: true, flag: "0" })).toBe(false)
    expect(shouldReportMissingKeys({ production: true, flag: "1" })).toBe(true)
  })

  it("refuses a malformed switch in every mode", () => {
    expect(() =>
      shouldReportMissingKeys({ production: false, flag: "yes" })
    ).toThrow(InvalidReportMissingKeysError)
    expect(() =>
      shouldReportMissingKeys({ production: true, flag: "yes" })
    ).toThrow(InvalidReportMissingKeysError)
  })
})
