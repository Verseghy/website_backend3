---
name: backend3
description: Use when adding a feature to website_backend3 — a vertical slice, a new admin-editable entity (soft delete, revisions, activity log), a public /api/v1 endpoint with its test fixtures, a step of the legacy MySQL importer, a Rust dependency, a Loco task, or code that needs the current time. Gives backend3's own recipes on top of the vendored Loco skill, with the ADRs and file paths each step follows.
---

# backend3 recipes

These recipes are backend3's way of doing its seven repeated tasks. They sit on
top of the vendored Loco skill (`.claude/skills/loco/`, which says how Loco
works) and under the project rules in `CLAUDE.md` (which win where the two
disagree). The ADRs in `docs/design/adr/` are the source of truth: each step
names the ADR that decides it, and when a step and its ADR disagree, the ADR
wins and this file is fixed in the same PR.

Before any recipe:

- Find the parity IDs the work finishes in `docs/design/parity.md` and their
  issue numbers on the "website_backend3 parity" board (ADR 0019 section 1).
- Read the ADRs the work touches. The overview `docs/design/README.md` maps
  features to ADRs.
- Start the local database: `podman-compose up -d postgres`, which creates
  backend3_test, backend3_development and backend3_e2e, then run
  `cargo loco db migrate` (never `db create`: loco-rs 1.2.0 connects to the
  configured database before it runs any subcommand, so it fails on a missing
  database).

---

## 1. Add a vertical slice

A slice carries one feature end to end in one PR (ADR 0012 section 1). ADR 0025
adds six classes of slices that carry part of a feature (library and shared
infrastructure, server-first pairs, public endpoints, importer steps, editor and
paste libraries, test-only); the slice plan names each slice's class and the
slice it feeds. A slice that needs a new crate adds it with recipe 5.

1. **Branch** from `master` (a PR may build on at most one other open PR) and
   open the PR as a **draft** right away.
2. **Schema**: a migration in `migration/src/` (`cargo loco generate migration`
   or `generate model`), following ADR 0009 (naming, `bigint` identity ids,
   `timestamptz`, `CHECK`s, an index on every foreign key) and ADR 0010 for
   instants. SQL that takes a date, hour, month or year from a
   `timestamptz`, in a migration or a model, names the school's zone (ADR
   0010 points 4 and 10): `(date AT TIME ZONE 'Europe/Budapest')::date`, or
   `date_trunc('month', starts_at, 'Europe/Budapest')`, or a zone bound as a
   parameter from `budapest::ZONE_NAME`. The SQL date lint,
   `tests/architecture/sql_dates.rs`, fails on a cast to `date` (`::date`,
   `CAST(... AS date)`, `date(x)`), a `date_trunc(` or a `current_date` that
   names no zone or another zone, such as `'UTC'`, in a string literal or
   `.sql` file under `src/` or `migration/`. Its module documentation lists
   what it cannot see and what stays a review rule (the hour, `extract`,
   `to_char`). If the entity is admin-editable, it joins the
   safety net in this PR: recipe 2. The slice adds only its own tables to
   `App::truncate` in `src/app.rs`, never `users`, `activity_log` or
   `revisions` (the `AGENTS.md` commands say why and how to start a
   development database over). Every migration updates the schema snapshot
   in the same PR (ADR 0009): `tests/schema/snapshot.rs` reads the migrated
   schema from `pg_catalog` and fails until
   `tests/schema/snapshots/schema.snap` matches. Read the `.snap.new` it
   writes (each object is one line that starts with
   its kind and name), accept it with `cargo insta review`, and commit it.
   The snapshot shows only what the module doc of `tests/schema/catalog.rs`
   lists. Before it compares, the test runs the guard of
   `tests/schema/guard.rs`, which fails on anything else in the schema (a
   view, a sequence, a grant, a policy, a storage parameter, a comment on an
   index, a disabled foreign-key trigger, ...), within the limits that its
   module documentation lists, and names each object and the rule that found
   it. A migration
   that needs such an object, in the same PR: extends that reader to show it,
   narrows the guard rule so it no longer reports it, changes the guard case
   in `tests/schema/guard_cases.rs` that expected it, and updates the
   snapshot. A new extension goes into the guard's `EXTENSIONS` with every
   object it owns.
