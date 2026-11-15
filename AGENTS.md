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
| `src/app.rs` | `Hooks`: routes, workers, tasks, initializers, the `load_config` checks (the database guard, the typed settings, the session's `auth.jwt` and, in production, the refusal of every published key), the app clock, the one source of "now" for the rules that compare with it (`clock(&ctx)`, recipe 7 of `.claude/skills/backend3/SKILL.md`; stored audit instants come from PostgreSQL's clock, its step 6), the metrics registry (`after_context`, `after_routes`), the upload handles (`after_context`: the media settings and the one image optimiser permit per process), the middleware stack (`middlewares`: Loco's default, with the `304` CORS middleware of `controllers::v1::caching` right outside `etag`), and `serve`, which trims trailing slashes before routing and runs the `/metrics` listener next to the HTTP server |
| `src/bin/main.rs` | the `backend3-cli` binary (`cargo loco` runs it), with the CLI pre-check, which refuses the data-wiping `db` commands, every `db seed` and `doctor --config` in production |
| `src/controllers/` | thin HTTP handlers: `admin_ui.rs` (`/admin`); `metrics.rs` (`/metrics` on its own port and the HTTP request metrics) with `metrics/runtime.rs` (the pool, queue and process families); `not_found.rs`, the one answer for unknown paths; `admin/` (`/api/admin`), whose `routes()` mounts every admin handler under the `Cache-Control: no-store` layer, the JSON handlers also under the 4 MiB JSON body limit, so far `admin/auth.rs` (the sign-in, `/auth/me` and the logout), `admin/session.rs`, the `AdminSession` extractor that every handler for a signed-in account takes, which reads the account and the roles and extra grants it holds on every request, and `admin/media.rs`, which holds what every upload endpoint shares (the upload handles, the one-file-part reader and the answer to every upload error); `v1/` (`/api/v1`), whose `routes()` in `v1/mod.rs` is the one builder of every public route and adds the CORS layer and the error caching headers, `v1/caching.rs`, the caching headers of every public answer (the `200` helpers with the weak `ETag`, and the middleware that keeps the CORS header on a `304`), and so far `v1/openapi.rs`, the OpenAPI document that every public endpoint joins; and the other admin and public endpoints as the slices add them |
| `src/dtos/` | wire types (serde, ts-rs, utoipa); the API contract (ADR 0005). `admin/` and `v1/` per API; `shared.rs` holds what both APIs send, the calendar day `CalendarDate` (`YYYY-MM-DD`) and the event wall time `EventWallTime` (`YYYY-MM-DD HH:MM:SS`), which convert to and from the stored instant through `domain::budapest` (ADR 0010) |
| `src/models/` | fat Active Record models; `_entities/` is generated by `cargo loco db entities` and never edited; `media.rs` is the uploads volume (ADR 0015 §6, §7), a model module without an entity: an upload streamed into `media.tmp_dir`, optimised one at a time and renamed into place under `media.root` |
| `src/domain/` | pure logic only: no I/O, no `AppContext`, no `async fn`, no clock or randomness (ADR 0008); `authz.rs` holds the 16 permissions, the 4 roles and the role-to-permission mapping, which no table stores (ADR 0007 §3) |
| `src/workers/`, `src/tasks/` | background workers and tasks (ADR 0016); `src/tasks/import_mysql/` is the legacy import (ADR 0017, `docs/design/data-migration.md`), built only with the default Cargo feature `import-mysql`, which also brings its MySQL driver; its pure parts, in `src/domain/import_mysql/`, are built in every build |
| `migration/` | Sea-ORM migrations (ADR 0009) |
| `config/` | `development`, `test`, `e2e` and `production` configuration; typed settings under `settings:`, among them `settings.media` (ADR 0015 §3), whose local directories are under the git-ignored `target/media/<env>/` |
| `tests/` | integration, request and architecture tests (`tests/architecture.rs`, with the SQL date lint of ADR 0010 point 10 in `tests/architecture/sql_dates.rs`), the `/storage` media contract suite (`tests/media_contract/`), the schema snapshot (`tests/schema/`, ADR 0009), the tz-data parity test of backend3's and PostgreSQL's Budapest midnights, with the snapshot of backend3's (`tests/tz_parity/`, ADR 0041), the expected DDL of the legacy MySQL database (`tests/fixtures/legacy/schema.sql`), which compose.yaml's MySQL source loads, and the shared helpers of `tests/support/`, among them the safety-net checks that every admin-editable entity runs on its write methods (`tests/support/safety_net.rs`, ADR 0011; recipe 2 step 9 of the project skill). `tests/support/` also holds the `/api/v1` test harness (public-api.md §16, §17): the named fixtures with their soft-deleted twins (`fixtures.rs`), the response check against the served OpenAPI document (`openapi_schema.rs`), and the route samples (`public_samples.rs`) and rules (`public_rules.rs`) of the global suite `tests/requests/public_global.rs`; recipe 3 of `.claude/skills/backend3/SKILL.md` says what each endpoint adds to them |
| `examples/media_contract.rs` | runs that suite from the command line against a running server (ADR 0015 §3, the co-mount test) |
| `deploy/media/` | the media server: its nginx configuration, which website_k8s copies byte for byte, and its pinned image (`deploy/media/README.md`, ADR 0015 §1) |
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
(`src/domain/import_mysql/source_tables.rs`, NWS-01).

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
npm run e2e            # Playwright (Chromium) against http://localhost:5150/admin
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
otherwise. A migration updates the schema
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
