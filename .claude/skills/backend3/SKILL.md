---
name: backend3
description: Use when adding a feature to website_backend3 — a vertical slice, a new admin-editable entity (soft delete, revisions, activity log), a public /api/v1 endpoint with its test fixtures, a step of the legacy MySQL importer, a Rust dependency, a Loco task, code that needs the current time, an activity-log entry, or a queue worker. Gives backend3's own recipes on top of the vendored Loco skill, with the ADRs and file paths each step follows.
---

# backend3 recipes

These recipes are backend3's way of doing its nine repeated tasks. They sit on
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
   object it owns. Every new table also goes on exactly one list of what
   the legacy import does with it, or
   `tests/tasks/import_mysql_target.rs` fails: a table that the import
   writes is already on `IMPORT_TARGET_TABLES`
   (`src/domain/import_mysql/target.rs`) under its name in the insert order
   of `data-migration.md` section 4.3, so the migration uses exactly that
   name; any other table (a release-1 table of ADR 0019 §5, for example)
   joins `NEVER_WRITTEN` in that test with the reason. The empty-target
   guard of the import skips a listed table that the schema lacks, so a
   table created under another name would otherwise escape it.
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
   §4). An event that ends the account's sessions (ADR 0006 §2: logout,
   password change and reset, deactivation and soft delete, two-factor
   changes) moves `token_version` only through
   `users::ActiveModel::revoke_sessions`: one `UPDATE` that increments it in
   PostgreSQL and writes the event's other changed (set) columns with it.
   Never set `token_version` yourself; a read-then-write loses a concurrent
   increment and can leave a revoked session valid.
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
   without a midnight, a day after `budapest::CHRONO_TZ_LAST_CHANGE_YEAR`:
   `NonexistentWallTime`, `NoMidnight`, `AfterTzTable`) is a validation error
   on the field (ADR 0010 point 6, ADR 0041 §2). `to_midnight` refuses every
   day after that year because backend3 stores none (chrono-tz's table ends
   there); the public month and grid bounds, which are never stored, use
   `budapest::date_to_midnight`, which converts every year.
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
   (research: DECISIONS R15 (D74)). Mount it above the
   `DefaultBodyLimit::max(JSON_BODY_LIMIT)` layer of `routes()`, so its body
   may be at most 4 MiB (ADR 0023 §5).
   An upload handler (an entity image field, later the media library) is
   mounted between that layer and the `no-store` layer, so it keeps the
   128 MiB of `server.middlewares.limit_payload` (ADR 0015 §6). It takes
   `Result<Multipart, MultipartRejection>` last, reads the one `file` part
   with `controllers::admin::media::receive_image_part` (the media and image
   settings of `media_uploads(&ctx)`), stores it with
   `models::media::store_entity_image` (folder, `random_name_bytes()`, and
   the same settings and the gate), and answers every error with
   `upload_error_response`, which maps each refusal to its Hungarian
   `ErrorCode` and its status (ADR 0021 §4): an upload that would leave less
   than `media.min_free_bytes` free is `507 insufficient_storage` (research:
   DECISIONS R24 (D121)). The model writes no row and no activity entry:
   the handler stores `stored.stored_value` on its entity in its own
   transaction. That is the legacy rows' shape, the bare `<32 hex>.<ext>`
   or `index/<32 hex>.<ext>` for a post's index image
   (`media_rules::stored_value`, data-migration.md §6), never `stored.path`,
   the path below the root: the public URL is
   `{public_base_url}/{disk}/{stored value}` (public-api.md §2.3), and the
   admin's is `/storage/` followed by `stored.path` (ADR 0015 §3). The
   slice that adds the column joins the file-owner registry with one line,
   `FileOwner { entity: "<table>", column: "<column>", folder: <the
   EntityImageFolder> }` in `FILE_OWNERS` (`src/domain/owned_files.rs`):
   the `DeleteOwnedFiles` worker checks a file only against the registered
   columns of its folder and refuses a folder that has none (recipe 9).
   The entity's revision snapshot type lists the column in `FIELDS` under
   the same name (recipe 2 step 6): a purge reads the column's files from
   every revision by that name, and `files_of_record` refuses a snapshot
   type that lacks it.
   `tests/models/owned_files.rs` runs the reference query of every
   registered column against the migrated schema. A replace or a clear
   removes no file: the old file stays while a revision shows it, and goes
   with the record's purge (step 7 of recipe 2).
   Request tests of an upload endpoint follow `tests/requests/media_upload.rs`
   and give each test its own media volume (`tests/support::TempMedia`).
   A handler for a signed-in account takes
   `controllers::admin::session::AdminSession`, which loads the account on
   every request and answers `401 unauthorized` itself when it is trashed,
   anonymised or inactive, when the token's `tv` is stale, or when the scope
   is not the route's (ADR 0006 §1). Once the session holds, it also reads
   the roles and extra grants the account holds into `session.grants`
   (`domain::authz::AccountGrants`: `effective()` and `is_admin()`), so a
   role change applies on the next request (ADR 0007 §5); never query
   `user_roles` or `user_permissions` in a handler. The default accepts only a full
   `admin` session; `AdminSession<AdminOrPasswordChange>` is for the three
   routes ADR 0006 §4 opens to a `password_change` session. Never take
   Loco's `JWT` or `JWTWithUser` in an admin handler: they skip these
   checks. A request test covers the route's `401` without a session.