3. **Entities**: `cargo loco db migrate`, then `cargo loco db entities`. Never
   edit `src/models/_entities/`; `src/models/mod.rs` allows `missing_docs` for
   it, so the generated code needs no doc comments. The entities derive no
   serde (`[package.metadata.db.entity]` in `Cargo.toml`): responses and
   request bodies are DTOs (ADR 0005), and fixtures and seeds are built
   through the models, because Loco's YAML `db::seed` needs serde on the
   entity. Both `db entities` and `generate model` shell out to
   `sea-orm-cli`, which is not part of the build: install the version that
   matches the locked `sea-orm` once, with
   `cargo install sea-orm-cli@2.0.4 --locked` (the `AGENTS.md` commands). The
   first run for a table also writes `src/models/<name>.rs`, whose generated
   `before_save` reads the system time: change it as recipe 7 step 5 says.
   The `users` model's `before_save` only gives a new account its `pid` and
   never stamps `updated_at`, because account events leave it alone (ADR 0011
   §4).
4. **Pure logic** (validation rules, orderings, conversions) goes in
   `src/domain/<name>.rs` with unit tests and at least one proptest property
   per function with a non-trivial input space (ADR 0008 rule 1).
   `tests/architecture.rs` fails on I/O, `async fn` or crate imports there.
5. **Model**: finders on `impl Model`, transitions on `impl ActiveModel`,
   `Validatable` and `before_save` in `src/models/<name>.rs`. Write methods
   take the connection or transaction and the actor explicitly (ADR 0008 rules
   2-4, ADR 0011 section 2).
6. **DTOs**: request, response and error types in `src/dtos/admin/` with
   `serde` and `ts_rs::TS` exported to `admin/src/bindings/`, and in
   `src/dtos/v1/` with `utoipa` for public types (ADR 0005 sections 1-3; every
   `i64` field carries `#[ts(type = "number")]`). serde's derived
   `Deserialize` also reads a struct from a JSON array of its field values in
   order. A request DTO that must be a JSON object implements `Deserialize`
   with a visitor that has only `visit_map` and hands the map to the derived
   fields (`deny_unknown_fields`) through
   `serde::de::value::MapAccessDeserializer`, as
   `dtos::admin::auth::LoginRequest` does. Never read a `serde_json::Map` or
   `Value` first: that builds the whole body in memory before any member is
   checked, and keeps the last of two members with the same name. A calendar
   day (`posts.date`, a canteen day) is a `dtos::shared::CalendarDate` and an
   event's start or end an `EventWallTime`, in both APIs (ADR 0005 §3): build
   one from the stored instant with `CalendarDate::from_midnight` or
   `EventWallTime::from_instant`, whose error is a defect in the stored data
   (a server error), and turn a received one into an instant with
   `to_midnight` or `to_instant`, whose error (a spring-gap time, a day
   without a midnight) is a validation error on the field (ADR 0010 point 6).
   Never format or parse these forms in a handler. `src/domain/date_text.rs`
   holds the strict text forms, which other pure code may reuse; whether the
   date part of the public keyset cursor reads them, or backend2's
   `%Y-%m-%d` that public-api.md §5.2 names, is for `public-posts-list` to
   decide with its cursor.
7. **Admin API**: thin handlers in `src/controllers/admin/<name>.rs`. Every
   route is declared with its permission in the admin route table (deny by
   default, ADR 0007 sections 4-5); errors use the envelope and `ErrorCode`
   of ADR 0021 section 4 (`controllers::admin::error_response`). Until the
   route table exists, mount the route in `controllers::admin::routes()`,
   above its last line: that layer gives every admin response
   `Cache-Control: no-store` (ADR 0023 §6), and loco-rs 1.2.0 applies a
   group's layer only to the routes added before it.
   `tests/requests/auth_login.rs` fails for an admin route without it.
   A handler that reads a JSON body takes `Result<Json<T>, JsonRejection>`
   and answers a refused body with `controllers::admin::json_rejection_response`:
   `422 invalid_body`, or `413 body_too_large` above the request body limit
   (research: DECISIONS R15 (D74)).
