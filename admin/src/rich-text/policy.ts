/**
 * content-v1 as typed data: the admin's reader of `policy.json` (editor.md
 * §3.9).
 *
 * `policy.json` is written by `cargo loco task rich_text_manifest` from the
 * Rust policy and committed beside this file. It is the one source of the
 * dialect on the client: the TipTap schema (`site-extensions.ts`), and later
 * the paste filter and `canonicalStyle`, read their tag, attribute, CSS and
 * URL rules from it instead of restating them, so the editor and the server
 * cannot drift apart silently (ADR 0013 §1).
 *
 * The file is a boundary, so it is validated member by member when this
 * module loads. A manifest whose shape the admin does not understand (a new
 * value kind, a renamed member, a newer `POLICY_VERSION`) fails every test
 * that imports it, with the path of the member and what was expected,
 * instead of reaching an editor as `undefined`. The types keep the file's own
 * member names (`policy_version`, `opaque_only`, ...), so a reader can match
 * them with editor.md §3.9 and the Rust manifest without a mapping table.
 *
 * Code that applies the policy takes it as an argument (editor.md §1.2 D6);
 * {@link CONTENT_V1_POLICY} is the committed copy that callers pass.
 */
import policyJson from "./policy.json"

/** The dialect this admin implements (editor.md §3, `POLICY_VERSION`). */
export const POLICY_VERSION = "content-v1"

/** How an attribute's value is checked (editor.md §3.2, §3.9). */
export type AttributeValueRule =
  | { readonly kind: "style" }
  | { readonly kind: "text" }
  | { readonly kind: "marker" }
  | {
      readonly kind: "integer"
      readonly min: number
      readonly max: number
      readonly omitted: number | null
    }
  | {
      readonly kind: "integer_or_full"
      readonly min: number
      readonly max: number
      readonly full: string
    }
  | {
      readonly kind: "enumerated"
      readonly values: readonly string[]
      readonly omitted: string | null
    }
  | { readonly kind: "forced"; readonly value: string }
  | { readonly kind: "link_rel"; readonly value: string }
  | { readonly kind: "image_src" }
  | { readonly kind: "embed_src" }
  | { readonly kind: "link_href" }

/** One attribute of an element, in canonical order (editor.md §3.2). */
export interface AttributePolicy {
  /** The attribute name. */
  readonly name: string
  /** Its value rule. */
  readonly value: AttributeValueRule
}

/** The value grammar of one CSS property (editor.md §3.3, §3.9). */
export type StyleGrammar =
  | {
      readonly kind: "keyword"
      readonly values: readonly string[]
      readonly omitted: string | null
    }
  | {
      readonly kind: "px" | "auto_or_px" | "em"
      readonly min: number
      readonly max: number
    }
  | { readonly kind: "colour"; readonly opaque_only: boolean }
  | {
      readonly kind:
        "border_side" | "border_widths" | "border_styles" | "border_colours"
    }
  | { readonly kind: "locked"; readonly value: string }

/** The grammar kinds, as `StyleGrammar["kind"]` lists them. */
export type StyleGrammarKind = StyleGrammar["kind"]

/** One CSS property an element's `style` may carry (editor.md §3.3). */
export interface StyleRule {
  /** The property name, lower case. */
  readonly property: string
  /** Its value grammar. */
  readonly grammar: StyleGrammar
  /** `stored`, or `input_only` for a shorthand that is expanded, never stored. */
  readonly role: "stored" | "input_only"
  /** When the declaration is present in a stored style. */
  readonly presence: "optional" | "always" | "when_sized"
  /** The §3.3 group that waits for the Windows captures, or `null`. */
  readonly provisional:
    | "excel_border_weights"
    | "excel_general_alignment"
    | "excel_vertical_alignment"
    | null
}

/** The rules of one element. */
export interface ElementPolicy {
  /** The attributes it may carry, in canonical order. */
  readonly attributes: readonly AttributePolicy[]
  /** Its CSS properties, or `null` for an element without a `style`. */
  readonly style: readonly StyleRule[] | null
}

/** The `{width, style, colour}` parts of one cell border side. */
export interface BorderParts {
  /** The width in px. */
  readonly width: number
  /** The border style keyword. */
  readonly style: string
  /** The colour. */
  readonly colour: string
}

