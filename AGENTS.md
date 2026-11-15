# Agent guide for this Loco app

This is a [Loco](https://loco.rs) app — **Rails for Rust**. When you are unsure
how something should work here, the answer is almost always "the way Rails does
it." Where Loco diverges, it is because Rust forced it.

## What backend3 is

website_backend3 is the backend of the Verseghy Ferenc Gimnázium website. It
replaces the Laravel admin (website_backend) and the read-only GraphQL API
(website_backend2) with one Loco 1.2.0 application on PostgreSQL that serves:

- the admin API at `/api/admin`;
- the admin UI at `/admin`, a React app (ra-core with shadcn-admin-kit) built
  from `admin/` and served by backend3 from `admin/dist`;
- the public read API at `/api/v1`, which the public website uses.

The design is recorded before the code, in [`docs/design/`](docs/design/README.md):
the architecture overview, the ADRs in [`docs/design/adr/`](docs/design/adr/README.md),
the parity checklist, the public API contract and the data-migration rules.
Read the ADRs that cover your change before you write it. When code and an ADR
disagree, the ADR wins; changing a decision needs a new ADR. Project rules that
override generic Loco advice (for example `src/domain/`, below) are decided in
the ADRs and summarised in `CLAUDE.md`.

## Read this first

A complete Loco skill ships with this app at **`.claude/skills/loco/`**, matched
to the exact `loco-rs` version in `Cargo.toml`:

| File | What it gives you |
|---|---|
| `.claude/skills/loco/SKILL.md` | start here — the router, `AppContext`, project layout, CLI |
| `.claude/skills/loco/doctrine.md` | what good Loco code looks like; read before writing any |
| `.claude/skills/loco/api-index.md` | every public `loco_rs` symbol, generated from rustdoc — **check here before guessing an API name** |
| `.claude/skills/loco/recipes/` | how to add a model, endpoint, worker, task, mailer, middleware, auth, tests |

If your tool supports Agent Skills, it will load `SKILL.md` automatically. If
not, read it directly — it is a normal markdown file.

The skill is vendored verbatim from the locked `loco-rs` crate and is never
edited: `scripts/sync-loco-skill.sh` re-syncs it, and CI fails when it drifts
(ADR 0024 section 3). backend3's own recipes are in `.claude/skills/backend3/`.

## The three rules that prevent most mistakes

1. **Generate, then edit.** `cargo loco generate <thing>` writes the file *and*
   the wiring. Rust has no autoloading; hand-wiring is how "the handler exists
   but 404s" happens.
2. **Use the batteries.** This app already has an ORM, queue, scheduler, mailer,
   task runner, storage, cache, and test harness. Adding a crate for something
   Loco already does is the most common mistake.
3. **Fat model, slim controller.** Domain logic on the model; handlers parse,
   call a model method, and render.

## Layout

| Path | What lives there |
|---|---|
| `src/app.rs` | `Hooks`: routes, workers, tasks, initializers, the `load_config` checks (the database guard, the typed settings, the session's `auth.jwt` and, in production, the refusal of every published key), the app clock, the one source of "now" for the rules that compare with it (`clock(&ctx)`, recipe 7 of `.claude/skills/backend3/SKILL.md`; stored audit instants come from PostgreSQL's clock, its step 6), the metrics registry (`after_context`, `after_routes`), the layer that drops a client's `x-request-id` so that Loco makes every request's id (`after_routes`; research: DECISIONS R23 (D119)), the fallback `/storage` router (`after_routes`, when `settings.media.serve_storage` is on), the upload handles (`after_context`: the media settings and the one image optimiser permit per process), the handles of the `DeleteOwnedFiles` worker (`after_context`: the media settings and the checked file-owner registry), the session limits of the sliding refresh (`after_context`: the absolute limit of ADR 0006 §8), the admin origin (`after_context`: `settings.admin_auth.origin`, which the admin route table's guard compares with the `Origin` of a cookie-authenticated write, ADR 0044), the middleware stack (`middlewares`: Loco's default, with the `304` CORS middleware of `controllers::v1::caching` right outside `etag`), and `serve`, which trims trailing slashes before routing and runs the `/metrics` listener next to the HTTP server |
| `src/bin/main.rs` | the `backend3-cli` binary (`cargo loco` runs it), with the CLI pre-check, which refuses the data-wiping `db` commands, every `db seed` and `doctor --config` in production |
| `src/controllers/` | thin HTTP handlers: `admin_ui.rs` (`/admin`); `metrics.rs` (`/metrics` on its own port and the HTTP request metrics) with `metrics/runtime.rs` (the pool, queue and process families); `not_found.rs`, the one answer for unknown paths; `request_id.rs`, the layer that drops a client's `x-request-id`; `storage.rs`, the fallback `/storage` router of ADR 0015 §4 (`ServeDir` on `media.root` with the media contract's guards and headers, outside Loco's middleware), which `App::after_routes` mounts only when `settings.media.serve_storage` is on; `admin/` (`/api/admin`): `admin/routes.rs`, the admin route table, which declares every admin route with its requirement (a permission, the `admin` role, any active user, or public; ADR 0007 §5) and is the one builder of the admin router: its guard answers `403 origin_mismatch` to a write that carries the session cookie (or the two-factor cookie) without exactly the admin origin in its `Origin` header (`admin/origin.rs`, ADR 0044), then `401` and then `403 forbidden`, all before the handler runs, and adds the passive sliding refresh of the session cookie to the answer (ADR 0006 §1), every response is `Cache-Control: no-store`, and every route but an upload route is under the 4 MiB JSON body limit; `serving` marks the routes that each row of the access table stands for, which its tests hold to the row (recipe 10 of `.claude/skills/backend3/SKILL.md`); so far `admin/auth.rs` (the sign-in, `/auth/me` and the logout, with their activity entries, and the explicit refresh `/auth/refresh`), `admin/session.rs`, which opens the session for the guard, reading the account and the roles and extra grants it holds on every request, builds the renewed session cookie of the sliding refresh from the session limits that `App::after_context` stores (`SessionLimits`), and holds the `AdminSession` extractor that hands the session to every handler for a signed-in account, `admin/client_context.rs`, the `ClientContext` extractor of the request id and the client address that an account entry records, and `admin/media.rs`, which holds what every upload endpoint shares (the upload handles, the one-file-part reader and the answer to every upload error); `v1/` (`/api/v1`), whose `routes()` in `v1/mod.rs` is the one builder of every public route and adds the CORS layer and the error caching headers, `v1/caching.rs`, the caching headers of every public answer (the `200` helpers with the weak `ETag`, and the middleware that keeps the CORS header on a `304`), and so far `v1/openapi.rs`, the OpenAPI document that every public endpoint joins; and the other admin and public endpoints as the slices add them |
| `src/dtos/` | wire types (serde, ts-rs, utoipa); the API contract (ADR 0005). `admin/` and `v1/` per API; `shared.rs` holds what both APIs send, the calendar day `CalendarDate` (`YYYY-MM-DD`) and the event wall time `EventWallTime` (`YYYY-MM-DD HH:MM:SS`), which convert to and from the stored instant through `domain::budapest` (ADR 0010) |
| `src/models/` | fat Active Record models; `_entities/` is generated by `cargo loco db entities` and never edited; `media.rs` is the uploads volume (ADR 0015 §6, §7), a model module without an entity: an upload streamed into `media.tmp_dir`, optimised one at a time and renamed into place under `media.root`; `owned_files.rs`, also without an entity, tells whether a row or a revision still refers to an owned file and removes the files nothing refers to (ADR 0011 §6) |
| `src/domain/` | pure logic only: no I/O, no `AppContext`, no `async fn`, no clock or randomness (ADR 0008); `authz.rs` holds the 16 permissions, the 4 roles and the role-to-permission mapping, which no table stores (ADR 0007 §3), the `Requirement` of an admin route (§5), and the access table `RESOURCE_ACCESS` (§4), one row per ra-core resource, from which `access_map` and `session_access_map` compute the access map that `/auth/me` sends (§7); `origin.rs` holds the `Origin` rule of cookie-authenticated admin writes (ADR 0006 §3, ADR 0044); `session.rs` holds the admin session check and the sliding refresh's expiry arithmetic (ADR 0006 §1, §8: the passive threshold, the two-hour renewal, the absolute limit) |
| `src/workers/`, `src/tasks/` | background workers and tasks (ADR 0016); `src/workers/delete_owned_files.rs` is the `DeleteOwnedFiles` worker, which removes the owned files that a purge left unreferenced (below, "Queue workers"); `src/tasks/import_mysql/` is the legacy import (ADR 0017, `docs/design/data-migration.md`), built only with the default Cargo feature `import-mysql`, which also brings its MySQL driver; its pure parts, in `src/domain/import_mysql/`, are built in every build |
| `migration/` | Sea-ORM migrations (ADR 0009) |
| `config/` | `development`, `test`, `e2e` and `production` configuration; typed settings under `settings:`, among them `settings.admin_auth` (ADR 0006 §3, §6, §8: the TOTP key, the absolute session limit, the credential lifetimes and the admin origin, written out in every file; the admin origin is what a cookie-authenticated admin write must carry in its `Origin` header, `http://localhost:5150` locally, with development's port following `PORT`, and `HOST` in production, ADR 0044) and `settings.media` (ADR 0015 §3), whose local directories are under the git-ignored `target/media/<env>/`, and whose `serve_storage` is on only in `e2e` |
| `data/` | versioned data that tasks read in the pod; the image ships it as `/app/data/` (ADR 0045: the `Containerfile`'s runtime stage, which `tests/image_data/` checks, since GitHub Actions never builds the image). `data/import/` (`/app/data/import/`) holds the legacy import's defect resolution list, `resolutions.toml`, the default of the task's `resolutions` argument (data-migration.md §8.2; schema and validation in `src/domain/import_mysql/resolutions.rs`), and a README that says what may be committed there: ids, dates, choices and source values only, never names or content. `data/seed/` (`/app/data/seed/`) is where the release-1 seed files go (data-migration.md §13 item 7) |
| `tests/` | integration, request and architecture tests (`tests/architecture.rs`, with the SQL date lint of ADR 0010 point 10 in `tests/architecture/sql_dates.rs` and the lint of ADR 0007 §5 that only the admin route table builds `/api/admin` routes in `tests/architecture/admin_mounts.rs`), the `/storage` media contract suite (`tests/media_contract/`), the schema snapshot (`tests/schema/`, ADR 0009), the tz-data parity test of backend3's and PostgreSQL's Budapest midnights, with the snapshot of backend3's (`tests/tz_parity/`, ADR 0041), the expected DDL of the legacy MySQL database (`tests/fixtures/legacy/schema.sql`), which compose.yaml's MySQL source loads, the access case table `tests/fixtures/authz_cases.json` (ADR 0007 §7), which `tests/domain/authz_cases.rs` and the admin's `src/auth/authz-cases.test.ts` both run, and the shared helpers of `tests/support/`, among them the safety-net checks that every admin-editable entity runs on its write methods (`tests/support/safety_net.rs`, ADR 0011; recipe 2 step 9 of the project skill) and the probe tables, accounts and session tokens of the admin route table's tests (`tests/support/admin_probe.rs`; the table's own test is `tests/requests/admin_routes.rs`, recipe 10, and its `Origin` check's is `tests/requests/admin_origin.rs`), with `ADMIN_ORIGIN`, the `Origin` that a request test sends with every write that carries the session cookie, and the session tokens placed on the app clock's timeline for the tests of the sliding refresh (`tests/support/session_refresh.rs`, recipe 7 step 4). `tests/support/` also holds the `/api/v1` test harness (public-api.md §16, §17): the named fixtures with their soft-deleted twins (`fixtures.rs`), the response check against the served OpenAPI document (`openapi_schema.rs`), and the route samples (`public_samples.rs`) and rules (`public_rules.rs`) of the global suite `tests/requests/public_global.rs`; recipe 3 of `.claude/skills/backend3/SKILL.md` says what each endpoint adds to them |
| `examples/media_contract.rs` | runs that suite from the command line against a running server (ADR 0015 §3, the co-mount test) |
| `deploy/media/` | the media server: its nginx configuration, which website_k8s copies without its comment lines (Verseghy/website_k8s#83), and its pinned image (`deploy/media/README.md`, ADR 0015 §1) |
| `admin/` | the admin UI: an npm project (ADR 0004, ADR 0021) |
| `docs/design/` | the design: overview, ADRs, specs |
| `about.toml`, `about.hbs` | cargo-about's configuration and template for the licence notices of the crates in the binary, which `scripts/crate-notices.sh` writes into the image's `/app/licenses/` and checks on every PR (ADR 0042) |
| `scripts/` | repository scripts; their tests are `scripts/tests/test-*.sh` |

## Commands

The throwaway local PostgreSQL 18 runs from `compose.yaml` on `127.0.0.1:5432`
(user `loco`, password `loco`; local values, not secrets). Its data lives in a
tmpfs, so every start begins empty. CI starts the same file, so CI, compose
and production share the image family, the major and the locale: the official
`postgres` image at PostgreSQL 18
(website_k8s@f0286dc:verseghy-website/postgres/kustomization.yaml), which is
what ADR 0020 asks of the tests, initialised with the builtin `C.UTF-8` locale
(`--locale-provider=builtin --locale=C.UTF-8`,
website_k8s@f0286dc:verseghy-website/postgres/deployment.yaml; research:
DECISIONS R13). `compose.yaml` passes those arguments in
`POSTGRES_INITDB_ARGS`, and the CI `Test` job fails unless `template1` reports
that locale. The minor may differ: production's Argo CD Image Updater follows
the 18 minor releases by itself, while `compose.yaml` pins its tag and digest
and nothing bumps them automatically, so a compose bump is a manual change.
The image also brings PostgreSQL's tz data, which the tz-data parity test
compares with backend3's: a compose bump records the image's tz data version
in the comment of `compose.yaml`, states it in its PR and passes that test,
and before production moves to an image whose tz data differs from
compose.yaml's, compose.yaml follows it to that tag (ADR 0041 §3). The test
reads the version from the image's tzdata.zi with
`pg_read_file`, which needs a superuser or a member of `pg_read_server_files`;
compose's `loco` is the image's bootstrap superuser, so a local database
with a lesser test role fails that check.

A container keeps the environment it was created with. Run in the checkout that
created it, `podman-compose up -d postgres` (or `docker compose up -d
postgres`) sees that `compose.yaml` changed the service and recreates the
container, so it picks up a change such as the locale arguments on its own;
`podman-compose up -d --force-recreate postgres` recreates it in any case.
`podman start`, `podman-compose start` or `restart`, and `up --no-recreate`
reuse the old container: one created before the locale arguments keeps the
image's default `en_US.utf8`, although the tmpfs makes initdb run again at
every start.

```sh
podman-compose up -d postgres     # local PostgreSQL with backend3_test, backend3_development and backend3_e2e
podman-compose --profile import up -d mysql  # the legacy import's MySQL source (below); the tests need it
cargo loco db migrate             # LOCO_ENV defaults to development
cargo loco start                  # http://localhost:5150; health check: GET /_ping
curl http://127.0.0.1:9464/metrics  # the metrics port of the running server (below)
cargo loco generate <thing> ...   # generate, then edit (the Loco skill lists the generators)
cargo loco db entities            # after a migration: regenerate src/models/_entities/
```

`cargo loco db entities` and `cargo loco generate model` run `sea-orm-cli`,
which the build does not install. Install the version that matches the locked
`sea-orm` (2.0.4 in `Cargo.lock`) once: `cargo install sea-orm-cli@2.0.4 --locked`.
The guardrail hook checks `sea-orm-cli` commands like the other database
commands (ADR 0024 section 4).

Do not run `cargo loco db create`: loco-rs 1.2.0 connects to the configured
database before it runs any subcommand, so `db create` fails with "database
... does not exist". compose.yaml creates all three databases instead.

Local database URLs: `postgres://loco:loco@localhost:5432/backend3_development`,
`.../backend3_test` and `.../backend3_e2e`. Tests, development and E2E refuse
any database that is not local.

The legacy import (`import_mysql`, data-migration.md) reads a MySQL source.
Locally and in the CI `Test` job that is the `mysql` service of
`compose.yaml`, under the profile `import` (so a plain `podman-compose up -d`
leaves it out): production's MySQL 8.4 image, pinned like the PostgreSQL one,
loaded at every start with the legacy schema of
`tests/fixtures/legacy/schema.sql` and no rows, on `127.0.0.1:3306`. Its
account `loco`/`loco` (throwaway values, not secrets) has exactly the
cut-over source account's grants, `SELECT` and `LOCK TABLES` on `verseghy`.
The import_mysql tests read it, so the test suite needs it running; a test
that cannot reach it fails and says how to start it. Its URL is
`mysql://loco:loco@127.0.0.1:3306/verseghy`; set it as
`IMPORT_SOURCE_DATABASE_URL` for a local `cargo loco task import_mysql`.
In `test`, `development` and `e2e` the task refuses a source that is not
local (localhost, 127.0.0.1, ::1 or a Unix socket), the container-network
name `mysql` included (ADR 0024 §4, layer 3), and it reads only the legacy
tables of its allow-list, never the newsletter's subscriber list
(`src/domain/import_mysql/source_tables.rs`, NWS-01). It reads them in one
locked session (`src/tasks/import_mysql/session.rs`, data-migration.md
§3.3): `LOCK TABLES ... READ` over the allow-list only, the schema
fingerprint against `src/tasks/import_mysql/expected_schema.rs`, every
table's `AUTO_INCREMENT` and the refusal of a `snapshot_at` earlier than
the latest timestamp in the source. The whole run is one transaction on
the target, which `dry-run` rolls back and `apply` commits, and it first
refuses a target whose `roles` or `user_permissions` CHECK differ from the
code, or in which a table it writes holds a row (`src/tasks/import_mysql/target.rs`,
data-migration.md §4.2; the invariant below). Before any of that, and before
the task lock, it reads the resolution file of its `resolutions` argument
(by default `/app/data/import/resolutions.toml`, the image's copy of
`data/import/resolutions.toml`; give `resolutions:data/import/resolutions.toml`
for a local run) and refuses one that fails validation, naming the entry and
the field of every problem (`src/domain/import_mysql/resolutions.rs`,
data-migration.md §8.2). Each table step detects its defects from the
source rows on every run with the generic `DefectDetector`
(`src/domain/import_mysql/defects.rs`, ADR 0017 §3, ADR 0047): `apply`
stops at the first instance that the list does not cover or whose entry's
precondition does not hold, while a dry run lists every one, goes on with
the defect's proposed default and exits non-zero at its end. How a
defect's entries work is fixed per defect (`DefectId::entry_policy`, ADR
0047 §4): D10's are optional (an unlisted page is published), and a D3
entry pins the index image that editors set in the source. Every run
writes `report.md`, the editors' sign-off sheet `signoff.md` (ids, live-site
paths and proposed resolutions only; `src/domain/import_mysql/signoff.rs`)
and `summary.json` into the report directory.

Uploaded media go below `settings.media` (ADR 0015 §3): locally
`target/media/<env>/public` (the served tree), `tmp` (upload staging) and
`trash`, all git-ignored. Create the root once,
`mkdir -p target/media/development/public` (or `e2e`): without it every
upload is refused before its body is read, as in production before the
volume is mounted; the staging directory and the entity image folders are
created on the first upload.
Tests make a media volume of their own below `target/tmp`
(`tests/support::TempMedia`). Request bodies may be 128 MiB
(`server.middlewares.limit_payload`), the admin API's JSON bodies 4 MiB.

To start the development database over, recreate it: `podman-compose down`,
then `podman-compose up -d postgres`, which starts all three databases empty
(the tmpfs), then `cargo loco db migrate`. Never use `db truncate` or
`db reset` for this; the guardrail hook blocks both. `App::truncate` never
empties `users`, `activity_log` or `revisions`: the two history tables are
append-only and reject `TRUNCATE` (ADR 0011 §5), and they reference
`users`, so none of the three can be truncated on its own. It leaves the
accounts' `user_roles` and `user_permissions` with them, and `roles`, which
only the migration seeds.

The admin UI (`admin/`, Node.js LTS major in `admin/.node-version`):

```sh
cd admin
npm ci                 # install exactly what package-lock.json pins
npm run dev            # Vite dev server; proxies /api to http://localhost:5150
npm run build          # writes admin/dist, which backend3 serves at /admin, with its licence notices (licenses.txt, ADR 0038)
npm run lint           # ESLint
npm run typecheck      # tsc -b --noEmit, which also checks the generated bindings
npm test               # Vitest unit tests (vitest run)
npm run licenses       # licence check of the production dependencies
npm run clean-room     # no @react-admin/ (Enterprise) package in package.json or the lockfile
npm run e2e            # Playwright against http://localhost:5150: the admin in Chromium, /storage in three engines
```

`npm run e2e` expects backend3 to be running already with `LOCO_ENV=e2e`,
serving the built `admin/dist`
(`VITE_REPORT_MISSING_KEYS=1 VITE_CLIPBOARD_RECORDER=1 npm run build`, as in
CI: the first makes any missing Hungarian string fail the run, the second adds
the development-only clipboard recorder that its E2E test drives; then, from
the repository root, `LOCO_ENV=e2e cargo loco start`, which migrates backend3_e2e
on boot, and `LOCO_ENV=e2e cargo loco db seed`, which creates the seed account
below). Every admin page but the sign-in screen needs a session, so the E2E
tests sign in with that account (`signIn` in `admin/e2e/fixtures.ts`).

The E2E server also serves `/storage`, from `target/media/e2e/public`, with
the fallback router (`config/e2e.yaml`): it stands in for the backend3 host,
which has no media server beside it here. `admin/e2e/storage.spec.ts` writes
its files into that directory and opens them in Chromium, Firefox and WebKit
(editor.md §13 P9, §16.1), so the browsers come first:
`npx playwright install --with-deps chromium firefox webkit` (CI's E2E job
does the same). Playwright's WebKit needs Ubuntu's libraries; on another
distribution, run the `storage-webkit` project inside the official Playwright
image of the locked version (`mcr.microsoft.com/playwright:v<version>-noble`)
with the host network.

### The seed account

`cargo loco db seed` (`Hooks::seed` in `src/app.rs`) creates one invented
account, `SEED_ACCOUNT` in `src/domain/seed.rs`: „Próba Adminisztrátor”,
`seed-admin@example.org`, password `public-throwaway-seed-password`, with the
`admin` role and so every permission (ADR 0007 §2). To sign in locally, run
`cargo loco db seed` once against the development database, then
`cargo loco start` and `npm run dev`, and sign in with it. The password is
published on purpose and protects nothing: no hash of it is committed (the
seed hashes it with Argon2id when it runs), and only the local development,
test and e2e databases ever hold the account. A second seed leaves an
existing account with that address exactly as it is, its roles included, so
an account seeded before the roles existed keeps none; to start over,
recreate the database. `admin/e2e/seed-account.ts` holds the same values for
the E2E suite, and `tests/seed/mod.rs` fails when the two differ.

Public-safety rule 1 allows the password in three files only: this one,
`src/domain/seed.rs` and `admin/e2e/seed-account.ts` (ADR 0040 §5, which
amends ADR 0034 §2; research: DECISIONS R24 (D120)). Anywhere else, a test or
a design doc included, name it by `SEED_ACCOUNT` and do not copy it. The
exception holds only while production refuses every seed, through the two
refusals below: a change that would let a seed reach production or staging
needs the maintainer's answer first.

The seed never reaches production or staging, which both run
`LOCO_ENV=production`. Production holds no account before the cut-over
import, which refuses to run once `users` holds a row (data-migration.md
§4.2), and its first accounts come only from that import (ADR 0006 §4). A
production seed would leave a known account there and block the import
(ADR 0040; research: DECISIONS R9 (D10)). Two refusals keep it out:

- the CLI pre-check (`src/domain/cli_guard.rs`) refuses every `db seed` in
  production, with or without `--reset`, `--from`, `--dump` or
  `--dump-tables`, before anything loads or connects (`--dump` would write
  every table, password hashes included, to files);
- `Hooks::seed` itself refuses every environment but development, test and
  e2e (`domain::seed::check_environment`), whatever started it.

### The pre-import invariant

Nothing writes a row into production's `users`, `activity_log`,
`revisions` or any other table that the cut-over import or the cut-over
seed writes before the cut-over import runs; staging, which also runs
`LOCO_ENV=production`, holds the same until its rehearsal import. The
cut-over seed is the release-1 seed of ADR 0019 §5 (the partner banners,
the site settings and one Diáktoll issue), which runs after the import
(data-migration.md §13, open item 7). The maintainer accepted this
invariant on 2026-10-25 (research: DECISIONS R9 (D10)).

The import relies on it for the tables it writes: it never merges into
existing data, so it refuses a target in which any of them holds a row,
and names the first (data-migration.md §4.2; `IMPORT_TARGET_TABLES` in
`src/domain/import_mysql/target.rs`, which lists them with the entity
tables of data-migration.md §6). `roles` is the one exception: its
migration seeds it, and the import only reads it. The seed's own tables
are not on that list, because the import does not write them; the
invariant keeps them empty all the same.

So code that could write such a row in production refuses to before the
import, or never runs there: every `db seed` is refused in production (the
two refusals above), and a change that would let anything else write one
there (an account, an activity entry, a revision, a seed record) needs a
guard of its own in the same PR. To start a database over, recreate it
(the commands above); never empty these tables in place.

## Rust dependencies

Every requirement of the workspace is declared once, in
`[workspace.dependencies]` of the root `Cargo.toml`, with a comment that says
why it is there. The crate that uses it inherits it with
`<crate> = { workspace = true }`, adding only the `features` or the
`optional = true` that this crate alone needs. Dependabot rewrites no other
table of the root manifest, so a requirement in backend3's own dependency
tables would get every update beyond it as a Cargo.lock-only PR that fails
every `--locked` job. `migration/` inherits as well, so that each requirement
and its reason live in one place and the two crates cannot drift apart. Write
the workspace entry with bare keys, `<crate> = "<requirement>"` or
`<crate> = { version = "<requirement>", ... }`: Dependabot does not find a
dotted or quoted key. `scripts/ci/check-workspace-deps.sh` (the `Toolchain`
job) rejects a requirement declared anywhere else, or written in a form that
Dependabot does not find, and says how to fix it. The steps are recipe 5 of
`.claude/skills/backend3/SKILL.md`.

Every image ships the licence notices of the crates in the binary
(`/app/licenses/`, ADR 0042). `scripts/crate-notices.sh` writes them with
cargo-about in the Containerfile's rust stage, and the required
`cargo-deny check bans licenses sources` job runs it on every PR, so a
dependency change can fail it: when a crate's own licence file would give
way to the standard text of its licence, or a file that `about.toml`
clarifies has changed, the script prints the file's SHA-256 for its
clarification there; read the file before you add or renew one. Run it
locally with `bash scripts/crate-notices.sh --out target/licenses` (into a
missing or empty directory). It needs the cargo-about version that the
Containerfile pins (`ARG CARGO_ABOUT_VERSION`): its Linux musl release
binary, or `cargo install cargo-about@<version> --locked --features cli`.

## Queue workers

Background jobs run on Loco's Postgres queue, inside the web process
(`start --server-and-worker`, the image's default command), never with
`tokio::spawn`. The test and e2e configurations run them inline
(`workers.mode: ForegroundBlocking`), so `perform_later` returns the job's
own outcome (ADR 0016 §3). Every worker keeps the rules of ADR 0016 §3:
idempotent, small arguments, a failure returned as an error and never a
retry loop. Recipe 9 of `.claude/skills/backend3/SKILL.md` adds one.

The development configuration queues jobs as production does
(`workers.mode: BackgroundQueue`), and a plain `cargo loco start` serves
HTTP only: its jobs wait in `pg_loco_queue` and never run, so a local
purge leaves its files in place. Run `cargo loco start --server-and-worker`
locally to serve and run jobs in one process, as the image does, or run
`cargo loco start --worker` beside the server; `cargo loco jobs dump
--status queued --folder target/jobs` shows what waits.

- **File jobs.** ADR 0016 §3 has a job record its outcome on the row it
  works on. A job whose work is files has no such row: `DeleteOwnedFiles`
  runs after a purge removed its row, and the media trash purge and the
  sitemap job, when they come, work on files that no row describes. Their
  outcome is the state of the files: the unreferenced files are gone, the
  trash keeps nothing older than 30 days, the sitemap is written. A file job
  that fails leaves orphans, files that nothing refers to, which ADR 0011 §6
  counts as harmless, and never a row that refers to a missing file. It
  returns an error, Loco 1.2.0 marks the job `failed` with that text
  (`jobs dump --status failed`), and the recovery is the second one that
  ADR 0016 §3 names: once the cause is fixed (the volume mounted, a folder
  that is a symbolic link replaced), an operator runs `backend3-cli jobs
  retry` (or `jobs retry --id <job id>`) in a backend3 pod, which mounts the
  volume. That is an operator procedure of the private operations
  runbook, never an automatic step. The job is idempotent, so the retry is
  safe.
- **Owned files.** An owned file is an entity image: an author's or a
  colleague's photo, a post's index or gallery image. Its entity's row
  stores its value, and only that row, live or trashed, and the entity's
  revisions refer to it; media-library files under `uploads/` are never
  owned (ADR 0011 §6). `DeleteOwnedFiles` (`src/workers/delete_owned_files.rs`)
  takes the files' paths below `media.root` and nothing else, and removes a
  file only when no row and no revision snapshot holds its stored value. A
  replaced or cleared image therefore stays while a revision shows it, and
  goes with its record's purge (parity SHR-15, MEDIA-01). It checks a file
  against the columns of the file-owner registry, `FILE_OWNERS` in
  `src/domain/owned_files.rs`, which every upload slice joins with one line
  for its column, and it refuses a path whose folder has no registered
  owner. A purge collects its record's files from the row and from every
  revision with `models::owned_files::files_of_record::<S>`, where `S` is
  the entity's revision snapshot type: it reads each registered column
  from the row and from the snapshot member of the same name, so `S::FIELDS`
  lists every registered column under its own name, and it refuses a type
  that does not. It enqueues the job with `commit_then_enqueue`, which
  commits the purge's transaction first (recipe 2 step 7 of the project
  skill).

## Metrics

backend3 serves `/metrics`, in the OpenMetrics text format, on a port of its
own: `settings.metrics` (`binding`, an IP address, and `port`, which is never
`server.port`; 9464 in every committed configuration, `METRICS_PORT` in
development). `App::serve` runs that listener and the HTTP server together
with `tokio::try_join!`, never with `tokio::spawn`, and stops both on one
signal (ADR 0023 §9, ADR 0008).

- **The metrics port is never routed.** No Route, Ingress or proxy rule
  targets it, in any environment, and the application router has no
  `/metrics` route: on the HTTP port the path is an unknown path like any
  other. Only the hub's monitoring may reach the port, by a path that its own
  ADR designs (ADR 0014, ADR 0023 §9). `tests/requests/metrics.rs` asserts
  the HTTP-port side.
- **One registry per application context, never a global one.**
  `App::after_context` creates the `prometheus_client` registry, registers
  every metric family on it and stores `controllers::metrics::Metrics` in
  the shared store. Code that records a metric gets its family handle
  passed down from there. A new family is registered in that hook.
- **Scrape-time families.** Values that are read rather than recorded (the
  database pool, the job queue in `pg_loco_queue`, the process from
  `/proc/self`) are sampled by `RuntimeMetrics::refresh` at the start of
  every scrape and written by a collector on the same registry
  (`src/controllers/metrics/runtime.rs`). A source that cannot be read
  leaves its families out of that scrape and never fails it, so a missing
  family means "could not be read". A database read uses the application's
  pool, so it gets a server-side `statement_timeout` shorter than the
  scrape's own timeout (a dropped query keeps its connection until the
  server answers), and only one runs at a time.
  The queue query lives in `src/models/job_queue.rs`, because raw queries
  stay out of controllers (ADR 0008 rule 4).
- **Bounded labels.** Label values come from sets the code bounds, never
  from raw client input: the matched route template, not the request path;
  the nine standard methods or `other`; the status code, or 499 when the
  connection closed first (`src/domain/http_metrics.rs`); the five job
  states Loco writes or `other` (`src/domain/runtime_metrics.rs`).
- **The names and the port are settled.** Asked to confirm the metric
  names (`http_server_request_duration_seconds` with 499,
  `db_client_connections{state}`, `loco_queue_jobs{state}`,
  `loco_queue_oldest_queued_job_age_seconds` and the standard `process_*`
  names) and port 9464, the maintainer answered "Confirm as built" on
  2026-10-25 (research: DECISIONS R15, D85). The hub-monitoring ADR that
  ADR 0023 plans (follow-up 5) will build its alerts on them, so a rename
  or another port is the maintainer's decision.
- **The snapshot.** A PR that adds or renames a family updates the
  metric-name snapshot in `tests/requests/metrics.rs`.

## Before you call it done

Run the same gate as CI, from the repository root:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
podman-compose up -d postgres
podman-compose --profile import up -d mysql  # the import's MySQL source
TZ=UTC cargo nextest run --locked --workspace --all-targets
TZ=Pacific/Chatham cargo nextest run --locked --workspace --all-targets  # a second process TZ (ADR 0010)
# without the import-mysql feature (data-migration.md §3.3): no MySQL driver in the build, and the rest passes
cargo tree --locked --workspace --no-default-features --target all --edges normal,build,dev --invert sqlx-mysql  # prints nothing
TZ=UTC cargo nextest run --locked --workspace --all-targets --no-default-features
cargo test --locked --workspace --doc     # nextest does not run doctests
# the media contract against deploy/media/nginx.conf in its pinned image (docker or podman):
cargo nextest run --locked --workspace --all-targets --run-ignored only -E 'test(/^media_contract::nginx::/)'
bash scripts/tests/run-all.sh             # the shell-script tests
bash scripts/sync-loco-skill.sh --check   # the vendored Loco skill is unchanged
rm -rf target/licenses && bash scripts/crate-notices.sh --out target/licenses  # the crates' licence notices (ADR 0042)
(cd admin && npm run lint && npm run typecheck && npm test && npm run licenses && npm run clean-room)
```

A change to `admin/` or to the user flows also runs `npm run e2e` as described
above. A change to a DTO regenerates `admin/src/bindings/` and the OpenAPI
snapshot, and commits them (ADR 0005). A change to the rich-text policy
(`src/domain/rich_text/`) regenerates the admin's copy of it with
`cargo loco task rich_text_manifest` and commits `admin/src/rich-text/policy.json`
and `css-vectors.jsonl` (editor.md §3.9); the `Contract` job fails on either
otherwise. A change to the editor (`admin/src/rich-text/`), to its fixtures
or to the sanitiser's regression fixtures (`tests/fixtures/regressions/`),
which the editor loads too, rewrites the editor's golden files,
`tests/fixtures/e1/`, when `npm test` (or `npm run test:e1`) runs in
`admin/`; commit them too (editor.md §13.6, P2 and P3): the `Contract` job
fails when they differ from what the editor writes, and
`tests/rich_text/golden.rs` fails when the server would change one. A
migration updates the schema
snapshot, `tests/schema/snapshots/schema.snap`, in the same PR (ADR 0009):
the test writes a `.snap.new` beside it; read the diff, accept it with
`cargo insta review` (or rename the file), and commit it with the migration.
Before it compares, the test runs the guard of `tests/schema/guard.rs`, which
fails on anything in the schema that the snapshot does not show (a view, a
grant, a policy, a storage parameter, a disabled foreign-key trigger, ...),
within the limits that its module documentation lists, and names it; recipe 1
of the project skill says what to do then.

## More

- Design: [`docs/design/README.md`](docs/design/README.md) and the [ADR index](docs/design/adr/README.md)
- Loco docs: <https://loco.rs/docs/> (they describe the latest release; the vendored skill matches the version this app locks)
- Framework agent guide: <https://loco.rs/AGENTS.md>

This file starts from the `AGENTS.md` that `loco new` 1.2.0 generates
(Apache-2.0) and was changed for backend3; see [`THIRD_PARTY.md`](THIRD_PARTY.md).
