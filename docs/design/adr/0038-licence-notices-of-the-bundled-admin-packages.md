# 0038. Licence notices of the bundled admin packages

- Status: Accepted (2026-11-01)
- Decision questions: none: records the maintainer's answer of 2026-10-25 to the decision-register item D88 (research: DECISIONS R15), which settles the point that [ADR 0027](0027-per-package-licence-exceptions-and-the-loco-advisory-ignores.md) left open, and the answers of 2026-11-01 to the third decision register's items D114, which confirms the five parts of the build that go beyond D88's chosen option, and D115, which settles how the crates linked into the backend3 binary get their notices (research: DECISIONS R23). It amends one bullet of [ADR 0012](0012-delivery-review-commits-and-licensing.md) §4.

## Context

[ADR 0012](0012-delivery-review-commits-and-licensing.md) §4 says that dependencies "are checked, not listed": code that cargo or npm fetches is checked in CI and not indexed in `THIRD_PARTY.md`. [ADR 0027](0027-per-package-licence-exceptions-and-the-loco-advisory-ignores.md) found what that leaves out for the admin, and recorded it as undecided: "the minified admin JavaScript keeps none [of the notices] ... This ADR does not decide how `admin/dist` carries the notices of bundled packages."

| Fact | Value | Source |
|---|---|---|
| What the licences ask | MIT, ISC and the BSD licences ask that the copyright and permission notice be included in all copies or substantial portions; Apache-2.0 §4 asks the same of the licence and of any `NOTICE` file. Every package that the admin bundles today is under one of these or under OFL-1.1 (the Inter font), which lets the notice travel as a separate text file. No bundled package has a `NOTICE` file. | the licence texts; `admin/license-policy.json`; the installed packages |
| What the bundle keeps | A minified Vite build keeps no source comments. In the stylesheet, only Tailwind's `/*! tailwindcss ... \| MIT License \| https://tailwindcss.com */` line survives. | research: decision register D88; ADR 0027, References |
| Where the bundle goes | The Containerfile's admin stage builds `admin/dist` from `admin/` alone, every image carries it at `/app/admin/dist`, and backend3 serves it to every visitor of `/admin` ([ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md) §2 and §3). | ADR 0004; `Containerfile` |
| Vite's option | Vite 8.3.5, the version that `admin/package-lock.json` installs, has `build.license: boolean \| { fileName: string }`, default `false`. `true` writes `.vite/license.md` into the output directory; `fileName` sets another path relative to it, and a name that ends in `.json` gets the raw JSON entries instead of Markdown. Each entry is the package's name, version, `license` field and the text of its licence file. The documentation suggests a banner comment through `build.rolldownOptions.output.postBanner` to point built code at the file. | vite 8.3.5 (npm): `dist/node/index.d.ts` (`LicenseOptions`), `dist/node/chunks/node.js` (`licensePlugin`, `licenseEntryToMarkdown`); Vite documentation, "build.license" and Features, "License" (the site showed 8.3.4 on 2026-10-31) |
| What Vite's scan sees | The plugin walks `chunk.moduleIds` of the JavaScript chunks and maps each module under `node_modules` to its package. A build of `website_backend3@4065ce4`'s admin lists 56 entries (55 packages; `react-error-boundary` comes in two versions), exactly the packages that the chunks' source maps name. | the same `licensePlugin`; a `vite build --sourcemap` of that commit |
| Packages it does not see | `admin/src/index.css` imports `tailwindcss`, `tw-animate-css`, `shadcn/tailwind.css` and `@fontsource-variable/inter`. Tailwind's Vite plugin inlines them while it compiles the stylesheet, so none of them becomes a module of a chunk, and none is in Vite's file. The last one also brings the seven Inter font files that the build copies into `assets/`. `tw-animate-css` and `shadcn/tailwind.css` use Tailwind's `@utility`, `@custom-variant` and `@theme` rules, so they cannot be imported from JavaScript instead. | the same build; the packages' CSS files |
| Code it does not see | The scan skips every module outside `node_modules`. 72 of the 88 files of `admin/src` that the chunks' source maps name are code copied from shadcn-admin-kit, shadcn/ui and ra-language-hungarian, all MIT, whose licence files sit beside them (`THIRD_PARTY.md`, ADR 0012 §4), so the bundle carries that code without its notices too. The scan also skips virtual modules (`\0` ids): Vite's module-preload polyfill and Rolldown's runtime helpers, a few lines of glue that the bundler generates into the entry chunk. | the same build and `licensePlugin`; `THIRD_PARTY.md` |
| How the file would be served | tower-http 0.6.11's `ServeDir` sends the bare type that `mime_guess` 2.0.5 knows for the extension: `text/markdown` for `.md`, `text/plain` for `.txt`, with no `charset`. A browser decodes a text body without a charset with a locale-dependent default, which need not be UTF-8, and the copied Hungarian strings' licence file names its author with accented letters. Chromium shows `text/markdown` inline, Firefox offers it as a download; every browser shows `text/plain`. | tower-http@0.6.11:src/services/fs/serve_dir/open_file.rs; mime_guess@2.0.5:src/mime_types.rs; `admin/src/i18n/LICENSE.ra-language-hungarian` |

