# Third-party code in this repository

website_backend3 is licensed `AGPL-3.0-only` (see [LICENSE](LICENSE)). A few
paths hold source that was copied in from other projects. They keep their own
licence, and their licence file sits next to them. This file indexes every
such path, as [ADR 0012](docs/design/adr/0012-delivery-review-commits-and-licensing.md)
section 4 requires. A PR that adds or re-syncs vendored code updates this file
in the same PR.

Dependencies that cargo or npm fetch are not vendored, so they are not listed
here. CI checks their licences instead: `cargo-deny` against `deny.toml`, and
the `Admin licenses` job against `admin/license-policy.json`
([ADR 0022](docs/design/adr/0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md)
section 4).

## Vendored code

| Path | Upstream | Version or commit | SPDX | Licence file | Modified |
|---|---|---|---|---|---|
| `.claude/skills/loco/` (17 files) | `skills/loco/` of the [`loco-rs`](https://crates.io/crates/loco-rs/1.2.0) crate on crates.io ([source](https://github.com/loco-rs/loco)) | 1.2.0, the version locked in `Cargo.lock` (checksum `5386baf405736a2620001f4561b00fa4874dae99eac3ccf01713fabc9ef13af4`) | `Apache-2.0` | `.claude/skills/loco/LICENSE`, a byte copy of the crate's `LICENSE` (the crate has no `NOTICE` file) | No. Kept verbatim; `scripts/sync-loco-skill.sh --check` and the `Loco skill` CI job enforce it ([ADR 0024](docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) section 3) |
| `admin/src/components/admin/` (86 files) | [shadcn-admin-kit](https://github.com/marmelab/shadcn-admin-kit) registry block `https://marmelab.com/shadcn-admin-kit/r/v2.0.0/admin.json` | v2.0.0, built from tag `v2.0.0` at commit `11983abe4afc60a3904991820526aafe4d0588d3` | `MIT` (Copyright (c) 2025 marmelab) | `admin/src/components/admin/LICENSE`, a byte copy of the kit's `LICENSE` at v2.0.0 | Yes, 19 files; each change is listed in `admin/src/components/admin/LOCAL_CHANGES.md` |
| `admin/src/lib/utils.ts`, `admin/src/lib/field.type.ts`, `admin/src/lib/i18nProvider.ts` | shadcn-admin-kit, the same registry block (the kit's `src/lib/` helpers) | v2.0.0, commit `11983abe4afc60a3904991820526aafe4d0588d3` | `MIT` (Copyright (c) 2025 marmelab) | `admin/src/components/admin/LICENSE` | No |
| `admin/AGENTS.md` | shadcn-admin-kit `rules/AGENTS.md` (the registry's `~/AGENTS.md` target, which the shadcn CLI resolves to the project directory) | v2.0.0, commit `11983abe4afc60a3904991820526aafe4d0588d3` | `MIT` (Copyright (c) 2025 marmelab) | `admin/src/components/admin/LICENSE` | No. Kept as the registry installs it; overrides go into `admin/CLAUDE.md` ([ADR 0024](docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) section 2) |
| `admin/src/components/ui/` (29 files) | [shadcn/ui](https://github.com/shadcn-ui/ui) primitives, style `base-vega`, installed from the shadcn registry as dependencies of the kit block | shadcn-ui/ui default branch at commit `e55b60b1014cccfb9ac62f97e0d20b5b649f4a54` (2026-10-24); the registry is not versioned | `MIT` (Copyright (c) 2023 shadcn) | `admin/src/components/ui/LICENSE`, a byte copy of shadcn-ui/ui's `LICENSE.md` | Yes, 5 files; each change is listed in `admin/src/components/ui/LOCAL_CHANGES.md` |
| `admin/src/hooks/use-mobile.ts` | shadcn/ui registry, style `base-vega` (a dependency of the sidebar primitive) | commit `e55b60b1014cccfb9ac62f97e0d20b5b649f4a54` (2026-10-24) | `MIT` (Copyright (c) 2023 shadcn) | `admin/src/components/ui/LICENSE` | No |
| `admin/src/index.css` | shadcn/ui `templates/vite-app/src/index.css`, plus the theme variables and imports the CLI writes for the `base-vega` style | commit `e55b60b1014cccfb9ac62f97e0d20b5b649f4a54` (2026-10-24) | `MIT` (Copyright (c) 2023 shadcn) | `admin/src/components/ui/LICENSE` | No |
| `admin/src/i18n/hu.ts` (the inherited `ra.*` strings) | [ra-language-hungarian](https://github.com/phelion/ra-language-hungarian) | 1.4.0, commit `043a984f32c1d5ec7170033fb2f4d22bad1be86a` (its `index.js` equals the npm tarball's, whose integrity matches npm's `dist.integrity`) | `MIT` | `admin/src/i18n/LICENSE.ra-language-hungarian` | Yes. The inherited strings were reviewed one by one and adapted, and backend3's own keys were added ([ADR 0021](docs/design/adr/0021-hungarian-only-admin-ui.md)) |

Notes:

- **The ra-language-hungarian licence file.** Neither the 1.4.0 npm tarball
  nor the upstream repository ships a licence file; the README says the
  translation is MIT-licensed and `package.json` says `"license": "MIT"`. As
  ADR 0021 section 1 directs, `LICENSE.ra-language-hungarian` therefore holds
  the standard MIT text with the copyright holder named in the package's
  `author` field and the years of its npm publish history.
- **How the admin was installed.** shadcn CLI 4.22.1 (with `@shadcn/registry`
  0.2.0), Node.js 24, from the repository root with a throwaway `HOME`:
  `shadcn init --template vite --preset vega --name admin https://marmelab.com/shadcn-admin-kit/r/v2.0.0/admin.json`.
  The registry's `~/AGENTS.md` target resolved to `admin/AGENTS.md`; the root
  `AGENTS.md` and the home directory were left untouched
  ([ADR 0004](docs/design/adr/0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md),
  O57). `admin/CLAUDE.md` describes how to re-run the CLI safely.
- **The admin scaffold.** The Vite project files that the CLI copied from
  shadcn/ui's `templates/vite-app` at commit
  `e55b60b1014cccfb9ac62f97e0d20b5b649f4a54` (MIT, Copyright (c) 2023 shadcn,
  `admin/src/components/ui/LICENSE`) are configuration boilerplate:
  `admin/.prettierrc` (unchanged), `admin/components.json` (written by the
  CLI), and `admin/eslint.config.js`, `admin/index.html`, `admin/tsconfig.json`,
  `admin/tsconfig.app.json`, `admin/tsconfig.node.json`, `admin/vite.config.ts`,
  `admin/.gitignore`, `admin/.prettierignore` and `admin/package.json`
  (all changed for backend3). The template's `src/main.tsx`, `src/App.tsx` and
  `README.md` were replaced by backend3's own files.

## Generated starting points

`loco new` 1.2.0 (the [`loco`](https://crates.io/crates/loco/1.2.0) CLI crate,
`Apache-2.0`, whose `LICENSE` is byte-identical to `.claude/skills/loco/LICENSE`)
generated the first version of the Rust application from its `base_template/`:
`loco new --name backend3 --db postgres --bg queue-postgres --assets none`.
backend3 then removed what it does not use and rewrote the rest. These files
are generator output that backend3 owns and changes freely; they are listed so
the origin of the text that remains is clear.

| Path | From the template | Modified |
|---|---|---|
| `AGENTS.md` | `base_template/AGENTS.md`; its sections "Read this first", "The three rules that prevent most mistakes" and "More" keep Loco's text | Yes: extended for backend3 as [ADR 0024](docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) section 2 requires, and its gate replaced by CI's |
| `.cargo/config.toml` | `base_template/.cargo/config.toml.t` (the `loco` and `loco-tool` aliases) | Yes: comments added |
| `Cargo.toml`, `src/`, `migration/`, `config/`, `tests/` | `base_template/` (the app skeleton) | Yes: rewritten; the generated auth controller, users model, mailers, views, demo worker and fixtures were removed |