/** One embed provider (editor.md §3.6, `iframe[src]`). */
export interface EmbedProvider {
  /** The exact hosts it may use. */
  readonly hosts: readonly string[]
  /** The path before the embed id. */
  readonly path_prefix: string
  /** The query parameters it may carry, in the order they must appear. */
  readonly query: readonly {
    readonly name: string
    readonly value: "digits" | "id"
  }[]
}

/** The URL rules (editor.md §3.6). */
export interface UrlPolicy {
  /** The schemes a URL attribute may use. */
  readonly schemes: readonly string[]
  /** backend3's own hosts. */
  readonly own_hosts: readonly string[]
  /** The `/storage/` prefix. */
  readonly storage_path_prefix: string
  /** The `img[src]` path prefix and extensions. */
  readonly image: {
    readonly path_prefix: string
    readonly extensions: readonly string[]
  }
  /** The extensions a link into `/storage/` on an own host must not end in. */
  readonly link: { readonly forbidden_storage_extensions: readonly string[] }
  /** The embed scheme, id characters and providers. */
  readonly embed: {
    readonly scheme: string
    readonly id_alphabet: string
    readonly providers: readonly EmbedProvider[]
  }
}

/** The whole manifest, member by member as editor.md §3.9 lists it. */
export interface RichTextPolicy {
  /** Always {@link POLICY_VERSION}. */
  readonly policy_version: string
  /** The content-v1 elements, in §3.2 order. */
  readonly tags: readonly string[]
  /** One entry per tag, in the same order. */
  readonly elements: Readonly<Record<string, ElementPolicy>>
  /** Elements removed together with their content (§3.5, first row). */
  readonly removed_with_content: readonly string[]
  /** Names ammonia's namespace check removes with their content (§3.5). */
  readonly removed_by_namespace_check: readonly string[]
  /** Tags that may never enter the tag list (§3.5). */
  readonly forbidden_tags: readonly string[]
  /** The style alphabet and the cell border constants. */
  readonly css: {
    readonly alphabet: string
    readonly border: {
      readonly styles: readonly string[]
      readonly width_keywords: Readonly<Record<string, number>>
      readonly initial: BorderParts
      readonly unset: BorderParts
      readonly max_width: number
    }
  }
  /** The URL rules. */
  readonly urls: UrlPolicy
}

/** The error a malformed manifest raises. */
export class PolicyError extends Error {
  override name = "PolicyError"
}

const FIX =
  "Regenerate it from the repository root with `cargo loco task rich_text_manifest` " +
  "(editor.md §3.9); if the manifest's shape changed on purpose, update admin/src/rich-text/policy.ts with it."

/** Raises a {@link PolicyError} that names the member and the fix. */
function fail(path: string, problem: string): never {
  throw new PolicyError(`policy.json: ${path} ${problem}. ${FIX}`)
}

/** Shows a value in an error message, shortened. */
function show(value: unknown): string {
  const text = JSON.stringify(value) ?? String(value)
  return text.length > 60 ? `${text.slice(0, 57)}...` : text
}

function object(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    fail(path, `must be an object, not ${show(value)}`)
  }
  return value as Record<string, unknown>
}

function string(value: unknown, path: string): string {
  if (typeof value !== "string") {
    fail(path, `must be a string, not ${show(value)}`)
  }
  return value
}

function nonNegative(value: unknown, path: string): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    fail(path, `must be a non-negative number, not ${show(value)}`)
  }
  return value
}

function boolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    fail(path, `must be true or false, not ${show(value)}`)
  }
  return value
}

function array<T>(
  value: unknown,
  path: string,
  item: (value: unknown, path: string) => T
): readonly T[] {
  if (!Array.isArray(value)) {
    fail(path, `must be an array, not ${show(value)}`)
  }
  return value.map((entry, index) => item(entry, `${path}[${index}]`))
}

function strings(value: unknown, path: string): readonly string[] {
  return array(value, path, string)
}

function nullable<T>(
  value: unknown,
  path: string,
  item: (value: unknown, path: string) => T
): T | null {
  return value === null ? null : item(value, path)
}

function oneOf<const T extends string>(
  value: unknown,
  path: string,
  allowed: readonly T[]
): T {
  if (
    typeof value !== "string" ||
    !(allowed as readonly string[]).includes(value)
  ) {
    fail(path, `is ${show(value)}; expected one of ${allowed.join(", ")}`)
  }
  return value as T
}