8. **Public endpoint**, if the feature has one: recipe 3.
9. **Admin screen** under `admin/src/`: resources through backend3's own data
   provider in `admin/src/data-provider/` (never `ra-data-json-server` or
   `ra-data-simple-rest`), every string from `admin/src/i18n/hu.ts` with the
   ADR 0021 section 6 terms, and the trash and history patterns of ADR 0011
   section 8, built clean-room: never an `@react-admin/` package
   (`npm run clean-room` fails on one). Every screen sits behind the sign-in
   (`requireAuth`, ADR 0006); a refused request shows its envelope's message
   through `envelopeMessage` of `admin/src/api/error-envelope.ts` (ADR 0021
   section 4), and the auth provider signs out on `401` only.
10. **Importer step** for the feature's legacy tables: recipe 4.
11. **Tests at every layer**: domain unit tests and proptests, in-module;
    domain tests that need fixture files (such as the image corpus in
    `tests/fixtures/images/`) in `tests/domain/`, because `src/domain/` may
    not read files; model tests in `tests/models/`; request tests in
    `tests/requests/`; admin unit tests next to the components (`npm test`);
    one Playwright test of the main flow
    (`npm run e2e`), which signs the seed account in first with `signIn(page)`
    of `admin/e2e/fixtures.ts` and lists any HTTP error answer it expects in
    `expectedHttpErrors` (`admin/CLAUDE.md`, "The admin gate"). A fixture the
    E2E flow needs in the database is created by `Hooks::seed` through the
    models (`AGENTS.md`, "The seed account"), which refuses every environment
    but development, test and e2e. Tests that a generator writes use
    `boot_test` and `serial_test`, which backend3 does not depend on: rewrite
    them to `tests/support::with_app` or `with_server`, which give each test
    its own database and always drop it. A test of a time-dependent rule
    starts at a chosen instant with `with_app_at` or `with_server_at`
    (recipe 7).
12. **Regenerate the contract**: run the test suite so ts-rs rewrites
    `admin/src/bindings/` and the OpenAPI snapshot updates, review both, and
    commit them (the `Contract` job fails on drift, ADR 0005 section 4). A
    slice that changes the rich-text policy also runs
    `cargo loco task rich_text_manifest` and commits
    `admin/src/rich-text/policy.json` and `css-vectors.jsonl`, which the same
    job checks (editor.md §3.9).
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
   keep every reference (ADR 0011 §6, ADR 0006 §7). The instants come from
   PostgreSQL's clock (recipe 7 step 6): `created_at` and `updated_at` keep
   their `CURRENT_TIMESTAMP` defaults.
2. **Uniqueness over live rows** uses partial unique indexes
   `... WHERE deleted_at IS NULL` (ADR 0011 section 3, ADR 0009 constraints).
   Decide for each rule whether trashed rows count, as the ADRs say.
