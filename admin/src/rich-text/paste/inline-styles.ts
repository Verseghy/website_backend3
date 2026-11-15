/**
 * Step 6 of the paste filter (editor.md §5.2): every `<style>` rule of the
 * clipboard document is inlined into the elements it matches, and then
 * every `<style>` element is removed.
 *
 * Excel keeps the look of every cell in class rules (`.xl65 {…}`) and Word
 * part of every paragraph's (`p.MsoNormal {…}`); a `<style>` can also sit in
 * the `<body>` (Excel for the web). prosemirror-view 1.42.7 has an inliner
 * of its own (`readHTML`), but it appends class CSS after the inline style,
 * which drops the right border of a merged Excel cell (ADR 0013 Context,
 * research: editor design, clipboard §3.3). This module applies the rules
 * the way the cascade does instead, and leaves prosemirror-view nothing to
 * inline:
 *
 * - the rules of every `<style>` in the document, `<body>` included, in
 *   document order, each selector of a selector list on its own, except a
 *   sheet the source page did not apply on screen: a `media` other than
 *   `all` or `screen` (`print`, or a condition the clipboard cannot
 *   evaluate), or one inside a `<noscript>`, which the inert parser reads as
 *   markup ({@link isAppliedStyleElement});
 * - sorted by specificity, then by source order, the element's own `style`
 *   attribute last, so it wins as an inline style does;
 * - written into the element's `style` attribute as one declaration list.
 *   The element's CSSOM then reads the cascaded value of each property
 *   (`element.style.getPropertyValue`), which is how the later steps read
 *   it: CSSOM keeps the last declaration of each property and resolves
 *   shorthands against longhands in order (§5.2 step 6, "Normalise through
 *   a scratch element"; the element is its own scratch element).
 *
 * The rules are read from each `<style>` element's CSSOM `sheet`, which a
 * document from an inert `DOMParser` has in every engine (the research
 * verified it for Excel's 43 rules: research: editor design, clipboard
 * §2.5), or, where the engine built none (jsdom, under `npm test`), from a
 * constructed sheet over the same text ({@link sheetRules}). A rule is a
 * style rule when CSSOM says so
 * (`CSSRule.type === 1`), which leaves out `@page`, `@font-face`, `@list`
 * and `@media` blocks. A selector that is invalid, that names a
 * pseudo-element (the rule styles generated content, not the element), or
 * that the engine cannot match is skipped; the paste goes on without it.
 *
 * Office CSS uses type, class and simple descendant selectors only, so the
 * specificity computed here ({@link selectorSpecificity}) covers what Office
 * writes exactly, and a web page's selectors closely enough for the generic
 * path, which promises no fidelity (§5.5).
 */
import { asciiLowerCase } from "../css/declarations"

/** A selector's specificity: id selectors, class-like selectors, type selectors. */
export type Specificity = readonly [number, number, number]

/** The pseudo-elements CSS 2 wrote with one colon. */
const LEGACY_PSEUDO_ELEMENTS = new Set([
  "before",
  "after",
  "first-line",
  "first-letter",
])

/** The pseudo-classes whose specificity is that of their most specific argument. */
const ARGUMENT_PSEUDO_CLASSES = new Set(["is", "not", "has", "matches"])

/** Whether a code unit may continue a CSS name (an identifier) without an escape. */
function isNameUnit(character: string): boolean {
  return /[A-Za-z0-9_-]/.test(character) || character.charCodeAt(0) >= 0x80
}

/** The index after a name that starts at `start` (escapes included), or `start`. */
function nameEnd(text: string, start: number): number {
  let index = start
  while (index < text.length) {
    if (text[index] === "\\") {
      index = Math.min(index + 2, text.length)
    } else if (isNameUnit(text[index])) {
      index += 1
    } else {
      break
    }
  }
  return index
}

/**
 * The index after the bracket that closes the one at `start`, quotes and
 * escapes skipped, or `-1` when it is never closed.
 */
function blockEnd(text: string, start: number): number {
  const closing: Record<string, string> = { "(": ")", "[": "]" }
  const stack: string[] = []
  let quote: string | null = null
  for (let index = start; index < text.length; index += 1) {
    const character = text[index]
    if (character === "\\") {
      index += 1
    } else if (quote !== null) {
      if (character === quote) {
        quote = null
      }
    } else if (character === '"' || character === "'") {
      quote = character
    } else if (Object.hasOwn(closing, character)) {
      stack.push(closing[character])
    } else if (character === stack[stack.length - 1]) {
      stack.pop()
      if (stack.length === 0) {
        return index + 1
      }
    }
  }
  return -1
}

/**
 * Splits a selector list at its top-level commas.
 *
 * @param list - a rule's `selectorText`, such as `td, .xl65`.
 * @returns the selectors, trimmed, without empty ones; commas inside
 *   brackets, parentheses and strings do not split.
 */
