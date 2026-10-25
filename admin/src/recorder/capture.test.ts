// @vitest-environment node
import { describe, expect, it, vi } from "vitest"

import {
  buildCapture,
  CAPTURE_FORMAT,
  CAPTURE_VERSION,
  captureFileName,
  serializeCapture,
} from "./capture"

const CONTEXT = {
  capturedAt: new Date("2026-10-25T08:30:00.000Z"),
  userAgent: "test-agent/1.0",
}

/** A Word-like clipboard on Windows: HTML, text, RTF and a picture; invented. */
function wordClipboard() {
  const data: Record<string, string> = {
    "text/html":
      '<p>Eredmény</p><img src="file:///C:/Users/teszt/AppData/Local/Temp/clip.png">',
    "text/plain": "Eredmény\r\nC:\\Users\\teszt\\Documents",
    "text/rtf": "{\\rtf1 must never be read}",
  }
  return {
    types: ["text/html", "text/plain", "text/rtf", "Files"],
    getData: vi.fn((format: string) => data[format] ?? ""),
    files: [
      new File(["png"], "C:\\Users\\teszt\\kep.png", { type: "image/png" }),
    ],
  }
}

describe("buildCapture", () => {
  it("records the types, the scrubbed texts and each file's type and size", () => {
    expect(buildCapture(wordClipboard(), CONTEXT)).toEqual({
      format: CAPTURE_FORMAT,
      version: CAPTURE_VERSION,
      capturedAt: "2026-10-25T08:30:00.000Z",
      userAgent: "test-agent/1.0",
      types: ["text/html", "text/plain", "text/rtf", "Files"],
      html: '<p>Eredmény</p><img src="file:///C:/Users/USER/AppData/Local/Temp/clip.png">',
      text: "Eredmény\r\nC:\\Users\\USER\\Documents",
      files: [{ type: "image/png", size: 3 }],
      scrubbedPaths: 2,
    })
  })

  it("reads only text/html and text/plain, and keeps no RTF, file name or bytes", () => {
    const source = wordClipboard()
    const file = serializeCapture(buildCapture(source, CONTEXT))
    expect(source.getData.mock.calls).toEqual([["text/html"], ["text/plain"]])
    expect(file).not.toMatch(/rtf1|teszt|kep/)
  })

  it("tells an absent format (null) from an empty one", () => {
    const capture = buildCapture(
      { types: ["text/plain"], getData: () => "", files: [] },
      CONTEXT
    )
    expect(capture.html).toBeNull()
    expect(capture.text).toBe("")
  })

  it("accepts the array-like types and files of a real DataTransfer", () => {
    const capture = buildCapture(
      {
        types: { length: 1, 0: "Files" },
        getData: () => "",
        files: { length: 1, 0: { type: "", size: 0 } },
      },
      CONTEXT
    )
    expect(capture.types).toEqual(["Files"])
    expect(capture.files).toEqual([{ type: "", size: 0 }])
  })

  it("refuses an invalid date with a message that names it", () => {
    expect(() =>
      buildCapture(wordClipboard(), {
        capturedAt: new Date("not a date"),
        userAgent: "x",
      })
    ).toThrow(/capturedAt must be a valid Date/)
  })
})

describe("captureFileName and serializeCapture", () => {
  it("name the file after the paste's instant and write indented JSON", () => {
    const capture = buildCapture(wordClipboard(), CONTEXT)
    expect(captureFileName(capture)).toBe(
      "clipboard-capture-2026-10-25T08-30-00-000Z.json"
    )
    const text = serializeCapture(capture)
    expect(text).toMatch(/^\{\n {2}"format": "backend3-clipboard-capture",\n/)
    expect(text.endsWith("}\n")).toBe(true)
    expect(JSON.parse(text)).toEqual(capture)
  })
})
