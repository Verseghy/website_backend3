import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { CONTENT_V1_POLICY } from "../policy"
import type { PasteFilter } from "./normalize-pasted-html"
import { escapeText, filterPastedHtml, plainTextFallback } from "./paste-filter"
import { WORD_NAMESPACE } from "./paste-source"
import { pasteViolations } from "@/test/paste-invariants"

const policy = CONTENT_V1_POLICY
const parser = new DOMParser()

/** A filter that always throws, as an unforeseen input would make it. */
const throwing: PasteFilter = () => {
  throw new Error("unforeseen input")
}

describe("escapeText", () => {
  it("escapes &, < and > only", () => {
    expect(escapeText(`a & <b> "c" 'd'`)).toBe(`a &amp; &lt;b&gt; "c" 'd'`)
  })
})

describe("plainTextFallback (editor.md §5.5)", () => {
  it("keeps the text, one paragraph per line, escaped", () => {
    expect(
      plainTextFallback(
        "<html><head><title>cím</title><style>p{}</style></head><body><p>első <b>sor</b></p><p>második &lt;sor&gt;</p><table><tr><td>a</td><td>b</td></tr></table><script>x()</script><ul><li>pont</li></ul>a<br>b</body></html>junk",
        parser
      )
    ).toBe(
      "<p>első sor</p><p>második &lt;sor&gt;</p><p>ab</p><p>pont</p><p>a</p><p>b</p>"
    )
  })

  it("gives nothing for a clipboard without text, and when parsing fails", () => {
    expect(plainTextFallback("<img src=x><p> </p>", parser)).toBe("")
    expect(plainTextFallback("<p>a</p>", {} as DOMParser)).toBe("")
  })

  it("only ever writes paragraphs of escaped text", () => {
    fc.assert(
      fc.property(fc.string({ maxLength: 200 }), (html) => {
        const fallback = plainTextFallback(html, parser)
        expect(fallback).toMatch(/^(<p>[^<>]*<\/p>)*$/)
        expect(pasteViolations(fallback, policy)).toEqual([])
      })
    )
  })
})

describe("filterPastedHtml", () => {
  it("returns the filter's result", () => {
    expect(filterPastedHtml("<p><b>a</b></p>", { policy, parser })).toEqual({
      kind: "filtered",
      html: "<p><strong>a</strong></p>",
      source: "generic",
      report: {
        moved: { media: [] },
        dropped: {
          hiddenRows: 0,
          hiddenCells: 0,
          hiddenOther: 0,
          fontFamilies: [],
          frames: 0,
          youtubeRefusals: [],
          embedParams: [],
        },
      },
    })
  })

  it("falls back to the text when the filter throws, with the detected source and the error", () => {
    const outcome = filterPastedHtml(
      `<html xmlns:w="${WORD_NAMESPACE}"><body><p>a</p><p>b</p></body></html>`,
      { policy, parser },
      throwing
    )
    expect(outcome).toMatchObject({
      kind: "fallback",
      html: "<p>a</p><p>b</p>",
      source: "word",
    })
    expect(outcome.kind === "fallback" && outcome.error).toEqual(
      new Error("unforeseen input")
    )
  })

  it("never throws, even without a parser", () => {
    const outcome = filterPastedHtml(
      "<p>a</p>",
      { policy, parser: undefined as unknown as DOMParser },
      throwing
    )
    expect(outcome).toMatchObject({ kind: "fallback", html: "", source: null })
  })
})
