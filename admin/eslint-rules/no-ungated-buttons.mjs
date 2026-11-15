/**
 * ESLint rule `local/no-ungated-buttons`: the create, edit, delete and
 * bulk-delete buttons go through backend3's access wrappers (ADR 0007 §7).
 *
 * The admin hides what the signed-in account may not do, but none of the
 * kit's buttons asks `canAccess`. `src/auth/access-buttons.tsx` wraps each
 * one in a check of the access map of `/auth/me`; this rule reports every
 * import of the kit's own `CreateButton`, `EditButton`, `DeleteButton` and
 * `BulkDeleteButton`, from the kit's index or from the button's file, and
 * every re-export of them, so that a screen, or a kit update that brings
 * them back into the kit's own pages, cannot render an action the account
 * would be refused. Type imports (`CreateButtonProps` …) are allowed. The
 * ESLint config exempts the wrappers' module and tests.
 *
 * A source is matched with or without its extension, `.js` and `.jsx`
 * included: with TypeScript's `bundler` resolution,
 * `@/components/admin/create-button.js` loads `create-button.tsx`. A
 * dynamic `import()` of the kit's index or of a button's file is reported
 * as a namespace import is, since its module object holds the buttons; a
 * computed source cannot be read and is left alone.
 *
 * A re-export of a whole module (`export * from "./create-button"`, the
 * kit's index) is not reported: it uses no button.
 *
 * @type {import("eslint").Rule.RuleModule}
 */
import { posix } from "node:path"

/** The kit's buttons, and the wrapper that replaces each. */
const WRAPPERS = new Map([
  ["CreateButton", "CanAccessCreateButton"],
  ["EditButton", "CanAccessEditButton"],
  ["DeleteButton", "CanAccessDeleteButton"],
  ["BulkDeleteButton", "CanAccessBulkDeleteButton"],
])

/** The module that holds the wrappers, as an import names it. */
const WRAPPER_MODULE = "@/auth/access-buttons"

/**
 * The kit's index: `components/admin`, or its `index` file, with any of
 * the extensions an import may write for it (`.ts`, `.tsx`, `.js`, `.jsx`).
 */
const KIT_INDEX = /(?:^|\/)components\/admin(?:\/index(?:\.[jt]sx?)?)?$/

/**
 * The file of one of the four buttons in the kit's directory, with any of
 * the extensions an import may write for it.
 */
const BUTTON_FILE =
  /(?:^|\/)components\/admin\/(?:create|edit|delete|bulk-delete)-button(?:\.[jt]sx?)?$/

/**
 * The module an import source names, as a POSIX path that the patterns
 * above can read: `@/x` is `src/x`, and a relative source is resolved
 * against the importing file; a package name stays as it is.
 *
 * @param {string} source - the import source.
 * @param {string} filename - the importing file.
 * @returns {string} the resolved module path.
 */
function resolveSource(source, filename) {
  if (source.startsWith("@/")) {
    return `src/${source.slice(2)}`
  }
  if (source.startsWith("./") || source.startsWith("../")) {
    const importer = filename.replaceAll("\\", "/")
    return posix.join(posix.dirname(importer), source)
  }
  return source
}

/**
 * Whether an import source names the kit's index or a button's file.
 *
 * @param {string} source - the import source.
 * @param {string} filename - the importing file.
 * @returns {boolean} `true` for a module that exports a kit button.
 */
function isKitButtonSource(source, filename) {
  const resolved = resolveSource(source, filename)
  return KIT_INDEX.test(resolved) || BUTTON_FILE.test(resolved)
}

/**
 * The text of a dynamic import's source when it is written out: a string
 * literal, or a template literal without expressions.
 *
 * @param {import("estree").Expression} node - the `import()` argument.
 * @returns {string | undefined} the source, or `undefined` when only the
 *   running program can compute it.
 */
