// @vitest-environment node
/**
 * Tests of `local/no-ui-literals` (ADR 0021 §7 "No literals"), run by Vitest
 * through ESLint's RuleTester.
 */
import { RuleTester } from "eslint"
import { describe, it } from "vitest"

import rule from "./no-ui-literals.mjs"

// RuleTester uses the test framework's globals when they exist; Vitest's are
// not global in this project, so hand them over explicitly.
RuleTester.describe = describe
RuleTester.it = it
RuleTester.itOnly = it.only

const ruleTester = new RuleTester({
  languageOptions: {
    ecmaVersion: "latest",
    sourceType: "module",
    parserOptions: { ecmaFeatures: { jsx: true } },
  },
})

ruleTester.run("no-ui-literals", rule, {
  valid: [
    // Translated text.
    { code: "const A = () => <p>{t('dashboard.welcome')}</p>" },
    { code: "const A = () => <Translate i18nKey='ra.action.save' />" },
    // Key-shaped props, which ra-core and the kit translate.
    { code: "const A = () => <TextInput label='ra.auth.email' />" },
    {
      code: "const A = () => <TextInput label={'resources.posts.fields.title'} />",
    },
    // Computed values.
    {
      code: "const A = () => <Input placeholder={translate('ra.action.search')} />",
    },
    // Text without letters is not language.
    { code: "const A = () => <span>· × – 42 | /</span>" },
    { code: "const A = () => <button aria-label='…'>×</button>" },
    // Props that are not user-visible text.
    {
      code: "const A = () => <div className='flex gap-2' data-testid='dashboard' />",
    },
    // Whitespace between elements.
    { code: "const A = () => (\n  <div>\n    <span />\n  </div>\n)" },
  ],
  invalid: [
    {
      code: "const A = () => <p>Save</p>",
      errors: [{ messageId: "jsxText", data: { text: "Save" } }],
    },
    {
      code: "const A = () => <span className='sr-only'>Bezárás</span>",
      errors: [{ messageId: "jsxText" }],
    },
    {
      code: "const A = () => <Input placeholder='Search...' />",
      errors: [
        {
          messageId: "textProp",
          data: { prop: "placeholder", text: "Search..." },
        },
      ],
    },
    {
      code: "const A = () => <button aria-label={'Toggle Sidebar'} />",
      errors: [
        {
          messageId: "textProp",
          data: { prop: "aria-label", text: "Toggle Sidebar" },
        },
      ],
    },
    {
      code: "const A = () => <button title={`Close`} />",
      errors: [
        { messageId: "textProp", data: { prop: "title", text: "Close" } },
      ],
    },
    {
      code: "const A = () => <TextInput label='Email' />",
      errors: [
        { messageId: "textProp", data: { prop: "label", text: "Email" } },
      ],
    },
    {
      code: "const A = () => <Tip tooltip='Edit link' />",
      errors: [
        { messageId: "textProp", data: { prop: "tooltip", text: "Edit link" } },
      ],
    },
    {
      // A sentence that happens to contain dots is not a key.
      code: "const A = () => <TextInput label='e.g. Main page' />",
      errors: [{ messageId: "textProp" }],
    },
  ],
})
