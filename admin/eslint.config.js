// ESLint configuration of the admin (`npm run lint`, the `Admin lint` CI job,
// ADR 0022 §3). It starts from the shadcn Vite template's config and adds
// backend3's rules.
import js from "@eslint/js"
import globals from "globals"
import reactHooks from "eslint-plugin-react-hooks"
import reactRefresh from "eslint-plugin-react-refresh"
import tseslint from "typescript-eslint"
import { defineConfig, globalIgnores } from "eslint/config"

import noUiLiterals from "./eslint-rules/no-ui-literals.mjs"

export default defineConfig([
  globalIgnores([
    "dist",
    "coverage",
    "playwright-report",
    "test-results",
    // ts-rs output: generated, committed and never edited (ADR 0005).
    "src/bindings",
  ]),
  {
    files: ["**/*.{ts,tsx}"],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommended,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      globals: globals.browser,
    },
    rules: {
      "react-refresh/only-export-components": "off",
    },
  },
  {
    // Vendored code copied by the shadcn CLI (shadcn-admin-kit and shadcn/ui,
    // listed in THIRD_PARTY.md). It is linted with the rule set upstream lints
    // it with (shadcn-admin-kit v2.0.0 eslint.config.js: react-hooks 5's two
    // classic rules and `_`-prefixed unused names) instead of being rewritten,
    // so a kit re-sync stays a reviewable diff. Upstream code uses `any`
    // deliberately. The "No literals" rule below still applies to it
    // (ADR 0021 §3: kit strings are translated in place).
    files: [
      "src/components/admin/**/*.{ts,tsx}",
      "src/components/ui/**/*.{ts,tsx}",
      "src/hooks/use-mobile.ts",
      "src/lib/utils.ts",
      "src/lib/field.type.ts",
      "src/lib/i18nProvider.ts",
    ],
    linterOptions: { reportUnusedDisableDirectives: "off" },
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
      "@typescript-eslint/no-unused-vars": [
        "error",
        {
          varsIgnorePattern: "^_",
          argsIgnorePattern: "^_",
          ignoreRestSiblings: true,
        },
      ],
      "react-hooks/immutability": "off",
      "react-hooks/refs": "off",
      "react-hooks/set-state-in-effect": "off",
      "react-hooks/use-memo": "off",
      "react-hooks/incompatible-library": "off",
      "react-hooks/preserve-manual-memoization": "off",
      "react-hooks/purity": "off",
      "react-hooks/static-components": "off",
    },
  },
  {
    // Node-side files: configs, the licence check, the lint rule, E2E tests.
    files: [
      "*.{js,ts}",
      "scripts/**/*.mjs",
      "eslint-rules/**/*.mjs",
      "e2e/**/*.ts",
    ],
    extends: [js.configs.recommended],
    languageOptions: {
      globals: globals.node,
    },
  },
  {
    // The development-only clipboard recorder (src/recorder/) is reached
    // only through the dynamic import that main.tsx guards with
    // __CLIPBOARD_RECORDER__, which the image build drops (admin/CLAUDE.md).
    // A static import from any other application module would ship its code
    // in every build. Tests may import it; vite.config.ts is not under src/.
    files: ["src/**/*.{ts,tsx}"],
    ignores: ["src/main.tsx", "src/recorder/**", "src/**/*.test.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: [
                "@/recorder",
                "@/recorder/*",
                "**/recorder",
                "**/recorder/*",
              ],
              message:
                "src/recorder/ is development-only: only main.tsx may load it, through its guarded dynamic import.",
            },
          ],
        },
      ],
    },
  },
  {
    // ADR 0021 §7 "No literals": every UI string comes from src/i18n/hu.ts.
    files: ["src/**/*.tsx"],
    ignores: ["src/i18n/**", "src/**/*.test.tsx", "src/test/**"],
    plugins: { local: { rules: { "no-ui-literals": noUiLiterals } } },
    rules: {
      "local/no-ui-literals": "error",
    },
  },
])
