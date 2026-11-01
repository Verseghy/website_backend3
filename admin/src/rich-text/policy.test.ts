// @vitest-environment node
import fc from "fast-check"
import { describe, expect, it } from "vitest"

import policyJson from "./policy.json"
import {
  attributeRuleOf,
  CONTENT_V1_POLICY,
  elementOf,
  POLICY_VERSION,
  PolicyError,
  readPolicy,
  requireAttributeRule,
  requireStyleGrammar,
  styleRuleOf,
} from "./policy"
import type { Json } from "@/test/rich-text-policy"
import { policyJsonCopy, policyWith } from "@/test/rich-text-policy"

type JsonObject = { [key: string]: Json }

/** The object at a dotted path of the JSON copy. */
function at(json: JsonObject, path: string): JsonObject {
  return path
    .split(".")
    .reduce<Json>(
      (value, key) => (value as JsonObject)[key],
      json
    ) as JsonObject
}

describe("CONTENT_V1_POLICY", () => {
  it("is the committed policy.json, read without loss", () => {
    const { $comment, ...members } = policyJson as Record<string, unknown>
    expect(typeof $comment).toBe("string")
    expect(CONTENT_V1_POLICY).toEqual(members)
    expect(CONTENT_V1_POLICY.policy_version).toBe(POLICY_VERSION)
  })

  it("lists the content-v1 elements of editor.md §3.2 in order", () => {
    expect(CONTENT_V1_POLICY.tags).toEqual(
      "p h1 h2 h3 h4 strong em u s sub sup br hr blockquote li tr tbody span ul ol a img table td th iframe".split(
        " "
      )
    )
    expect(Object.keys(CONTENT_V1_POLICY.elements)).toEqual(
      CONTENT_V1_POLICY.tags
    )
  })
})

describe("readPolicy", () => {
  it.each<[string, (json: JsonObject) => void, string]>([
    [
      "a newer policy version",
      (json) => {
        json.policy_version = "content-v2"
      },
      'policy_version is "content-v2", but this admin implements content-v1',
    ],
    [
      "a forbidden tag, in any case",
      (json) => {
        ;(json.tags as Json[]).push("Script")
        at(json, "elements").Script = { attributes: [], style: null }
      },
      'tags holds the forbidden tag "Script"',
    ],
    [
      "elements out of tag order",
      (json) => {
        const { p, ...rest } = at(json, "elements")
        json.elements = { ...rest, p }
      },
      "elements must have one entry per tag, in tag order",
    ],
    [
      "a tag without an element entry",
      (json) => {
        delete at(json, "elements").iframe
      },
      "elements must have one entry per tag",
    ],
    [
      "an unknown attribute rule",
      (json) => {
        at(json, "elements.a.attributes.0.value").kind = "url"
      },
      'elements.a.attributes[0].value.kind is "url"; expected one of style, text',
    ],
    [
      "an unknown grammar",
      (json) => {
        at(json, "elements.span.style.3.grammar").kind = "rem"
      },
      'elements.span.style[3].grammar.kind is "rem"; expected one of keyword',
    ],
    [
      "a range whose min is above its max",
      (json) => {
        at(json, "elements.span.style.3.grammar").min = 4
      },
      "elements.span.style[3].grammar has min 4 above max 3",
    ],
    [
      "a negative bound",
      (json) => {
        at(json, "elements.ol.attributes.0.value").min = -1
      },
      "elements.ol.attributes[0].value.min must be a non-negative number, not -1",
    ],
    [
      "a style list without a style attribute",
      (json) => {
        at(json, "elements.strong").style = []
      },
      "elements.strong.style must be null for an element without a style attribute",
    ],
    [
      "a style attribute without a style list",
      (json) => {
        at(json, "elements.p").style = null
      },
      "elements.p.style must list the CSS properties of an element with a style attribute",
    ],
    [
      "an unknown provisional group",
      (json) => {
        at(json, "elements.td.style.0").provisional = "word_borders"
      },
      'elements.td.style[0].provisional is "word_borders"',
    ],
    [
      "an embed query value of another kind",
      (json) => {
        at(json, "urls.embed.providers.0.query.0").value = "number"
      },
      'urls.embed.providers[0].query[0].value is "number"; expected one of digits, id',
    ],
    [
      "a border keyword width that is not a number",
      (json) => {
        at(json, "css.border.width_keywords").thin = "1px"
      },
      'css.border.width_keywords.thin must be a non-negative number, not "1px"',
    ],
    [
      "a tag that is not a string",
      (json) => {
        ;(json.tags as Json[])[0] = 1
      },
      "tags[0] must be a string, not 1",
    ],
  ])("refuses %s, naming the member", (_name, change, message) => {
    const json = policyJsonCopy()
    change(json)
    expect(() => readPolicy(json)).toThrow(PolicyError)
    expect(() => readPolicy(json)).toThrow(message)
  })

  it("names the command that regenerates the file", () => {
    expect(() => readPolicy([])).toThrow(
      "policy.json: the file must be an object, not []. Regenerate it from the repository root with `cargo loco task rich_text_manifest`"
    )
  })

  /** Every member of an object in the manifest, as a path of keys. */
  function memberPaths(value: Json, path: string[] = []): string[][] {
    if (value === null || typeof value !== "object") {
      return []
    }
    const entries: [string, Json][] = Array.isArray(value)
      ? value.map((item, index) => [String(index), item])
      : Object.entries(value)
    return entries.flatMap(([key, item]) => {
      const here = [...path, key]
      const own = Array.isArray(value) ? [] : [here]
      return [...own, ...memberPaths(item, here)]
    })
  }

  // Members whose absence is a valid manifest: the comment is not read, and
  // the keyword widths are a map whose entries are data, not members.
  const optional = (path: string[]) =>
    path[0] === "$comment" ||
    path.slice(0, 3).join(".") === "css.border.width_keywords"
  const required = memberPaths(policyJsonCopy()).filter(
    (path) => !optional(path)
  )

  it("refuses the manifest without any one of its members", () => {
    fc.assert(
      fc.property(fc.constantFrom(...required), (path) => {
        const json = policyJsonCopy()
        const parent = path
          .slice(0, -1)
          .reduce<Json>(
            (value, key) => (value as JsonObject)[key],
            json
          ) as JsonObject
        delete parent[path[path.length - 1]]
        expect(() => readPolicy(json)).toThrow(PolicyError)
      }),
      { numRuns: 300 }
    )
  })

  it("reads any change of the comment as the same policy", () => {
    fc.assert(
      fc.property(fc.string(), (comment) => {
        const json = policyJsonCopy()
        json.$comment = comment
        expect(readPolicy(json)).toEqual(CONTENT_V1_POLICY)
      })
    )
  })
})