8. **Public endpoint**, if the feature has one: recipe 3.
9. **Admin screen** under `admin/src/`: resources through backend3's own data
   provider in `admin/src/data-provider/` (never `ra-data-json-server` or
   `ra-data-simple-rest`), every string from `admin/src/i18n/hu.ts` with the
   ADR 0021 section 6 terms, and the trash and history patterns of ADR 0011
   section 8.
10. **Importer step** for the feature's legacy tables: recipe 4.
11. **Tests at every layer**: domain unit tests and proptests, in-module;
    domain tests that need fixture files (such as the image corpus in
    `tests/fixtures/images/`) in `tests/domain/`, because `src/domain/` may
    not read files; model tests in `tests/models/`; request tests in
    `tests/requests/`; admin unit tests next to the components (`npm test`);
    one Playwright test of the main flow
    (`npm run e2e`). Tests that a generator writes use `boot_test` and
    `serial_test`, which backend3 does not depend on: rewrite them to
    `tests/support::with_app` or `with_server`, which give each test its own
    database and always drop it. A test of a time-dependent rule starts at a
    chosen instant with `with_app_at` or `with_server_at` (recipe 7).
12. **Regenerate the contract**: run the test suite so ts-rs rewrites
    `admin/src/bindings/` and the OpenAPI snapshot updates, review both, and
    commit them (the `Contract` job fails on drift, ADR 0005 section 4).
13. **Docs**: update the ADR or spec the slice changes, and `AGENTS.md`,
    `CLAUDE.md` or this skill if a rule, command or path changed.
14. **Gate**: the "Before you call it done" block of `AGENTS.md`.
15. **PR**: a title such as `feat(db,api,admin): add labels with trash and history`
    (ADR 0012 section 3), a body that names the parity IDs and ADRs and has one
    `Closes #N` line per finished parity issue. The slice goes in as one PR as
    built: about 500 hand-written lines is guidance, not a limit, and neither
    is the plan's estimate (ADR 0031). Split it for size only when the
    maintainer or another human reviewer asks, and then by feature, never by
    layer; a size remark from an automated review agent is not a reason to
    split, hold or re-plan it. Never drop tests or docs to make it smaller.
    Mark it ready only when every check is green; Tide then merges it.

---

## 2. Add an entity under the safety net

Every admin-editable entity gets audit columns, soft delete with a trash,
revisions and activity-log entries in the PR that adds it (ADR 0011 section 1).

1. **Columns** (ADR 0011 section 2) in the entity's migration:
   `created_at`, `updated_at` as `timestamptz NOT NULL`; `created_by`,
   `updated_by` as nullable `bigint` foreign keys to `users(id)`; plus
   `deleted_at timestamptz NULL` and `deleted_by` unless the entity cannot be
   deleted (like site settings). Index the foreign keys. Declare them
   `ON DELETE RESTRICT`, as `users`' own are, not Loco's default `SET NULL`
   for a nullable reference: accounts are anonymised, never deleted, and
   keep every reference (ADR 0011 §6, ADR 0006 §7).
2. **Uniqueness over live rows** uses partial unique indexes
   `... WHERE deleted_at IS NULL` (ADR 0011 section 3, ADR 0009 constraints).
   Decide for each rule whether trashed rows count, as the ADRs say.
3. **Model methods** in `src/models/<name>.rs` (ADR 0011 section 7): create,
   update, delete, restore and purge take the transaction and the actor (a
   user id or `System`) as arguments. Create and update write the row, one
   revision and one activity entry in the same transaction; delete and
   restore write only the activity entry. A relation change (labels on a post)
   updates the parent's audit columns and writes a parent revision.
4. **The live-row helper**: one finder that adds `deleted_at IS NULL`, used by
   every public query on every table it touches (ADR 0011 section 3). Every
   model function that the public API calls and that takes a database
   connection is a live-row finder, and its name has the word `live`
   (`find_live_by_email`, `list_live_page`). A model function that takes no
   connection reads no rows (ADR 0008 rules 2 and 4), so the public API may
   call it under any name: `serve_content` (ADR 0013), which takes the cache
   and the metrics family, and the read counter (public-api.md section 15),
   which takes its in-memory counts. The architecture test
   (`tests/architecture.rs`) fails when a module under `src/controllers/v1/`
   or `src/dtos/v1/` names an entity, a column, a query trait, an entity
   finder or raw SQL; calls through `crate::models` a function without that
   word that takes a connection or that `src/models/` does not define; or
   calls a method (`post.labels(db)`) with the name of such a model
   function. It also fails when a function with that word in `src/models/`
   has no `DeletedAt.is_null()`, no `deleted_at IS NULL` in its SQL and no
   call of another live-row finder.
