// @vitest-environment node
/**
 * Tests of `local/no-ungated-buttons` (ADR 0007 §7: the create, edit,
 * delete and bulk-delete buttons go through the access wrappers), run by
 * Vitest through ESLint's RuleTester, with the JavaScript parser and with
 * typescript-eslint's for the type-only imports.
 */
import { RuleTester } from "eslint"
import tseslint from "typescript-eslint"
import { describe, it } from "vitest"

import rule from "./no-ungated-buttons.mjs"

// RuleTester uses the test framework's globals when they exist; Vitest's are
// not global in this project, so hand them over explicitly.
RuleTester.describe = describe
RuleTester.it = it
RuleTester.itOnly = it.only

/** A screen of backend3's own. */
const SCREEN = "/repo/admin/src/labels/LabelList.tsx"
/** A page of the kit, which imports its siblings relatively. */
const KIT_PAGE = "/repo/admin/src/components/admin/edit.tsx"

const ruleTester = new RuleTester({
  languageOptions: { ecmaVersion: "latest", sourceType: "module" },
})

ruleTester.run("no-ungated-buttons", rule, {
  valid: [
    // The wrappers.
    {
      code: 'import { CanAccessCreateButton, CanAccessBulkDeleteButton } from "@/auth/access-buttons"',
      filename: SCREEN,
    },
    // The kit's other members, from its index or their files.
    {
      code: 'import { List, DataTable, TextField, ShowButton } from "@/components/admin"',
      filename: SCREEN,
    },
    {
      code: 'import { ShowButton } from "./show-button"',
      filename: KIT_PAGE,
    },
    {
      code: 'import { ExportButton } from "@/components/admin/export-button"',
      filename: SCREEN,
    },
    // A button of the same name that is not the kit's.
    {
      code: 'import { DeleteButton } from "@/trash/delete-button"',
      filename: SCREEN,
    },
    {
      code: 'import { EditButton } from "some-package"',
      filename: SCREEN,
    },
    // A relative path that leaves the kit's directory.
    {
      code: 'import { DeleteButton } from "../trash/delete-button"',
      filename: KIT_PAGE,
    },
    // Re-exporting a whole module uses no button (the kit's index).
    {
      code: 'export * from "./create-button"',
      filename: "/repo/admin/src/components/admin/index.ts",
    },
    // Exports without a source.
    { code: "const A = 1; export { A }", filename: SCREEN },
    // A dynamic import of another kit file, or of the wrappers.
    {
      code: 'const page = import("@/components/admin/list.js")',
      filename: SCREEN,
    },
    {
      code: 'const gated = import("@/auth/access-buttons")',
      filename: SCREEN,
    },
    // A computed source cannot be read.
    {
      code: "const load = (name) => import(name)",
      filename: SCREEN,
    },
  ],
  invalid: [
    {
      code: 'import { CreateButton } from "@/components/admin"',
      filename: SCREEN,
      errors: [
        {
          messageId: "ungated",
          data: {
            name: "CreateButton",
            wrapper: "CanAccessCreateButton",
            module: "@/auth/access-buttons",
          },
        },
      ],
    },
    {
      code: 'import { List, EditButton, DeleteButton as Remove } from "@/components/admin/index"',
      filename: SCREEN,
      errors: [
        {
          messageId: "ungated",
          data: {
            name: "EditButton",
            wrapper: "CanAccessEditButton",
            module: "@/auth/access-buttons",
          },
        },
        {
          messageId: "ungated",
          data: {
            name: "DeleteButton",
            wrapper: "CanAccessDeleteButton",
            module: "@/auth/access-buttons",
          },
        },
      ],
    },
    {
      code: 'import { BulkDeleteButton } from "@/components/admin/bulk-delete-button"',
      filename: SCREEN,
      errors: [{ messageId: "ungated" }],
    },
    // The kit's own pages, as a kit update writes them.
    {
      code: 'import { DeleteButton } from "./delete-button"',
      filename: KIT_PAGE,
      errors: [{ messageId: "ungated" }],
    },
    {
      code: 'import { CreateButton } from "../components/admin/create-button.tsx"',
      filename: "/repo/admin/src/labels/LabelList.tsx",
      errors: [{ messageId: "ungated" }],
    },
    {
      code: 'import { CreateButton } from "../components/admin"',
      filename: "C:\\repo\\admin\\src\\labels\\LabelList.tsx",
      errors: [{ messageId: "ungated" }],
    },
    // Re-exports and namespace imports reach them too.
    {
      code: 'export { EditButton } from "@/components/admin"',
      filename: SCREEN,
      errors: [{ messageId: "ungated" }],
    },
    {
      code: 'import * as Kit from "@/components/admin"',
      filename: SCREEN,
      errors: [{ messageId: "namespace" }],
    },
    // The `.js` and `.jsx` spellings that TypeScript's `bundler`
    // resolution maps onto the kit's `.ts` and `.tsx` files.
    {
      code: 'import { CreateButton } from "@/components/admin/create-button.js"',
      filename: SCREEN,
      errors: [{ messageId: "ungated" }],
    },
    {
      code: 'import { EditButton } from "./edit-button.jsx"',
      filename: KIT_PAGE,
      errors: [{ messageId: "ungated" }],
    },
    {
      code: 'import { BulkDeleteButton } from "@/components/admin/index.js"',
      filename: SCREEN,
      errors: [{ messageId: "ungated" }],
    },
    // Dynamic imports, of a button's file or of the kit's index.
    {
      code: 'const Remove = lazy(() => import("@/components/admin/delete-button").then((m) => ({ default: m.DeleteButton })))',
      filename: SCREEN,
      errors: [
        {
          messageId: "dynamic",
          data: {
            source: "@/components/admin/delete-button",
            module: "@/auth/access-buttons",
          },
        },
      ],
    },
    {
      code: "const kit = import(`@/components/admin`)",
      filename: SCREEN,
      errors: [{ messageId: "dynamic" }],
    },
    {
      code: 'const create = import("./create-button.js")',
      filename: KIT_PAGE,
      errors: [{ messageId: "dynamic" }],
    },
  ],
})

const typeScriptTester = new RuleTester({
  languageOptions: {
    ecmaVersion: "latest",
    sourceType: "module",
    parser: tseslint.parser,
  },
})

typeScriptTester.run("no-ungated-buttons (TypeScript)", rule, {
  valid: [
    // Types carry no button.
    {
      code: 'import type { CreateButtonProps } from "@/components/admin/create-button"',
      filename: SCREEN,
    },
    {
      code: 'import type { EditButton } from "@/components/admin"',
      filename: SCREEN,
    },
    {
      code: 'import { type DeleteButtonProps } from "@/components/admin/delete-button"',
      filename: SCREEN,
    },
    {
      code: 'export type { BulkDeleteButtonProps } from "@/components/admin"',
      filename: SCREEN,
    },
    {
      code: 'import type { CreateButtonProps } from "@/components/admin/create-button.js"',
      filename: SCREEN,
    },
  ],
  invalid: [
    {
      code: 'import { DeleteButton, type DeleteButtonProps } from "@/components/admin/delete-button"',
      filename: SCREEN,
      errors: [{ messageId: "ungated" }],
    },
  ],
})