The register offered three options (research: decision register D88): turn on Vite's `build.license` and serve the file under `/admin` ("No new dependency: one config line, plus a test that the file exists and lists every bundled package"); generate a notices file and link it from the admin UI; or rely on `THIRD_PARTY.md`. It recommended the first, "the smallest change that puts the notices inside the distributed bundle, written by the bundler that knows exactly what it included", and asked the slice to check the option against Vite's documentation for the pinned version. The maintainer answered on 2026-10-25: "Turn on Vite's build.license and serve the file under /admin" (research: DECISIONS R15). The third option was not chosen because "the published images and the served bundle carry no notices"; that reason holds for the copied code as much as for the packages.

The slice that built the option went beyond D88's "one config line" in five parts, four additions and one reading of what need not be listed, and asked the maintainer about them before it landed (research: third decision register D114). The maintainer answered on 2026-11-01 (research: DECISIONS R23), with the recommended option. The register's question, the answer, and the consequence the register gave for that option:

- **D114**, "D88 chose Vite's build.license: 'one config line, plus a test that the file exists and lists every bundled package'. admin-licence-notices goes beyond that line in five parts, (a) to (e) below: four additions and one reading of what need not be listed. ADR 0038 records each one, and each can be dropped on its own. Do you confirm all five as built, confirm only some, or keep only D88's one line?": "Confirm all five as built". Its consequence: "No code change. ADR 0038 quotes this answer and the PR opens with every part as built: the notices file lists the stylesheet packages and the copied code, every chunk names the file, and every text file of the build states UTF-8."

The five parts, as the register named them, and where this ADR records each:

| Part | What it is | Recorded in |
|---|---|---|
| (a) Stylesheet packages | The `licenceNotices` plugin adds the four packages that `admin/src/index.css` `@import`s, and with the last of them the Inter font files, which Vite's scan does not see. | Section 1, "One file for every bundled package" |
| (b) Copied code | The plugin appends a `# Code copied into the admin` section with the licence files of the code copied into `admin/src` that the bundle holds. | Section 1, "The copied code's licence files, in the same file" |
| (c) Banner | Every JavaScript chunk starts with a comment that names the file. | Section 1, "A pointer from every chunk" |
| (d) Charset | The `/admin` router adds `charset=utf-8` to every `text/*` file of the build, not only to `licenses.txt`. | Section 2 |
| (e) Bundler helpers unlisted | Vite's module-preload polyfill and Rolldown's runtime helpers, which the bundler generates, are not listed. | Section 1, "Its limits", the third item |

The same slice found that the crates linked into the backend3 binary, which the image also ships, are outside D88, and asked about them too (research: third decision register D115). Section 3 records that answer.

## Decision

### 1. The file

`admin/vite.config.ts` sets `build.license` to `{ fileName: "licenses.txt" }`, so every build, the image's included, writes the notices to `admin/dist/licenses.txt`, at the root of `dist/`. The bullets below also record parts (a), (b), (c) and (e) of D114 (Context), which the maintainer confirmed as built.

- **The root of `dist/`, not Vite's `.vite/license.md`.** The URL stays plain, and `.vite/` stays Vite's own metadata directory (where `build.manifest` would write), out of the served paths.
- **A `.txt` name.** Vite writes Markdown into every name that does not end in `.json`, and Markdown reads as plain text by design. `.txt` gets `text/plain`, which every browser shows; `.md` would be a download in Firefox. JSON would not be readable in a browser.
- **One file for every bundled package.** `admin/scripts/licence-notices.mjs` holds a small Vite plugin, `licenceNotices`, with no new dependency. After Vite has written the file (`generateBundle` with `order: "post"`), it adds, in Vite's own entry format, the packages that Vite's scan cannot see:
  - every package that a stylesheet module of the bundle names in an `@import` rule (a bare specifier, found in `node_modules` as Node.js finds it);
  - every package that an emitted asset was copied from (its source path under `node_modules`).

  It fails the build when Vite wrote no notices file, or when an imported package cannot be found, so a misconfigured build never ships without notices.