/** A min/max pair whose min is not above its max. */
function range(
  member: Record<string, unknown>,
  path: string
): { min: number; max: number } {
  const min = nonNegative(member.min, `${path}.min`)
  const max = nonNegative(member.max, `${path}.max`)
  if (min > max) {
    fail(path, `has min ${min} above max ${max}`)
  }
  return { min, max }
}

function attributeValue(value: unknown, path: string): AttributeValueRule {
  const rule = object(value, path)
  const kind = oneOf(rule.kind, `${path}.kind`, [
    "style",
    "text",
    "marker",
    "integer",
    "integer_or_full",
    "enumerated",
    "forced",
    "link_rel",
    "image_src",
    "embed_src",
    "link_href",
  ])
  switch (kind) {
    case "integer":
      return {
        kind,
        ...range(rule, path),
        omitted: nullable(rule.omitted, `${path}.omitted`, nonNegative),
      }
    case "integer_or_full":
      return {
        kind,
        ...range(rule, path),
        full: string(rule.full, `${path}.full`),
      }
    case "enumerated":
      return {
        kind,
        values: strings(rule.values, `${path}.values`),
        omitted: nullable(rule.omitted, `${path}.omitted`, string),
      }
    case "forced":
    case "link_rel":
      return { kind, value: string(rule.value, `${path}.value`) }
    default:
      return { kind }
  }
}

function attribute(value: unknown, path: string): AttributePolicy {
  const entry = object(value, path)
  return {
    name: string(entry.name, `${path}.name`),
    value: attributeValue(entry.value, `${path}.value`),
  }
}

function grammar(value: unknown, path: string): StyleGrammar {
  const rule = object(value, path)
  const kind = oneOf(rule.kind, `${path}.kind`, [
    "keyword",
    "px",
    "auto_or_px",
    "em",
    "colour",
    "border_side",
    "border_widths",
    "border_styles",
    "border_colours",
    "locked",
  ])
  switch (kind) {
    case "keyword":
      return {
        kind,
        values: strings(rule.values, `${path}.values`),
        omitted: nullable(rule.omitted, `${path}.omitted`, string),
      }
    case "px":
    case "auto_or_px":
    case "em":
      return { kind, ...range(rule, path) }
    case "colour":
      return {
        kind,
        opaque_only: boolean(rule.opaque_only, `${path}.opaque_only`),
      }
    case "locked":
      return { kind, value: string(rule.value, `${path}.value`) }
    default:
      return { kind }
  }
}

function styleRule(value: unknown, path: string): StyleRule {
  const rule = object(value, path)
  return {
    property: string(rule.property, `${path}.property`),
    grammar: grammar(rule.grammar, `${path}.grammar`),
    role: oneOf(rule.role, `${path}.role`, ["stored", "input_only"]),
    presence: oneOf(rule.presence, `${path}.presence`, [
      "optional",
      "always",
      "when_sized",
    ]),
    provisional: nullable(rule.provisional, `${path}.provisional`, (v, p) =>
      oneOf(v, p, [
        "excel_border_weights",
        "excel_general_alignment",
        "excel_vertical_alignment",
      ])
    ),
  }
}

function element(value: unknown, path: string): ElementPolicy {
  const entry = object(value, path)
  return {
    attributes: array(entry.attributes, `${path}.attributes`, attribute),
    style: nullable(entry.style, `${path}.style`, (v, p) =>
      array(v, p, styleRule)
    ),
  }
}

function borderParts(value: unknown, path: string): BorderParts {
  const parts = object(value, path)
  return {
    width: nonNegative(parts.width, `${path}.width`),
    style: string(parts.style, `${path}.style`),
    colour: string(parts.colour, `${path}.colour`),
  }
}