3. **Model methods** in `src/models/<name>.rs` (ADR 0011 section 7): create,
   update, delete, restore and purge take the transaction and the actor (a
   user id or `System`) as arguments. Create and update write the row first
   and then call `models::revisions::record_save(&txn, actor, Save { .. })`
   in the same transaction: it locks the record's row, writes the next
   revision and the `create`, `update` or `revert` entry (recipe 8) that
   names it, with the changed field names of an update computed against the
   latest revision; set the entry's `label` for a content record only.
   Delete and restore write only the activity entry. An update or restore
   sets `updated_at` (and delete `deleted_at`) to
   `Expr::current_timestamp()`, never to an app-clock instant (recipe 7
   step 6). A relation change (labels on a post)
   updates the parent's audit columns and writes a parent revision. Both
   `record_save` and `revisions::append` take a transaction, never the
   pool: the row lock that numbers concurrent saves lasts until the save
   ends (`src/models/revisions.rs`). The importer calls `append` itself,
   with `Actor::Import` and the reasons `import_original` and `import`, and
   writes one entry per table (recipe 4).
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
6. **Revision snapshot**: a plain struct in the entity's domain module that
   implements `domain::revision::RevisionSnapshot`: `ENTITY_TYPE` is the
   table name, `FIELDS` the explicit allow-list of the editable fields plus
   relation id lists (every one serialised, `null` for no value, never a
   `skip_serializing_if`), `SCHEMA_VERSION` from 1; never secrets
   (`SECRET_FIELDS`), never the entity model (ADR 0011 section 4). It
   derives serde only, no ts-rs or utoipa (ADR 0008 rule 6). A shape change
   bumps `SCHEMA_VERSION` and appends one `UpcastStep` from the old version
   to `UPCASTS`, a pure function over the JSON members with a unit test of
   its own; `check_snapshot_type::<T>()` in the type's unit tests fails on
   a gap in the registry. The importer's `import` revisions use the same
   type, so it exists before the entity's importer step.
7. **Permanent delete** (`admin` role only) follows the entity's row of the
   ADR 0011 section 6 table, in a transaction that called
   `activity_log::with_permanent_delete_flag` first: the `revisions`
   trigger lets a `DELETE` of the record's revisions through only then.
   Write the record first (the purge deletes the row on the condition that
   it is still in the trash; the anonymisation updates the account), then
   remove the revisions with
   `models::revisions::delete_for_record::<T>(&txn, id)`, never with a
   `DELETE` of your own. It locks the record's row before it deletes, so a
   save that is still running ends first and its revision is deleted too;
   a plain `DELETE` would miss that revision, which would then outlive the
   purge or keep the personal data that the anonymisation removes.
   Owned files are removed by the `DeleteOwnedFiles` job after the commit
   (ADR 0011 §6, ADR 0016; recipe 9). Before the row and its revisions
   go, collect the files they name with
   `models::owned_files::files_of_record::<S>(&txn, owners, id)`, where `S`
   is the entity's revision snapshot type, with the registry of
   `workers::delete_owned_files::owned_files(&ctx)?`: it reads the
   registered columns of the row and the members of the same name in every
   one of its revisions, an older snapshot upcast first, because an image
   replaced long ago is shown only by an old revision and nothing would
   name it after the purge. It refuses an `S` whose `FIELDS` lack a
   registered column, and skips with a warning what names no file. End
   the transaction with
   `workers::delete_owned_files::commit_then_enqueue(&ctx, txn, &paths)`
   instead of `txn.commit()`: it commits, then enqueues the jobs, so no
   job runs while the purge may still roll back. After the commit it
   never fails; `AfterCommit::NotEnqueued` means the files stay as harmless
   orphans, which the log names.
8. **Admin API and UI**: the restore, purge and revisions routes of ADR 0011
   section 8 behind the resource's permission (ADR 0007 section 4), the
   trash filter on the list, and the „Előzmények” panel on the edit page.
