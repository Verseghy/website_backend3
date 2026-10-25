@AGENTS.md

# Project rules

These rules win over generic advice, including the vendored Loco skill in `.claude/skills/loco/` (ADR 0024 section 2). They summarise decisions; the ADRs in [`docs/design/adr/`](docs/design/adr/README.md) are the source of truth, so read the linked ADR before you act on a rule. The repeated tasks have recipes in the project skill, [`.claude/skills/backend3/`](.claude/skills/backend3/SKILL.md). Work under `admin/` also follows `admin/CLAUDE.md`.

## The one override of Loco's doctrine

Loco's doctrine (`.claude/skills/loco/doctrine.md`) applies everywhere except in one narrow place: pure logic lives in `src/domain/` ([ADR 0008](docs/design/adr/0008-code-layout-a-pure-src-domain-beside-loco-models.md)). These are ADR 0008's six rules, verbatim; agents and reviewers check code against them:

1. **`src/domain/` is pure.** No I/O, no `AppContext`, no `async fn`. It does not read files, the network, the database, environment variables, the clock or a random number generator. "Now", "today" and random input are function arguments. Fixed facts, such as the school's time zone, are constants in `src/domain/` ([ADR 0010](docs/design/adr/0010-all-instants-stored-as-timestamptz.md)). Every public function has unit tests. A function has a non-trivial input space unless every argument is a boolean or a small closed enum. Strings, collections, dates and times, integers, and structs that contain any of them are always non-trivial. Every function with a non-trivial input space also has at least one proptest property. Only functions over booleans and small closed enums may rely on exhaustive unit tests instead.
2. **Code that talks to an external system takes explicit arguments.** External systems include the database, the uploads volume or storage driver, the legacy MySQL database and any HTTP service. Such code receives the connection or handle and a typed config struct as function arguments. It never reads `std::env`, never takes `&AppContext`, and never looks up a global registry.
3. **Only controllers, workers, tasks and initializers touch `AppContext`.** `src/app.rs` also does, for the `Hooks` wiring. They take apart `ctx.db`, `ctx.storage`, `ctx.cache` and the typed settings, and pass the pieces down explicitly.
4. **Models stay fat Active Record.** Finders live on `impl Model`. State transitions live on `impl ActiveModel`, consume `self` and return the saved `Model`. Field rules use `Validatable` and `before_save`. Model methods take the connection or transaction handle explicitly, as Loco's own finders do. They call into `src/domain/` for pure computation. Raw queries in controllers stay forbidden.
5. **No other new top-level directories.** No `services/`, `repositories/`, `utils/`, `helpers/`, `common/`, `shared/` or `lib/` under `src/`. External-system code that no model owns lives in the module of its only caller, for example `src/tasks/import_mysql/`. External-system code that several callers share and no Sea-ORM entity owns lives in `src/models/<name>.rs` as a model module without an entity, for example `src/models/media.rs`.
6. **Wire types live in `src/dtos/`** ([ADR 0005](docs/design/adr/0005-rust-dtos-are-the-api-contract-ts-rs-utoipa-openapi.md)). Domain types never derive ts-rs or utoipa, and DTOs convert to and from domain types at the controller boundary.

The test for where logic goes: if it needs the database or another external system, it belongs on a model or in its caller's module; otherwise it goes in `src/domain/`. backend3 renders no server-side HTML and sends no mail, so Loco's `src/views/` and `src/mailers/` are not used; where the skill says "render a view", backend3 converts to a DTO in `src/dtos/`.

## Configuration is passed as explicit arguments

