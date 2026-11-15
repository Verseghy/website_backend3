import fc from "fast-check"
import { describe, expect, it } from "vitest"

import {
  appliesOnScreen,
  compareSpecificity,
  inlineStyleSheets,
  isAppliedStyleElement,
  selectorSpecificity,
  splitSelectorList,
} from "./inline-styles"

const parser = new DOMParser()

describe("splitSelectorList", () => {
  it.each([
    ["td", ["td"]],
    ["td, .xl65 ,p.MsoNormal", ["td", ".xl65", "p.MsoNormal"]],
    ["a[title='x,y'], b", ["a[title='x,y']", "b"]],
    [":is(p, li) span, em", [":is(p, li) span", "em"]],
    ['a[title="\\",x"]', ['a[title="\\",x"]']],
    [" , ,", []],
  ])("splits %j", (list, selectors) => {
    expect(splitSelectorList(list)).toEqual(selectors)
  })

  it("never splits inside brackets and keeps every non-comma character", () => {
    fc.assert(
      fc.property(
        fc.array(fc.constantFrom("td", ".xl65", "p.Mso", "#a", "a[x]"), {
          minLength: 1,
          maxLength: 6,
        }),
        (selectors) => {
          expect(splitSelectorList(selectors.join(", "))).toEqual(selectors)
        }
      )
    )
  })
})

describe("selectorSpecificity", () => {
  it.each<[string, [number, number, number]]>([
    ["td", [0, 0, 1]],
    [".xl65", [0, 1, 0]],
    ["p.MsoNormal", [0, 1, 1]],
    ["li.MsoNormal p", [0, 1, 2]],
    ["#main .x > a:hover", [1, 2, 1]],
    ["td[class~=xl65]", [0, 1, 1]],
    ["*", [0, 0, 0]],
    ["*|td", [0, 0, 1]],
    [":is(#a, .b) p", [1, 0, 1]],
    [":not(.a.b)", [0, 2, 0]],
    [":where(#a) p", [0, 0, 1]],
    ["li:nth-child(2n+1)", [0, 1, 1]],
    ["p.a\\:b", [0, 1, 1]],
  ])("counts %s as %j", (selector, specificity) => {
    expect(selectorSpecificity(selector)).toEqual(specificity)
  })

  it.each([
    "p::before",
    "p:after",
    "p:first-line",
    "",
    "p:is(",
    "a[x",
    "#",
    ".",
    "p:",
    "&",
  ])("gives no specificity for %j", (selector) => {
    expect(selectorSpecificity(selector)).toBeNull()
  })

  it("orders by ids, then classes, then types", () => {
    expect(compareSpecificity([0, 1, 0], [0, 0, 9])).toBeGreaterThan(0)
    expect(compareSpecificity([1, 0, 0], [0, 9, 9])).toBeGreaterThan(0)
    expect(compareSpecificity([0, 1, 1], [0, 1, 1])).toBe(0)
  })

  it("never throws, and counts at least one type for a bare tag name", () => {
    fc.assert(
      fc.property(fc.string({ maxLength: 40 }), (selector) => {
        const specificity = selectorSpecificity(selector)
        expect(specificity === null || specificity.length === 3).toBe(true)
      }),
      { numRuns: 500 }
    )
    fc.assert(
      fc.property(fc.stringMatching(/^[a-z][a-z0-9]{0,8}$/), (tag) => {
        expect(selectorSpecificity(tag)).toEqual([0, 0, 1])
      })
    )
  })
})