9. **Tests**: in the entity's model tests, implement
   `tests/support::safety_net::SafetyNetEntity` for the entity's own create
   and update methods and call `assert_safety_net`: it checks revision 1
   and the `create` entry, the next revision and the `update` entry with
   exactly the changed fields, the audit columns and the one instant of
   both, and that a refused activity entry rolls the save back
   (`tests/models/safety_net.rs` proves it on the test-only `probe_notes`
   entity of `tests/support/probe_notes.rs`, which is also a complete
   example). Add model tests for delete, restore, each refusal and the
   entries they write, and a request test that every public read path
   ignores trashed rows, through the soft-deleted twins of the public
   fixtures (recipe 3 step 6).

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
   shared normaliser, `domain::normalise::normalise`, and its output through
   the N9 checks of `check_fixed_point` (`data-migration.md` section 7, its
   readings in 7.4). A new normaliser rule adds its step to the pass in
   `src/domain/normalise/mod.rs`, its `Rule` variants, and a
   before-and-after pair to `tests/fixtures/normalise/`. Unit tests cover every
   rule of sections 5-7 for the table, each defect including a precondition
   mismatch, and each exemption flag of section 4.3; proptests cover
   round trips and idempotence.
3. **The step** in `src/tasks/import_mysql/` (code that only the task uses,
   ADR 0008 rule 5), called from `run(&source_db, &target_db, &ImportOptions,
   &mut ReportParts)` in `src/tasks/import_mysql/mod.rs`, inside the one
   target transaction that `run` opens (`target::in_import_transaction`,
   whose `dry-run` rolls back and `apply` commits), after the checks of the
   code tables and of the empty target. It reads the source rows inside the
   locked source session (`src/tasks/import_mysql/session.rs`): it extends
   `session::read` to read them on `SourceSession::connection` before the
   session closes, so every row is read under the one `LOCK TABLES` (only
   the task reads `ctx` and the source setting
   `settings.import.source_database_url`, which production and development
   fill from `IMPORT_SOURCE_DATABASE_URL`; data-migration.md section 4.1),
   and only with `tasks::import_mysql::source::select_rows`, the source
   query layer: it reads only the 17 tables of the allow-list in
   `src/domain/import_mysql/source_tables.rs` and refuses any other table,
   the newsletter's first (NWS-01), before it sends anything. It selects
   the columns of the table's entry in
   `src/tasks/import_mysql/expected_schema.rs`, which the session's
   fingerprint has checked. Its errors carry no part of the source URL.
   Only `src/tasks/import_mysql/source.rs` and `session.rs` build or send a
   statement to the source or open a connection to it:
   `tests/architecture.rs` fails when another file of the import task names
   sqlx's statement or connection API (`sqlx::query`, `Executor`,
   `MySqlPoolOptions`, `MySqlPool::connect`, ...). A new kind of statement
   goes into a file that joins `SOURCE_STATEMENT_FILES` in that test, names
   its tables from `SOURCE_TABLES` and maps the driver's errors with
   `source::source_error`, as `session.rs` does. The sequences use the
   session's `AUTO_INCREMENT` values (`SourceFacts`, data-migration.md
   §5.1). A target table that the step writes and that
   `IMPORT_TARGET_TABLES` (`src/domain/import_mysql/target.rs`) does not
   list joins it, so the guard refuses a target in which it holds a row
   (section 4.2), and the insert order of section 4.3 names it in the same
   place (`tests/domain/import_target_tables.rs` compares the two). The
   step's first test with synthetic rows also covers the refusals that the
   empty compose source could not: the non-empty refusal for the step's
   own entity table, and, when its legacy table has write-instant columns,
   a `snapshot_at` earlier than its rows refused through the whole task.
   The step inserts in the foreign-key order of
   section 4.3 with explicit values (no model save hooks), and writes the
   `import` (and `import_original`) revisions of the entity's revision
   snapshot type (recipe 2 step 6) with `models::revisions::append` and
   `Actor::Import`, or with a bulk insert that keeps the same rules
   (`domain::revision::snapshot_object`, `check_revision`, numbers from 1
   per record), and the per-table activity entry (ADR 0011 section 4).