5. **Dependencies**: refuse a delete while live rows depend on the target, and
   a restore while a row it depends on is trashed or a live row holds its
   unique value (409). The refusal names the blocking rows in a Hungarian
   message through an `ErrorCode` (ADR 0011 section 3, ADR 0021 section 4).
6. **Revision DTO**: an explicit allow-list of the editable fields plus
   relation id lists, with a `schema_version`; never secrets (ADR 0011
   section 4). A shape change bumps `schema_version` and adds a pure upcaster
   in `src/domain/` with a test per version step.
7. **Permanent delete** (`admin` role only) follows the entity's row of the
   ADR 0011 section 6 table; owned files are removed by a queue job after the
   commit (ADR 0016).
8. **Admin API and UI**: the restore, purge and revisions routes of ADR 0011
   section 8 behind the resource's permission (ADR 0007 section 4), the
   trash filter on the list, and the „Előzmények” panel on the edit page.
9. **Tests**: model tests for delete, restore, each refusal, the revision and
   activity rows written per operation, and rollback on error; a request test
   that every public read path ignores trashed rows, through the
   soft-deleted twins of the public fixtures (recipe 3 step 6).

---

## 3. Add a public endpoint with its fixtures

The public API at `/api/v1` replaces backend2's GraphQL API. Its contract is
`docs/design/public-api.md`. The one-day utoipa spike (ADR 0005 section 5)
has passed; `docs/design/openapi-spike.md` records it and the rules for
public DTOs and operations that steps 2 and 5 follow.

1. **Find the endpoint** in `public-api.md` section 7: path, parameters,
   schema (section 6), visibility rules (section 3), paging (section 5),
   errors (section 8), caching (section 9), size limits (section 10) and its
   "Tests" paragraph.
2. **DTOs** in `src/dtos/v1/`: responses with `utoipa::ToSchema`, query and
   path parameters with `utoipa::IntoParams`, camelCase through serde (ADR
   0005 sections 2-3). Instants, dates and event wall times use the forms of
   ADR 0010 and the shared newtypes of `src/dtos/shared.rs` (recipe 1 step
   6); the first public DTO that uses `CalendarDate` or `EventWallTime` adds
   its `ToSchema` derive with the pattern of the form. The derive publishes
   the type's doc comment as the schema's `description` (rule 3), which is
   written for API users (rule 5), while today's doc comment is written for
   the admin, whose TypeScript binding carries it too: that slice rewords
   the comment for both readers, or gives the schema its own text with
   `#[schema(description = ...)]`. Every `Option` field
   carries `#[schema(required = true)]`, documented ranges are stated with
   `minimum` and `maximum`, and a newtype states its pattern on the derive
   with `#[schema(value_type = String, pattern = ...)]` (openapi-spike.md
   section 3, rules 1, 3 and 4).
3. **Pure parts first** in `src/domain/` (parameter parsing per section 2.5,
   the keyset cursor `src/domain/cursor.rs`, date ranges), with unit tests for
   every documented row and the proptest properties of `public-api.md`
   section 16.1. The parameters of section 2.5 live in
   `src/domain/v1_params.rs`: the handler takes the raw query (axum's
   `RawQuery`) and the path values as strings, decodes the query with
   `v1_params::Query::parse` and reads each parameter with `Query::first`
   (the first value wins), then checks them in the order of section 8, path
   parameters first, returning the first `Rejection`. A new parameter adds
   its check there and its `Rejection` variant at its place in that order.
4. **Query on the model** (`posts::Model::list_live_published(db, filter, cursor, limit)`
   style) using the live-row helper of recipe 2. The handler reads rows only
   through such live-row finders, whose names have the word `live`; it calls
   `serve_content` for a body and the read counter for a counted read
   directly, because they take no database connection (recipe 2 step 4).
