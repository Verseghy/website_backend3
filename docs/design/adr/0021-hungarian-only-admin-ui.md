# 0021. Hungarian-only admin UI

- Status: Accepted (2026-10-24)
- Decision questions: Q40

## Context

The people who use the admin are Hungarian: staff publishers and secretaries, plus rotating student writers (40 accounts). The public site, the content and the editors' own instructions are Hungarian. backend1's admin is mostly English, and the toolkit backend3 builds on ships English only.

| Fact | Value | Source |
|---|---|---|
| Who edits | staff publishers and secretaries plus rotating student writers; 40 accounts, 4 roles | research: DECISIONS rows 6, 10a, 10d; research: SYNTHESIS §3.3.2 |
| backend1's language | English. Backpack's own strings plus English labels hard-coded in the controllers ("Featurable", "Brass", "Soup", "Meals", "Preview link:"). Hungarian appears only in six validation messages, the dashboard card "Képek" and the events date picker. `app.locale = en`. CKEditor follows the browser's language. | research: SYNTHESIS §2.2 (SHR-18), §3.13.1, §3.13.2 |
| A clash of words | backend1 calls the dishes "Menus" and the navigation "Menu", and its Hungarian canteen messages use „menü” for a dish („Legalább két menü hozzáadása szükséges”) | research: SYNTHESIS §2.8, §3.13.2; website_backend@0dead98:app/Http/Requests/CanteensRequest.php |
| Admin vs public wording | colleague categories are "Brass", "Teachers", "Lecturer", "Administrator", "Maintenance Staff" in the admin and „Vezetőség”, „Tanárok”, „Óraadók”, „Nevelő, oktató munkát közvetlenül segítők”, „Karbantartók, portások, takarítók” on the public site | research: SYNTHESIS §2.7 |
| Mechanism | ra-core's `i18nProvider`, typically `polyglotI18nProvider(locale => messages, defaultLocale, locales, options)` from `ra-i18n-polyglot`; resource and field labels come from `resources.<name>.name` and `resources.<name>.fields.<field>` keys; components translate with `useTranslate` and `<Translate i18nKey>` | research: SYNTHESIS §6.23; stack-loco-react-admin §3.6 |
| The kit's default | `polyglotI18nProvider(() => defaultMessages, "en", [{ name: "en", value: "English" }], { allowMissing: true })`, where `defaultMessages` is the default import of `ra-language-english` | shadcn-admin-kit@c1503dd:src/lib/i18nProvider.ts |
| Locale switcher | the kit's `LocalesMenuButton` renders nothing when only one language is configured | shadcn-admin-kit@c1503dd:src/components/admin/locales-menu-button.tsx |
| Community Hungarian pack | `ra-language-hungarian` 1.4.0, MIT, last published 2023-11-08. It has 145 of the 164 keys of `ra-language-english` 5.15.5. Missing: `ra.action.reset`, `ra.action.search_columns`, `ra.action.select_all_button`, `ra.action.update_application`, `ra.auth.email`, `ra.guesser.empty.message`, `ra.guesser.empty.title`, `ra.message.access_denied`, `ra.message.authentication_error`, `ra.message.placeholder_data_warning`, `ra.message.select_all_limit_reached`, `ra.navigation.clear_filters`, `ra.navigation.no_filtered_results`, `ra.notification.application_update_available`, `ra.notification.not_authorized`, `ra.notification.offline`, `ra.page.access_denied`, `ra.page.authentication_error`, `ra.validation.unique` | research: SYNTHESIS §6.1, §6.23 |
| What the kit uses | 63 distinct `ra.*` keys. 8 of them are missing from the Hungarian pack. 3 of those 8 (`ra.action.clear_search`, `ra.navigation.breadcrumb_drawer_title`, `ra.navigation.breadcrumb_drawer_instructions`) are not in the English pack either; the kit supplies inline English defaults. | research: SYNTHESIS §6.23 |
| The kit's rich-text block | 0 translation calls and at least 15 hard-coded English strings, for example the tooltip "Edit link", the placeholder "Enter URL", the aria-label "Text color", and "Copy Image" | research: SYNTHESIS §6.17 |
| Kit install model | the kit is a shadcn registry: its source files are copied into the project and become project code that we may edit (MIT) | research: SYNTHESIS §6.15 |
| Translation quality | nobody has reviewed the community pack's wording | research: SYNTHESIS §8 O60 |

## Decision

### 1. One language

The admin UI is Hungarian only. The page declares `<html lang="hu">`. One locale is configured, so the kit renders no language switcher. No English UI is designed, maintained or tested. English text appears only as the fallback for a missing Hungarian key, which is a defect that the checks in §7 stop before a release.

### 2. One complete message file