4. **Refusals** (section 4.2): an uncovered defect instance, a precondition
   mismatch or a constraint violation stops the import with a message that
   names the defect id, the row id and the rule. A legacy calendar day
   converts through `budapest::date_to_stored_midnight`, never
   `date_to_midnight`, and its `NoMidnight` or `AfterTzTable` (a day after
   `budapest::CHRONO_TZ_LAST_CHANGE_YEAR`) stops the import and reports the
   row (ADR 0010 point 6, ADR 0041 §2).
5. **Verification and report**: add the table to the row counts and digests
   of section 10 and to the run report of section 11.3: the step's data joins
   `ReportParts` in `src/domain/import_mysql/report.rs`, as the source
   session's `source` does, and the step fills it through the `parts` that
   `run` receives as soon as the step ends, so the report of a run that
   fails later still shows it; `RunSummary` renders `summary.json` and
   `report.md` from it. A step that fills a Markdown section
   replaces its "No import step of this build reports here yet." line. Add
   new `summary.json` fields without changing `REPORT_FORMAT`; rename or
   remove one only with a `REPORT_FORMAT` bump.
6. **Fixtures**: extend the synthetic legacy fixture (section 12) with every
   production shape of the table that `public-api.md` section 17.2 and the
   ADR 0019 section 2 table list, written by hand in the legacy format, and
   assert the result through the public API's request helpers.
7. **Safety**: the source must be a local throwaway MySQL in `test`,
   `development` and `e2e`. The guard hook blocks a non-local
   `IMPORT_SOURCE_DATABASE_URL` for cargo commands (ADR 0024 section 4,
   layer 2), and the task itself refuses a source that is not local there
   (layer 3, `src/domain/import_mysql/source_guard.rs`): the host must be
   `localhost`, `127.0.0.1` or `::1`, or the URL's `socket` parameter must
   name a Unix socket; a container-network name such as `mysql` is
   refused. Never point a run at the production database; the operator
   runs the production import in the cut-over window.
8. **Run it**: start the source, `podman-compose --profile import up -d
   mysql` (the legacy schema of `tests/fixtures/legacy/schema.sql`; a step
   that needs rows adds its synthetic fixture), then
   `IMPORT_SOURCE_DATABASE_URL=mysql://loco:loco@127.0.0.1:3306/verseghy
   cargo loco task import_mysql mode:dry-run snapshot_at:<RFC 3339, UTC>
   resolutions:<file> storage_root:<dir> report:<empty dir>` against the local
   databases, then check `report.md` and `summary.json` in the report
   directory. The task refuses an unset source (empty or `disabled`), an
   unknown argument, a `snapshot_at` that is not UTC or lies after the start
   of the run, and a report directory that is not empty or lies inside
   `storage_root`. Once it runs, it refuses a target whose `roles` or
   `user_permissions` CHECK differ from the code, or in which a table it
   writes holds a row (recreate the local database to start over, as the
   `AGENTS.md` commands say), a source whose schema differs from the
   expected DDL, and a `snapshot_at` earlier than the latest timestamp in the
   source. The task and its MySQL driver exist only with the default
   Cargo feature `import-mysql`; code and tests that use the driver are
   behind it, and the CI `Test` job builds and tests once without it. Its
   tests set the source in the context they run the task with
   (`tests/tasks/import_mysql.rs`), because `config/test.yaml` names none;
   the source layer's and the source session's tests read the compose
   source (`tests/tasks/import_mysql_source.rs`,
   `tests/tasks/import_mysql_session.rs`), and the target's checks have
   theirs in `tests/tasks/import_mysql_target.rs`.
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
6. **Licence notices**: `bash scripts/crate-notices.sh --out target/licenses`
   (after `rm -rf target/licenses`), which writes the notices that the image
   ships and fails on a gap, as the `cargo-deny check bans licenses sources`
   job does (ADR 0042). If it says that a crate gets the standard text
   although it ships a licence file, or that a file a clarification names
   has changed, read the file and add or renew the crate's clarification in
   `about.toml` with the SHA-256 it prints, in the one form `about.toml`
   uses (its comment says which). A licence that `deny.toml` allows for one
   crate only goes into that crate's `accepted` in `about.toml` too. A new
   `-sys` crate that compiles bundled C code under a licence its `license`
   field does not declare (as libwebp-sys and libdeflate-sys do) gets a
   clarification that names the bundled licence file.