- **The copied code's licence files, in the same file.** The plugin then appends a section, `# Code copied into the admin`, with one `## <path>` entry and the text of every licence file that lies in the directory of a bundled module of `admin/src/` or in a parent directory up to `src/`; today `src/components/admin/LICENSE`, `src/components/ui/LICENSE` and `src/i18n/LICENSE.ra-language-hungarian`. Copied code keeps its licence file beside it (ADR 0012 §4), so the bundle decides which files are listed, and no list is kept by hand. A licence file there is named in capitals (`LICENSE`, `LICENCE` or `COPYING`, alone or with a suffix), so that a module of backend3's own, such as a future `licence-link.tsx`, is never taken for one. Code whose licence file lies in another directory (`src/lib/utils.ts`, `src/hooks/use-mobile.ts`, `src/index.css`) is covered because the code beside its licence file is bundled too; the build test checks that for every row of `THIRD_PARTY.md` (section 4). The path names the licence file, and `THIRD_PARTY.md` says which paths each one covers.
- **Its limits.**
  - It reads the `@import` rules of the stylesheets that are modules of the bundle, not of the package stylesheets that those rules pull in. None of today's imported packages imports another package from its CSS, and a test fails if one starts to.
  - Like Vite, it reads licence files only, not an Apache-2.0 `NOTICE` file. No bundled package and no copied code has one today, and a test fails if one arrives.
  - Code that the bundler itself generates into the chunks, Vite's module-preload polyfill and Rolldown's runtime helpers, is not listed. They come from virtual modules that no package scan sees, they are a few lines of generated glue, which backend3 does not read as a substantial portion of either project, and listing `vite` would add its 98 KB licence file, which lists the packages bundled into Vite's own tooling rather than into the admin.
- **A pointer from every chunk.** `build.rolldownOptions.output.postBanner` starts every JavaScript chunk with `/*! Licence notices of the bundled code: /admin/licenses.txt */`, as Vite's documentation suggests, so a chunk copied on its own still says where its notices are. Rolldown adds it after minification, so it survives.

### 2. Serving it under `/admin`

The routing table of [ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md) §3 already covers the file: "`/admin/<name>.<ext>` that exists in `dist/` root ... the file, 200, `no-cache`". So `/admin/licenses.txt` is served by that row, with the `/admin` security headers of ADR 0023 §6, and needs no route of its own.

The `/admin` router states `charset=utf-8` on every `text/*` response of the file services that names no charset (`src/domain/admin_path.rs`, `utf8_text_content_type`). The licence texts carry accented names, and every text file of the admin build is UTF-8: Vite writes UTF-8, and the files it copies from `admin/public/` are repository files, which are UTF-8 too. The rule covers every text file rather than the notices file alone, so no file of the build depends on a browser's default. Other types are left alone: JSON and SVG state their encoding themselves. ADR 0004's table has no `Content-Type` column, so this adds to it without changing a row. This rule is part (d) of D114 (Context), which the maintainer confirmed as built.

### 3. What this ADR amends

| Amended text | Change |
|---|---|
| ADR 0012 §4, "Dependencies are checked, not listed. Code fetched by cargo or npm is not vendored, so it is checked in CI instead of being indexed in `THIRD_PARTY.md`" | Still true for `THIRD_PARTY.md`. In addition, the npm packages bundled into `admin/dist`, and the licence files of the code copied into `admin/src` that the bundle holds, are listed with their licence texts in `admin/dist/licenses.txt`, which ships in every image and is served at `/admin/licenses.txt`. `THIRD_PARTY.md` says where the file is. |

ADR 0027 is not amended: it decided nothing about the notices, and this ADR settles the point it left open. ADR 0004 is not amended (section 2).

