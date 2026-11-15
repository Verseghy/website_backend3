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
- **ra-core's query-string stays on 7.** ra-core 5.15.6 parses list and
  create URLs with `@marmelab/query-string` 7.1.4, marmelab's CommonJS fork
  of query-string 7.1.3, which `require()`s `@marmelab/decode-uri-component`
  0.2.3, a fork of decode-uri-component 0.2.2 with the CVE-2026-45822 fix
  backported. That is the upstream fix the maintainer chose to wait for, and
  the chain stays as ra-core ships it until a ra-core release leaves the
  query-string 7 line. An npm `overrides` entry or a Vite alias for
  `@marmelab/query-string` or `@marmelab/decode-uri-component` never points
  at a vendored or patched copy of either: the maintainer's rule is to never
  vendor a fix and to always wait for the upstream release (research:
  DECISIONS R22, D113). `src/ra-core-query-string.test.ts` stays, and must
  stay green. Two published release lines break ra-core, so never override
  or alias the `@marmelab` names to them either:
  - query-string 8 and later, which export only a default: ra-core's named
    imports fail to link, which `npm run build` and every test that imports
    ra-core report;
  - decode-uri-component 0.4 and later, which are ESM-only: the `require()`
    gets a namespace object and every non-empty query throws.
    `npm run build`, the other unit tests and the E2E suite stay green (no
    screen reads a query yet); only that test fails, in Node and in a
    production bundle.

  The app's own query-string (10, for the kit's `filter-form.tsx`) is a
  separate package. Drop this note, and that test's override reasoning, when
  a ra-core release leaves the query-string 7 line.

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
- **Authentication (ADR 0006).** The auth provider in
  `src/auth/auth-provider.ts` is the only code that talks to
  `/api/admin/auth`. The session is an `HttpOnly` cookie, so the admin never
  reads or stores a token: `checkAuth` and `getIdentity` ask `/auth/me`.
  `checkError` signs out on `401` only, never on `403`. Every page needs a
  session (`requireAuth` in `src/App.tsx`); the only page without one is the
  Hungarian sign-in screen `src/auth/SignInPage.tsx`, which replaces the
  kit's login page and carries the source link. A refused request's message
  comes from its error envelope through `src/api/error-envelope.ts`
  (`errors.<code>`, or the server's text for an unknown code, ADR 0021 §4).
  Signing out ends every session of the account on the server
  (`POST /auth/logout`, ADR 0006 §2): the header shows backend3's
  `src/layout/AppUserMenu.tsx` instead of the kit's `UserMenu`, and its
  „Kijelentkezés” calls `useSignOut` (`src/auth/use-sign-out.ts`), which
  passes `SIGN_OUT_PARAMS` to `logout`. Only that call posts: ra-core calls
  `logout({})` by itself whenever it finds no session, and then there is
  nothing to end. A sign-out that the server did not confirm (`204`, or
  `401` for a session that had ended already) keeps the user signed in and
  says so; it never looks done while the cookie still holds a session.
- **Clean room (ADR 0011 §8, ADR 0018 §3.4).** Never install an
  `@react-admin/` package (react-admin's Enterprise Edition), and never read
  or copy Enterprise source, types, styles or examples. `npm run clean-room`
  (in the `Admin licenses` CI job) fails when `package.json` or the lockfile
  names one.
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
  shadcn-admin-kit and shadcn/ui under MIT. Keep their `LICENSE` files where
  they are: the licence-notices plugin finds them beside the bundled code
  (ADR 0038). Any
  change to them is listed in that directory's `LOCAL_CHANGES.md`, and adding or
  re-syncing vendored files updates `THIRD_PARTY.md` at the repository root in
  the same PR. They keep their upstream formatting (Prettier ignores them).
  backend3's own files that adapt kit code, such as `src/layout/AppUserMenu.tsx`
  (from the kit's `user-menu.tsx`), are not vendored: backend3 owns and
  formats them, but `THIRD_PARTY.md` lists each one with the kit's licence
  file and what changed.
- **Installing kit or shadcn components.** Run the shadcn CLI from `admin/`
  with `HOME` set to a throwaway directory, never against a real home
  directory: the template's post-init step runs `git init` and `git commit`,
  and the registry's `~/AGENTS.md` target resolves to the project directory
  (`admin/AGENTS.md`). Then review the diff, re-apply `LOCAL_CHANGES.md`, and
  remove any nested `.git` the CLI created.
- **Dependencies.** Install with `npm ci`; `package-lock.json` is committed.
  Never vendor a patched copy of a dependency to fix it; wait for the
  upstream release (research: DECISIONS R22, D113). Copying a package's code
  into `src/` to patch it is vendoring too. The kit code that ADR 0012 §4
  copies and owns is not covered: it changes as the vendored-code rule above
  says. A new production dependency must pass `npm run licenses`; a licence
  that is not in `license-policy.json` is added only in a PR that states why
  it may be combined with AGPL-3.0-only (ADR 0022 §4). `allow` holds only
  licences of ADR 0022 §4's allow-list (deny.toml's `allow`); any other
  licence, including 0BSD and BlueOak-1.0.0, is an `exceptions` entry that
  names the package, one entry per package (ADR 0027).
- **Licence notices (ADR 0038).** Every build writes the licence notices of
  the bundled packages to `dist/licenses.txt`, which backend3 serves at
  `/admin/licenses.txt`: Vite's `build.license` lists the packages of the
  JavaScript chunks, and the `licenceNotices` plugin of
  `scripts/licence-notices.mjs` adds the packages that a stylesheet
  `@import`s or that an asset is copied from, then the licence files of the
  copied code of `src/`: every capitalised `LICENSE`, `LICENCE` or `COPYING`
  file in the directory of a bundled module or in a parent up to `src/`.
  Keep both in `vite.config.ts`, with the same file name.
  `scripts/licence-notices.build.test.mjs` builds the admin and fails when a
  bundled package is missing from the file or has no licence text, when a
  bundled path of `../THIRD_PARTY.md` lacks its licence file, or when a
  bundled package or `src/` brings a `NOTICE` file, which the file would
  leave out; when it fails for a new dependency or newly copied code, fix the
  cause it names rather than the test.
- **Editor pins (editor.md §4.1).** Every `@tiptap/*` package is a direct
  dependency at one exact version, and prosemirror-view, prosemirror-model
  and prosemirror-tables are pinned exactly in `overrides`. Add or move them
  together, in `package.json` first and then with one `npm install`: with
  npm 11.19, adding an override to an already installed tree left a nested
  copy of prosemirror-view under each of its dependents, and `npm dedupe`
  removed them only together with unrelated version changes.
  `src/rich-text/pins.test.ts` fails on a range, a second version or a second
  copy.
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
npm run clean-room  # no @react-admin/ (Enterprise) package in package.json or the lockfile
npm run build       # writes dist/, which backend3 serves at /admin
npm run e2e         # Playwright: the admin in Chromium, /storage in Chromium, Firefox and WebKit
```

`npm run e2e` needs backend3 already running with `LOCO_ENV=e2e` and serving
a `dist/` built as CI builds it
(`VITE_REPORT_MISSING_KEYS=1 VITE_CLIPBOARD_RECORDER=1 npm run build`), and
the seed account (`LOCO_ENV=e2e cargo loco db seed`; the root `AGENTS.md`,
"The seed account"). The admin specs fail on any CSP violation or console
error (`e2e/fixtures.ts`). A test of a page with a session calls
`signIn(page)` from `e2e/fixtures.ts` first. A test that expects an HTTP
error answer, such as the `401` of `/auth/me` before the sign-in, lists it in
`expectedHttpErrors` (status and path): Chromium logs every such answer as a
console error, and the guard ignores only the listed ones. A test that signs
out ends every session of the seed account, which the other tests share in
parallel, so it goes into `e2e/sign-out.spec.ts`: `playwright.config.ts` runs
that file in its own `sign-out` project, after every other admin test has passed,
one test at a time. `e2e/storage.spec.ts` opens files under `/storage` in all
three engines, so it needs
`npx playwright install --with-deps chromium firefox webkit`, and it does not
use that guard: the scripts a sandboxed file tries to run are reported as
exactly those errors, which is what it tests.
