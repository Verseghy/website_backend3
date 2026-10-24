# Local changes to shadcn-admin-kit

The files in this directory, plus `src/lib/utils.ts`, `src/lib/field.type.ts`
and `src/lib/i18nProvider.ts`, are copied from the shadcn-admin-kit registry,
version 2.0.0 (`https://marmelab.com/shadcn-admin-kit/r/v2.0.0/admin.json`,
built from tag v2.0.0, commit `11983abe4afc60a3904991820526aafe4d0588d3`), by
shadcn CLI 4.22.1. They are MIT-licensed; see `LICENSE` here and the
repository's `THIRD_PARTY.md`.

Copy-and-own is the kit's model (ADR 0012 §4). backend3 changed the files
below; every other file is byte-for-byte what the CLI installed. A kit update
re-adds the block and re-applies these changes.

| File | Change | Why |
|---|---|---|
| `app-sidebar.tsx` | The brand reads `app.title` instead of the hard-coded "Acme Inc."; the logo link, the dashboard link and the dashboard item's active match go through `useBasename()` | ADR 0021 §3; ra-core's `LinkBase` adds no basename, so `to="/"` left `/admin` (ADR 0004 §3, `src/router.tsx`) |
| `authentication.tsx` | The sign-in link goes through `useBasename()` | same basename reason |
| `list.tsx`, `create.tsx`, `edit.tsx`, `show.tsx` | The breadcrumb's dashboard link goes through `useBasename()`; the English children of `<Translate>` are removed | basename reason; every key exists in `hu.ts`, so the English fallback text is dead and would break the "No literals" check (ADR 0021 §7) |
| `autocomplete-input.tsx` | The search placeholder uses `ra.action.search`; the empty-result text uses `kit.autocomplete.no_match` | ADR 0021 §3 |
| `breadcrumb.tsx` | The drawer trigger's `aria-label` uses `kit.breadcrumb.show_path`; the English children of `<Translate>` are removed | ADR 0021 §3 |
| `bulk-actions-toolbar.tsx`, `cancel-button.tsx`, `list-pagination.tsx`, `user-menu.tsx` | The English children of `<Translate>` are removed | ADR 0021 §7 |
| `columns-button.tsx` | The clear button's `aria-label` uses `ra.action.clear_input_value`; "Reset" uses `ra.action.reset` | ADR 0021 §3 |
| `data-table.tsx` | "No results found." uses `kit.data_table.empty` | ADR 0021 §3 |
| `theme-mode-toggle.tsx` | The screen-reader label and the three choices use `ra.action.toggle_theme` and `kit.theme.*` | ADR 0021 §3 |
| `error.tsx` | A chunk-load error shows the update prompt (`src/update-prompt/`) instead of the generic error; the development-only block of links to the kit's documentation, StackOverflow and the paid Enterprise Edition is removed | ADR 0004 §5; the links are untranslated vendor help that editors never need |
| `login-page.tsx` | The demo side panel (the "Acme Inc" logo, the testimonial) and the demo credentials hint are removed; the heading, labels and button use `ra.auth.*` | ADR 0021 §3; the sign-in screen itself is designed in the authentication slice (ADR 0006) |
| `ready.tsx` | The text uses `kit.ready.*`; the links to the kit's documentation, demo and repository are removed | ADR 0021 §3 |
| `layout.tsx` | The column element is a `<div>` instead of `<main>`; `<main>` now wraps only the page content (with the error boundary), and a new optional `footer` prop renders after it | A `<footer>` inside `<main>` is a generic element, not the contentinfo landmark (HTML-AAM), so backend3's page footer (`src/layout/AppFooter.tsx`, with the AGPL-3.0 §13 source link, ADR 0012 §4) needs a place outside `<main>` |

Configuration, not code, also matters: `src/App.tsx` passes
`disableTelemetry` to `<Admin>`, because `admin.tsx` otherwise loads a
tracking image from marmelab's server in production builds, which the admin
CSP (`img-src 'self'`, ADR 0023 §6) forbids.