5. **Handler** in `src/controllers/v1/<name>.rs` with `#[utoipa::path(...)]`
   that gives the full `/api/v1/...` path, a `tag` and a unique
   `operation_id` (openapi-spike.md section 3, rules 2 and 6), registered
   through Loco `Routes` (`Routes::get` records the method directly; the
   route-coverage test fails for a route whose method Loco cannot read) and
   listed in `paths(...)` of `PublicApi` in `src/controllers/v1/openapi.rs`.
   The module's `routes()` writes each path in full and sets no prefix, and
   is merged in `controllers::v1::routes()` (`src/controllers/v1/mod.rs`)
   before its `.layer(...)` call, never added in `App::routes`: that builder
   adds the CORS layer of ADR 0014 and the panic handler, and
   `Routes::merge` drops a prefix. If `cargo loco generate controller`
   injected an `.add_route(...)` line into `App::routes`, move it there. A
   client error is a `crate::dtos::v1::error::Error` answered with
   `Ok(error.into_response())` (a refused parameter with
   `Ok(Error::from(rejection).into_response())`), never turned into a
   `loco_rs::Error`, which Loco logs at `ERROR` level; a database failure
   stays `?` and becomes Loco's `500`. Answer a `200` with
   `controllers::v1::caching::cacheable_json` (`no-cache` and the weak
   `ETag` of section 9, which Loco's `etag` middleware turns into a `304`),
   or `caching::preview_json` for a preview (`no-store`, no `ETag`); never
   with `format::json`, which sets neither. The route layers give every
   error `no-store`, and the OpenAPI document describes the caching headers
   of every `GET` operation by itself (openapi-spike.md section 3, rule 7).
6. **Fixtures**: add every fixture that `public-api.md` section 17.1 lists for
   this endpoint, under the name used there, as invented data (never copied
   from production), to `FIXTURES` in `tests/support/fixtures.rs` (its
   module documentation has the rules). A fixture's load function writes its
   rows directly, through the models on the transaction that
   `FixtureWriter::db` gives, never through `/api/admin`; gives every row an
   explicit id below `TWIN_ID_OFFSET`; and writes the soft-deleted twin of
   every public row with `FixtureWriter::twin` (or `twin_with`, for a column
   that is unique over trashed rows too), as ADR 0011 asks of every public
   row type. A test loads the fixtures it reads with
   `fixtures::load(&ctx.db, &["<§17.1 name>", ...])`, which returns the twin
   ids. `tests/requests/public_fixtures.rs` checks that each name is a §17.1
   name and that all fixtures load together. Check that each shape of the
   ADR 0019 section 2 table that the endpoint serves has its named fixture.
   Shapes that only legacy data has go into the import-path fixtures of
   section 17.2 (recipe 4).
7. **Specification tests** in `tests/requests/public_<name>.rs`: one test per
   rule, named after the rule (for example
   `home_paging_visits_every_published_post_once`), asserting full order,
   parsed JSON field by field, the response against its schema
   (`openapi_schema::assert_matches_operation` with the document from
   `served_document`), the twins' absence (a `404` for a twin's id, lists in
   full order without them), and the headers of section 16.2, with the
   helpers of `tests/support` (`public_get`, `assert_public_cors`,
   `assert_public_not_found`). Every `/api/v1` route in Loco's route table
   (`tests/support::public_paths`) gets the CORS and method checks of
   `tests/requests/public_router.rs` and the caching checks of
   `tests/requests/public_caching.rs` without further work. Time-dependent
   tests inject "now" (recipe 7).
8. **Route samples** for the global suite of section 16.3
   (`tests/requests/public_global.rs`, which fails until they exist): add a
   `RouteSamples` entry for each new route to `ROUTE_SAMPLES` in
   `tests/support/public_samples.rs` (its module documentation has the
   rules). Give path values and `accepted` values that reach a live row of
   the named fixtures: the base request must get the route's cacheable
   `200`, and its body must list a row whose twin one of those fixtures
   wrote (`rows: Rows::Twinned`; `Rows::NoIds` with the reason only for an
   answer without row ids). For every documented query parameter give an
   `other` value that is answered differently (a refused one is the usual
   choice), and a `Spaced::Value` that the route answers differently with a
   plus sign than with a space (a search text only the spaced form
   matches), or `Spaced::Unobservable` with the reason when no value can
   show it (a `limit`). An optional parameter that changes the answer, such
   as the preview `token`, stays out of the base request (`in_base: false`)
   and names the path of the row it is accepted on (`path: Some(...)`, the
   draft). The suite reports samples that cannot show a rule, then checks
   the trailing slash, `POST`, unknown and repeated parameters, `+`
   decoding, CORS, the error envelope, the schema, the twins and the `304`
   of every route, and sends random strings to every parameter.