describe("elementOf, styleRuleOf and attributeRuleOf", () => {
  it("find a rule by element and name", () => {
    expect(
      styleRuleOf(CONTENT_V1_POLICY, "ul", "list-style-type")?.grammar
    ).toEqual({
      kind: "keyword",
      values: ["circle", "square"],
      omitted: "disc",
    })
    expect(attributeRuleOf(CONTENT_V1_POLICY, "a", "rel")).toEqual({
      kind: "link_rel",
      value: "noopener noreferrer",
    })
  })

  it.each([
    ["an element without a style", "strong", "color"],
    ["a property the element lacks", "p", "color"],
    ["an element outside content-v1", "div", "text-align"],
    ["a name of the object prototype", "constructor", "style"],
  ])("answer undefined for %s", (_name, tag, name) => {
    expect(elementOf(CONTENT_V1_POLICY, tag) === undefined).toBe(
      tag !== "strong" && tag !== "p"
    )
    expect(styleRuleOf(CONTENT_V1_POLICY, tag, name)).toBeUndefined()
    expect(attributeRuleOf(CONTENT_V1_POLICY, tag, name)).toBeUndefined()
  })
})

describe("requireStyleGrammar and requireAttributeRule", () => {
  it("return the rule of the expected kind", () => {
    expect(
      requireStyleGrammar(CONTENT_V1_POLICY, "span", "font-size", "em")
    ).toEqual({ kind: "em", min: 0.5, max: 3 })
    expect(
      requireAttributeRule(CONTENT_V1_POLICY, "a", "target", "forced")
    ).toEqual({ kind: "forced", value: "_blank" })
  })

  it.each<[string, () => unknown, string]>([
    [
      "a property of another grammar",
      () => requireStyleGrammar(CONTENT_V1_POLICY, "span", "font-size", "px"),
      "The policy has no stored font-size on span with a px grammar",
    ],
    [
      "an input-only shorthand",
      () =>
        requireStyleGrammar(CONTENT_V1_POLICY, "span", "background", "colour"),
      "The policy has no stored background on span with a colour grammar",
    ],
    [
      "a missing property",
      () => requireStyleGrammar(CONTENT_V1_POLICY, "p", "color", "colour"),
      "found null",
    ],
    [
      "an attribute of another rule",
      () =>
        requireAttributeRule(CONTENT_V1_POLICY, "ol", "start", "enumerated"),
      "The policy has no start attribute on ol with a enumerated rule",
    ],
    [
      "a changed policy",
      () =>
        requireAttributeRule(
          policyWith((json) => {
            const a = (json.elements as JsonObject).a as JsonObject
            a.attributes = (a.attributes as Json[]).slice(0, 1)
          }),
          "a",
          "target",
          "forced"
        ),
      "The policy has no target attribute on a with a forced rule",
    ],
  ])("refuse %s", (_name, call, message) => {
    expect(call).toThrow(PolicyError)
    expect(call).toThrow(message)
  })
})
