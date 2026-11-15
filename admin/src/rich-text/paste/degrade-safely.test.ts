/**
 * The degrade-safely suite of the paste filter (editor.md §13.5, ADR 0013
 * §3, "Testing"): every source outside v1's paste targets goes through the
 * whole paste path, and only invariants are asserted, never a snapshot.
 *
 * It runs twice: under jsdom with the other unit tests (`npm test`), and in
 * Chromium, Firefox and WebKit through Vitest's browser mode
 * (`npm run test:browser`, `vitest.browser.config.ts`), because the filter
 * reads styles through each engine's CSSOM.
 *
 * Inputs: the roosterjs samples of Excel and Word for the web, the
 * synthetic Google Docs, Google Sheets, LibreOffice, Safari and web-page
 * shapes (`admin/test/fixtures/`), and random clipboard HTML over the pools
 * of `src/test/paste-arbitrary.ts`. For each:
 *
 * - neither the filter nor the paste handlers throw, and the plain-text
 *   fallback is never taken;
 * - the filter's output passes P-paste (only content-v1, no `<style>`);
 * - the stored result `x` satisfies `J(x) == x`, holds no CSSOM spelling,
 *   and every image in it has an address the server keeps. Which pictures a
 *   paste may keep at all (none from outside sources, §5.3) is asserted by
 *   the picture rules' own tests;
 * - the fixtures' stored results are golden files of `tests/fixtures/e1/`
 *   (`paste-*.html`, `src/test/golden-e1.ts`), so the server checks P2 and P3
 *   on them too.
 */
import { Editor } from "@tiptap/core"
import fc from "fast-check"
import { afterAll, describe, expect, it } from "vitest"

import type { EditorNotice } from "../extensions/editor-notice"
import { PASTE_NOTICES } from "../extensions/office-paste"
import { loadDocument } from "../load-document"
import { imageSrc } from "../media-src"
import { CONTENT_V1_POLICY } from "../policy"
import { toStoredHtml } from "../serialize/to-stored-html"
import { siteExtensions } from "../site-extensions"
import { filterPastedHtml } from "./paste-filter"
import { clipboardHtml } from "@/test/paste-arbitrary"
import { clipboardPasteEvent, pasteFixtures } from "@/test/paste-fixtures"
import { pasteViolations } from "@/test/paste-invariants"

const policy = CONTENT_V1_POLICY
const parser = new DOMParser()
const notices: EditorNotice[] = []
const editor = new Editor({
  element: document.createElement("div"),
  extensions: siteExtensions({
    policy,
    notify: (notice) => notices.push(notice),
  }),
})
afterAll(() => editor.destroy())

/** The stored HTML of the editor's document. */
const save = () => toStoredHtml(editor.state.doc, { policy, document })

/** `J`: load, then save. */
function J(html: string): string {
  loadDocument(editor, html)
  return save()
}

/** Pastes clipboard HTML into an empty body, as a browser does, and saves. */
function paste(html: string): string {
  notices.length = 0
  loadDocument(editor, "<p></p>")
  editor.commands.setTextSelection(1)
  editor.view.dom.dispatchEvent(clipboardPasteEvent({ "text/html": html }))
  return save()
}

/**
 * What stored HTML's markup holds that the editor never writes: a `<style>`,
 * a comment, or a CSSOM spelling (`rgb()`, a grouped `border-width`) in an
 * attribute. Text is not checked: a pasted `<xmp>` or `<textarea>` holds its
 * markup as text, which the editor keeps, escaped, as text.
 */
function markupProblems(html: string): string[] {
  const template = document.createElement("template")
  template.innerHTML = html
  const problems: string[] = []
  const walker = document.createTreeWalker(
    template.content,
    NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_COMMENT
  )
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    if (node.nodeType === Node.COMMENT_NODE) {
      problems.push(`a comment: ${node.textContent}`)
      continue
    }
    const element = node as Element
    if (element.localName === "style") {
      problems.push("a <style>")
    }
    for (const { name, value } of [...element.attributes]) {
      if (/rgb\(|border-width:/.test(value)) {
        problems.push(`<${element.localName} ${name}="${value}">`)
      }
    }
  }
  return problems
}

/** Asserts the invariants of one input. */
function degradesSafely(html: string): void {
  const outcome = filterPastedHtml(html, { policy, parser })
  expect(outcome.kind).toBe("filtered")
  expect(
    pasteViolations(outcome.html, policy, {
      sliceMarker: outcome.kind === "filtered" && outcome.source === "internal",
    })
  ).toEqual([])
  const stored = paste(html)
  expect(notices).not.toContain(PASTE_NOTICES.fallback)
  expect(J(stored)).toBe(stored)
  expect(markupProblems(stored)).toEqual([])
  for (const [, src] of stored.matchAll(/<img src="([^"]*)"/g)) {
    expect(imageSrc(src, policy.urls)).toBe(src)
  }
}

describe("the paste filter degrades safely (editor.md §13.5)", () => {
  it.each(pasteFixtures().map(({ name, html }) => [name, html]))(
    "on %s",
    (_name, html) => {
      degradesSafely(html)
    }
  )

  // A shrunk case of the random documents (WebKit): the raw text of an
  // `<xmp>` holds markup, which stays text.
  it("on markup inside a raw-text element", () => {
    degradesSafely(
      '<p><xmp><p style="color: rgb(192, 0, 0) !important"></p></xmp></p>'
    )
    expect(paste('<xmp><b style="color: rgb(1, 2, 3)">x</b></xmp>')).toBe(
      '<p>&lt;b style="color: rgb(1, 2, 3)"&gt;x&lt;/b&gt;</p>'
    )
  })

  // Shrunk cases of the random documents with the editor's own slice
  // marker: a context that does not match the slice made
  // prosemirror-transform's fitting throw during the paste, until the
  // filter left the context out (`keptSliceMarker`).
  it.each([
    "<p data-pm-slice='1 1 [\"blockquote\",null]'>a</p>a",
    '<p data-pm-slice=\'2 2 -1 ["bulletList",null,"listItem",null]\'>a</p><p>b</p>',
    '<p data-pm-slice=\'3 3 ["table",null,"tableRow",null,"tableCell",{"colspan":1}]\'>a</p><ul><li>b</li></ul>',
  ])("on a slice marker whose context does not match its slice: %s", (html) => {
    degradesSafely(html)
    expect(paste(html)).toContain("a")
  })

  // Each run pastes through the whole editor twice; under jsdom on a busy
  // machine 150 runs take longer than Vitest's default 5 s.
  it("on random clipboard HTML", () => {
    fc.assert(
      fc.property(clipboardHtml, (html) => {
        degradesSafely(html)
      }),
      { numRuns: 150 }
    )
  }, 60_000)

  it("keeps the text of every fixture", () => {
    for (const { name, html } of pasteFixtures()) {
      const stored = paste(html)
      expect(stored.replace(/<[^>]*>/g, "").trim(), name).not.toBe("")
    }
  })
})
