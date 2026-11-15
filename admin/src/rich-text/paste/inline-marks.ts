/**
 * Inline formatting on the generic path: every formatting element of the
 * pasted HTML becomes the content-v1 marks it stands for (editor.md §5.2
 * steps 9 and 10, §3.2).
 *
 * content-v1 has `strong`, `em`, `u`, `s`, `sub`, `sup` and a `span` that
 * carries a colour, a highlight and an `em` size (§3.3); `b`, `i`, `ins`,
 * `strike`, `del` and `font` are not in it, and a `span`'s
 * `font-weight` or `font-style` is not either. So each formatting element
 * (`FORMATTING_TAGS`) is replaced by exactly the marks it has:
 *
 * - **its own tag counts as its lowest-precedence declaration** (§5.2 step
 *   9): `b` and `strong` are bold, `i` and `em` italic, `u` and `ins`
 *   underlined, `s`, `strike` and `del` struck through, `sub` and `sup` a
 *   script;
 * - **its own declaration overrides its tag**: `font-weight` (600 or more,
 *   `bold` or `bolder` is bold; anything lower, `normal` or `lighter` is
 *   not), `font-style` (`italic` or `oblique`), `text-decoration-line`
 *   (`underline`, `line-through`; any other value, `none` included, sets
 *   neither) and `vertical-align` (`super`, `sub`; `baseline` neither). So
 *   Google Docs' `<b style="font-weight:normal" id="docs-internal-guid-…">`
 *   around a whole document is not bold, and its
 *   `<span style="font-weight:700">` runs are;
 * - **its colour, highlight and size** go onto one `span` through
 *   `canonicalStyle` (a size only as `em` within 0.5 to 3), and a black text
 *   colour and a white or transparent highlight are dropped, which every
 *   Office and Google run carries (§3.3 `span` `color`, §5.2 step 9).
 *   `font[color]` counts as a colour under the element's own style.
 *
 * This is the generic path's reading, element by element. §5.2 step 9
 * resolves marks per text run instead: a run's effective weight, style,
 * decoration, script, colour and size from its whole chain of ancestors up
 * to its block, so that `<b><span style="font-weight:400">x</span></b>` is
 * not bold and a font size is taken relative to the source's base size.
 * That needs the Word and Excel base rules and the captures, and the Word
 * runs slice replaces this module's element-by-element rule with it.
 *
 * The editor's own clipboard carries content-v1 marks, which come back
 * unchanged: a `strong` stays a `strong`, and a `span`'s canonical style is
 * kept as written, black colour included.
 */
import { canonicalStyle } from "../css/canonical-style"
import { asciiLowerCase } from "../css/declarations"
import type { RichTextPolicy } from "../policy"
import { readStoredDeclarations } from "../style-decls"
import { writeDeclarations } from "./paste-declarations"
import type { PasteSource } from "./paste-source"

/** The inline formatting elements that become marks. */
export const FORMATTING_TAGS: ReadonlySet<string> = new Set([
  "span",
  "font",
  "b",
  "strong",
  "i",
  "em",
  "u",
  "ins",
  "s",
  "strike",
  "del",
  "sub",
  "sup",
])

/** The `span` properties of content-v1 (editor.md §3.3). */
const SPAN_PROPERTIES = ["color", "background-color", "font-size"] as const

/** The marks of one formatting element. */
export interface InlineMarks {
  /** `strong`. */
  readonly bold: boolean
  /** `em`. */
  readonly italic: boolean
  /** `u`. */
  readonly underline: boolean
  /** `s`. */
  readonly strike: boolean
  /** `sub` or `sup`, or `null`. */
  readonly script: "sub" | "sup" | null
}

/**
 * Reads one cascaded declaration of an element.
 *
 * @param property - the property name, lower case.
 * @returns its value, or `""` when the element does not declare it.
 */
export type DeclarationReader = (property: string) => string

/** A keyword value: trimmed and lower case. */
const keyword = (value: string) => asciiLowerCase(value.trim())

/** Bold from a `font-weight` value, or `null` when there is none. */
function weightIsBold(value: string): boolean | null {
  const weight = keyword(value)
  if (weight === "") {
    return null
  }
  if (weight === "bold" || weight === "bolder") {
    return true
  }
  const number = Number(weight)
  return Number.isFinite(number) && /^[0-9.]+$/.test(weight)
    ? number >= 600
    : false
}

/**
 * The marks a formatting element stands for: its tag's, overridden by its
 * own declarations.
 *
 * @param tag - the element's tag name, lower case.
 * @param read - its cascaded declarations ({@link DeclarationReader}).
 * @returns the marks.
 */
