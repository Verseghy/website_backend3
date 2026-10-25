# 0030. App and kit namespaces in the Hungarian message file

- Status: Accepted (2026-10-25)
- Decision questions: none: amends [ADR 0021](0021-hungarian-only-admin-ui.md) §2, which records Q40. It records the maintainer's answer of 2026-10-25 to register item D24 (research: DECISIONS R9).

## Context

[ADR 0021](0021-hungarian-only-admin-ui.md) §2 makes `admin/src/i18n/hu.ts` the single source of every user-visible string in the admin and lists its namespaces in a table: `ra.*`, `resources.*`, `editor.*`, the screen namespaces (`tree.*`, `canteen.*`, `media.*`, `trash.*`, `history.*`, `activity.*`, `auth.*`, `dashboard.*`), `enums.*` and `errors.<code>`. Other decisions add strings that fit none of these rows. ADR 0021 §3 routes every hard-coded string of the copied kit files through a translation key. [ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md) §5 adds an update prompt, and [ADR 0012](0012-delivery-review-commits-and-licensing.md) §4 adds a footer link to the running source.

The bootstrap (`Verseghy/website_backend3#186`) built `hu.ts` with two namespaces that the table does not list, `app.*` and `kit.*`, and asked the maintainer to accept them or to move the strings. `Verseghy/website_backend3#195` item 3 carries the question. Later, the clipboard recorder (`Verseghy/website_backend3#213`) added a third namespace that the table does not name, `recorder.*`, for a development-only screen.

| Fact | Value | Source |
|---|---|---|
| `ra.*` has an exact definition | Every key of `ra-language-english` at the installed version (164 keys at 5.15.5), plus the three kit-only keys of ADR 0021's Context. `hu.test.ts` checks both directions. Key parity fails on an English key that `hu.ts` lacks. A second test fails on an `ra.*` key that is neither in the English pack nor one of the three. `hu.ts` has 167 `ra.*` keys. | ADR 0021 §2, §7; `website_backend3@ad496da:admin/src/i18n/hu.test.ts` |
| What `app.*` holds | 12 keys: `app.title` and four groups. `app.title` is the document title, the `<Admin>` title and the brand of the copied sidebar; a test pins `index.html`'s `<title>` to it. `app.update.*` is the update prompt of ADR 0004 §5, in the public site's wording. `app.footer.*` is the footer's about link and the AGPL-3.0 §13 source link of ADR 0012 §4, with the source link's title. `app.about.*` is the about page: what the admin is, its licence, its source and the running commit; the page's heading calls `ra.message.about`. `app.data_provider.not_available` is the notice an editor sees when a data-provider call has no handler yet. | `website_backend3@ad496da:admin/src/i18n/hu.ts`, `admin/src/main.tsx`, `admin/src/App.tsx`, `admin/src/components/admin/app-sidebar.tsx`, `admin/src/layout/AppFooter.tsx`, `admin/src/source-link/SourceLink.tsx`, `admin/src/about/AboutPage.tsx`, `admin/src/update-prompt/UpdatePrompt.tsx`, `admin/src/data-provider/data-provider.ts` |
| What `kit.*` holds | 17 keys in seven groups, one per kit component: `autocomplete`, `breadcrumb`, `data_table`, `pagination`, `ready`, `sidebar`, `theme`. Each replaces an English string that a copied shadcn-admin-kit 2.0.0 or shadcn/ui file hard-codes. Each edit is listed in the `LOCAL_CHANGES.md` file of its directory. | `website_backend3@ad496da:admin/src/components/admin/LOCAL_CHANGES.md`, `admin/src/components/ui/LOCAL_CHANGES.md` |
| Kit strings that ra-core already has | The same edits reuse an `ra.*` key wherever the English pack has the string. For example, "Close" became `ra.action.close`, "Go to previous page" became `ra.navigation.previous`, the autocomplete's search placeholder became `ra.action.search`, and the labels of the kit's sign-in page became `ra.auth.*`. Only strings with no English-pack key got a `kit.*` key. | the two `LOCAL_CHANGES.md` files |
| What `recorder.*` holds | 9 keys: the strings of the „Vágólap-rögzítő” page at `/admin/clipboard-recorder`, the development-only clipboard recorder of editor.md §13.1, which records paste captures for the editor's test fixtures. The page translates with `useT()`. Its route is on the dev server and in a build with `VITE_CLIPBOARD_RECORDER=1`, never in the image; the strings stay in `hu.ts` in every build. The same PR added `recorder` to the namespace list of `key-usage.test.ts`. | `website_backend3@c2977ee:admin/src/i18n/hu.ts`, `admin/src/recorder/ClipboardRecorderPage.tsx`, `admin/src/i18n/key-usage.test.ts`; [editor.md](../editor.md) §13.1 |
| The checks already cover both namespaces | `key-usage.test.ts` scans the string literals of 15 namespaces, `app` and `kit` among them (16 since the clipboard recorder added `recorder`). The `local/no-ui-literals` lint rule runs over every `.tsx` file under `admin/src` outside `src/i18n/` and the tests, so it covers the copied kit files too. The missing-key report of `provider.ts`, which the E2E build turns on, reports every key-shaped key that `hu.ts` lacks, in any namespace; only an optional `resources.*` override that comes with a fallback is exempt. | `website_backend3@ad496da:admin/src/i18n/key-usage.test.ts`, `admin/eslint.config.js`, `admin/eslint-rules/no-ui-literals.mjs`, `admin/src/i18n/provider.ts` |
| What no check compares | No test compares the top-level keys of `hu.ts` with the namespace list of `key-usage.test.ts`. The scan sees only the listed namespaces, so it would not see a missing key in a namespace that `hu.ts` has and the list lacks. | `website_backend3@ad496da:admin/src/i18n/key-usage.test.ts` |