The crates linked into the backend3 binary are not listed in `admin/dist/licenses.txt`. That file follows the admin's bundle, which every visitor of `/admin` receives; the binary reaches only those who pull the image. How the binary's crates get their notices was settled on 2026-11-01 (research: DECISIONS R23). After its context (that the image ships the binary, which links third-party crates and Rust's standard library, and carries none of their notices), the register asked the question below; then come the answer and the consequence the register gave for that option (research: third decision register D115):

- **D115**, "Should the image build generate a notices file for the binary and ship it in the image, also serve it under /admin, or leave the binary out?": "Generate it in the image build and ship it in the image". Its consequence: "One build slice. The rust stage downloads cargo-about's Linux musl release, pinned by version and SHA-256 like rustup-init, and after the release build writes the linked crates' notices to a file in the image, such as /app/licenses/crates.html. It works from a committed about.toml and template: the release target, no build or dev dependencies, a libwebp-sys clarification, and the standard text for the 17 crates without a licence file. The stage adds the NOTICE files, the IJG statement and the toolchain's COPYRIGHT-library.html beside it. A PR check runs the same generation with --fail, plus a check for NOTICE files the file does not carry, so a gap fails before master. An ADR amends ADR 0012 §4, as ADR 0038 does for the admin. No route changes."

The `binary-licence-notices` build slice carries out that answer:

- **What it ships.** A notices file of the crates linked into the binary, generated by cargo-about from `Cargo.lock` after the release build of the Containerfile's rust stage, with the crates' `NOTICE` files, the IJG statement and the Rust toolchain's `COPYRIGHT-library.html` (the standard library's notices) beside it.
- **Where.** In the image, under `/app` with the binary (for example `/app/licenses/crates.html`). It is not served over HTTP, and nothing in `admin/dist` or under `/admin` changes.
- **Its check.** A PR check runs the same generation with `--fail` and fails on a `NOTICE` file that the notices file does not carry, so a gap fails before it reaches `master`.

That slice records its own decision in an ADR that amends ADR 0012 §4 for the binary, as this ADR does for the admin. The binary's notices are a file of their own, outside the admin's bundle that this ADR governs, so this ADR's scope needs no later amendment.

### 4. What must be true in code and tests

- `admin/scripts/licence-notices.test.mjs`: unit and property tests of every helper of the plugin (the `@import` reader, the package of a specifier and of a path, the licence file names, the directory walk, the entry format, the merges, the reader of a notices file, the asset text) and of the plugin over fake bundles, including both build failures and an asset handed over as bytes.
- `admin/scripts/licence-notices.build.test.mjs` runs `vite build --sourcemap` as the Containerfile's admin stage does, and fails unless:
  - `licenses.txt` is at the root of `dist/` and nothing is under `.vite/`;
  - every package that a JavaScript chunk's source map names is listed at its installed version;
  - every package that the admin's own stylesheets import is listed, and so is the package of the Inter font files;
  - every listed package is installed (in `package-lock.json`), once;
  - every entry carries its licence text;
  - no bundled package and no file under `admin/src` has a `NOTICE` file;
  - for every row of `THIRD_PARTY.md` under `admin/src` whose code the chunks' source maps or the admin's stylesheets name, its licence file is listed, and every listed licence file appears once, with its committed text;
  - no imported package's stylesheets import another package (the plugin's limit);
  - every JavaScript chunk starts with the banner.

  It reads the file with the plugin's own reader, which takes a heading only in the form the file writes it, so a licence text with Markdown headings of its own is read as one entry.
- `tests/requests/admin_ui.rs`: `/admin/licenses.txt` of the fixture `dist/` answers 200 with `text/plain; charset=utf-8`, `no-cache` and the admin security headers, and its non-ASCII text arrives intact; `HEAD` answers 200 with the same headers and no body; the other text files of the fixture state the charset, and `favicon.svg` keeps `image/svg+xml`.
- `admin/e2e/licence-notices.spec.ts`: the real build, served by backend3, opens `/admin/licenses.txt` in Chromium as text that lists a package of the chunks, one of the stylesheet and the copied code's section, with the accented licence file of the Hungarian strings intact, and the entry script starts with the banner.

## Consequences

**Positive**

- Every copy of the admin, the image's included, carries the notices of the code in it, the copied code's included, written at build time from what the bundler included. No list is maintained by hand.
- The file follows the bundle: a dependency update that adds, removes or upgrades a bundled package changes the file in the same build, and the build test fails if a package or a copied path slips through.
- No new dependency and one file, as D88's chosen option said.

**Negative**

- `licenceNotices` is backend3's own code beside Vite's option: about 300 lines of code (650 with their documentation) and about 1,300 lines of tests, which the next Vite major may make redundant or break. The build test notices either.
- The plugin's stylesheet scan is one level deep (section 1). A future nested import fails the build test until the plugin follows it.
- A future bundled package that ships no licence file fails the build test. Its licence's notice duty is then checked and the way it is met recorded before the package is accepted.
- A future bundled package or copied code with an Apache-2.0 `NOTICE` file fails the build test until the plugin adds that file too.
- Copied code whose licence file lies in another directory is listed only through the code beside that file. If that code ever leaves the bundle, the build test fails, and the licence file moves next to the code that stays, or the plugin learns the mapping.