export function ownMarks(tag: string, read: DeclarationReader): InlineMarks {
  let bold = tag === "b" || tag === "strong"
  let italic = tag === "i" || tag === "em"
  let underline = tag === "u" || tag === "ins"
  let strike = tag === "s" || tag === "strike" || tag === "del"
  let script: "sub" | "sup" | null = tag === "sub" || tag === "sup" ? tag : null
  const weight = weightIsBold(read("font-weight"))
  if (weight !== null) {
    bold = weight
  }
  const style = keyword(read("font-style"))
  if (style !== "") {
    italic = style === "italic" || style.startsWith("oblique")
  }
  // Engines without the longhand in their CSSOM give it through the shorthand.
  const decoration = keyword(
    read("text-decoration-line") || read("text-decoration")
  )
  if (decoration !== "") {
    const lines = decoration.split(/\s+/)
    underline = lines.includes("underline")
    strike = lines.includes("line-through")
  }
  const vertical = keyword(read("vertical-align"))
  if (vertical === "super" || vertical === "sub") {
    script = vertical === "super" ? "sup" : "sub"
  } else if (vertical === "baseline") {
    script = null
  }
  return { bold, italic, underline, strike, script }
}

/**
 * The style of the `span` a formatting element becomes, or `null` for no
 * `span`.
 *
 * @param policy - the content-v1 policy.
 * @param source - the paste's source: the editor's own clipboard keeps its
 *   canonical style as written.
 * @param element - the formatting element.
 * @param read - its cascaded declarations.
 * @returns the canonical `span` style without black text and without a
 *   white or transparent highlight (other sources), or `null`.
 */
export function spanStyle(
  policy: RichTextPolicy,
  source: PasteSource,
  element: Element,
  read: DeclarationReader
): string | null {
  if (source === "internal") {
    return canonicalStyle(policy, "span", element.getAttribute("style") ?? "")
  }
  const colourHint =
    element.localName === "font" ? element.getAttribute("color") : null
  const declarations = [
    colourHint === null ? null : `color: ${colourHint}`,
    ...SPAN_PROPERTIES.map((property) => {
      const value = read(property).trim()
      return value === "" ? null : `${property}: ${value}`
    }),
  ].filter((part) => part !== null)
  const stored = new Map(
    readStoredDeclarations(policy, "span", declarations.join("; "))
  )
  if (stored.get("color") === "#000000") {
    stored.delete("color")
  }
  // `transparent` stays a keyword in canonical form, but an engine may
  // serialise it as `rgba(0, 0, 0, 0)`, which is `#00000000` there.
  const highlight = stored.get("background-color")
  if (
    highlight === "transparent" ||
    highlight === "#ffffff" ||
    highlight === "#ffffffff" ||
    /^#[0-9a-f]{6}00$/.test(highlight ?? "")
  ) {
    stored.delete("background-color")
  }
  return stored.size === 0 ? null : writeDeclarations(policy, "span", stored)
}

/**
 * Replaces a formatting element by the content-v1 marks it stands for,
 * around its children; an element with no marks and no `span` style is
 * unwrapped.
 *
 * An element that already is the one wrapper it needs (a `span` that keeps
 * only its style, a `strong` that stays bold and nothing else) is rewritten
 * in place: its attributes go, the `span` gets its canonical style, and its
 * children stay where they are. Moving them would move their whole subtree
 * (`normalizeStructure`, "Cost"), which nested runs such as
 * `<span style="color: red">` inside one another made quadratic.
 *
 * @param element - the formatting element; it leaves the document, unless
 *   it is rewritten in place.
 * @param marks - its marks ({@link ownMarks}).
 * @param style - its `span` style ({@link spanStyle}), or `null`.
 */
export function replaceWithMarks(
  element: Element,
  marks: InlineMarks,
  style: string | null
): void {
  const document = element.ownerDocument
  const tags: string[] = []
  if (style !== null) {
    tags.push("span")
  }
  const markTags: [boolean, string][] = [
    [marks.bold, "strong"],
    [marks.italic, "em"],
    [marks.underline, "u"],
    [marks.strike, "s"],
    [marks.script !== null, marks.script ?? "sup"],
  ]
  for (const [present, tag] of markTags) {
    if (present) {
      tags.push(tag)
    }
  }
  if (tags.length === 1 && tags[0] === element.localName) {
    for (const attribute of [...element.attributes]) {
      element.removeAttributeNode(attribute)
    }
    if (style !== null) {
      element.setAttribute("style", style)
    }
    return
  }
  const children = [...element.childNodes]
  if (tags.length === 0) {
    element.replaceWith(...children)
    return
  }
  const wrappers = tags.map((tag) => document.createElement(tag))
  if (style !== null) {
    wrappers[0].setAttribute("style", style)
  }
  for (let index = 1; index < wrappers.length; index += 1) {
    wrappers[index - 1].append(wrappers[index])
  }
  wrappers[wrappers.length - 1].append(...children)
  element.replaceWith(wrappers[0])
}