The maintainer was offered three options (research: DECISIONS R9, register item D24):

- **Accept with an amending ADR:** ADR 0021 §2 gains two rows, and the code is unchanged.
- **Accept as an ADR 0001 §6 editorial note:** lighter, but the table changes without a record of the answer.
- **Move the strings:** the kit strings join `ra.*`, which breaks its definition and needs exceptions in the checks; the `app.*` strings are spread across `dashboard.*` and `auth.*`.

The maintainer's answer, on 2026-10-25: "Accept with an amending ADR" (research: DECISIONS R9, register item D24).

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0021 §2, the namespace table | It gains a `kit.*` row and an `app.*` row, and the screen row also names `recorder.*`. Every other row holds as written. | 2 |

The maintainer's answer covers the two new rows. Naming `recorder.*` in the screen row adds no rule: the clipboard recorder is a custom screen, its strings are that screen's own, and section 3 already sends a custom screen's strings to the screen's namespace. The row lists its namespaces by name, so the statement of section 2 that `hu.ts` has no top-level key outside the table needs this name too.

Everything else in ADR 0021 holds as written. That includes the definition of `ra.*` with its three kit-only keys, the wiring of §3, the server messages of §4, the checks of §7 and the review of §8. Section 3 of this ADR says how the two new rows sit beside the existing ones, and section 4 says how the §7 checks apply to them; neither adds a rule to an existing row.

The bootstrap's code already follows this ADR, so the admin's behaviour does not change. The same PR adds one test to `key-usage.test.ts` (section 4), with a helper and two tests of the helper, and changes comments in `hu.ts` and `key-usage.test.ts`. The chosen option's "the code is unchanged" holds for the admin's runtime code: no string moves and no component changes. The new test changes no runtime code; it guards the table of section 2.

### 2. The namespace table

ADR 0021 §2's table, with the two rows this ADR adds and `recorder.*` named in the screen row:

