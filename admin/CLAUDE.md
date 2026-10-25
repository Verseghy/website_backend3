@AGENTS.md

# backend3 admin: project overrides

`AGENTS.md` above is shadcn-admin-kit's agent file exactly as the kit's
registry installs it; it is never edited, so a kit reinstall overwrites it
cleanly. **Where it and this file disagree, this file wins** (ADR 0024 §2).
The root `CLAUDE.md` and `AGENTS.md` apply here too. The design is in
`../docs/design/`; read ADR 0004 (this directory), ADR 0021 (Hungarian UI) and
ADR 0005 (bindings) before changing anything here.

## Overrides of the kit's AGENTS.md

- **Data provider.** The only data provider is backend3's own, in
  `src/data-provider/data-provider.ts`; it talks to backend3's admin API at
  `/api/admin` (ADR 0004 §1). Never install or use `ra-data-json-server`,
  `ra-data-simple-rest` or JSONPlaceholder, and never use the kit's
  `example-admin` block. A slice adds a resource by registering
  `ResourceHandlers` for it, typed against `src/bindings/`.
- **TypeScript config.** Keep `verbatimModuleSyntax: true`; the kit's advice
  to set it to `false` does not apply (the copied files use `import type`, and
  `npm run typecheck` passes with it on).
- **No guessers.** Do not ship `ListGuesser`, `EditGuesser` or `ShowGuesser`
  screens. Every screen is an explicit component whose labels come from
  `src/i18n/hu.ts`.
- **Mounting `<Admin>`.** `src/App.tsx` mounts the kit's `<Admin>` with
  `basename="/admin"` inside the react-router 7 browser router of
  `src/router.tsx`, never on its own (ra-core would fall back to a hash
  router, ADR 0004 §3), and always with `disableTelemetry` (the kit otherwise
  loads a tracking image from marmelab's server, which the admin CSP forbids).
- **react-router stays on major 7.** ra-core 5.15.5 accepts `^6.28.1 || ^7.1.1`
  only; npm's `latest` tag is 8.

## Rules for this directory

- **Hungarian only (ADR 0021).** Every user-visible string comes from
  `src/i18n/hu.ts`. backend3's own components translate with `useT()` from
  `src/i18n/keys.ts`, which only accepts keys that exist; copied kit files use
  ra-core's `useTranslate` or `<Translate i18nKey>`. No literals in JSX text or
  in `label`, `placeholder`, `title`, `aria-label` or `tooltip` props: the
  `local/no-ui-literals` lint rule fails them. Use ADR 0021 §6's terms
  („bejegyzés”, „lomtár”, „étel”, never „menü” alone). A slice adds its
  resources, enums, error codes and screen namespaces to `hu.ts` in the same PR.
- **Namespaces (ADR 0021 §2, amended by ADR 0030).** `ra.*` is exactly
  `ra-language-english` plus the three kit-only keys; a fourth needs an
  amending ADR, so until one exists a kit update that calls one is edited to
  use `kit.*`. In a copied kit file, reuse an `ra.*` key when ra-core has the
  string, and otherwise add `kit.<component>.<name>` and list the edit in
  `LOCAL_CHANGES.md`. In backend3's own code, a new key goes in its resource,
  editor, screen, enum or error namespace; `app.*` is only the shell that no
  screen owns (title, footer, about page, update prompt, data-provider
  notice); ADR 0030 §3 has the details. A new top-level namespace needs an
  amending ADR and an entry in `NAMESPACES` of `src/i18n/key-usage.test.ts`,
  which a test there checks (ADR 0030 §4).
- **Links to fixed admin paths** go through the basename:
  `` to={`${useBasename()}/about`} ``, never `to="/about"`. ra-core's
  `LinkBase` is a plain router link; `useCreatePath` and `useRedirect` add the
  basename themselves. `src/router.test.ts` rejects a root-absolute `to`.
- **Bindings (ADR 0005).** `src/bindings/` is written by ts-rs from the Rust
  DTOs (`cargo test` at the repository root) and committed. Never edit it by
  hand; lint and Prettier ignore it, `npm run typecheck` checks it.
- **Vendored code (ADR 0012 §4).** `src/components/admin/`,
  `src/components/ui/`, `src/lib/utils.ts`, `src/lib/field.type.ts`,
  `src/lib/i18nProvider.ts` and `src/hooks/use-mobile.ts` are copied from
  shadcn-admin-kit and shadcn/ui under MIT. Keep their `LICENSE` files. Any
  change to them is listed in that directory's `LOCAL_CHANGES.md`, and adding or
  re-syncing vendored files updates `THIRD_PARTY.md` at the repository root in
  the same PR. They keep their upstream formatting (Prettier ignores them).
- **Installing kit or shadcn components.** Run the shadcn CLI from `admin/`
  with `HOME` set to a throwaway directory, never against a real home
  directory: the template's post-init step runs `git init` and `git commit`,
  and the registry's `~/AGENTS.md` target resolves to the project directory
  (`admin/AGENTS.md`). Then review the diff, re-apply `LOCAL_CHANGES.md`, and
  remove any nested `.git` the CLI created.
- **Dependencies.** Install with `npm ci`; `package-lock.json` is committed.
  A new production dependency must pass `npm run licenses`; a licence that is
  not in `license-policy.json` is added only in a PR that states why it may be
  combined with AGPL-3.0-only (ADR 0022 §4). `allow` holds only licences of
  ADR 0022 §4's allow-list (deny.toml's `allow`); any other licence,
  including 0BSD and BlueOak-1.0.0, is an `exceptions` entry that names the
  package, one entry per package (ADR 0027).
