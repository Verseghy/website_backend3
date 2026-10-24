---
name: backend3
description: Use when adding a feature to website_backend3 — a vertical slice, a new admin-editable entity (soft delete, revisions, activity log), a public /api/v1 endpoint with its test fixtures, or a step of the legacy MySQL importer. Gives backend3's own recipes on top of the vendored Loco skill, with the ADRs and file paths each step follows.
---

# backend3 recipes

These recipes are backend3's way of doing its four repeated tasks. They sit on
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

A slice carries one feature end to end in one PR (ADR 0012 section 1).

1. **Branch** from `master` (a PR may build on at most one other open PR) and
   open the PR as a **draft** right away.
2. **Schema**: a migration in `migration/src/` (`cargo loco generate migration`
   or `generate model`), following ADR 0009 (naming, `bigint` identity ids,
   `timestamptz`, `CHECK`s, an index on every foreign key) and ADR 0010 for
   instants. If the entity is admin-editable, it joins the safety net in this
   PR: recipe 2.
3. **Entities**: `cargo loco db migrate`, then `cargo loco db entities`. Never
   edit `src/models/_entities/`. Both `db entities` and `generate model` shell
   out to `sea-orm-cli`, which is not part of the build: install the version
   that matches the locked `sea-orm` once, with
   `cargo install sea-orm-cli@2.0.4 --locked` (the `AGENTS.md` commands).
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
   `i64` field carries `#[ts(type = "number")]`).
7. **Admin API**: thin handlers in `src/controllers/admin/<name>.rs`. Every
   route is declared with its permission in the admin route table (deny by
   default, ADR 0007 sections 4-5); errors use the envelope and `ErrorCode`
   of ADR 0021 section 4.
8. **Public endpoint**, if the feature has one: recipe 3.
9. **Admin screen** under `admin/src/`: resources through backend3's own data
   provider in `admin/src/data-provider/` (never `ra-data-json-server` or
   `ra-data-simple-rest`), every string from `admin/src/i18n/hu.ts` with the
   ADR 0021 section 6 terms, and the trash and history patterns of ADR 0011
   section 8.
10. **Importer step** for the feature's legacy tables: recipe 4.
11. **Tests at every layer**: domain unit tests and proptests; model tests in
    `tests/models/`; request tests in `tests/requests/`; admin unit tests next
    to the components (`npm test`); one Playwright test of the main flow
    (`npm run e2e`). Tests that a generator writes use `boot_test` and
    `serial_test`, which backend3 does not depend on: rewrite them to
    `tests/support::with_app` or `with_server`, which give each test its own
    database and always drop it.
12. **Regenerate the contract**: run the test suite so ts-rs rewrites
    `admin/src/bindings/` and the OpenAPI snapshot updates, review both, and
    commit them (the `Contract` job fails on drift, ADR 0005 section 4).
13. **Docs**: update the ADR or spec the slice changes, and `AGENTS.md`,
    `CLAUDE.md` or this skill if a rule, command or path changed.
14. **Gate**: the "Before you call it done" block of `AGENTS.md`.
15. **PR**: a title such as `feat(db,api,admin): add labels with trash and history`
    (ADR 0012 section 3), a body that names the parity IDs and ADRs and has one
    `Closes #N` line per finished parity issue, about 500 hand-written lines or
    fewer (split by feature, never by layer). Mark it ready only when every
    check is green; Tide then merges it.

---

## 2. Add an entity under the safety net

Every admin-editable entity gets audit columns, soft delete with a trash,
revisions and activity-log entries in the PR that adds it (ADR 0011 section 1).

1. **Columns** (ADR 0011 section 2) in the entity's migration:
   `created_at`, `updated_at` as `timestamptz NOT NULL`; `created_by`,
   `updated_by` as nullable `bigint` foreign keys to `users(id)`; plus
   `deleted_at timestamptz NULL` and `deleted_by` unless the entity cannot be
   deleted (like site settings). Index the foreign keys.
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
   every public query on every table it touches (ADR 0011 section 3).
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
   that every public read path ignores trashed rows.

---

## 3. Add a public endpoint with its fixtures

The public API at `/api/v1` replaces backend2's GraphQL API. Its contract is
`docs/design/public-api.md`; the one-day utoipa spike (ADR 0005 section 5)
must have passed before the first `/api/v1` slice.

1. **Find the endpoint** in `public-api.md` section 7: path, parameters,
   schema (section 6), visibility rules (section 3), paging (section 5),
   errors (section 8), caching (section 9), size limits (section 10) and its
   "Tests" paragraph.
2. **DTOs** in `src/dtos/v1/`: responses with `utoipa::ToSchema`, query and
   path parameters with `utoipa::IntoParams`, camelCase through serde (ADR
   0005 sections 2-3). Instants, dates and event wall times use the forms of
   ADR 0010 and the shared newtypes.
3. **Pure parts first** in `src/domain/` (parameter parsing per section 2.5,
   the keyset cursor `src/domain/cursor.rs`, date ranges), with unit tests for
   every documented row and the proptest properties of `public-api.md`
   section 16.1.
4. **Query on the model** (`posts::Model::list_published(db, filter, cursor, limit)`
   style) using the live-row helper of recipe 2.
5. **Handler** in `src/controllers/v1/<name>.rs` with `#[utoipa::path(...)]`,
   registered through Loco `Routes`, and added to the OpenAPI derive. Set
   `Cache-Control` and the weak `ETag` as section 9 says.
6. **Fixtures**: add every fixture that `public-api.md` section 17.1 lists for
   this endpoint, under the name used there, as invented data (never copied
   from production) loaded by the request tests. Check that each shape of the
   ADR 0019 section 2 table that the endpoint serves has its named fixture.
   Shapes that only legacy data has go into the import-path fixtures of
   section 17.2 (recipe 4).
7. **Specification tests** in `tests/requests/public_<name>.rs`: one test per
   rule, named after the rule (for example
   `home_paging_visits_every_published_post_once`), asserting full order,
   parsed JSON field by field, the response against its schema, and the
   headers of section 16.2. Time-dependent tests inject "now".
8. **Contract**: update the OpenAPI insta snapshot, bump `info.version`'s
   minor by hand for an addition, and keep the route-coverage test green
   (ADR 0005 section 4). A snapshot change is an API change: say so in the PR.
9. **PR**: `feat(api): ...` (or with `db`/`admin` scopes if the slice adds
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