| Namespace | Contents |
|---|---|
| `ra.*` | every key of `ra-language-english` at the installed ra-core version (164 keys at 5.15.5), plus the three kit-only keys listed in ADR 0021's Context |
| `kit.*` (added by this ADR) | the strings of the copied shadcn-admin-kit and shadcn/ui files under `admin/src/components/` that upstream hard-codes in English and that no `ra-language-english` key expresses. A key is `kit.<component>.<name>`, grouped by the kit component that shows the string, for example `kit.data_table.empty` or `kit.sidebar.toggle`. |
| `app.*` (added by this ADR) | backend3's own application shell, which no single screen owns: the application title (`app.title`), the update prompt of ADR 0004 §5 (`app.update.*`, in the public site's wording), the page footer with the about link and the source link of ADR 0012 §4 (`app.footer.*`), the about page (`app.about.*`), and the notice for a data-provider call that has no handler yet (`app.data_provider.*`) |
| `resources.<resource>.name`, `resources.<resource>.fields.<field>` | every resource name and every field label, list column, filter, helper text and placeholder |
| `editor.*` | every string of the rich-text editor (ADR 0013): toolbar tooltips, dialogs, placeholders, aria-labels and the mandatory alt-text message |
| `tree.*`, `canteen.*`, `media.*`, `trash.*`, `history.*`, `activity.*`, `auth.*`, `dashboard.*`, `recorder.*` (named by this ADR) | the custom screens and components of ADR 0018 (tree, week editor), ADR 0015 (media library), ADR 0011 (trash, history, activity log), ADR 0006 (sign-in, two-factor set-up, password reset) and ADR 0019 (dashboard and release-1 screens), and the development-only clipboard recorder of [editor.md](../editor.md) §13.1 (`recorder.*`) |
| `enums.*` | option labels: colleague categories, dish courses, menu item kinds, post and page states, role and permission display names |
| `errors.<code>` | one message per stable error code the admin API returns (ADR 0021 §4), with named placeholders for its parameters |

`hu.ts` has no top-level key outside this table.

### 3. Which namespace a new key belongs to

The rows of section 2 decide where a key goes. This section says where the two new rows end and the existing ones begin; it changes none of the existing rows. It also does not say which existing key a component may call. ADR 0021 §3 lets backend3's own code call any key of `hu.ts`, and this ADR does not narrow that: the about page's heading calls `ra.message.about`, while the footer's link to the page has `app.footer.about`.

**backend3's own code.** A key that `hu.ts` lacks goes in the row that ADR 0021 §2 gives it: `resources.*` for a resource name or a field label, `editor.*` for the rich-text editor, the screen's namespace for a custom screen (for example `auth.*` for the sign-in screen of ADR 0006), `enums.*` for an option label and `errors.<code>` for a server error code. `app.*` takes only what none of these rows covers: the shell that backend3 adds around the screens and that no single screen owns, which is the title, the footer, the about page, the update prompt and the data provider's notice.

**The kit's rich-text block** stays outside `kit.*`. ADR 0021 §2 gives every string of the rich-text editor to `editor.*`, and the editor PR of ADR 0013 translates the block (ADR 0021, Follow-up work).

**Every other copied kit or shadcn/ui file.** These rules restate the `kit.*` row and the bootstrap's edits, which the two `LOCAL_CHANGES.md` files list. Take the first rule that fits:

1. The application title is `app.title`, also in the brand of the copied `app-sidebar.tsx`.
2. A string that `ra-language-english` has a key for uses that `ra.*` key.
3. Any other string is `kit.<component>.<name>`. The edit is listed in the `LOCAL_CHANGES.md` of its directory, like every other change to a kit file (ADR 0021 §3).

A string that ra-core computes for a kit component from another row, such as a field label from `resources.<resource>.fields.<field>`, is not a string of the kit file; it stays in its row.

**A fourth kit-only key.** `ra.*` keeps ADR 0021 §2's definition: the English pack plus the three kit-only keys that ADR 0021's Context lists. This ADR does not extend that list, so a fourth such key would need an amending ADR of its own. A kit update that starts calling another `ra.*` key that `ra-language-english` lacks therefore falls outside `ra.*`: defining the key there fails the stale-key test of `hu.test.ts`, and leaving it out fails key usage. Until an amending ADR extends the list, the kit file is edited to call a `kit.*` key instead, which rule 3 already covers.