7. **Check**: `bash scripts/ci/check-workspace-deps.sh` (the `Toolchain`
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

The rules that compare with "now" read one source, the app clock in the
`AppContext` (ADR 0010 point 7), so every time-dependent rule can be tested
at a chosen instant (public-api.md section 16.2). The instants that rows
store, the audit columns and the instants of revisions and activity
entries, come from PostgreSQL's clock instead (step 6). `App::after_context` registers the system clock; the
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
   path sets the audit columns (ADR 0011 sections 2 and 7), from
   PostgreSQL's clock (step 6). The `Local::now()` examples of the vendored
   Loco skill (`doctrine.md`, `recipes/model-and-migration.md`) do not apply
   here.
6. **Stored instants come from PostgreSQL's clock.** The audit columns
   (`created_at`, `updated_at`, `deleted_at`) and the instants of revisions
   and activity entries are set by the database, never from the app clock:
   the `CURRENT_TIMESTAMP` column defaults on insert, and
   `Expr::current_timestamp()` in an UPDATE (research: DECISIONS R15, D90).
   An account's `last_login_at` follows the same rule, a reading that
   extends D90 to that column so that it equals the `occurred_at` of the
   account's newest `login` entry (`users::ActiveModel::record_sign_in`
   writes `GREATEST(last_login_at, CURRENT_TIMESTAMP)` and says why).
   Both give the start of the transaction, so every row that one save
   writes carries the same instant, and model write methods take no "now"
   for them (ADR 0011 section 7's `update(txn, actor, changes)`). The app
   clock still decides the rules that compare with "now" (today's date,
   expiry, throttle windows). A test of a stored instant compares it with
   PostgreSQL's own `now()` in the same transaction, or with two
   `clock_timestamp()` readings around the call, never with the
   `TestClock`. That reads the database's clock, not the test process's, so
   step 4's list of wall-clock reads stays complete. The one exception is the
   importer, which keeps the legacy `created_at` and `updated_at` of imported
   rows and writes them explicitly (ADR 0011 section 2, recipe 4).

---

## 8. Record an activity entry

Every data change, sign-in or account event and media operation writes one
entry to the append-only `activity_log` (ADR 0011 section 5). The pure parts
are in `src/domain/activity.rs` (`Actor`, `Action`, `check_entry`,
`changed_field_names`); the model is `src/models/activity_log.rs`.

1. **Build the entry** from `Entry::new(Action::...)` with struct-update
   syntax, setting only what the action has (the example on
   `models::activity_log::Entry`):
   - `entity`: `EntityRef::record("<table>", id)`, or
     `EntityRef::table("<table>")` for an entry about a whole table (the
     importer writes one `import` entry per table per run, with counts);
   - `revision_id`: the revision the action wrote, if any; it needs a
     record. A create, update or revert gets its entry from
     `models::revisions::record_save`, which sets the record and the
     revision for it (recipe 2 step 3), so an entry is built by hand only
     for an action that writes no revision, such as a delete or a restore;
   - `label`: the display label of a content record (its title or name), so
     the log stays readable after a purge. Only with an `entity`, and never
     on a sign-in or account entry or for `users`, `authors` or
     `colleagues`, which are people: their entries keep only the id. A media
     operation has no label: its paths go in `details`, and the trash reads
     a deleted file's former path from there. The table's CHECKs refuse
     the same (ADR 0011 section 5), because a label can never be removed;
   - `details`: a JSON object, never the changed values. For an update,
     `changed_fields_details(&changed_field_names(&before, &after))` over
     the revision snapshots before and after (`{"changed_fields": [...]}`);
     for a media operation, the old and new path and the size; for a batch,
     the counts;
   - `request_id`: the controller takes it from Loco's `LocoRequestId`
     request extension, so the entries of one request group together;
     `None` in tasks, jobs and the importer. It is always a UUID v4 that
     Loco made: Loco would keep a client's `x-request-id` header as the id
     (loco-rs 1.2.0 `src/controller/middleware/request_id.rs`), so
     `App::after_routes` drops that header from every request before Loco's
     middlewares run (`src/controllers/request_id.rs`; research: DECISIONS
     R23 (D119)), and no entry holds text a caller chose, signed in or not.
     The same id is in the request's log lines and the response's
     `x-request-id` header. Do not read the header yourself;
   - `ip`: only for sign-in and account events, the address that Loco's
     `remote_ip` middleware resolved (ADR 0023 §2), canonical (an IPv4
     address in its IPv6 form becomes the IPv4 address); `None` where the
     configuration leaves the middleware off.

   An admin handler gets the request id and the address from the extractor
   `controllers::admin::client_context::ClientContext`, whose inner
   `activity_log::RequestContext` it passes down to the model method that
   writes the entry, as `users::ActiveModel::record_sign_in`, `log_out` and
   `users::record_failed_sign_in` do. A sign-in or account entry is about
   the account: `entity` is `EntityRef::record("users", id)`. Its actor is
   the account that acted (`Actor::User(id)`), which is how the
   anonymisation finds the entries whose `ip` it clears (`actor_id = <id>`,
   ADR 0011 section 6). The one exception so far is `login_failed`: its
   caller has not signed in, so it is written by `Actor::System`, and
   section 6 names it separately ("`login_failed` entries for the
   account"), so a failed attempt is never put on the account's owner.
   Every other account entry, including one written before a full sign-in
   (`password_changed` by a reset link, `recovery_code_used` at the
   two-factor step), takes the account as its actor unless its own slice
   decides otherwise and amends section 6 to match; `account_locked` is
   settled by the ADR that adds it (login-throttle-address).
2. **Record it on the change's transaction**:
   `activity_log::record(&txn, actor, entry)`, with the actor passed down
   from the controller (`Actor::User(session.user.id)`, the account of the
   handler's `AdminSession`, recipe 1 step 7), or `Actor::System` in tasks
   and queue jobs and `Actor::Import` in the importer (ADR 0011 section 2). A
   failed insert then rolls the change back, and a rolled-back change leaves
   no entry. Only an entry that records no data change (a failed sign-in)
   goes on a connection of its own, so it survives the request's rollback:
   its model function takes the pool, `&DatabaseConnection`, never a
   transaction (`users::record_failed_sign_in(&ctx.db, ...)`), so it cannot
   be put on the request's transaction by mistake. A refused entry of a
   failed sign-in never changes its answer: the handler logs it and answers
   the same `401` as for an unknown address, so a database that refuses
   writes cannot tell which addresses belong to an account (ADR 0006
   section 4, ADR 0023 section 3; `controllers::admin::auth`).
   `record` refuses an entry that breaks a rule of `check_entry` before it
   writes anything. Leave `occurred_at` alone: it comes from PostgreSQL's
   clock (recipe 7 step 6).
3. **Never update or delete an entry.** The table's triggers refuse every
   `DELETE` and `TRUNCATE` (so `App::truncate` never covers the table), and
   every `UPDATE` but one: inside a transaction that called
   `activity_log::with_permanent_delete_flag(&txn)`, an UPDATE that only
   sets `ip` to NULL. Only the anonymisation of a user issues it, for that
   user's own sign-in and account entries (ADR 0011 section 6). The
   `revisions` table reads the same flag: it refuses every `UPDATE` and
   `TRUNCATE`, and a `DELETE` outside a permanent-delete transaction, so a
   purge (and the anonymisation, for the account record's revisions) sets
   it, writes the record, and then deletes the revisions with
   `models::revisions::delete_for_record`, which locks the record's row
   first (recipe 2 step 7). Set
   the flag on the outermost transaction of the permanent delete, never on a
   nested one: a nested transaction is a savepoint, and committing it keeps
   a `SET LOCAL` until the outer transaction ends (only rolling it back
   undoes the flag), so the work after it would run with the flag too.
4. **A new action** needs an ADR that adds it to ADR 0011 section 5 first.
   Then, in one PR: a row in the `actions!` table of `src/domain/activity.rs`
   (in its group; `Action::ALL` follows), a migration that drops and re-adds
   the `activity_log_action_known` CHECK with the whole list (ADR 0009's
   rule for string codes), and the schema snapshot. A new account action
   also drops and re-adds `activity_log_ip_account_only` and
   `activity_log_label_content_only` with the whole list of account
   actions. `tests/models/activity_log.rs` fails while the CHECKs and
   `Action::ALL` differ.
5. **Tests**: model tests assert the entries each operation writes (action,
   actor, entity, label, the changed field names) and that a failed entry
   rolls the change back. Compare `occurred_at` with the database's clock,
   as recipe 7 step 6 says.

---

## 9. Add a queue worker

Out-of-request work runs on Loco's Postgres queue inside the web process,
never with `tokio::spawn` (ADR 0016 §3). A recurring job is enqueued by a
task that a CronJob runs (recipe 6), because CronJob pods never mount the
uploads volume (ADR 0015 §3). `AGENTS.md`, "Queue workers", has the rules
this recipe follows.

1. **Generate** it with `cargo loco generate worker <name>`: it writes
   `src/workers/<name>.rs` with `Worker`, `WorkerArgs` and `class_name()`
   (the job's name in `jobs dump` and `jobs retry`), and registers it in
   `App::connect_workers`. Rewrite the generated test,
   `tests/workers/<name>.rs`, on `tests/support::with_app` (recipe 1 step
   11); `config/test.yaml` runs jobs inline (`ForegroundBlocking`), so
   `perform_later` returns the job's own outcome.
   `config/development.yaml` queues jobs (`BackgroundQueue`), and a plain
   `cargo loco start` runs none of them: start the server locally with
   `cargo loco start --server-and-worker`, as the image does, to see the
   job run (`AGENTS.md`, "Queue workers").
2. **Arguments**: ids and small values only, never file contents
   (ADR 0016 §3). Do not refuse unknown members of `WorkerArgs`
   (`deny_unknown_fields`): Loco 1.2.0 merges a failed job's error into its
   stored data as an `error` member, and `jobs retry` runs the job with that
   data. Check the arguments at the start of `perform`, before any work,
   with a pure rule in `src/domain/` where there is one.
3. **Handles**: what the job needs besides `ctx.db` (settings, a registry)
   comes from a struct that `App::after_context` stores in the shared
   store, as `OwnedFiles` and `MediaUploads` do, so a test can replace it
   with a volume and registrations of its own. The work itself lives on a
   model or a model module without an entity, which takes the connection
   and the handles as arguments (ADR 0008 rules 2 and 3); blocking
   file-system calls go through `tokio::task::spawn_blocking`, which the
   job awaits.
4. **Idempotent**: two runs give the result of one; a test runs the job
   twice on the same input and checks that the second changes nothing and
   does not fail (ADR 0016, "What must be true").
5. **Outcome**: a job that works on a row records its outcome there
   (ADR 0016 §3), and a reconciliation task re-enqueues the rows still
   marked as not done. A job whose work is files (`DeleteOwnedFiles`, the
   media trash purge, the sitemap) has no row to mark: its outcome is the
   state of the files, a failure leaves only orphans that nothing refers to
   (ADR 0011 §6), and its recovery is an operator's `jobs retry` once the
   cause is fixed (`AGENTS.md`, "Queue workers"). Either way a failure
   returns an error, and never loops with `sleep` or retries by itself.
   Let the job try all of its items before it fails, and name each failed
   item in the error, which Loco stores on the failed job.
6. **Enqueue after the commit**: a job that follows a database change is
   enqueued only once that change has committed, so it never acts on a
   change that rolls back. `commit_then_enqueue` in
   `src/workers/delete_owned_files.rs` is the pattern: it takes the
   transaction by value, commits, then enqueues with `perform_all_later`
   (one round trip for several jobs), and after the commit it logs a
   failed enqueue rather than fail the request whose change stands.
7. **Tests** in `tests/workers/<name>.rs`: the work and its idempotence,
   each refusal of the arguments, each failure (it returns an error, and
   the other items were still done), the order around the commit (a commit
   that fails enqueues nothing), and, with a `PgQueue` on the test's pool
   in `BackgroundQueue` mode, the job as the queue stores it
   (`tests/workers/delete_owned_files.rs` has the set-up).