function literalSource(node) {
  if (node.type === "Literal" && typeof node.value === "string") {
    return node.value
  }
  if (node.type === "TemplateLiteral" && node.expressions.length === 0) {
    return node.quasis[0]?.value.cooked ?? undefined
  }
  return undefined
}

/**
 * The exported name an import or export specifier refers to.
 *
 * @param {import("estree").Identifier | import("estree").Literal} node - the specifier's `imported` or `local` node.
 * @returns {string} the name.
 */
function nameOf(node) {
  return node.type === "Identifier" ? node.name : String(node.value)
}

/**
 * Whether a declaration or specifier is type-only. typescript-eslint marks
 * `import type`, `export type` and `{ type X }` with an `importKind` or
 * `exportKind` of `"type"`; ESTree's own types declare neither, and the
 * JavaScript parser sets neither.
 *
 * @param {object} node - an import or export declaration or specifier.
 * @returns {boolean} `true` for a type-only node.
 */
function isTypeOnly(node) {
  const kinds = /** @type {{ importKind?: string, exportKind?: string }} */ (
    node
  )
  return kinds.importKind === "type" || kinds.exportKind === "type"
}

/** @type {import("eslint").Rule.RuleModule} */
const rule = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Use backend3's access wrappers instead of the kit's create, edit, delete and bulk-delete buttons (ADR 0007 §7)",
    },
    schema: [],
    messages: {
      ungated:
        '"{{name}}" shows its action to every account; use {{wrapper}} from "{{module}}", which asks canAccess first (ADR 0007 §7).',
      namespace:
        'A namespace import of "{{source}}" reaches the kit\'s ungated buttons; import the members you need by name, and the buttons from "{{module}}" (ADR 0007 §7).',
      dynamic:
        'A dynamic import of "{{source}}" reaches the kit\'s ungated buttons; import the members you need by name, and the buttons from "{{module}}" (ADR 0007 §7).',
    },
  },
  create(context) {
    const filename = context.filename

    /**
     * Reports the specifiers of `node` that name a kit button.
     *
     * @param {import("estree").ImportDeclaration | import("estree").ExportNamedDeclaration} node - the declaration.
     * @param {string} source - its source, for the message.
     * @param {(specifier: any) => import("estree").Identifier | import("estree").Literal} exported - the node that holds the kit's name of a specifier.
     */
    function reportSpecifiers(node, source, exported) {
      for (const specifier of node.specifiers) {
        if (isTypeOnly(specifier)) {
          continue
        }
        if (specifier.type === "ImportNamespaceSpecifier") {
          context.report({
            node: specifier,
            messageId: "namespace",
            data: { source, module: WRAPPER_MODULE },
          })
          continue
        }
        if (
          specifier.type !== "ImportSpecifier" &&
          specifier.type !== "ExportSpecifier"
        ) {
          continue
        }
        const name = nameOf(exported(specifier))
        const wrapper = WRAPPERS.get(name)
        if (wrapper !== undefined) {
          context.report({
            node: specifier,
            messageId: "ungated",
            data: { name, wrapper, module: WRAPPER_MODULE },
          })
        }
      }
    }

    return {
      ImportDeclaration(node) {
        const source = String(node.source.value)
        if (isTypeOnly(node) || !isKitButtonSource(source, filename)) {
          return
        }
        reportSpecifiers(node, source, (specifier) => specifier.imported)
      },
      ExportNamedDeclaration(node) {
        if (node.source === null || node.source === undefined) {
          return
        }
        const source = String(node.source.value)
        if (isTypeOnly(node) || !isKitButtonSource(source, filename)) {
          return
        }
        reportSpecifiers(node, source, (specifier) => specifier.local)
      },
      ImportExpression(node) {
        const source = literalSource(node.source)
        if (source === undefined || !isKitButtonSource(source, filename)) {
          return
        }
        context.report({
          node,
          messageId: "dynamic",
          data: { source, module: WRAPPER_MODULE },
        })
      },
    }
  },
}

export default rule