A string that fits no row needs a namespace that the table lacks. An amending ADR adds it first, as this ADR does.

### 4. How the checks of ADR 0021 §7 apply

| Check | `app.*` | `kit.*` |
|---|---|---|
| Key parity | Does not apply: the check compares `ra.*` with the installed `ra-language-english`. | Does not apply, for the same reason. The stale-key test of `hu.test.ts` ("has no ra.* key that neither ra-core nor the kit uses") fails on an `ra.*` key outside the English pack and the three kit-only keys. That test is why the kit's own strings have a namespace of their own (section 3). |
| Key usage | Applies. Most of the shell translates with `useT()`, and `data-provider.ts` declares its key `satisfies MessageKey`, so `npm run typecheck` rejects an `app.*` key that `hu.ts` lacks there. `main.tsx` and `App.tsx` call `i18nProvider.translate("app.title")`, and the copied `app-sidebar.tsx` calls ra-core's untyped `translate("app.title")`; the literal scan of `key-usage.test.ts` covers those calls. | Applies. The copied kit files call ra-core's untyped `translate`, `<Translate i18nKey>` or pass a key in a prop. The literal scan of `key-usage.test.ts` covers them. |
| Resource coverage | Does not apply. | Does not apply. |
| No literals | Applies, as to every other `.tsx` file of backend3. | Applies to the copied kit and shadcn/ui files too. That is why their hard-coded English strings needed keys at all. A prop such as `label="kit.data_table.empty"` passes because it is shaped like a key. |
| Error codes | Does not apply. | Does not apply. |
| Smoke test | Applies: the missing-key report of `provider.ts` covers every namespace, and the end-to-end run fails on any fallback to English. | Applies, in the same way. |

The namespace list of `key-usage.test.ts` is the table of section 2, and the literal scan sees only the keys of the listed namespaces. A test in the same file, added with this ADR, fails when `hu.ts` has a top-level key that the list lacks. Its message says that a new namespace needs an amending ADR first. So `hu.ts` cannot gain a namespace that the scan skips. The list may name namespaces that `hu.ts` does not have yet, because the vertical slices add them (ADR 0021, Follow-up work). That the list matches the table is checked in review: a PR that adds a namespace to the table adds it to the list in the same PR.

The review of ADR 0021 §8 covers `app.*` and `kit.*` like every other namespace. The update prompt keeps the public site's wording, as ADR 0004 §5 requires; `hu.test.ts` pins it.

## Consequences

**Positive**

- `ra.*` keeps its exact definition, so the key-parity test and the stale-key test stay exact, with no list of exceptions.
- Every key of `hu.ts` belongs to a namespace in the table. ADR 0021 §2 is again the complete list, and the namespace list for the Hungarian review of §8 is this table. A test keeps `hu.ts` and the scan's namespace list in step.
- A kit update finds every kit string that backend3 translated in one namespace, next to the `LOCAL_CHANGES.md` files.
- The admin's behaviour does not change: the bootstrap's code already follows this ADR. Only the test of section 4, its helper with the helper's tests, and some comments change with it.

**Negative**

- The line between `app.*` and a screen namespace needs judgment. The existing rows come first (section 3), and a string that fits no row waits for an amending ADR.
- `kit.*` keys follow the kit's components. A kit update that renames or drops a component leaves its keys behind. No check finds an `app.*` or `kit.*` key that nothing uses, because the stale-key test covers `ra.*` only. The PR of the kit update removes those keys itself.
- The test of section 4 checks `hu.ts` against the namespace list, not the list against this table. Review keeps those two in step.

**Follow-up work, in the same PR as this ADR**

- ADR 0021's Status line, the [ADR index](README.md) and the index of the [design overview](../README.md#architecture-decision-records).
- The test of section 4 in `key-usage.test.ts`, with the tests of its helper.
- The module comment of `hu.ts` cites this ADR instead of calling `app.*` a namespace that no ADR covers. The comments of `key-usage.test.ts` and the kit section of `hu.ts` cite it too.
- `admin/CLAUDE.md` and the root `CLAUDE.md` name the two namespaces and point to this ADR.