export function splitSelectorList(list: string): string[] {
  const selectors: string[] = []
  let depth = 0
  let quote: string | null = null
  let start = 0
  for (let index = 0; index < list.length; index += 1) {
    const character = list[index]
    if (character === "\\") {
      index += 1
    } else if (quote !== null) {
      if (character === quote) {
        quote = null
      }
    } else if (character === '"' || character === "'") {
      quote = character
    } else if (character === "(" || character === "[") {
      depth += 1
    } else if ((character === ")" || character === "]") && depth > 0) {
      depth -= 1
    } else if (character === "," && depth === 0) {
      selectors.push(list.slice(start, index))
      start = index + 1
    }
  }
  selectors.push(list.slice(start))
  return selectors.map((selector) => selector.trim()).filter((s) => s !== "")
}

/**
 * Compares two specificities, as the cascade orders them.
 *
 * @returns a negative number when `a` is less specific than `b`, a positive
 *   one when it is more specific, and `0` when they are equal.
 */
export function compareSpecificity(a: Specificity, b: Specificity): number {
  return a[0] - b[0] || a[1] - b[1] || a[2] - b[2]
}

/**
 * The specificity of one complex selector (CSS Selectors 4 §17): id
 * selectors; classes, attribute selectors and pseudo-classes; type
 * selectors. `:is()`, `:not()` and `:has()` count as their most specific
 * argument, `:where()` counts nothing, and any other functional
 * pseudo-class counts as one pseudo-class.
 *
 * @param selector - one selector of a selector list
 *   ({@link splitSelectorList}).
 * @returns the specificity, or `null` when the selector names a
 *   pseudo-element (`::before`, or CSS 2's `:before`) or cannot be read; the
 *   caller skips such a selector.
 */
export function selectorSpecificity(selector: string): Specificity | null {
  const text = selector.trim()
  if (text === "") {
    return null
  }
  let ids = 0
  let classes = 0
  let types = 0
  let index = 0
  while (index < text.length) {
    const character = text[index]
    if (/[\s>+~|*]/.test(character)) {
      // Combinators, the universal selector and namespace bars count nothing.
      index += 1
      continue
    }
    if (character === "#" || character === ".") {
      const end = nameEnd(text, index + 1)
      if (end === index + 1) {
        return null
      }
      if (character === "#") {
        ids += 1
      } else {
        classes += 1
      }
      index = end
      continue
    }
    if (character === "[") {
      const end = blockEnd(text, index)
      if (end < 0) {
        return null
      }
      classes += 1
      index = end
      continue
    }
    if (character === ":") {
      if (text[index + 1] === ":") {
        return null
      }
      const end = nameEnd(text, index + 1)
      const name = asciiLowerCase(text.slice(index + 1, end))
      if (name === "" || LEGACY_PSEUDO_ELEMENTS.has(name)) {
        return null
      }
      if (text[end] !== "(") {
        classes += 1
        index = end
        continue
      }
      const close = blockEnd(text, end)
      if (close < 0) {
        return null
      }
      if (ARGUMENT_PSEUDO_CLASSES.has(name)) {
        const inner = splitSelectorList(text.slice(end + 1, close - 1)).map(
          selectorSpecificity
        )
        if (inner.length === 0 || inner.some((part) => part === null)) {
          return null
        }
        const [a, b, c] = (inner as Specificity[]).reduce((max, part) =>
          compareSpecificity(part, max) > 0 ? part : max
        )
        ids += a
        classes += b
        types += c
      } else if (name !== "where") {
        classes += 1
      }
      index = close
      continue
    }
    const end = nameEnd(text, index)
    if (end === index) {
      return null
    }
    types += 1
    index = end
  }
  return [ids, classes, types]
}

/** One style rule of a sheet, as CSSOM gives it. */
interface StyleRule {
  readonly selectorText: string
  readonly style: { readonly cssText: string }
}

/**
 * Whether a `<style>` element's `type` makes it a CSS style sheet: none, an
 * empty one or `text/css` in any case (HTML, "update a style block").
 */
function isCssStyleElement(element: Element): boolean {
  const type = element.getAttribute("type")
  return type === null || type === "" || asciiLowerCase(type) === "text/css"
}

/** The media queries that hold on any screen, whatever its size. */
const SCREEN_QUERIES = new Set(["all", "screen", "only all", "only screen"])

/**
 * Whether a `<style>` element's `media` attribute lets it apply on every
 * screen: no attribute, an empty one, or a list in which one query is
 * `all` or `screen` (`only` allowed, whitespace collapsed, any case).
 *
 * The source page applied a `<style media="print">` only when printed, and
 * one with a condition (`screen and (max-width: 600px)`) only on some
 * screens, which the clipboard does not tell. Such a sheet is skipped, as
 * an `@media` block inside a sheet is (step 6 reads style rules only), so
 * step 7 never removes content that was shown on screen.
 *
 * @param element - a `<style>` element.
 * @returns `true` when its rules are inlined.
 */
export function appliesOnScreen(element: Element): boolean {
  const media = element.getAttribute("media")
  // An empty media query list matches every medium (Media Queries 4 §2.1).
  if (media === null || /^[\t\n\f\r ]*$/.test(media)) {
    return true
  }
  return media
    .split(",")
    .map((query) => asciiLowerCase(query.replace(/[\t\n\f\r ]+/g, " ").trim()))
    .some((query) => SCREEN_QUERIES.has(query))
}