`admin/src/i18n/hu.ts` is the single source of every user-visible string in the admin. It is a TypeScript module that exports one message tree:

| Namespace | Contents |
|---|---|
| `ra.*` | every key of `ra-language-english` at the installed ra-core version (164 keys at 5.15.5), plus the three kit-only keys listed in Context |
| `resources.<resource>.name`, `resources.<resource>.fields.<field>` | every resource name and every field label, list column, filter, helper text and placeholder |
| `editor.*` | every string of the rich-text editor (ADR 0013): toolbar tooltips, dialogs, placeholders, aria-labels and the mandatory alt-text message |
| `tree.*`, `canteen.*`, `media.*`, `trash.*`, `history.*`, `activity.*`, `auth.*`, `dashboard.*` | the custom screens and components of ADR 0018 (tree, week editor), ADR 0015 (media library), ADR 0011 (trash, history, activity log), ADR 0006 (sign-in, two-factor set-up, password reset) and ADR 0019 (dashboard and release-1 screens) |
| `enums.*` | option labels: colleague categories, dish courses, menu item kinds, post and page states, role and permission display names |
| `errors.<code>` | one message per stable error code the admin API returns (§4), with named placeholders for its parameters |

The file may start from `ra-language-hungarian` 1.4.0. If it does, the copied strings keep their MIT terms in a licence file of their own, as decision 14 and ADR 0012 require for vendored code. `admin/src/i18n/LICENSE.ra-language-hungarian` holds a byte copy of the upstream repository's licence file at the matching version. The 1.4.0 npm tarball itself ships no licence file, only `index.js`, `package.json` (with `"license": "MIT"`) and `README.md`. If the upstream repository has none either, the file holds the standard MIT text with the copyright holder named in the package's `author` field, and the bootstrap PR says which source it used. `THIRD_PARTY.md` gets a row for `admin/src/i18n/hu.ts`: upstream URL, version 1.4.0, SPDX `MIT`, that licence file, modified: yes. Every inherited key is reviewed like a new one; the package is not a runtime dependency. Strings follow Hungarian typography: „…” for quotes (»…« inside them), an en dash for ranges, and singular nouns after numerals („3 bejegyzés”), so most count messages need a single form.

### 3. Wiring

- `admin/src/i18n/provider.ts` builds the provider with `polyglotI18nProvider`: default locale `hu`, one locale entry (`hu`, „Magyar”), and messages that are the English pack with the Hungarian tree laid over it (a deep merge). A key missing from `hu.ts` therefore shows English text instead of a raw key; that is the only job English has.
- In development and in tests a missing key is reported loudly. The deep merge means polyglot never sees a missing `ra.*` key (English answers instead), so its own missing-key handling cannot raise the alarm. `provider.ts` therefore wraps the provider's `translate`: outside production, it checks the key against the Hungarian tree of `hu.ts` itself, not the merged one, and logs `console.error` with the key before falling back to English. The CI checks of §7 (key parity, key usage, smoke test) turn the same gaps into failed tests. Production shows the English fallback silently.
- The kit's copied files under `admin/src/components/` are edited so that every hard-coded user-visible string goes through a translation key. The list of local changes to kit files sits next to the kit's notice (ADR 0012).
- backend3's own components never contain user-visible literals. They call `useTranslate()` or `<Translate i18nKey>` with a key from `hu.ts`.

### 4. Messages that come from the server

The admin API answers a refused or invalid request with one error envelope. This ADR defines it, because the Hungarian UI is its main reader. ADR 0005 §3 points here for admin error bodies, and [public-api.md](../public-api.md) §6.15 defines the public envelope `{ error, description }`.

```ts
{
  "error": ErrorCode,                          // stable code
  "description": string,                       // English free text
  "field": string | null,                      // the input the error belongs to
  "params": Record<string, string | number>    // values for the message placeholders
}
```

| Member | Rule |
|---|---|
| `error` | a stable code. A rule of one resource has a dotted `<resource>.<reason>` code, for example `post.published_locked`, `menu.max_depth`, `canteen.dish_count` (ADR 0018) or `search.term_too_long` (ADR 0020). A code that belongs to no resource is snake_case and keeps the public spelling where it means the same thing, for example `not_found`, or `invalid_credentials` and `too_many_attempts` (ADR 0023). The public API keeps its own snake_case codes ([public-api.md](../public-api.md) §8). |
| `description` | English text for script clients that use a Bearer token (ADR 0006) and for logs. The admin shows it only when it does not know the code. |
| `field` | the request field the error belongs to, spelt as the DTO serialises it (for example `parentId`), or `null` for an error on the whole request. The admin shows the message at that form input. |
| `params` | named values for the placeholders of the message, for example `{ "days": 12 }`; `{}` when there are none |