**Later**

- The slice that finishes I18N-01 completes the §7 checks over the whole admin. It sends the namespace list of section 2 to the Hungarian review.

**What must be true in code and tests**

- `hu.ts` has no top-level key outside the table of section 2; the test of section 4 checks it against the namespace list of `key-usage.test.ts`.
- `ra.*` is exactly what ADR 0021 §2 defines; the tests of `hu.test.ts` keep checking it.
- The namespace list of `key-usage.test.ts` holds every namespace of the table.
- Every `kit.*` key is used by a copied kit or shadcn/ui file, and the edit that uses it is listed in that directory's `LOCAL_CHANGES.md`.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Accept the two namespaces as an editorial note under ADR 0001 §6 | register item D24 (research: DECISIONS R9) | Not chosen by the maintainer. An editorial fix may not change what a decision means (ADR 0001 §6), and two new namespaces change what the table allows. The table would also change without a record of the answer. |
| Move the strings: kit strings into `ra.*`, shell strings into `dashboard.*` and `auth.*` | register item D24 (research: DECISIONS R9) | Not chosen by the maintainer. `ra.*` would stop being exactly the English pack plus three keys, and the parity and stale-key tests would need a list of exceptions. The shell strings belong to no single screen. |
| One namespace for both kinds of string | not offered; considered while writing this ADR | `kit.*` strings come from upstream code and are checked again at every kit update; `app.*` strings are backend3's own. One namespace would hide which strings a kit update has to check. |
| Add a fourth kit-only key to `ra.*` when a kit update calls one | not offered; considered while writing this ADR | Not decided here. It would change ADR 0021 §2's definition of `ra.*`, which the maintainer's answer left as it is. An amending ADR can still extend the list of three when a kit update needs it (section 3). |

## References

- research: DECISIONS R9, register item D24 (answer of 2026-10-25); DECISIONS 40 (answer of 2026-10-24)
- research: QUESTIONS Q40
- `Verseghy/website_backend3#186` (the bootstrap, "Also for the maintainer"); `Verseghy/website_backend3#195` item 3; `Verseghy/website_backend3#213` (the clipboard recorder)
- `website_backend3@ad496da`: `admin/src/i18n/hu.ts`, `admin/src/i18n/hu.test.ts`, `admin/src/i18n/key-usage.test.ts`, `admin/src/i18n/keys.ts`, `admin/src/i18n/provider.ts`, `admin/eslint.config.js`, `admin/eslint-rules/no-ui-literals.mjs`, `admin/src/components/admin/LOCAL_CHANGES.md`, `admin/src/components/ui/LOCAL_CHANGES.md`
- `website_backend3@ad496da`, the users of `app.*`: `admin/src/main.tsx`, `admin/src/App.tsx`, `admin/src/components/admin/app-sidebar.tsx`, `admin/src/layout/AppFooter.tsx`, `admin/src/source-link/SourceLink.tsx`, `admin/src/about/AboutPage.tsx`, `admin/src/update-prompt/UpdatePrompt.tsx`, `admin/src/data-provider/data-provider.ts`
- `website_backend3@c2977ee`, the user of `recorder.*`: `admin/src/recorder/ClipboardRecorderPage.tsx`, with `admin/src/i18n/hu.ts` and `admin/src/i18n/key-usage.test.ts` at that commit
- npm: `ra-language-english` 5.15.5
- Amended: [ADR 0021](0021-hungarian-only-admin-ui.md) §2
- Related: [ADR 0001](0001-record-architecture-decisions.md) §6, [ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md) §5, [ADR 0006](0006-admin-authentication-and-account-lifecycle.md), [ADR 0012](0012-delivery-review-commits-and-licensing.md) §4, [ADR 0013](0013-rich-text-editor-and-html-sanitiser.md); [editor.md](../editor.md) §13.1; [parity.md](../parity.md) I18N-01