**Follow-up work**

- The `binary-licence-notices` slice ships the notices of the crates linked into the binary in the image, with the ADR that amends ADR 0012 §4 for it (section 3, D115).
- Nothing more for the admin. Whether the admin's about page links to the file was not part of D88; the option that included a link was not chosen.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Generate a notices file next to `THIRD_PARTY.md` and link it from the admin UI | research: decision register D88 | Not chosen: the maintainer chose Vite's option. |
| Rely on `THIRD_PARTY.md` in the public repository | research: decision register D88 | Not chosen: the image and the served bundle would carry no notices. |
| A Rollup licence plugin | research: decision register D88 (candidates) | Adds a dependency; Vite's own option does the same scan. |
| Vite's option alone, without `licenceNotices` | measured for this ADR; research: third decision register D114 ("Keep only D88's one line") | Not chosen: the maintainer confirmed all five parts as built. It leaves out the four stylesheet packages, the font files' package and the copied code, which the option's own test ("lists every bundled package") would catch for the packages. |
| Only some of parts (a) to (e) | research: third decision register D114 ("Confirm some; name in the note which to drop") | Not chosen: the maintainer confirmed all five as built. |
| Import the stylesheet packages from JavaScript, so Vite's scan sees them | measured for this ADR | `tw-animate-css` and `shadcn/tailwind.css` hold Tailwind rules that only the Tailwind-compiled stylesheet understands; only the font package could move. |
| Leave the copied code to `THIRD_PARTY.md` and its licence files in the repository | considered for section 1 | The image and the served bundle would carry 72 files of MIT code without their notices: the gap that D88 closed for the packages. |
| A hand-kept list of the copied code's licence files in `vite.config.ts` | considered for section 1 | One more list to keep in step with `THIRD_PARTY.md`; the walk from the bundled modules needs none, and the build test checks it against `THIRD_PARTY.md`. |
| Read `THIRD_PARTY.md` in the plugin | considered for section 1 | The Containerfile's admin stage builds from `admin/` alone, so the file is not there; the build test, which runs in a full checkout, reads it instead. |
| List `vite` and `rolldown` for the helpers they generate | considered for section 1 | Vite's licence file is 98 KB of the licences of the packages bundled into its own tooling, for a few lines of generated glue. |
| Vite's default `.vite/license.md`, or a `.md` name at the root | Vite documentation | A hidden metadata directory in the URL; Firefox downloads `text/markdown` instead of showing it. |
| The JSON form (`licenses.json`) | Vite documentation | Not readable as a page. |
| A dedicated `/admin` route with its own `Content-Type` | considered for section 2 | The root-file row already serves the file; the charset rule covers every text file instead of one. |
| Generate the binary's notices, ship them in the image and also serve them under `/admin` | research: third decision register D115 | Not chosen: the maintainer chose to ship them in the image only (section 3). |
| Leave the crates of the binary without notices | research: third decision register D115 | Not chosen: the image build generates their notices and ships them in the image (section 3). |

## References

- research: DECISIONS R15 (D88, answer of 2026-10-25); research: decision register D88 (the question, its context and the options)
- research: DECISIONS R23 (D114 and D115, answers of 2026-11-01); research: third decision register D114 and D115 (the questions, their context and the options)
- Vite documentation: <https://vite.dev/config/build-options#build-license>, <https://vite.dev/guide/features#license>
- vite 8.3.5 (npm): `dist/node/index.d.ts` (`LicenseOptions`, `BuildEnvironmentOptions.license`), `dist/node/chunks/node.js` (`licensePlugin`, `licenseEntryToMarkdown`, `findLicenseFile`), `LICENSE.md`
- rolldown 1.2.15 (npm): `OutputOptions.postBanner` ("after ... minification")
- tower-http@0.6.11:src/services/fs/serve_dir/open_file.rs (the `Content-Type` of `ServeDir`); mime_guess@2.0.5:src/mime_types.rs
- `THIRD_PARTY.md` (the copied code and its licence files); `Containerfile` (the admin stage)
- The MIT licence, the ISC licence, the BSD-2-Clause and BSD-3-Clause licences, Apache License 2.0 §4, SIL Open Font License 1.1
- Amended: [ADR 0012](0012-delivery-review-commits-and-licensing.md) §4, one bullet
- Related: [ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md) §3, [ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §6, [ADR 0027](0027-per-package-licence-exceptions-and-the-loco-advisory-ignores.md)