`ErrorCode` is a Rust enum in `src/dtos/admin/` whose variants serialise, through serde renames, to these strings. Like every admin DTO it derives `serde` and `ts_rs::TS` and is exported to `admin/src/bindings/` (ADR 0005 §2). The envelope itself is an admin DTO as well.

| Status | When |
|---|---|
| 401 | no valid session (ADR 0006, ADR 0007); the admin asks the user to sign in again without dropping an open form (ADR 0006) |
| 403 | a permission or record-level rule refuses the operation (ADR 0007), for example `post.published_locked` |
| 404 | `not_found` |
| 409 | the request conflicts with stored state: a uniqueness rule, a stale `updated_at` (ADR 0018 §4.4), a restore that collides with a live row, or rows that depend on the target (ADR 0011) |
| 422 | the submitted values fail validation, for example `post.date_required`, `menu.max_depth` or `canteen.dish_count` |
| 429 | sign-in is throttled, `too_many_attempts` (ADR 0023) |

The admin displays `errors.<code>` from the message file, filled with `params`, and falls back to the server's English `description` only if the code is unknown. This is how the "Hungarian message" of ADR 0007's 403 reaches the screen: the server sends the code, the admin renders the Hungarian text. The `errors` section of `hu.ts` is declared with `satisfies Record<ErrorCode, string>`, so adding a code on the server without a Hungarian message fails the admin type check.

### 5. Formats

| What | Format |
|---|---|
| Date | `2026. 10. 24.` via `Intl.DateTimeFormat('hu-HU')` (checked with Node.js on 2026-10-24, as were the other `Intl` examples here) |
| Date and time | `2026. 10. 24. 14:05`, 24-hour clock, shown in Europe/Budapest (ADR 0010) |
| Week | weeks start on Monday; the canteen editor shows ISO week numbers („44. hét”) |
| Numbers and sizes | `Intl.NumberFormat('hu-HU')`: `12 345,5` (four-digit numbers stay ungrouped, `1234,5`); file sizes as „12,4 MB” |
| Sorting in the browser | `Intl.Collator('hu')`, so client-side lists agree with the server's ICU `hu` order (ADR 0020) |
| Values sent to the API | always ISO 8601 and plain numbers, never the displayed form |

The display format of the kit's date and date-time inputs, which may follow the browser's own locale, is checked in the first slice that uses them; if they ignore `hu`, the slice wraps them.

### 6. Terminology

One term per concept, used on every screen and in every message. Other ADRs use these terms.

| Concept | Hungarian | Note |
|---|---|---|
| post | bejegyzés | |
| draft | piszkozat | |
| published (post or page) | közzétéve; actions „Közzététel” and „Közzététel visszavonása” | |
| featured | kiemelt | backend1 said "Featurable" |
| preview link | előnézeti link | |
| label | címke | |
| author | szerző | a by-line, not an admin user |
| colleague | munkatárs | categories exactly as the public site's headings (Context) |
| canteen | menza | |
| canteen day | menzanap | |
| week editor | heti étlap | |
| dish | étel | never „menü” |
| course | fogás: „Leves”, „Főétel”, „Desszert” | backend1 said "Soup", "Main dish", "Dessert" |
| event | esemény | |
| page | oldal | |
| hidden (page) | rejtett | |
| internal note | belső megjegyzés | |
| slug | URL-név | |
| SEO description | SEO-leírás | |
| navigation menu | Információk menü; an entry is a „menüpont”, a top-level entry with children a „menücsoport” | „menü” alone is avoided because of the dish clash |
| media library | médiatár | ADR 0015 |
| trash | lomtár; „Visszaállítás”, „Végleges törlés” | ADR 0011 |
| history | előzmények; one revision is a „változat” | ADR 0011 |
| activity log | tevékenységnapló | ADR 0011 |
| user | felhasználó; „Aktív”, „Aktiválás”, „Deaktiválás” | ADR 0006 |
| permission | jogosultság, with the labels proposed in ADR 0007 (for example „Bejegyzések közzététele”, „Menza szerkesztése”, and „Információk menü szerkesztése” for `edit menu`) | identifiers stay English; where ADR 0007's proposals and this table differ, this table wins |
| role | szerepkör | display names are settled in the message file and checked in the §8 review before go-live; editors' wording feedback arrives as issues after go-live (decision 42) |
| release-1 screens | „Értesítések”, „Partnerbannerek”, „Oldalbeállítások”, „Diáktoll-számok” | ADR 0019, labels as proposed in ADR 0007 |

### 7. Completeness checks

These run with the admin lint, type-check and unit tests in the PR checks (ADR 0022). Each one fails the PR.