9. **Contract**: review the `.snap.new` the tests write beside
   `tests/requests/snapshots/openapi_document.snap` and accept it (`cargo
   insta review`), bump `info.version`'s minor by hand for an addition, and
   keep the route-coverage test of `tests/requests/openapi.rs` green (ADR
   0005 section 4). A snapshot change is an API change: say so in the PR.
10. **PR**: `feat(api): ...` (or with `db`/`admin` scopes if the slice adds
   them), `Closes #N` for the PUB or NEW parity IDs it finishes.

---

## 4. Add an importer step

The `import_mysql` task (ADR 0017) imports the legacy MySQL data once, at the
cut-over, and every night in the rehearsals. `docs/design/data-migration.md`
is its specification; each slice adds the step for its own tables.

1. **Read the table's mapping** in `data-migration.md` section 6, the global
   rules of section 5, the defects (D-numbers) of section 8 that touch it and
   its verification checks in section 10.
2. **Pure transforms** in `src/domain/` (ADR 0008): one function from legacy
   row values to the target row, taking every input as an argument, including
   `snapshot_at` and the resolution entries. Rich-text bodies go through the
   shared normaliser (`data-migration.md` section 7). Unit tests cover every
   rule of sections 5-7 for the table, each defect including a precondition
   mismatch, and each exemption flag of section 4.3; proptests cover
   round trips and idempotence.
3. **The step** in `src/tasks/import_mysql/` (code that only the task uses,
   ADR 0008 rule 5): it reads the source rows through the source connection it
   is given (`run(&source_db, &target_db, &ImportOptions)`; only the task reads
   `ctx` and `IMPORT_SOURCE_DATABASE_URL`, data-migration.md section 4.1),
   inserts in the foreign-key order of section 4.3 with explicit values (no
   model save hooks), and writes the `import` (and `import_original`)
   revisions and the per-table activity entry (ADR 0011 section 4).
4. **Refusals** (section 4.2): an uncovered defect instance, a precondition
   mismatch or a constraint violation stops the import with a message that
   names the defect id, the row id and the rule.
5. **Verification and report**: add the table to the row counts and digests
   of section 10 and to the run report of section 11.3.
6. **Fixtures**: extend the synthetic legacy fixture (section 12) with every
   production shape of the table that `public-api.md` section 17.2 and the
   ADR 0019 section 2 table list, written by hand in the legacy format, and
   assert the result through the public API's request helpers.
7. **Safety**: the source must be a local throwaway MySQL in `test` and
   `development`; the import task refuses a non-local source there, and the
   guard hook blocks a non-local `IMPORT_SOURCE_DATABASE_URL` for cargo
   commands (ADR 0024 section 4). Never point a run at the production
   database; the operator runs the production import in the cut-over window.
8. **Run it**: `cargo loco task import_mysql mode:dry-run snapshot_at:<RFC 3339>
   resolutions:<file> storage_root:<dir> report:<empty dir>` against the local
   databases, then check the report.
9. **PR**: the `import` scope, for example `feat(import): import labels and post labels`,
   with the D-numbers it resolves and the parity IDs it finishes.

---

## 5. Add a Rust dependency

Every requirement of the workspace lives once, in `[workspace.dependencies]`
of the root `Cargo.toml`, and the crates inherit it. Dependabot rewrites only
that table of the root manifest, so a requirement in the root package's own
tables gets every update beyond it as a Cargo.lock-only PR that fails every
`--locked` job. `migration/` inherits too, although Dependabot would rewrite
its manifest: each requirement and its reason then live in one place, and the
two crates cannot drift apart (the header of `Cargo.toml` and the cargo entry
of `.github/dependabot.yml`).

1. **Need**: check that Loco does not already do it (`AGENTS.md` rule 2) and
   that the crate's licence is on the `deny.toml` list (ADR 0022 section 4).