- Settings live under `settings:` in `config/<env>.yaml` and deserialize once, at boot, into the typed `Settings` struct, which names the bad key in every error. Secrets come in through `get_env` in the YAML ([ADR 0008, "Typed configuration"](docs/design/adr/0008-code-layout-a-pure-src-domain-beside-loco-models.md#typed-configuration)).
- Code that talks to an external system takes its connection or handle and the typed settings sub-struct it needs as function arguments. It never reads `std::env`, never takes `&AppContext` and never uses a global registry (rules 2 and 3 above). `clippy.toml` bans the `std::env` readers and `tokio::spawn`.
- Only controllers, workers, tasks, initializers and `src/app.rs` read `ctx`, and they pass the pieces down.

## Commits and pull requests

[ADR 0012](docs/design/adr/0012-delivery-review-commits-and-licensing.md) sections 1-3 decide these.

- **Drafts.** Every PR opens as a draft. Mark it ready for review only when the work is complete and every check is green. Until the PR-check jobs are required contexts in Prow, Tide merges a ready PR without waiting for them, so the draft state (or `/hold`) is what keeps unfinished work off `master`.
- **Merging.** In this repository, agents merge their own PRs through Tide once the checks pass. Nothing is pushed to `master` directly, and the merge button and admin overrides are never used. In every other repository (website_k8s, website_backend2, website_frontend and the rest) agents open PRs and leave them for a human; they never merge there.
- **Shape.** One PR per vertical slice: migration, model, DTOs and handlers, the admin screen, the importer step, tests at each layer and docs together, about 500 hand-written lines or fewer. Branch from `master`; a PR may build on at most one other open PR. [ADR 0025](docs/design/adr/0025-delivery-exceptions-for-the-backend3-slice-plan.md) adds the slice classes that carry part of a feature (libraries, server-first pairs, public endpoints, importer steps, editor libraries, test-only slices) and the slice order.
- **Title.** Tide squash-merges, so the PR title becomes the commit subject. It must match `^(build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)\((api|admin|db|import|deploy|docs)(,(api|admin|db|import|deploy|docs))*\)!?: \S.*$`, in at most 90 characters, imperative, with no trailing period. Scopes: `api` (Rust application code and Rust dependencies), `admin` (everything under `admin/`), `db` (migrations), `import` (the legacy importer), `deploy` (`Containerfile`, `.tekton/`, `.github/workflows/`, build scripts, `.claude/settings.json` and hooks), `docs` (`docs/`, `README.md`, `THIRD_PARTY.md`, `AGENTS.md`, `CLAUDE.md` and the project skill). Example: `feat(db,api,admin): add labels with trash and history`.
- **Body.** Name the parity IDs and the ADRs the PR implements, so the final human review can follow it.
- **Attribution.** A `Co-Authored-By: Claude ... <noreply@anthropic.com>` trailer and a "Generated with Claude Code" line are allowed, not required.
- **No session links.** No claude.ai session link may appear anywhere in this public repository: not in commit messages, PR titles or bodies, issues, comments, code or docs. A session link is an http or https URL whose host is `claude.ai` and whose path starts with `/code/session_`. CI checks PR titles, bodies, commits and diffs (`No session links`); for issues and comments, this rule is the only guard.
- **Loco and Sea-ORM bumps** are dedicated PRs titled like `build(api): bump loco-rs to X.Y.Z`. They re-sync the vendored skill with `scripts/sync-loco-skill.sh`, read its diff (especially `doctrine.md` and `workflow.md`) and update every override in this file and in `.claude/skills/backend3/` that no longer holds ([ADR 0024](docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) section 3). Never edit `.claude/skills/loco/` by hand. Dependabot's version updates do not leave Loco's versions (axum, tower, tower-http, axum-test, tokio and the other crates `.github/dependabot.yml` lists), so a Loco bump moves those crates to the new Loco's versions itself and adjusts any rule whose reason no longer holds. Those rules list `update-types`, which Dependabot does not apply to security updates: a security PR that leaves Loco's versions is closed, and the advisory is fixed by a deliberate bump or by the Loco bump.

## Public-safety rules

This repository is public. Every file, commit, PR, issue and comment follows these rules ([ADR 0024](docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) section 1, [ADR 0001](docs/design/adr/0001-record-architecture-decisions.md) section 8); a violation is a defect:

1. No credentials, password hashes, tokens, keys, or values from any secret. The compose database's `loco`/`loco` are throwaway local values.
2. No description of live, unfixed weaknesses of a running system. Describe the target design instead ("backend3 enforces X").
3. No personal data: no names or e-mail addresses of editors or users, and no per-person facts. Aggregate counts are fine, and so are the public contact details the school site itself shows. Test fixtures are invented, never copied from production.
4. No claude.ai session links (above).

Also: no local paths (home directories, temporary or scratch directories) in committed files. A fact from the private research enters a public file only after it passes these rules, and is cited by section (for example "research: SYNTHESIS §4.12"); public code is cited as `<repo>@<commit>:<path>`.

## Parity issues

Each parity ID of [`docs/design/parity.md`](docs/design/parity.md) has one GitHub issue ([ADR 0019](docs/design/adr/0019-scope-parity-tracking-retired-features-release-1-additions.md) section 1). A PR in this repository that finishes an ID carries `Closes #N` in its body, one line per issue. A PR in another Verseghy repository that finishes one carries `Closes Verseghy/website_backend3#N`. What a closing PR must contain depends on the ID's disposition (ADR 0019 section 1). Never close a parity issue by hand; changing a disposition is a PR to `parity.md`.

## Guardrails

Three layers stop destructive commands ([ADR 0024](docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) section 4). Do not try to get around them; if one blocks legitimate work, stop and ask the maintainer.

1. **Deny rules** in `.claude/settings.json`: `cargo loco db reset`, `oc delete` of PVCs, PVs and Argo CD Applications, `--force --grace-period=0`, and `kubectl`.
2. **The hook** `.claude/hooks/guard-bash.sh` checks every Bash command, including `sh -c` wrappers and other spellings. It blocks kubectl; force deletion (`--force` on `oc delete` or `oc replace`, or with `--grace-period=0`); `oc delete` of a PVC, PV, Application or ApplicationSet, also when its arguments come from an `oc get` of one through `$(...)`, `xargs` or `-f -`; Loco's `db reset`, `db down`, `db truncate` and `db seed --reset` in every form (`cargo loco`, `backend3-cli`, inside `oc exec`, `oc rsh` or `oc debug`); `sea-orm-cli migrate fresh|refresh|reset|down`; and `cargo loco`, `cargo run`, `cargo test`, `cargo nextest`, `psql`, `pg_restore`, `mysql` and `sea-orm-cli` aimed at a database that is not local (localhost, 127.0.0.1, ::1 or a Unix socket), or run inside the cluster (in the command part of `oc exec`, `oc rsh` or `oc debug`, or in a script piped or fed to them). It fails closed when `jq` is missing. Its reason says what to do instead. Its tests are `scripts/tests/test-guard-bash.sh`.
3. **The code**: `App::load_config` refuses a non-local database in `test`, `development` and `e2e`, and the `dangerously_*` flags on a non-local host or in `production`; `src/bin/main.rs` refuses `db reset`, `db truncate` and `db seed --reset` in `production`.

Standing rules: use `oc`, never `kubectl`; never force-delete pods; never delete a PVC, PV or Argo CD Application. Schema changes are new forward migrations, applied with `cargo loco db migrate` to the local database. Personal `.claude/settings.local.json` files may add rules but cannot remove these.

## Hungarian-only admin UI

The admin UI is Hungarian only ([ADR 0021](docs/design/adr/0021-hungarian-only-admin-ui.md)). Every user-visible string comes from `admin/src/i18n/hu.ts` through `useTranslate` or `<Translate i18nKey>`; components contain no user-visible literals. Use the terminology of ADR 0021 section 6 (for example „bejegyzés”, „lomtár”, „étel”, never „menü” for a dish). The server sends stable error codes with `params`, and the admin shows the Hungarian `errors.<code>` message. Identifiers such as permission names stay in English (ADR 0021 section 6), and the server's English `description` is shown only for an unknown code (section 4).

## Where to look

- Always: [`docs/design/`](docs/design/README.md), the architecture overview and index, with the [ADRs](docs/design/adr/README.md), [`parity.md`](docs/design/parity.md), [`public-api.md`](docs/design/public-api.md), [`data-migration.md`](docs/design/data-migration.md), [`editor.md`](docs/design/editor.md) and [`cutover.md`](docs/design/cutover.md). They are enough to build and maintain backend3.
- If available: `verseghy-website-rewrite/` in the private Verseghy/runbooks repository, which holds the full research (`SYNTHESIS.md`, the research files, `QUESTIONS.md`, `DECISIONS.md`). It is background and evidence. Nothing from it enters this repository unless it passes the public-safety rules.

A PR that changes a rule, a command or the layout updates `AGENTS.md`, this file or the project skill in the same PR.