| Check | What fails |
|---|---|
| Key parity | a key of the installed `ra-language-english` that `hu.ts` lacks (the test prints the missing keys) |
| Key usage | a key literal passed to `translate`, `useTranslate`, `<Translate>` or `i18nKey` anywhere in `admin/src` that `hu.ts` lacks. backend3's own code uses a typed key helper derived from `hu.ts`, so the TypeScript compiler rejects unknown keys there. |
| Resource coverage | a registered resource without `resources.<name>.name`, or a field listed in its resource definition without a label |
| No literals | a user-visible string literal in JSX text or in a `label`, `placeholder`, `title`, `aria-label` or `tooltip` prop outside `admin/src/i18n/` and test files |
| Error codes | an `ErrorCode` without an entry in `errors` (the type check of §4) |
| Smoke test | the end-to-end suite signs in and opens every list, create and edit screen in a mode where any fallback to English fails the run |

### 8. Review

Before go-live a Hungarian-speaking maintainer reads the whole file against the running screens. That closes O60 for the text that ships. After go-live, editors report wording problems as ordinary issues.

## Consequences

**Positive**

- Editors work in their own language, with the same words the public site uses.
- The dish and navigation clash disappears from the UI.
- Missing translations are caught by CI, not by editors.
- Server messages and UI labels come from one reviewed file.
- No second language doubles the work of every screen.

**Negative**

- One file covering everything will be large (well over a thousand lines) and a frequent source of merge conflicts between vertical-slice PRs; each slice adds only its own namespace to keep conflicts small.
- Editing the kit's copied files to translate them makes kit updates a manual merge.
- Script clients and developers reading raw API errors see English descriptions, not the Hungarian text.
- An English-only volunteer developer has to read Hungarian labels in the running UI; the keys stay English.

**Follow-up work**

- Bootstrap PR: `hu.ts` with the complete `ra.*` set and the kit-only keys, the provider, the key-parity, key-usage and no-literal checks.
- Each vertical slice adds its resources, enums and error codes to the file in the same PR.
- Editor PR (ADR 0013): translate every toolbar, dialog and aria string, including the kit block's.
- The bootstrap PR adds the envelope DTO, the `ErrorCode` enum and the `satisfies Record<ErrorCode, string>` declaration, with a unit test that pins the serialised envelope.

## Alternatives considered

- B. Hungarian by default with an English switch, both languages complete for every label: rejected; nobody would use English, and every label would cost two translations and two reviews.
- C. English UI with Hungarian validation messages, as backend1 does: rejected; it keeps the mixed-language UI and the English option labels that differ from the public site.
- Use `ra-language-hungarian` as a runtime dependency: rejected; it has not been updated since November 2023, lacks 19 keys of the current English pack and 8 keys the kit uses, and cannot hold backend3's own strings.

## References

- research: QUESTIONS Q40; DECISIONS rows 6, 10a, 10d, 14 (licence files of vendored code), 40, 42 (staging and go-live), 43a
- research: SYNTHESIS §2.2 (SHR-18), §2.7, §2.8, §3.3.2 (roles), §3.13 (backend1's strings), §6.1, §6.15, §6.17, §6.23 (kit, rich text, Hungarian pack), §8 O60
- research: stack-loco-react-admin §3.6 (i18n mechanism)
- website_backend@0dead98:app/Http/Requests/CanteensRequest.php, app/Http/Controllers/Admin/ColleaguesCrudController.php, app/Http/Controllers/Admin/MenusCrudController.php
- shadcn-admin-kit@c1503dd:src/lib/i18nProvider.ts, src/components/admin/locales-menu-button.tsx, src/components/rich-text-input/
- npm: `ra-language-english` 5.15.5, `ra-language-hungarian` 1.4.0 (tarball contents: `index.js`, `package.json`, `README.md`), `ra-i18n-polyglot` 5.15.5
- [public-api.md](../public-api.md) §6.15 (public error envelope) and §8 (public error codes)
- Related ADRs: [0005](0005-rust-dtos-are-the-api-contract-ts-rs-utoipa-openapi.md), [0006](0006-admin-authentication-and-account-lifecycle.md), [0007](0007-authorization-four-roles-enforced-on-every-operation.md), [0010](0010-all-instants-stored-as-timestamptz.md), [0011](0011-safety-net-soft-delete-revisions-and-an-activity-log-for-eve.md), [0012](0012-delivery-review-commits-and-licensing.md), [0013](0013-rich-text-editor-and-html-sanitiser.md), [0015](0015-media-separate-static-server-media-library-image-optimisatio.md), [0018](0018-content-model-publishing-pages-menu-tree-canteen-week-editor.md), [0019](0019-scope-parity-tracking-retired-features-release-1-additions.md), [0020](0020-search-and-hungarian-ordering.md), [0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md), [0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)
