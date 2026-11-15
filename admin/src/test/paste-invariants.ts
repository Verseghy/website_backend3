/**
 * Test support for the paste filter: P-paste (editor.md §6, §13.3, §13.5),
 * "the output of `normalizePastedHtml` holds only content-v1 tags,
 * attributes and properties, and no `<style>`", checked against
 * `policy.json` by a checker separate from the filter's steps.
 *
 * {@link pasteViolations} reads the filter's output in an inert `template`
 * and checks every node against `policy.json`: the tag list, each tag's
 * attributes with their value rules (§3.2), each `style` in its canonical
 * spelling (§3.1, §3.3), the URL rules (§3.6), childless iframes (§7.5),
 * and no comment. It names every violation, so a failing test says what the
 * filter let through.
 *
 * It is not independent of the editor: it reads the value rules with the
 * editor's own readers (`canonicalStyle`, the URL rules of `media-src.ts`
 * and `link-href.ts`, the integer reader), which the filter uses too, so a
 * bug in one of them can hide here. The independent check is the server's:
 * the paste fixtures' stored results join the e1 golden files, which the P0
 * oracle and the sanitiser re-check (§13.6, P0, P2, P3).
 */
import { canonicalStyle } from "@/rich-text/css/canonical-style"
import { readIntegerAttribute } from "@/rich-text/extensions/site-lists"
import { isAllowedHref } from "@/rich-text/link-href"
import { embedSrc, imageSrc } from "@/rich-text/media-src"
import { SLICE_MARKER } from "@/rich-text/paste/paste-source"
import type { AttributeValueRule, RichTextPolicy } from "@/rich-text/policy"
import { attributeRuleOf } from "@/rich-text/policy"

/** Whether a value passes its attribute rule exactly as written. */
function valueKept(
  policy: RichTextPolicy,
  tag: string,
  rule: AttributeValueRule,
  value: string
): boolean {
  switch (rule.kind) {
    case "style":
      return canonicalStyle(policy, tag, value) === value
    case "text":
      return true
    case "marker":
      return value === ""
    case "integer": {
      const number = readIntegerAttribute(value, rule)
      return (
        number !== null && number !== rule.omitted && String(number) === value
      )
    }
    case "integer_or_full":
      return (
        value === rule.full ||
        readIntegerAttribute(value, {
          kind: "integer",
          min: rule.min,
          max: rule.max,
          omitted: null,
        })?.toString() === value
      )
    case "enumerated":
      return rule.values.includes(value) && value !== rule.omitted
    case "forced":
    case "link_rel":
      return value === rule.value
    case "image_src":
      return imageSrc(value, policy.urls) === value
    case "embed_src":
      return embedSrc(value, policy.urls) === value
    case "link_href":
      return isAllowedHref(value, policy.urls)
  }
}

/** Options of {@link pasteViolations}. */
export interface PasteViolationOptions {
  /**
   * Allow ProseMirror's `data-pm-slice`, which the filter keeps on the
   * editor's own clipboard (editor.md §5.2), in the shape the filter keeps
   * it: open depths, an optional wrapper count and an empty context
   * ({@link isKeptSliceMarker}).
   */
  readonly sliceMarker?: boolean
}

/**
 * Whether a `data-pm-slice` value has the shape the filter keeps, written
 * here without the filter's reader: up to three digits for each open depth
 * and for the optional wrapper count, then an empty context, `[]`.
 *
 * @param value - the attribute's value in the filter's output.
 */
export function isKeptSliceMarker(value: string): boolean {
  return /^[0-9]{1,3} [0-9]{1,3}(?: -[0-9]{1,3})? \[\]$/.test(value)
}

/**
 * The ways HTML breaks P-paste.
 *
 * @param html - the filter's output.
 * @param policy - the content-v1 policy.
 * @param options - what else is allowed.
 * @returns one line per violation; empty when the HTML holds only
 *   content-v1.
 */
export function pasteViolations(
  html: string,
  policy: RichTextPolicy,
  options: PasteViolationOptions = {}
): string[] {
  const template = document.createElement("template")
  template.innerHTML = html
  const violations: string[] = []
  const pending: Node[] = [...template.content.childNodes]
  while (pending.length > 0) {
    const node = pending.pop()!
    if (node.nodeType === node.COMMENT_NODE) {
      violations.push(`a comment: ${node.textContent}`)
      continue
    }
    if (node.nodeType !== node.ELEMENT_NODE) {
      continue
    }
    const element = node as Element
    const tag = element.localName
    if (!policy.tags.includes(tag)) {
      violations.push(`<${tag}> is not a content-v1 element`)
    } else {
      for (const { name, value } of [...element.attributes]) {
        if (name === SLICE_MARKER && options.sliceMarker === true) {
          if (!isKeptSliceMarker(value)) {
            violations.push(
              `<${tag} ${name}="${value}"> is not a slice marker the filter keeps`
            )
          }
          continue
        }
        const rule = attributeRuleOf(policy, tag, name)
        if (rule === undefined) {
          violations.push(`<${tag} ${name}> is not a content-v1 attribute`)
        } else if (!valueKept(policy, tag, rule, value)) {
          violations.push(
            `<${tag} ${name}="${value}"> is not stored as written`
          )
        }
      }
      if (tag === "iframe" && element.childNodes.length > 0) {
        violations.push("an <iframe> with child nodes")
      }
    }
    pending.push(...element.childNodes)
  }
  return violations
}
