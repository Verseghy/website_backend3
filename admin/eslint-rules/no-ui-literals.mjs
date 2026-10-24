/**
 * ESLint rule `local/no-ui-literals`: the "No literals" check of ADR 0021 §7.
 *
 * The admin is Hungarian only and every user-visible string comes from
 * `src/i18n/hu.ts`. This rule reports user-visible text written directly in
 * JSX, where it would bypass translation:
 *
 * - JSX text that contains a letter (`<p>Save</p>`);
 * - a string in a `label`, `placeholder`, `title`, `aria-label` or `tooltip`
 *   prop (`<Input placeholder="Search" />`), unless the string is shaped like
 *   a translation key (`label="ra.action.save"`), because ra-core and the kit
 *   translate those props.
 *
 * Text without letters (punctuation, digits, symbols such as "·") is not
 * language and is allowed. The ESLint config exempts `src/i18n/` and tests.
 *
 * @type {import("eslint").Rule.RuleModule}
 */

/** The props ADR 0021 §7 names as user-visible. */
const TEXT_PROPS = new Set([
  "label",
  "placeholder",
  "title",
  "aria-label",
  "tooltip",
])

/** Any letter, in any script: text with one is language, not a symbol. */
const LETTER = /\p{L}/u

/**
 * A translation key: dot-joined identifiers with at least one dot, such as
 * `ra.action.save`. ra-core and the kit pass such props through `translate`.
 */
const KEY_SHAPE = /^[A-Za-z_][\w-]*(\.[\w-]+)+$/

/**
 * Returns the static text of a JSX attribute value, or `null` when the value
 * is computed (and so not a literal).
 *
 * @param {import("estree-jsx").JSXAttribute["value"]} value - the attribute value node.
 * @returns {string | null} the literal text, or `null`.
 */
function staticText(value) {
  if (value === null) {
    return null
  }
  if (value.type === "Literal") {
    return typeof value.value === "string" ? value.value : null
  }
  if (value.type === "JSXExpressionContainer") {
    const { expression } = value
    if (expression.type === "Literal" && typeof expression.value === "string") {
      return expression.value
    }
    if (
      expression.type === "TemplateLiteral" &&
      expression.expressions.length === 0
    ) {
      return expression.quasis.map((quasi) => quasi.value.cooked ?? "").join("")
    }
  }
  return null
}

/**
 * Returns the name of a JSX attribute (`aria-label`, `title` …), or `null` for
 * a namespaced name, which this rule does not inspect.
 *
 * @param {import("estree-jsx").JSXAttribute["name"]} name - the attribute name node.
 * @returns {string | null} the plain attribute name.
 */
function attributeName(name) {
  return name.type === "JSXIdentifier" ? name.name : null
}

/** @type {import("eslint").Rule.RuleModule} */
const rule = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow user-visible literals in JSX; take every UI string from admin/src/i18n/hu.ts (ADR 0021 §7).",
    },
    schema: [],
    messages: {
      jsxText:
        'User-visible text "{{text}}" is written directly in JSX. Add a key to admin/src/i18n/hu.ts and render it with useT() (or ra-core\'s translate) instead.',
      textProp:
        'The {{prop}} prop has the literal "{{text}}". Pass a hu.ts key (for example label="ra.action.save") or a translated value instead.',
    },
  },
  create(context) {
    return {
      /** @param {import("estree-jsx").JSXText} node */
      JSXText(node) {
        const text = node.value.trim()
        if (LETTER.test(text)) {
          context.report({ node, messageId: "jsxText", data: { text } })
        }
      },
      /** @param {import("estree-jsx").JSXAttribute} node */
      JSXAttribute(node) {
        const prop = attributeName(node.name)
        if (prop === null || !TEXT_PROPS.has(prop)) {
          return
        }
        const text = staticText(node.value)
        if (text !== null && LETTER.test(text) && !KEY_SHAPE.test(text)) {
          context.report({ node, messageId: "textProp", data: { prop, text } })
        }
      },
    }
  },
}

export default rule