function urls(value: unknown, path: string): UrlPolicy {
  const member = object(value, path)
  const image = object(member.image, `${path}.image`)
  const link = object(member.link, `${path}.link`)
  const embed = object(member.embed, `${path}.embed`)
  return {
    schemes: strings(member.schemes, `${path}.schemes`),
    own_hosts: strings(member.own_hosts, `${path}.own_hosts`),
    storage_path_prefix: string(
      member.storage_path_prefix,
      `${path}.storage_path_prefix`
    ),
    image: {
      path_prefix: string(image.path_prefix, `${path}.image.path_prefix`),
      extensions: strings(image.extensions, `${path}.image.extensions`),
    },
    link: {
      forbidden_storage_extensions: strings(
        link.forbidden_storage_extensions,
        `${path}.link.forbidden_storage_extensions`
      ),
    },
    embed: {
      scheme: string(embed.scheme, `${path}.embed.scheme`),
      id_alphabet: string(embed.id_alphabet, `${path}.embed.id_alphabet`),
      providers: array(embed.providers, `${path}.embed.providers`, (v, p) => {
        const provider = object(v, p)
        return {
          hosts: strings(provider.hosts, `${p}.hosts`),
          path_prefix: string(provider.path_prefix, `${p}.path_prefix`),
          query: array(provider.query, `${p}.query`, (q, qp) => {
            const parameter = object(q, qp)
            return {
              name: string(parameter.name, `${qp}.name`),
              value: oneOf(parameter.value, `${qp}.value`, ["digits", "id"]),
            }
          }),
        }
      }),
    },
  }
}

function css(value: unknown, path: string): RichTextPolicy["css"] {
  const member = object(value, path)
  const border = object(member.border, `${path}.border`)
  const keywords = object(
    border.width_keywords,
    `${path}.border.width_keywords`
  )
  return {
    alphabet: string(member.alphabet, `${path}.alphabet`),
    border: {
      styles: strings(border.styles, `${path}.border.styles`),
      width_keywords: Object.fromEntries(
        Object.entries(keywords).map(([name, width]) => [
          name,
          nonNegative(width, `${path}.border.width_keywords.${name}`),
        ])
      ),
      initial: borderParts(border.initial, `${path}.border.initial`),
      unset: borderParts(border.unset, `${path}.border.unset`),
      max_width: nonNegative(border.max_width, `${path}.border.max_width`),
    },
  }
}

/**
 * Validates a parsed `policy.json` and returns it typed.
 *
 * Besides the shape of every member, it checks the invariants the admin
 * relies on: the version is {@link POLICY_VERSION}; `elements` has exactly
 * one entry per tag, in tag order; no tag is a forbidden tag (the same rule
 * as `scripts/ci/check-rich-text-manifest.sh`, so a hand-edited copy fails
 * here too); and an element has a `style` list exactly when it has a `style`
 * attribute (the manifest task refuses anything else, editor.md §3.9).
 *
 * @param json - the parsed file, of unknown shape.
 * @returns the policy, typed.
 * @throws {PolicyError} naming the first member that is wrong and the
 *   command that regenerates the file.
 */
export function readPolicy(json: unknown): RichTextPolicy {
  const root = object(json, "the file")
  const policyVersion = string(root.policy_version, "policy_version")
  if (policyVersion !== POLICY_VERSION) {
    fail(
      "policy_version",
      `is ${show(policyVersion)}, but this admin implements ${POLICY_VERSION}`
    )
  }
  const tags = strings(root.tags, "tags")
  const forbidden = strings(root.forbidden_tags, "forbidden_tags")
  for (const tag of tags) {
    if (forbidden.includes(tag.toLowerCase())) {
      fail("tags", `holds the forbidden tag ${show(tag)} (editor.md §3.5)`)
    }
  }
  const elementsMember = object(root.elements, "elements")
  const names = Object.keys(elementsMember)
  if (
    names.length !== tags.length ||
    names.some((name, i) => name !== tags[i])
  ) {
    fail(
      "elements",
      `must have one entry per tag, in tag order: ${show(tags)}, not ${show(names)}`
    )
  }
  const elements: Record<string, ElementPolicy> = {}
  for (const name of names) {
    const entry = element(elementsMember[name], `elements.${name}`)
    const styled = entry.attributes.some(
      (attribute) => attribute.name === "style"
    )
    if (styled !== (entry.style !== null)) {
      fail(
        `elements.${name}.style`,
        styled
          ? "must list the CSS properties of an element with a style attribute"
          : "must be null for an element without a style attribute"
      )
    }
    elements[name] = entry
  }
  return {
    policy_version: policyVersion,
    tags,
    elements,
    removed_with_content: strings(
      root.removed_with_content,
      "removed_with_content"
    ),
    removed_by_namespace_check: strings(
      root.removed_by_namespace_check,
      "removed_by_namespace_check"
    ),
    forbidden_tags: forbidden,
    css: css(root.css, "css"),
    urls: urls(root.urls, "urls"),
  }
}