2. **Workspace entry** in the root `Cargo.toml`, `[workspace.dependencies]`:
   the version requirement, with a comment that says why the crate and its
   features are there. `default-features = false` and a rename
   (`package = "..."`) go here, because a member cannot set them. So do the
   features that every crate using it needs. Write it as
   `<crate> = "<requirement>"` or `<crate> = { version = "<requirement>", ... }`,
   with bare keys, never dotted (`<crate>.version = ...`) or quoted
   (`"<crate>" = ...`) ones: Dependabot rewrites the requirement in place by
   searching for those forms, does not find the others, and their updates
   would be Cargo.lock-only PRs again (`check-workspace-deps.sh` rejects
   them). Avoid a rename where you can: Dependabot's parser skips a renamed
   entry, so it gets no version updates and is bumped by hand. Keep the
   literal `[dependencies]` out of the comments above the package's own
   table: `cargo loco generate deployment lambda` injects after the first
   line that matches `\[dependencies\]` (`scripts/tests/test-generator-anchors.sh`
   checks that).
3. **Member entry** in the crate that uses it (`[dependencies]` or
   `[dev-dependencies]` of the root package, or `migration/Cargo.toml`):
   `<crate> = { workspace = true }`, plus `features = [...]` that only this
   crate needs (features only add up) or `optional = true`. Nothing else goes
   here. Once the workspace entry exists, `cargo add <crate> -p backend3`
   (`--dev` for a test dependency, `-p migration` for the migrations) writes
   the equivalent `<crate>.workspace = true`; before that, `cargo add` writes
   a plain requirement into the member, which the check below rejects.
4. **Lockfile**: build once without `--locked` (`cargo build`) so
   `Cargo.lock` picks the crate up, and commit `Cargo.toml` and `Cargo.lock`
   together.
5. **Dependabot rule**: if the crate's types cross Loco's API or an ADR pins
   its major, add an ignore rule with its reason to the cargo entry of
   `.github/dependabot.yml`, following the rules in that file's header.
6. **Check**: `bash scripts/ci/check-workspace-deps.sh` (the `Toolchain`
   job runs it), `cargo deny --workspace --all-features check bans licenses
   sources`, then the `AGENTS.md` gate.

Generators: of the `cargo loco generate` templates in loco-gen 1.2.0, only
`deployment lambda` edits `Cargo.toml`. It adds
`lambda_http = { version = "1.3" }` to the package's `[dependencies]` (and
`[package.metadata.lambda]` at the end). backend3 ships as a container image
(ADR 0022) and never runs it, but if it or a future generator adds a
requirement to a member table, move it as in steps 2 and 3:
`check-workspace-deps.sh` names the entry and the fix. Bumps of `loco-rs` and
the Sea-ORM crates stay dedicated PRs (ADR 0024 section 3); they change the
same `[workspace.dependencies]` entries.

---

## 6. Add a task

A `CronJob` in website_k8s runs a recurring task as `backend3-cli task <name>`,
and an operator may run it by hand; the shared advisory lock keeps the two
apart (ADR 0016 §4).

1. **Generate** it with `cargo loco generate task <name>` and a snake_case
   name; rewrite the generated test on `tests/support::with_app` (recipe 1
   step 11).
2. **Lock**: after checking its arguments, `run` does all of its database
   work inside `with_task_lock(&ctx.db, &self.task().name, || async { ... })`
   (`src/models/task_lock.rs`) and returns `Ok(())` for both outcomes, so a
   run that finds the lock held exits 0. Lock under the registered name, and
   never take an advisory lock by hand. Write the call in the body of the
   `impl Task for <Type>` that `cargo loco generate task` writes, with
   `&self.task().name` as its second argument, and keep the generated
   `tasks.register(tasks::<name>::<Type>);` line in `register_tasks` itself
   (a `#[cfg(...)]` in front of it is fine): `tests/architecture.rs` fails
   for a `Task` impl without that call, for a registered task whose impl it
   cannot read at that path, and for any other statement in
   `register_tasks`, such as a helper that registers tasks (research:
   DECISIONS R15, D95). It sees the call, not what runs outside it, so
   review checks that all of the work is inside. The helper's holder sets
   its own timeouts and isolation level, so the database defaults an
   operator sets do not end or change it; the task's own transactions stay
   under them.
