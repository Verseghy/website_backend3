# Local changes to the shadcn/ui primitives

The files in this directory, plus `src/hooks/use-mobile.ts`, are shadcn/ui
primitives in the `base-vega` style, installed by shadcn CLI 4.22.1 on
2026-10-24 as registry dependencies of the shadcn-admin-kit 2.0.0 block (the
`shadcn-ui/ui` default branch was at commit
`e55b60b1014cccfb9ac62f97e0d20b5b649f4a54` then). They are MIT-licensed; see
`LICENSE` here and the repository's `THIRD_PARTY.md`.

backend3 changed the files below; every other file is byte-for-byte what the
CLI installed. Each change routes a hard-coded English string through
`src/i18n/hu.ts` (ADR 0021 §3, the "No literals" check of §7), using ra-core's
`useTranslate` or `<Translate>`.

| File | Strings now translated |
|---|---|
| `breadcrumb.tsx` | `aria-label` "breadcrumb" (`kit.breadcrumb.label`), screen-reader "More" (`kit.breadcrumb.more`) |
| `dialog.tsx` | both "Close" labels (`ra.action.close`) |
| `pagination.tsx` | `aria-label` "pagination" (`kit.pagination.label`), "Go to previous/next page" (`ra.navigation.previous`, `ra.navigation.next`), the default "Previous"/"Next" texts (`kit.pagination.previous`, `kit.pagination.next`), screen-reader "More pages" (`kit.pagination.more`) |
| `sheet.tsx` | screen-reader "Close" (`ra.action.close`) |
| `sidebar.tsx` | the mobile sheet's title and description (`kit.sidebar.title`, `kit.sidebar.description`), "Toggle Sidebar" on the trigger and the rail (`kit.sidebar.toggle`) |