- **Pinned majors.** `../.github/dependabot.yml` keeps Dependabot's version
  updates off the majors the ADRs pin: `@types/node` on the `.node-version`
  major, react-router 7, ra-core 5 and its peers, React 19, Vite 8,
  Tailwind 4 and tailwind-merge 3, the kit's `shadcn` and `@base-ui/react`,
  and TipTap 3 with its ProseMirror pins. It also keeps TypeScript on 6,
  because typescript-eslint does not support TypeScript 7 yet. A PR that moves
  one of them on purpose (a Node.js LTS move, a kit re-sync, the TypeScript 7
  move once typescript-eslint supports it) bumps the package itself and
  adjusts its rule there. Security updates are not filtered by these rules;
  close a security PR that leaves a pinned major and fix the advisory with a
  deliberate bump.
- **Configuration is explicit.** Only `src/main.tsx` reads `import.meta.env`,
  `window` or the constants `vite.config.ts` defines
  (`__CLIPBOARD_RECORDER__`); everything else gets its configuration as
  arguments or props.
- **Development-only code stays out of production.** The clipboard recorder
  (`src/recorder/`, editor.md §13.1) is reachable only through the dynamic
  import that `src/main.tsx` guards with `__CLIPBOARD_RECORDER__`: always on
  the dev server, in a build only with `VITE_CLIPBOARD_RECORDER=1`, never in
  an image (the Containerfile pins it to `0`). No other application module
  imports `src/recorder/`; ESLint's `no-restricted-imports` fails one that
  does. Only `vite.config.ts` (for `recorder-flag.ts`) and tests may import
  it. `src/recorder/production-bundle.test.ts` fails if the image build holds
  the recorder's route or code.
- **The build-time source commit.** `VITE_SOURCE_COMMIT` is empty or a full
  40-character commit SHA; `vite.config.ts` rejects anything else. The footer
  links to the running commit (ADR 0012 §4).

## The admin gate

Run from `admin/`, with the Node.js major in `.node-version`:

```sh
npm ci
npm run lint        # ESLint, including local/no-ui-literals
npm run typecheck   # tsc -b --noEmit over the app, the scripts and the E2E suite
npm test            # Vitest: unit tests, i18n completeness checks, licence-policy tests
npm run licenses    # the production-dependency licence check
npm run build       # writes dist/, which backend3 serves at /admin
npm run e2e         # Playwright (Chromium) against http://localhost:5150/admin
```

`npm run e2e` needs backend3 already running with `LOCO_ENV=e2e` and serving
a `dist/` built as CI builds it
(`VITE_REPORT_MISSING_KEYS=1 VITE_CLIPBOARD_RECORDER=1 npm run build`); it
fails on any CSP violation or console error.
