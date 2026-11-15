/**
 * Text colour, highlight and font size on `span` (editor.md §4.3 row 4,
 * §3.3; research: DECISIONS 15c).
 *
 * These are the TextStyleKit members of `@tiptap/extension-text-style`
 * 3.32 that content-v1 keeps: TextStyle (the `span` mark) with Color,
 * BackgroundColor and FontSize as its attributes. FontFamily and LineHeight
 * are left out, because content-v1 drops both (§3.4). The members are
 * listed one by one instead of through `TextStyleKit.configure`, because the
 * kit configures only stock members and every one of these is changed:
 *
 * - **Parse.** Each attribute reads its declaration through the raw style
 *   reader and keeps it only in the policy's grammar and spelling: colours as
 *   lower-case hex (`color` opaque `#rrggbb` only, `background-color` also
 *   `#rrggbbaa`, `transparent` and `currentcolor`), and `font-size` in `em`
 *   from 0.5 to 3. Stock keeps any value that is not empty, quotes stripped,
 *   so `var(--mt-accent-…)` (the kit's palette) or `12pt` would be stored.
 * - **Render.** An attribute renders only a value its grammar accepts, so a
 *   command given another value (`setColor("red")`) writes nothing the
 *   server would remove.
 * - **The mark.** A `span` becomes a TextStyle mark only when it carries at
 *   least one stored declaration. Stock accepts any `style`, which makes a
 *   bare mark of a `font-family` span; a bare `span` does not survive a
 *   reload (editor.md §6 step 2.2).
 * - **Nested spans.** A nested `span` inherits the declarations of the spans
 *   around it, and its own declarations win ({@link readSpanDeclarations}).
 *   Marks of one type replace each other, so without that the inner run
 *   would lose the outer colour when it has its own size. A run never
 *   carries two TextStyle marks, so the stored HTML never nests spans and a
 *   font size cannot compound (§3.3). Stock's `mergeNestedSpanStyles` gives
 *   the same inheritance by writing `<outer style>;<own style>` into each
 *   child span's `style` attribute before the child is parsed, so a span
 *   nested d levels deep holds every declaration around it. Past the
 *   reader's 32 declarations (`MAX_DECLARATIONS`, editor.md §7.3) its own
 *   ones are no longer read, and the inner run would take an outer colour
 *   that the server, which reads each `style` attribute on its own, does
 *   not store there. So stock's option is off and the inheritance is
 *   computed here from each attribute read on its own, which also leaves
 *   the parsed DOM unchanged.
 */
import type { Attribute } from "@tiptap/core"
import type { ParseRule, TagParseRule } from "@tiptap/pm/model"
import {
  BackgroundColor,
  Color,
  FontSize,
  TextStyle,
} from "@tiptap/extension-text-style"

import type { RichTextPolicy } from "../policy"
import { requireStyleGrammar } from "../policy"
import type { ScalarGrammar } from "../style-decls"
import {
  isCanonicalValue,
  keptDeclarations,
  resolveStoredDeclarations,
} from "../style-decls"

/** The mark the three attributes belong to. */
const TEXT_STYLE = "textStyle"

/**
 * The stored `span` declarations that apply to a parsed `span`: its own,
 * over those of the spans around it.
 *
 * Each `style` attribute is resolved on its own with
 * `resolveStoredDeclarations`, and the resolutions are folded from the
 * outermost span inwards, so for each property the innermost span that sets
 * it decides. A property whose winning declaration there is refused (`null`)
 * stays refused: an outer value never stands in for an inner declaration
 * the server would store differently. This is the result stock's
 * `mergeNestedSpanStyles` gives for spans of a few levels, without its
 * growing style strings.
 *
 * The walk stops below an element with a `contenteditable` attribute: when
 * ProseMirror re-reads the editor's own DOM, the spans above the editor's
 * root are the application's, not the document's.
 *
 * Its cost is linear in the nesting depth, which the server bounds (256,
 * editor.md §7.2) for every stored body.
 *
 * @param policy - the content-v1 policy.
 * @param element - the `span` a parse rule matched.
 * @returns the kept declarations by property.
 */
export function readSpanDeclarations(
  policy: RichTextPolicy,
  element: Element
): Map<string, string> {
  const spans: Element[] = []
  for (
    let current: Element | null = element;
    current !== null && !current.hasAttribute("contenteditable");
    current = current.parentElement
  ) {
    if (current.tagName === "SPAN") {
      spans.push(current)
    }
  }
  const folded = new Map<string, string | null>()
  for (const span of spans.reverse()) {
    for (const [property, value] of resolveStoredDeclarations(
      policy,
      "span",
      span.getAttribute("style")
    )) {
      folded.set(property, value)
    }
  }
  return keptDeclarations(folded)
}

/**
 * One `span` declaration as a TextStyle attribute.
 *
 * @param policy - the content-v1 policy.
 * @param attribute - the attribute name on the mark (`color`, ...).
 * @param property - the CSS property it stores (`color`, ...).
 * @param grammar - the property's grammar on `span`.
 * @returns the attribute specification.
 */
function spanDeclaration(
  policy: RichTextPolicy,
  attribute: string,
  property: string,
  grammar: ScalarGrammar
): Attribute {
  return {
    default: null,
    parseHTML: (element) =>
      readSpanDeclarations(policy, element).get(property) ?? null,
    renderHTML: (attributes) => {
      const value: unknown = attributes[attribute]
      return typeof value === "string" && isCanonicalValue(grammar, value)
        ? { style: `${property}: ${value}` }
        : {}
    },
  }
}

/**
 * The TextStyle mark and its three attributes.
 *
 * @param policy - the content-v1 policy; supplies the `span` grammars.
 * @returns the extensions, mark first.
 * @throws {PolicyError} when `span` does not store `color` and
 *   `background-color` as colours and `font-size` in `em`.
 */
export function createSiteTextStyle(policy: RichTextPolicy) {
  const grammars = {
    color: requireStyleGrammar(policy, "span", "color", "colour"),
    backgroundColor: requireStyleGrammar(
      policy,
      "span",
      "background-color",
      "colour"
    ),
    fontSize: requireStyleGrammar(policy, "span", "font-size", "em"),
  }
  const textStyle = TextStyle.extend({
    parseHTML() {
      return (this.parent?.() ?? []).map((rule): ParseRule => {
        // Stock 3.32 has one rule, on the `span` tag; a style rule would be
        // kept as it is.
        if (!("tag" in rule) || rule.tag === undefined) {
          return rule
        }
        const tagRule: TagParseRule = rule
        return {
          ...tagRule,
          getAttrs: (element: HTMLElement) => {
            // The stock rule runs first: it refuses a span without a style.
            // With mergeNestedSpanStyles off it changes no element.
            const stock = tagRule.getAttrs ? tagRule.getAttrs(element) : null
            if (stock === false) {
              return false
            }
            return readSpanDeclarations(policy, element).size > 0
              ? stock
              : false
          },
        }
      })
    },
  }).configure({ mergeNestedSpanStyles: false })

  const attributeOf = (name: keyof typeof grammars, property: string) => [
    {
      types: [TEXT_STYLE],
      attributes: {
        [name]: spanDeclaration(policy, name, property, grammars[name]),
      },
    },
  ]
  return [
    textStyle,
    Color.extend({
      addGlobalAttributes: () => attributeOf("color", "color"),
    }),
    BackgroundColor.extend({
      addGlobalAttributes: () =>
        attributeOf("backgroundColor", "background-color"),
    }),
    FontSize.extend({
      addGlobalAttributes: () => attributeOf("fontSize", "font-size"),
    }),
  ]
}