/**
 * The committed content-v1 policy, validated when this module loads. Pass it
 * to the code that applies the policy (`siteExtensions({ policy })`).
 */
export const CONTENT_V1_POLICY: RichTextPolicy = readPolicy(policyJson)

/**
 * The rules of one element. A tag is looked up as an own member only, so
 * that a name such as `constructor` is not read from the prototype.
 *
 * @param policy - the policy to read.
 * @param tag - the element's tag name.
 * @returns the element's rules, or `undefined` when it is not a content-v1
 *   element.
 */
export function elementOf(
  policy: RichTextPolicy,
  tag: string
): ElementPolicy | undefined {
  return Object.hasOwn(policy.elements, tag) ? policy.elements[tag] : undefined
}

/**
 * The rule of one CSS property on one element.
 *
 * @param policy - the policy to read.
 * @param tag - the element's tag name.
 * @param property - the property name, lower case.
 * @returns the rule, or `undefined` when the element has no such property
 *   (or no `style` at all, or is not a content-v1 element).
 */
export function styleRuleOf(
  policy: RichTextPolicy,
  tag: string,
  property: string
): StyleRule | undefined {
  return elementOf(policy, tag)?.style?.find(
    (rule) => rule.property === property
  )
}

/**
 * The value rule of one attribute on one element.
 *
 * @param policy - the policy to read.
 * @param tag - the element's tag name.
 * @param name - the attribute name.
 * @returns the rule, or `undefined` when the element has no such attribute.
 */
export function attributeRuleOf(
  policy: RichTextPolicy,
  tag: string,
  name: string
): AttributeValueRule | undefined {
  return elementOf(policy, tag)?.attributes.find(
    (attribute) => attribute.name === name
  )?.value
}

/**
 * The grammar of a property that code built on the policy relies on: it must
 * be stored on the element and have the expected kind. Code that reads the
 * policy calls this when it is built, so a policy change that would leave it
 * reading the wrong grammar stops it at once, with the rule it expected.
 *
 * @param policy - the policy to read.
 * @param tag - the element's tag name.
 * @param property - the property name, lower case.
 * @param kind - the grammar kind the caller implements.
 * @returns the property's grammar.
 * @throws {PolicyError} when the element does not store the property with a
 *   grammar of that kind.
 */
export function requireStyleGrammar<K extends StyleGrammarKind>(
  policy: RichTextPolicy,
  tag: string,
  property: string,
  kind: K
): Extract<StyleGrammar, { readonly kind: K }> {
  const rule = styleRuleOf(policy, tag, property)
  if (
    rule === undefined ||
    rule.role !== "stored" ||
    rule.grammar.kind !== kind
  ) {
    throw new PolicyError(
      `The policy has no stored ${property} on ${tag} with a ${kind} grammar (editor.md §3.3), which the code built on it reads; found ${show(rule ?? null)}.`
    )
  }
  return rule.grammar as Extract<StyleGrammar, { readonly kind: K }>
}

/**
 * The value rule of an attribute that code built on the policy relies on:
 * the element must have the attribute with a rule of the expected kind
 * (see {@link requireStyleGrammar}).
 *
 * @param policy - the policy to read.
 * @param tag - the element's tag name.
 * @param name - the attribute name.
 * @param kind - the rule kind the caller implements.
 * @returns the attribute's value rule.
 * @throws {PolicyError} when the element has no such attribute of that kind.
 */
export function requireAttributeRule<K extends AttributeValueRule["kind"]>(
  policy: RichTextPolicy,
  tag: string,
  name: string,
  kind: K
): Extract<AttributeValueRule, { readonly kind: K }> {
  const rule = attributeRuleOf(policy, tag, name)
  if (rule === undefined || rule.kind !== kind) {
    throw new PolicyError(
      `The policy has no ${name} attribute on ${tag} with a ${kind} rule (editor.md §3.2), which the code built on it reads; found ${show(rule ?? null)}.`
    )
  }
  return rule as Extract<AttributeValueRule, { readonly kind: K }>
}