describe("inlineStyleSheets (editor.md §5.2 step 6)", () => {
  /** Parses, inlines and returns the document. */
  const inline = (html: string) => {
    const document = parser.parseFromString(html, "text/html")
    inlineStyleSheets(document)
    return document
  }
  const styleOf = (document: Document, selector: string, property: string) =>
    (document.querySelector(selector) as HTMLElement).style.getPropertyValue(
      property
    )

  it("applies type, then class, then the element's own style", () => {
    const document = inline(
      "<style>.xl65 { text-align: center; color: blue } td { text-align: left; color: red; vertical-align: bottom }</style><table><tr><td class=xl65 style='color: green'>a</td></tr></table>"
    )
    expect(styleOf(document, "td", "text-align")).toBe("center")
    expect(styleOf(document, "td", "color")).toBe("green")
    expect(styleOf(document, "td", "vertical-align")).toBe("bottom")
  })

  it("keeps the merged cell's own right border over its class's border (the merged-cell rule)", () => {
    const document = inline(
      "<style>.xl66 { border: 0.5pt solid black }</style><table><tr><td class=xl66 colspan=2 style='border-right: 1pt solid black'>a</td></tr></table>"
    )
    expect(styleOf(document, "td", "border-right-width")).toBe("1pt")
    expect(styleOf(document, "td", "border-left-width")).toBe("0.5pt")
  })

  it("reads rules from a <style> in the body too, and in source order at equal specificity", () => {
    const document = inline(
      "<p class=a>x</p><style>.a { color: red }</style><style>.a { color: blue }</style>"
    )
    expect(styleOf(document, "p", "color")).toBe("blue")
  })

  it("applies each selector of a list with its own specificity", () => {
    const document = inline(
      "<style>p.x, td { color: red } p { color: blue }</style><p class=x>a</p>"
    )
    expect(styleOf(document, "p", "color")).toBe("red")
  })

  it("skips pseudo-element rules, invalid selectors and @-rules", () => {
    const document = inline(
      "<style>@page Section1 { margin: 2cm } p::before { color: red } p:unknown-thing { color: red } @media print { p { color: red } } p { text-align: right }</style><p>a</p>"
    )
    expect(styleOf(document, "p", "color")).toBe("")
    expect(styleOf(document, "p", "text-align")).toBe("right")
  })

  it("removes every <style> element and leaves unmatched elements alone", () => {
    const document = inline(
      "<head><style>td { color: red }</style></head><body><p style='color: green'>a</p><style>li { color: red }</style></body>"
    )
    expect(document.querySelectorAll("style")).toHaveLength(0)
    expect(document.querySelector("p")?.getAttribute("style")).toBe(
      "color: green"
    )
  })

  it("reads the engine's own sheet, or a constructed one where the engine built none (jsdom)", () => {
    const document = parser.parseFromString(
      "<style>p { color: red }</style><p>a</p>",
      "text/html"
    )
    const sheet = (document.querySelector("style") as HTMLStyleElement).sheet
    // Every browser engine builds the sheet of a DOMParser document; jsdom
    // builds none for a document without a window.
    expect(sheet === null).toBe(navigator.userAgent.includes("jsdom"))
    inlineStyleSheets(document)
    expect(styleOf(document, "p", "color")).toBe("red")
  })

  it.each(["text/plain", "text/x-template"])(
    "reads no rule from a <style type=%s>, which is no CSS style sheet",
    (type) => {
      const document = inline(
        `<style type="${type}">p { color: red }</style><style type="TEXT/CSS">p { text-align: right }</style><p>a</p>`
      )
      expect(styleOf(document, "p", "color")).toBe("")
      expect(styleOf(document, "p", "text-align")).toBe("right")
      expect(document.querySelectorAll("style")).toHaveLength(0)
    }
  )

  it("returns how many elements received rules", () => {
    const document = parser.parseFromString(
      "<style>.a { color: red }</style><p class=a>1</p><p class=a>2</p><p>3</p>",
      "text/html"
    )
    expect(inlineStyleSheets(document)).toBe(2)
  })

  it.each([
    ["print", "print"],
    ["a condition", "screen and (max-width: 600px)"],
    ["speech", "speech"],
  ])(
    "reads no rule from a <style> for %s, which the source page did not apply on every screen",
    (_name, media) => {
      const document = inline(
        `<style media="${media}">.x { display: none; color: red }</style><style media=" Screen , print">p { text-align: right }</style><p class=x>a</p>`
      )
      expect(styleOf(document, "p", "display")).toBe("")
      expect(styleOf(document, "p", "color")).toBe("")
      expect(styleOf(document, "p", "text-align")).toBe("right")
      expect(document.querySelectorAll("style")).toHaveLength(0)
    }
  )

  it("reads no rule from a <style> inside <noscript>, which the source page showed as text", () => {
    const document = inline(
      "<noscript><style>p { display: none }</style></noscript><p>a</p>"
    )
    expect(styleOf(document, "p", "display")).toBe("")
    expect(document.querySelectorAll("style")).toHaveLength(0)
  })

  it("never finds a <style> inside <template>, whose content is not in the document", () => {
    const document = inline(
      "<template><style>p { display: none }</style></template><p>a</p>"
    )
    expect(styleOf(document, "p", "display")).toBe("")
  })
})

describe("appliesOnScreen", () => {
  /** A `<style>` with a `media` attribute, or none for `null`. */
  const style = (media: string | null) => {
    const element = parser
      .parseFromString("<style></style>", "text/html")
      .querySelector("style")!
    if (media !== null) {
      element.setAttribute("media", media)
    }
    return element
  }

  it.each<[string | null, boolean]>([
    [null, true],
    ["", true],
    [" \t", true],
    ["all", true],
    ["screen", true],
    ["SCREEN", true],
    ["only  screen", true],
    ["print, screen", true],
    ["print", false],
    ["only print", false],
    ["screen and (min-width: 800px)", false],
    ["not print", false],
    [",", false],
  ])("reads media=%j as %s", (media, expected) => {
    expect(appliesOnScreen(style(media))).toBe(expected)
  })

  it("holds exactly when one query of the list is all or screen", () => {
    const query = fc.constantFrom(
      "all",
      "screen",
      " only screen ",
      "Print",
      "speech",
      "(color)",
      "screen and (color)",
      "not all",
      ""
    )
    fc.assert(
      fc.property(
        fc.array(query, { minLength: 1, maxLength: 4 }),
        (queries) => {
          const media = queries.join(",")
          const screen = queries.some((q) =>
            ["all", "screen", "only screen"].includes(q.trim().toLowerCase())
          )
          expect(appliesOnScreen(style(media))).toBe(
            screen || media.trim() === ""
          )
        }
      )
    )
  })
})

describe("isAppliedStyleElement", () => {
  it.each<[string, boolean]>([
    ["<style>p{}</style>", true],
    ['<style type="text/plain">p{}</style>', false],
    ['<style media="print">p{}</style>', false],
    ["<noscript><style>p{}</style></noscript>", false],
    ["<p>b</p><noscript><div><style>p{}</style></div></noscript>", false],
  ])("reads %s as %s", (html, expected) => {
    const element = parser
      .parseFromString(`${html}<p>a</p>`, "text/html")
      .querySelector("style")!
    expect(isAppliedStyleElement(element)).toBe(expected)
  })
})