3. **Idempotent**: two runs give the same result as one, because the
   `CronJob` retries a failed run. A task that needs the uploads volume only
   enqueues a job for the worker (ADR 0015 §3).
4. **Tests** in `tests/tasks/<name>.rs`, including the lock-held test that
   ADR 0016 (Consequences) requires of every task: take the lock with
   `support::hold_task_lock(&ctx.db, "<name>")`, run the task with
   `loco_rs::boot::run_task::<App>`, and assert `Ok` and that none of the
   work happened, as `tests/models/task_lock.rs` does for its probe task.
5. **CronJob**: a recurring task needs its `CronJob` in website_k8s, with the
   settings of the ADR 0016 §4 table; that PR is left for human review.

---

## 7. Use the current time

"Now" has one source, the app clock in the `AppContext` (ADR 0010 point 7),
so every time-dependent rule can be tested at a chosen instant (public-api.md
section 16.2). `App::after_context` registers the system clock; the
interface is `Clock` in `src/domain/clock.rs`.

1. **Read it once, at the edge.** A controller, worker or task takes
   `crate::app::clock(&ctx)?.now()` (a `DateTime<Utc>`) once per request or
   job run and passes the instant down. `src/domain/` takes "now" as an
   argument (ADR 0008 rule 1) and computes "today" from it in Europe/Budapest
   with `budapest::today(now)` (`src/domain/budapest.rs`, ADR 0010 points 5
   and 7). Models and external-system code take the instant, or
   `&dyn Clock` where one call needs several readings (the session extractor
   of ADR 0006 section 1), never `&AppContext`.
2. **Never read the system time yourself.** `tests/architecture.rs` fails on
   every other read of it in `src/`, unit tests included (`Utc::now`,
   `SystemTime::now`, Sea-ORM's `ChronoUtc::now` and the other forms the
   test lists), on chrono's `Local` (it reads the process `TZ`, ADR 0010
   point 4) and on `set_clock` outside `src/app.rs`. `Instant::now` stays
   allowed outside `src/domain/`: it measures durations and never becomes an
   instant.
3. **Tests.** Unit tests inside `src/` pass an instant, or `FixedClock` where
   the code takes a clock. Integration and request tests use
   `tests/support::with_app_at(instant, |router, ctx, clock| ...)` or
   `with_server_at(instant, |server, ctx, clock| ...)`; the `TestClock` stands
   still until `clock.advance(TimeDelta)` or `clock.set(instant)` moves it.
   It replaces the system clock after boot, so code that runs during boot
   (hooks, initializers) still sees the system clock.
4. **Limit: Loco's JWT reads the system time.** `JWT::generate_token` sets
   `exp` to the system time plus its `expiration` (seconds), and
   `JWT::validate` checks `exp` against the system time with a leeway of 0
   (loco-rs 1.2.0 `src/auth/jwt.rs`). A handler that wants a token to end at
   an instant `target` passes `target - clock(&ctx)?.now()` as the lifetime;
   in production the app clock is the system clock, so `exp` is `target`, to
   the second. Under a `TestClock` the lifetime is still exact, but `exp` is
   the wall clock plus it, and moving the test clock neither ages nor
   expires a token. Session tests (ADR 0006 section 1) assert exactly what
   the handler derives from the app clock (the lifetime, and values computed
   from it such as a cookie's `Max-Age`), and check `exp` only against that
   lifetime added to wall-clock readings taken just before and after the
   request. These readings and the boot test of the system clock are the
   only reads of the wall clock in tests.
5. **Generated code.** The first `cargo loco db entities` for a table with
   `updated_at` writes `src/models/<name>.rs` with a `before_save` that sets
   `updated_at` to `chrono::Utc::now()` on an update that left it unchanged
   (loco-rs 1.2.0 `src/db/entities.rs`). The check rejects that line, and
   ADR 0011 section 2 keeps some updates from changing `updated_at`, so
   delete the fallback and let `before_save` return `Ok(self)`: the write
   path sets the audit columns (ADR 0011 sections 2 and 7). The `Local::now()`
   examples of the vendored Loco skill (`doctrine.md`,
   `recipes/model-and-migration.md`) do not apply here.