/**
 * Whether a `<style>` element applied on the source page: it is a CSS
 * sheet for every screen ({@link isCssStyleElement}, {@link
 * appliesOnScreen}) and not inside a `<noscript>`. The source page ran
 * scripts, so a `<noscript>` there held text, not markup; the inert
 * `DOMParser` runs none and parses its content as elements, so its
 * `<style>` would otherwise style the paste (and step 7 could remove what
 * it hides). A `<template>`'s content is never in the document, so
 * `querySelectorAll` does not find its `<style>`.
 *
 * @param element - a `<style>` element of the clipboard document.
 * @returns `true` when its rules are inlined.
 */
export function isAppliedStyleElement(element: Element): boolean {
  return (
    isCssStyleElement(element) &&
    appliesOnScreen(element) &&
    element.closest("noscript") === null
  )
}

/**
 * The rules of one `<style>` element, from its own `sheet` or, when the
 * engine built none, from a constructed sheet over its text.
 *
 * Chromium, Firefox and WebKit build a sheet for every connected CSS
 * `<style>`, a `DOMParser` document's included. jsdom does not: a document
 * without a window gets no sheets, so under jsdom every `<style>` of a
 * clipboard document has `sheet === null`. A constructed `CSSStyleSheet`
 * (`replaceSync`, which every engine and jsdom implement) parses the same
 * text with the same CSS parser, belongs to no document, so it styles
 * nothing, and loads nothing, as `replaceSync` ignores `@import`. The
 * fallback keeps the step from depending on that difference.
 */
function sheetRules(element: Element): CSSRuleList | undefined {
  const own = (element as HTMLStyleElement).sheet
  if (own !== null && own !== undefined) {
    return own.cssRules
  }
  if (!isCssStyleElement(element) || typeof CSSStyleSheet !== "function") {
    return undefined
  }
  const constructed = new CSSStyleSheet()
  constructed.replaceSync(element.textContent ?? "")
  return constructed.cssRules
}

/**
 * The style rules of one `<style>` element, in source order. A sheet that
 * cannot be read (none, or one that throws) gives none.
 */
function styleRules(element: Element): StyleRule[] {
  let rules: CSSRuleList | undefined
  try {
    rules = sheetRules(element)
  } catch {
    return []
  }
  if (rules === undefined) {
    return []
  }
  const found: StyleRule[] = []
  for (let index = 0; index < rules.length; index += 1) {
    const rule = rules[index] as Partial<CSSStyleRule> | undefined
    // CSSRule.STYLE_RULE is 1; `@page` rules also have a selector text and
    // a style, so the type tells them apart.
    if (
      rule !== undefined &&
      rule.type === 1 &&
      typeof rule.selectorText === "string" &&
      typeof rule.style?.cssText === "string"
    ) {
      found.push(rule as StyleRule)
    }
  }
  return found
}

/** One rule that matched an element, with its place in the cascade. */
interface MatchedRule {
  readonly specificity: Specificity
  readonly order: number
  readonly declarations: string
}

/** A declaration list without its trailing `;` and whitespace. */
function withoutTrailingSemicolons(declarations: string): string {
  return declarations.replace(/[\s;]+$/, "").trim()
}

/**
 * Inlines every `<style>` rule of a parsed clipboard document into the
 * elements of its body, then removes every `<style>` (editor.md §5.2 step
 * 6).
 *
 * @param document - the clipboard HTML parsed by an inert `DOMParser`; its
 *   body is changed in place.
 * @returns how many elements received rules.
 */
export function inlineStyleSheets(document: Document): number {
  const body = document.body
  const styles = [...document.querySelectorAll("style")]
  const matched = new Map<Element, MatchedRule[]>()
  let order = 0
  for (const styleElement of styles) {
    if (!isAppliedStyleElement(styleElement)) {
      continue
    }
    for (const rule of styleRules(styleElement)) {
      const declarations = withoutTrailingSemicolons(rule.style.cssText)
      if (declarations === "") {
        continue
      }
      for (const selector of splitSelectorList(rule.selectorText)) {
        const specificity = selectorSpecificity(selector)
        if (specificity === null) {
          continue
        }
        let elements: Element[]
        try {
          elements = [...body.querySelectorAll(selector)]
        } catch {
          // A selector the engine cannot match: skipped, as CSS skips it.
          continue
        }
        order += 1
        for (const element of elements) {
          const rules = matched.get(element) ?? []
          rules.push({ specificity, order, declarations })
          matched.set(element, rules)
        }
      }
    }
  }
  for (const [element, rules] of matched) {
    rules.sort(
      (a, b) =>
        compareSpecificity(a.specificity, b.specificity) || a.order - b.order
    )
    const own = withoutTrailingSemicolons(element.getAttribute("style") ?? "")
    const all = [...rules.map((rule) => rule.declarations), own].filter(
      (part) => part !== ""
    )
    element.setAttribute("style", all.join("; "))
  }
  for (const styleElement of styles) {
    styleElement.remove()
  }
  return matched.size
}
