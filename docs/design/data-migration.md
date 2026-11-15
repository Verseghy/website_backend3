# Data migration: legacy MySQL to backend3 PostgreSQL

This is the specification of backend3's import task. The task copies the legacy `verseghy` MySQL database (written by website_backend, read by website_backend2) into backend3's PostgreSQL database. It runs many times as a rehearsal and once for real, in the cut-over window.

Binding decisions behind this document:

| ADR | What it fixes for the importer |
|---|---|
| [ADR 0002](adr/0002-postgresql-as-the-database.md) | the target is PostgreSQL |
| [ADR 0003](adr/0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md) | nightly rehearsals; one import in an evening write freeze |
| [ADR 0009](adr/0009-clean-domain-schema-with-preserved-integer-ids.md) | the target tables and constraints; ids preserved |
| [ADR 0010](adr/0010-all-instants-stored-as-timestamptz.md) | every instant is `timestamptz`, calendar dates included (as Europe/Budapest midnights); event wall times are Europe/Budapest |
| [ADR 0011](adr/0011-safety-net-soft-delete-revisions-and-an-activity-log-for-eve.md) | audit columns, soft delete, revisions, activity log |
| [ADR 0013](adr/0013-rich-text-editor-and-html-sanitiser.md) | the editor schema (the content-v1 shape of decision 15b) and the sanitiser allow-list that the body normaliser targets; [editor.md](editor.md) §3 defines content-v1 and §9 lists the import rules |
| [ADR 0017](adr/0017-legacy-data-import-normalisation-and-defect-resolution.md) | body normalisation, the editor-signed defect list, the dump flags |
| [ADR 0018](adr/0018-content-model-publishing-pages-menu-tree-canteen-week-editor.md) | posts, pages and menu model |

Counts in this document come from the production dump of 2026-10-23 17:01 UTC unless marked **[500]**, which means the 500-post / 24-page harvest of the public API (research: SYNTHESIS §4.7). They are reference values. The importer measures the real values on every run and reports them.

## Contents

1. [Scope](#1-scope)
2. [Pipeline](#2-pipeline)
3. [Source requirements](#3-source-requirements)
4. [Running the importer](#4-running-the-importer)
5. [Global transformation rules](#5-global-transformation-rules)
6. [Table mappings](#6-table-mappings)
7. [Rich-text normalisation](#7-rich-text-normalisation)
8. [Defect resolutions (editor sign-off)](#8-defect-resolutions-editor-sign-off)
9. [Not migrated](#9-not-migrated)
10. [Verification](#10-verification)
11. [Idempotence, dry run and determinism](#11-idempotence-dry-run-and-determinism)
12. [Testing the importer](#12-testing-the-importer)
13. [Open items](#13-open-items)
14. [References](#14-references)

## 1. Scope

In scope:

- every row of the 14 legacy tables that hold content, accounts and grants, mapped to 14 of the 15 target tables of ADR 0009 (the 15th, `roles`, is seeded by a migration and verified against the source);
- the rich-text bodies of posts and pages, normalised once to the new editor's form;
- the inline `data:` images inside bodies, which become files on the uploads volume;
- checks that every file the database points at exists on the uploads volume.

Out of scope:

- the media files themselves. They stay where they are, and their paths and URLs do not change (ADR 0015);
- the content that release 1 adds (notices, partner banners, site settings, Diáktoll issues). It has no legacy rows. ADR 0019 defines what is seeded at the cut-over (the current partner banners, the site settings, one Diáktoll issue); how that seed runs is open item 7 of §13;
- anything that writes to the legacy MySQL database. The importer only reads it.

## 2. Pipeline

```mermaid
flowchart LR
    prod[(production MySQL<br/>verseghy)] -->|"mysqldump --lock-all-tables"| dump[/dump file/]
    dump -->|load into a throwaway<br/>MySQL 8.4 container| drill[(source MySQL)]
    prod -.->|"cut-over only: frozen, super_read_only"| task
    drill --> task["import_mysql task<br/>(read under LOCK TABLES READ)"]
    res[/resolutions file<br/>editor-signed/] --> task
    task -->|pure transforms<br/>src/domain| plan[target rows + files + report]
    plan -->|one transaction| pg[(backend3 PostgreSQL)]
    plan -->|write-if-absent| vol[(uploads volume<br/>/storage)]
    pg --> verify{verification<br/>section 10}
    vol --> verify
    verify -->|pass| commit[commit]
    verify -->|fail| rollback[rollback + report]
```

The task reads the whole source in one locked session, computes every target row in memory with pure functions, writes everything in one transaction, verifies inside that transaction, and commits only if every check passes. The legacy data is 6.05 MiB, so the whole run takes seconds.

## 3. Source requirements

### 3.1 The dump

| Requirement | Why |
|---|---|
| Every dump used as an import source is taken with `mysqldump --lock-all-tables` (decision 32b) | The legacy tables are not transactional, so only a global read lock gives a consistent dump. `--lock-all-tables` holds that lock for the whole dump, which takes seconds at this size. |
| Default time-zone handling kept (never `--skip-tz-utc`) | `TIMESTAMP` values are then written in UTC, which §5.2 relies on. |
| Character set `utf8mb4` (the MySQL 8.4 default) | Hungarian `ő` and `ű` are outside `latin1`. |
| Contains the `verseghy` database | It is the only source database. |
| Handled as personal data | The dump contains the admin accounts. It is never committed, never attached to an issue or PR, and never uploaded to CI. It is stored only in the locations the private runbook names and on the LAN-only staging environment (ADR 0003). |

### 3.2 The production database before cut-over

Production MySQL is left as it is until the cut-over (decision 32b):

- no engine conversion, no change of the database default charset, no data clean-up. Every clean-up happens in the importer;
- backend2's hand-run migration 12 (website_backend2@7ba71da:`migrations/12-menu_items_fix_types.sql`) is **never** applied. It fails on today's data (`ERROR 1265 Data truncated for column 'lft'`), and after a back-fill it would break menu creation in the current admin (research: gap-07 §4).

### 3.3 How the importer reads the source

| Rule | Detail |
|---|---|
| Source kinds | Rehearsals read a throwaway MySQL 8.4 container loaded from a dump. The cut-over import reads the frozen production server directly with read-only access; the final dump taken just before it is the reference copy of the same frozen state (ADR 0003, [cutover.md](cutover.md)). The code path is the same. |
| Account | The source account needs only `SELECT` and `LOCK TABLES` on `verseghy`. Rehearsals use the throwaway container's own account. The cut-over uses a dedicated account with exactly these grants. The private runbook creates it in phase 1 of the window, together with the credential rotation and before the freeze ([cutover.md](cutover.md) §6, §11), because `super_read_only` also refuses account-management statements; the dress rehearsal verifies this. Creating it changes no `verseghy` table (decision 32b). Its connection URL is a key in the shared `backend-secrets` ExternalSecret (decision 33), exposed to the task as `IMPORT_SOURCE_DATABASE_URL` (§4.1). The key is removed once G1-G7 pass ([cutover.md](cutover.md) §7). Dropping the account is an account-management statement too, so it waits until the maintainer decommissions the legacy database (decision 41); until then the account can only read a frozen database. |
| Session settings | `SET time_zone = '+00:00'` (so `TIMESTAMP` columns come back in UTC), `SET NAMES utf8mb4`, `SET information_schema_stats_expiry = 0` (so `AUTO_INCREMENT` values are current), sent as the session's first statement, so they hold whatever the URL's `timezone` and `charset` parameters set at connect. |
| Source tables | The importer reads 17 of the 20 legacy tables: the 14 it migrates (§6) and the 3 it verifies without copying them (`roles`, §6.13; `permissions` and `role_has_permissions`, §6.15). That allow-list is `src/domain/import_mysql/source_tables.rs`. Every read goes through one query layer (`tasks::import_mysql::source::select_rows`) that refuses any other table, naming it, before it sends anything. Only the layer's file and the source session's (`tasks::import_mysql::session`) send statements to the source: `tests/architecture.rs` fails when another file of the import task names the driver's statement or connection API, and when code outside the import task names the driver at all. The session's `LOCK TABLES`, the schema fingerprint and its `AUTO_INCREMENT` and timestamp reads name their tables from the same list (`src/domain/import_mysql/source_session.rs`). So `newsletter` (the retired newsletter's subscriber list, NWS-01), `password_resets` and `migrations` are never selected, locked or fingerprinted, although the source account may read them. |
| Consistency | One session takes `LOCK TABLES <every source table> READ`, reads every row, reads every table's `AUTO_INCREMENT` and the latest timestamp of §4.1, then unlocks. Even a writable source cannot change in the middle of the read, and a `LOCK TABLES` session can read only the tables it locked, so the 3 tables off the list stay unread under the lock as well. A session that fails closes its connection, which ends its locks. |
| Schema fingerprint | Before reading rows, the importer compares every source table's columns, types (`COLUMN_TYPE`) and nullability, as `information_schema.COLUMNS` reports them, with the expected legacy DDL (research: gap-03 §5), `src/tasks/import_mysql/expected_schema.rs`, which tests keep equal to `tests/fixtures/legacy/schema.sql`. Any difference stops the run with a message that names every table and column that differs. It compares once before the lock, so that a missing table is named rather than failing `LOCK TABLES` with an error number, and once under the lock, where it is authoritative. |
| Driver | Plain `sqlx` queries with its `mysql` feature, behind a Cargo feature `import-mysql`. There are no Sea-ORM entities for the legacy tables. `import-mysql` is a default feature until the removal PR (open item 9). The PAC production build (decision 34a) and the GitHub Actions checks build with it, so the production image can run the cut-over import. CI also builds and tests once without `import-mysql`, to prove that the rest of backend3 does not depend on it. |

## 4. Running the importer

### 4.1 Interface

The importer is a Loco task named `import_mysql`. Only the task touches `AppContext` and the environment; everything below it takes explicit arguments (ADR 0008).

```sh
cargo loco task import_mysql \
  mode:dry-run \
  snapshot_at:2026-10-23T17:01:02Z \
  resolutions:data/import/resolutions.toml \
  storage_root:/path/to/storage \
  report:/path/to/report-dir
```

| Argument | Required | Meaning |
|---|---|---|
| `mode` | no, default `dry-run` | `dry-run` computes, writes and verifies everything, then rolls back and writes no files to `storage_root`. `apply` commits. |
| `snapshot_at` | yes | The moment the source was frozen or dumped (RFC 3339, UTC: `Z` or `+00:00`; another offset is refused, and the message gives the same instant in UTC). It is the `created_at` and `updated_at` of rows whose legacy table has no timestamps. It must not be earlier than any timestamp in the source, nor later than the start of the run. The source's timestamps are the instants at which a row was written: every `created_at`, `updated_at` and `deleted_at` of the tables the import reads (`SOURCE_TIMESTAMPS` in `src/domain/import_mysql/source_session.rs`), read by the rules of §5.2; the event times (`events_data.date_from`, `date_to`) and the calendar days (`posts_data.date`, `canteen_data.date`) are not. The source session reads the latest of them under the lock and refuses an earlier `snapshot_at`, naming that column and its instant. For this check only, a `canteen_data` audit value that a clock change leaves with two readings counts at the later one, `+01:00`, so that the check errs towards refusing: a value in the repeated autumn hour at its second instant, and one in the skipped spring hour at the offset in force before the change. |
| `resolutions` | no, default `/app/data/import/resolutions.toml` | Path of the editor-signed resolution file (§8). The default is where the image keeps the repository's `data/import/resolutions.toml` (research: DECISIONS R9, D12): the image ships the repository's `data/` directory as `/app/data/` ([ADR 0045](adr/0045-versioned-task-data-in-the-image-at-app-data.md)), so `data/import/` is `/app/data/import/` in the pod, and `data/seed/` (the release-1 seed files, §13 item 7) is `/app/data/seed/`. It must be a file the task can open, of at most 1 MiB of UTF-8, that passes the validation of §8.2. |
| `storage_root` | yes | The directory that is served as `/storage` (the uploads volume's public root). Read for file checks, written only in `apply` mode for the inline images of §7. It must be a directory the task can list. |
| `report` | yes | An existing, empty directory for the run report (§11.3), outside `storage_root`: the media server publishes everything under `storage_root`, and the report stays private. The check compares canonical paths, so a symbolic link cannot hide it there. |
| `canteen_audit_local_before` | no, default `2022-01-21T00:00:00Z` | Which `canteen_data` audit values are Budapest wall time (§5.2, ADR 0010 open item): values before this instant are, values at or after it are UTC. An RFC 3339 instant, or `all`. The default is the end of the backend2 fix window; backend2's fix 03 shifted only the rows that existed then (322 rows; research: SYNTHESIS §4.4.2). |

Loco 1.2.0 splits each task argument at its first `:` (loco-rs@1.2.0:`src/cli.rs`, `parse_key_val`), so values such as RFC 3339 instants may contain colons. Loco ignores the arguments a task does not read and keeps the last of two with the same key, so the task refuses every key that is not in the table above, naming it: a misspelt optional argument cannot fall back to its default. Every refusal names the argument and says what to write instead. The task checks the arguments, then the source setting, then the paths, then reads and validates the resolution file (§8.2), all before it takes the task lock (ADR 0016 §4); a run that finds the lock held exits 0 and writes nothing.

The source connection string comes from the environment variable `IMPORT_SOURCE_DATABASE_URL` (at the cut-over, a key of the shared `backend-secrets` ExternalSecret, §3.3), which `config/production.yaml` reads into the setting `settings.import.source_database_url`, with the empty string as its default. Only the task reads the setting; it builds the source connection and passes it, with the target and the `ImportOptions`, to the import as arguments. It is never a command-line argument, so it never lands in shell history or a process list. The empty string and the placeholder `disabled`, which the secret holds outside the cut-over window, both mean unset: the task then refuses to start, naming the setting. A value that is not a `mysql://` URL the MySQL driver accepts is refused the same way. In the `test`, `development` and `e2e` environments the task also refuses a source that is not local, naming the setting, before it connects (layer 3 of the guardrails, ADR 0024 §4): it checks where the driver would connect, which must be the host `localhost`, `127.0.0.1` or `::1`, or a Unix socket named by the URL's `socket` parameter. A container-network name such as `mysql` is not local. `production` applies no such rule, because the staging rehearsals and the cut-over import are operator runs. Locally and in CI the source is the `mysql` service of `compose.yaml` (§12), `mysql://loco:loco@127.0.0.1:3306/verseghy`. The URL holds the source password, so no message, log line or report shows any part of it, not even its host. An error of the source keeps only the MySQL error number and SQLSTATE, or the operating system's text of a socket error. It never keeps the server's text, which can name the account, or the driver's, which can name the host (a failed TLS handshake names the server name it expected); those errors get a fixed description of their kind. The source connection is opened on first use, by the one session that reads the source. The target is backend3's own configured database.

### 4.2 Refusal rules

The task refuses to start, or stops and rolls back, with a message that tells the operator what to fix, when:

| Condition | Message names |
|---|---|
| the target's `roles` rows, or the CHECK on `user_permissions`, differ from the roles and the 16 permission names of the code (`src/domain/authz.rs`, ADR 0007 §1, §3), onto which §6.13-§6.15 map the legacy data by name | each role or permission name that differs |
| any table the import writes already holds a row (`roles`, seeded by a migration, is only read) | the first non-empty table. The importer never merges into existing data, so it can never run again after go-live. The tables are `users`, `user_roles`, `user_permissions`, the entity and join tables of §6, `revisions` and `activity_log` (`IMPORT_TARGET_TABLES` in `src/domain/import_mysql/target.rs`), checked in that order; one that the target's schema does not have yet holds no row, and a warning in the run's log names it. Tests keep the list equal to the insert order of §4.3 and every table of the migrated schema on it or on a list of tables the import never writes, so a misspelt or renamed table cannot be skipped for good. The import locks them against other writers (`SHARE ROW EXCLUSIVE`) before it checks, so they stay as checked until it commits. `AGENTS.md` states the pre-import invariant that keeps them empty in production (research: DECISIONS R9, D10). |
| the source schema differs from the expected DDL | every table and column that differs |
| `snapshot_at` is earlier than the latest timestamp in the source (§4.1) | that column and its instant |
| the resolution file does not parse or fails validation | the entry and field |
| an entry's precondition does not match the source: the row changed since sign-off, no longer has its defect, or is gone (a D3 entry pins the index image that editors set in the current admin, so it holds once the rule no longer finds the post and the post has that image, and fails while the image is missing or differs, §8.1) | the defect id, the row id, the expected and the found value |
| a defect rule finds a row that the resolution file does not cover (except D10, whose entries are optional: an unlisted page is imported as published, §8.1) | the defect id and the row id |
| a target `NOT NULL`, `CHECK`, unique or foreign-key rule would fail | the table, row id and rule |
| an inline image file would overwrite a different existing file | the path |
| any verification check of §10 fails | the check id and the first failing rows |

In `apply`, the first refusal stops the run and rolls it back. A dry run treats the three refusals about the data differently, an entry whose precondition does not hold, an instance that the resolution file does not cover, and a row that a target rule refuses ([ADR 0047](adr/0047-import-dry-runs-collect-every-defect-and-refusal.md)): it records each one and goes on. An uncovered instance, and one whose entry no longer holds, take the defect's proposed default (the proposed resolution of §8.1, where the defect's rule can compute it from the source) or, without one, keep their source value; an entry for a row that the rule no longer finds changes nothing; a row that a target rule refuses is reported with its table, id and rule and left out (the table steps retry a failing batch row by row inside savepoints to find each one). At its end the dry run fails: its error counts the problems and names each of them (the first 50, then a count), so the task exits non-zero, and the report's Defects part lists every one and `signoff.md` every instance (§8.2, §11.3). Every other refusal stops a dry run as it stops `apply`. One generic detector, `DefectDetector` in `src/domain/import_mysql/defects.rs`, applies these rules to every defect: each table step brings the defect's rule, which detects every instance from the source rows on every run, and the detector pairs the instances with the entries of the defect's section by the post, page, date, label group or menu level they are about. How a defect's entries work follows §8.1 and belongs to the defect, not to the step (ADR 0047 §4): D1, D2, D5, D6, D8, D9 and N7 need an entry for every instance; D10's entries are optional, so an unlisted page takes the default (`published`) in both modes without a problem; a D3 entry pins the fix that editors made in the source, so it holds when the rule no longer finds its post and the post has the signed index image, and applies nothing; D4 and D7 have no entries and are only listed.

Database-touching agent commands are additionally blocked by the repository guardrails when `DATABASE_URL` is not local (ADR 0024). The production run is done by the operator in the cut-over window (ADR 0003).

### 4.3 Order of operations

```mermaid
sequenceDiagram
    participant T as import_mysql task
    participant S as source MySQL
    participant D as src/domain (pure)
    participant F as storage_root
    participant P as PostgreSQL
    T->>T: validate arguments and resolution file
    T->>P: BEGIN
    T->>P: roles and the user_permissions CHECK equal the code
    T->>P: lock the tables the import writes, refuse a row in any
    T->>S: session settings, fingerprint, LOCK TABLES ... READ, fingerprint
    T->>S: AUTO_INCREMENT values, latest timestamp, read all rows
    T->>S: UNLOCK TABLES
    T->>D: build target rows, apply resolutions, normalise bodies, compute menu positions
    D-->>T: rows, inline images, expected digests, report data
    T->>F: write inline images if absent (apply) or to a temp dir (dry-run)
    T->>P: insert in foreign-key order, revisions, activity entry
    T->>P: set identity sequences
    T->>P: verification queries and read-back digests (section 10)
    alt every check passes and mode is apply
        T->>P: COMMIT
    else dry-run or a failed check
        T->>P: ROLLBACK
    end
    T->>T: write the report
```

The whole run is one transaction on the target (`tasks::import_mysql::target::in_import_transaction`), which begins before the source is read: its first steps check the code tables and the empty target (§4.2), so a target that would be refused is refused before the source is locked. `apply` commits it, `dry-run` rolls it back, and a failed run always rolls back. It is always `READ COMMITTED`, whatever isolation level the server, database or role sets as the default: the guard locks the tables after the transaction's first query, and under `REPEATABLE READ` it would keep that query's snapshot and miss a row committed while its lock waited.

Insert order: `users`, `user_roles`, `user_permissions`, `authors`, `labels`, `posts`, `post_labels`, `dishes`, `canteen_days`, `canteen_day_dishes`, `events`, `colleagues`, `pages`, `menu_items` (parents before children), then the revisions and the activity-log entry of §5.4. `roles` is seeded by a migration before the import and is only read.

Rows are written with bulk inserts that carry every value explicitly. Model save hooks do not run, so no hook can overwrite a preserved timestamp or generate a second token. The importer calls the domain validators on every row, except the save-time rules that the resolution list or §6-§7 leave to the next edit: the date requirement on drafts (D5), the 2-3 dish rule (D7), the length limits (§5.3, reported), the mandatory image alt (decision 15, §7.1) and the images-only rule for gallery entries (the 5 legacy non-image entries stay, §6.1, [parity.md](parity.md) POS-F02). Each exemption is a named flag in the importer. Every exempted row is listed in the report. Database constraints are never exempted.

## 5. Global transformation rules

### 5.1 Ids and sequences

| Rule | Detail |
|---|---|
| Ids preserved | Every legacy row keeps its id. Ids are never renumbered. Gaps from hard deletes stay gaps (for example pages 1-7). |
| Join tables | Have no id (ADR 0009). |
| Sequence reset | After the inserts, each identity sequence is set so that the next id is `greatest(max(id), legacy AUTO_INCREMENT - 1) + 1`, with `setval(pg_get_serial_sequence('<table>', 'id'), <next> - 1)`. No id that MySQL issued is reissued (posts: 620 issued, 576 rows). |
| Dropped rows | Ids of rows dropped by a resolution (merged labels, duplicate canteen days) are never reused either, by the same rule. |
| New rows | A row the importer creates (the optional generic author of D2) gets its id from the sequence after the reset. |

### 5.2 Time values

Every legacy time column falls into one of these kinds (ADR 0010):

| Legacy kind | Columns migrated | Rule |
|---|---|---|
| `TIMESTAMP` (UTC instant) | `users.created_at/updated_at`, `menu_items.created_at/updated_at` | Read in the `+00:00` session; stored as that UTC instant. A `NULL` becomes `snapshot_at` and is reported. |
| `DATETIME` audit column written by Laravel (UTC wall time) | `posts_data`, `pages`, `events_data`, `canteen_menus` `created_at/updated_at` | Interpreted as UTC. |
| `DATETIME` audit column of `canteen_data` | `canteen_data.created_at/updated_at` | Values before `canteen_audit_local_before` (default `2022-01-21T00:00:00Z`) are interpreted as Europe/Budapest wall time and converted to UTC, because backend2's fix 03 shifted them to local time. Values at or after it were written by Laravel in UTC and are interpreted as UTC. |
| `DATETIME` event time (Europe/Budapest wall time) | `events_data.date_from/date_to` | Converted Europe/Budapest to UTC. An ambiguous autumn time takes the earlier instant (`+02:00`). A non-existent spring time stops the run (expected: 0 rows; the legacy times are almost all `00:00`-`00:01` and `23:58`-`23:59`). Seconds are kept. |
| `DATE` | `posts_data.date`, `canteen_data.date` | Converted to the `timestamptz` of the Europe/Budapest midnight of that day (decision 9, ADR 0010 point 2, ADR 0018 §1.3). Post dates first get the D1 and D5 resolutions of §8. Every day from 1984 on has exactly one Budapest midnight, so this conversion never meets the spring gap or the autumn overlap; an earlier day without one is refused by the ADR 0010 conversion module and stops the import (§4.2). The conversion goes through `budapest::date_to_stored_midnight`, which also refuses every day after 31 December of `budapest::CHRONO_TZ_LAST_CHANGE_YEAR` (2099), where chrono-tz's Budapest table ends; such a row stops the import in the same way ([ADR 0041](adr/0041-budapest-midnight-parity-and-a-two-sided-tz-data-procedure.md) §2). |
| No timestamps in the legacy table | `posts_labels`, `posts_authors`, `colleagues_data`; the join tables | `created_at = updated_at = snapshot_at`, the import instant of ADR 0011. Join tables have no timestamps in the target either. |
| Not migrated | `roles`, `permissions`, `password_resets`, `newsletter`, `migrations`; `pages.deleted_at`, `menu_items.deleted_at` | See §9. |

The conversion uses the one Europe/Budapest module of ADR 0010. Every converted event is checked by the round trip of §10 (V11).

### 5.3 Text, numbers, booleans and NULLs

| Rule | Detail |
|---|---|
| Plain text | Copied verbatim: no trimming, no Unicode normalisation, newlines kept. Titles, descriptions, names and colleague fields contain 0 HTML entities (research: SYNTHESIS §4.7.1), so they are not decoded. |
| Encoding | Every string must be valid UTF-8. A decoding error stops the run. |
| Length | No database length limits apply (ADR 0009). The domain validation limits are checked and every legacy value over a limit is reported, not truncated. |
| Unsigned integers | `int unsigned` and `smallint unsigned` become `bigint` and `smallint`. The largest legacy id is 1,222. |
| Booleans | `tinyint(1)` 0 and 1 become `false` and `true`. Any other value stops the run. |
| NULL | A NULL stays NULL unless a rule in §6 or §8 says otherwise. A NULL bound for a `NOT NULL` target column stops the run, naming table, id and column. |
| Codes | `canteen_menus.type` and `colleagues_data.category` are copied as `smallint` and must be in range (0-2, 0-4). `menu_items.type` is copied as text and must be one of `page_link`, `internal_link`, `external_link`. |

### 5.4 Columns and records every imported entity row gets

| Column or record | Value |
|---|---|
| `created_by`, `updated_by`, `deleted_by` (ADR 0011) | NULL. The legacy data records no authorship of changes. |
| `deleted_at` (ADR 0011) | NULL. No legacy row is soft-deleted. |
| `posts.content_text` (ADR 0020) | Computed from the normalised body by `html_to_search_text`, the same pure function the post model runs on every write. |
| `import` revision (ADR 0011) | Every imported row of an entity table gets a revision with reason `import` that holds its imported state. The users snapshot follows ADR 0011's allow-list, so it never contains a password hash. Join-table rows are recorded in their parent's snapshot. |
| `import_original` revision (ADR 0011) | When §7 changed a body, the row first gets an `import_original` revision that holds the legacy body exactly as stored in MySQL (decision 19: "original kept as a revision"), then its `import` revision. Reverting to it later goes through the normal write path, so the sanitiser applies. |
| Activity log (ADR 0011) | One `import` entry per table per run, by the `import` actor, with the row count, `snapshot_at`, the SHA-256 of the resolution file and the ids changed by each resolution in its details. No per-row entries. |

## 6. Table mappings

Each subsection maps one target table. "Verbatim" means the value is copied unchanged after the global rules of §5. Every entity table also gets the columns and records of §5.4; join tables get none.

### 6.1 `posts` from `posts_data` (576 rows: 541 published, 35 drafts)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved |
| `title` | `title` | verbatim (longest 95 characters) |
| `color` | `color` | verbatim; must match `^#[0-9A-Fa-f]{6}$` (576 of 576 do) |
| `description` | `description` | verbatim plain text; NULL kept (165 rows) |
| `content` | `content` | normalised by §7; NULL kept (34 rows, 28 of them published; D4) |
| `content_text` | none | derived from the normalised `content` (§5.4, ADR 0020); `''` when `content` is NULL |
| `index_image` | `index_image` | verbatim disk-relative path `index/<32 hex>.<ext>` (575 of 575 non-NULL values); the file must exist (V7); never NULL on a published post (D3) |
| `author_id` | `author_id` | verbatim; NULL on 7 rows: published 55, 61, 66 (D2) and 4 drafts, which stay NULL |
| `images` | `images` | `jsonb` array of file names. A JSON array is kept in order. A JSON object (36 rows, written by Backpack after a removal) becomes the array of its values ordered by numeric key. Every element must match `^[0-9a-f]{32}\.[A-Za-z0-9]+$`. Any other JSON shape stops the run. 5,053 references; the 5 non-image entries (3 PDFs on post 59, an `.asd` and an `.mp3` on post 385) are kept and reported. |
| `date` | `date` | the Europe/Budapest midnight of the legacy day, as `timestamptz` (§5.2); +1 day for the 181 rows of D1 before the conversion; `1970-01-01` to NULL on the 11 drafts of D5; NULL kept on 5 drafts |
| `featured` | `featured` | boolean (5 rows true, all published) |
| `published` | `published` | boolean |
| `preview_token` | `previewToken` | verbatim (573 tokens, each exactly 80 characters of `[A-Za-z0-9]`, so URL-safe). The 3 drafts with NULL get a new token in the format of ADR 0018. Tokens must be unique. |
| `publish_at` | none | NULL (column exists, unused in release 1; decision 21) |
| `created_at`, `updated_at` | same | `DATETIME` written as UTC (§5.2) |

### 6.2 `labels` from `posts_labels` (93 rows)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved, except the merged duplicates of D8 |
| `name` | `name` | verbatim |
| `color` | `color` | verbatim; must match the colour pattern (93 of 93 do) |
| `created_at`, `updated_at` | none | `snapshot_at` |

D8 merges the 5 rows of 2 duplicate-name groups into 2 rows. Expected target rows: 90.

### 6.3 `post_labels` from `posts_pivot_labels_data` (655 rows)

| Target column | Source column | Rule |
|---|---|---|
| `post_id` | `posts_id` | verbatim |
| `label_id` | `labels_id` | verbatim, re-pointed to the surviving label for merged ids (D8) |

Pairs that become identical after re-pointing collapse into one row; the dry run reports how many. The legacy table has 0 dangling references in either direction.

### 6.4 `authors` from `posts_authors` (49 rows)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved |
| `name` | `name` | verbatim |
| `description` | `description` | verbatim; NULL kept (set on 6 rows, at most 27 characters) |
| `image` | `image` | verbatim bare file name in `authors_images/`; NULL kept; the file must exist (V7) |
| `created_at`, `updated_at` | none | `snapshot_at` |

If D2 chooses the generic author, the importer adds one row named `Szerkesztőség` with no description and no image.

### 6.5 `colleagues` from `colleagues_data` (70 rows)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved |
| `name` | `name` | verbatim; `NOT NULL` in the target (never NULL in the source) |
| `jobs`, `subjects`, `roles`, `awards` | same | verbatim free text, newlines kept; NULL kept |
| `image` | `image` | verbatim bare file name in `colleagues_images/`; NULL kept (13 rows); the file must exist (V7) |
| `category` | `category` | `smallint` 0-4 (0 management, 1 teachers, 2 lecturers, 3 administration, 4 maintenance) |
| `created_at`, `updated_at` | none | `snapshot_at` |

### 6.6 `dishes` from `canteen_menus` (624 rows)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved |
| `name` | `menu` | verbatim (longest 58 characters) |
| `course` | `type` | `smallint`: 0 soup, 1 main dish, 2 dessert |
| `created_at`, `updated_at` | same | `DATETIME` written as UTC |

Dishes with equal names are not merged. The week editor shows such duplicates with their usage counts (decision 39).

### 6.7 `canteen_days` from `canteen_data` (1,187 rows)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved, except the dropped duplicates of D6 |
| `date` | `date` | the Europe/Budapest midnight of the legacy day, as `timestamptz` (§5.2) (weekdays only, 2019-04-01 onwards) |
| `created_at`, `updated_at` | same | Europe/Budapest wall time converted to UTC (§5.2) |

D6 keeps one row per date. Expected target rows: 1,179. The 16 days that break the 2-3 dish rule are imported unchanged (D7).

### 6.8 `canteen_day_dishes` from `canteen_pivot_menus_data` (2,921 rows)

| Target column | Source column | Rule |
|---|---|---|
| `canteen_day_id` | `data_id` | verbatim |
| `dish_id` | `menu_id` | verbatim |
| `position` | none | 0, 1, 2 within the day, ordered by the dish's course, then dish id (the legacy data has no slot order; ADR 0018) |

Links of the canteen days dropped by D6 are not imported. The legacy primary key order `(menu_id, data_id)` is not carried over; the target key is `(canteen_day_id, dish_id)`.

### 6.9 `events` from `events_data` (293 rows)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved |
| `title` | `title` | verbatim |
| `description` | `description` | verbatim; NULL kept (199 rows) |
| `color` | `color` | verbatim; must match the colour pattern (293 of 293 do) |
| `starts_at` | `date_from` | Europe/Budapest wall time to UTC (§5.2) |
| `ends_at` | `date_to` | Europe/Budapest wall time to UTC; must not be earlier than `starts_at` (0 legacy violations) |
| `created_at`, `updated_at` | same | `DATETIME` written as UTC |

### 6.10 `pages` from `pages` (39 rows)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved |
| `title` | `title` | verbatim |
| `slug` | `slug` | verbatim; all 39 are distinct and match `^[a-z0-9-]+$` |
| `internal_note` | `name` | verbatim when it differs from `title` (at least 14 rows, several used as notes such as "Eredmények <-- feltöltendő"); NULL when it equals `title`, which loses nothing |
| `content` | `content` | normalised by §7 (`NOT NULL`; the largest legacy page is 63,082 bytes) |
| `seo_description` | `extras.meta_description` | NULL (the key is NULL on all 39 rows) |
| `published` | none | `true` for every row, because every legacy page is publicly served today; D10 lets editors list pages to import as hidden |
| `created_at`, `updated_at` | same | `DATETIME` written as UTC |

Dropped after an assertion: `template` must be `page` on every row (39 of 39), `extras` must be the all-NULL triple `{"meta_title": null, "meta_keywords": null, "meta_description": null}` on every row (39 of 39), and `deleted_at` must be NULL (39 of 39). Any other value stops the run. The public API keeps returning `template = 'page'` and an empty `extras` object (ADR 0018), so the Angular contract does not change.

### 6.11 `menu_items` from `menu_items` (23 rows)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved |
| `name` | `name` | verbatim |
| `kind` | `type` | verbatim; must not be NULL (0 NULL today): 20 `page_link`, 3 `internal_link`, 0 `external_link` |
| `link` | `link` | verbatim for `internal_link` and `external_link`; must be NULL for `page_link` |
| `page_id` | `page_id` | verbatim for `page_link`, and the page must exist; set to NULL for every other kind. This fixes the 3 `internal_link` rows (ids 9, 10, 14) that carry the non-existent `page_id = 1` left by the old edit form. |
| `parent_id` | `parent_id` | verbatim (2 roots, 0 dangling) |
| `position` | `lft` (and the resolution file) | contiguous integers from 0 per parent, in today's served order (below) |
| `created_at`, `updated_at` | same | `TIMESTAMP` UTC (§5.2) |

Dropped: `rgt`, `depth` and `lft` (used only to compute `position`), and `deleted_at` after asserting it is NULL on every row.

**Served order.** Today both public APIs order siblings by `lft ASC`. The 5 items created after the last reorder (ids 25, 26, 29, 30, 31) have a NULL `lft`, so they are served first, in MyISAM's physical row order, which is not id order (research: gap-07 §3-4). The import reproduces that order (decision 23b, ADR 0018 §3.6):

1. Within each parent, the rows with a NULL `lft` come first, in the source's physical row order: the order in which a plain `SELECT` without `ORDER BY` returns them, which is the order of the dump's `INSERT` and what `ORDER BY lft` serves today. The numbered rows follow in `lft` order.
2. Physical order is an accident of storage, so it is also pinned: the resolution file records the order of the unpositioned items as captured from the live public API (D9). If step 1 gives a different order, the run stops.
3. V9 compares the whole resulting tree with the captured tree below.

| Parent | Children in served order (id, name) |
|---|---|
| 7 Rólunk | **25** Az iskola nyitvatartása, 8 Iskolatörténet, 14 Archívum, 13 Tablók, 12 Alapítvány, 11 Iskolapszichológus, 10 Fejlesztők, 9 Munkatársak |
| 15 További információk | **26** Versenyeredmények, **30** Szegő-verseny 1994-2023, **29** Egyetemi tanárok lettek, **31** Adatvédelem, 17 Tagozatok, 23 Búvárkodj a biológiában verseny, 22 Tarján Imre Emlékverseny, 21 Szegő Gábor Matematikaverseny, 20 Közösségi szolgálat, 19 OKTV eredmények, 24 Statisztikák, 18 Dokumentumok, 16 Beiskolázás |

Roots in served order: 7, then 15. The menu resource is capped at two levels (decision 23b); the importer asserts that no row is deeper. An unpositioned item that the resolution file does not list stops the run.

### 6.12 `users` from `users` (40 rows)

| Target column | Source column | Rule |
|---|---|---|
| `id` | `id` | preserved |
| `pid` | none | a new random UUID (Loco shape, ADR 0006) |
| `email` | `email` | verbatim; must be unique case-insensitively |
| `name` | `name` | verbatim |
| `password_hash` | `password` | the Laravel bcrypt string, carried unchanged. All 40 are 60-character `$2y$10$` hashes. backend3 verifies `$2y$` at login and rehashes to Argon2id on success (ADR 0006). Any other format stops the run. |
| `active` | none | `true` (decision 10e; stale accounts are deactivated later, based on `last_login_at`) |
| `last_login_at` | none | NULL |
| `token_version` | none | 0 |
| `must_change_password` | none | `false` |
| `temp_password_expires_at` and the two-factor columns | none | NULL (two-factor off) |
| `created_at`, `updated_at` | same | `TIMESTAMP` UTC; NULL becomes `snapshot_at` |

Dropped: `remember_token` (decision 10a). The report shows users as counts only, never e-mail addresses, names or hashes.

### 6.13 `roles` (4 rows; verified, not copied)

`roles` is seeded by a backend3 migration with the 4 roles `writer`, `secretary`, `supervisor` and `admin` (ids 1-4, as in the legacy table). The importer asserts that the source has exactly these ids and names with `guard_name = 'web'`, and maps source role ids to target roles by name.

### 6.14 `user_roles` from `model_has_roles` (44 rows)

| Target column | Source column | Rule |
|---|---|---|
| `user_id` | `model_id` | `bigint`; `model_type` must be `App\User` on every row |
| `role_id` | `role_id` | mapped by role name (§6.13) |

The one row that points at a deleted user is dropped and reported (decision 10a). Users with two roles keep both (3 users); the user with no role is imported without one and reported. Expected target rows: 43.

### 6.15 `user_permissions` from `model_has_permissions` (4 rows)

| Target column | Source column | Rule |
|---|---|---|
| `user_id` | `model_id` | `bigint`; `model_type` must be `App\User` |
| `permission` | `permission_id` | the permission's name from `permissions`; must be one of the 8 legacy names that ADR 0007 keeps |

The 4 legacy grants are 3 × `edit events` and 1 × `edit canteens`. Permissions themselves live in code (ADR 0007). The importer asserts that the source `permissions` table has exactly the 9 legacy names, and that the 20 `role_has_permissions` rows are the legacy map below. Of those 20 links, the 18 that use the 8 kept permissions must equal the role mapping in code. The 2 links to `edit newsletter` are the only ones dropped, because the newsletter is retired (decision 30) and no user holds that permission as a direct grant.

| Role | Legacy permissions |
|---|---|
| writer | edit posts, edit labels, edit authors |
| secretary | edit events, edit canteens, edit newsletter, edit colleagues |
| supervisor | edit posts, publish posts, edit labels, edit authors |
| admin | all 9 |

Permissions added by ADR 0007 (edit pages, edit menu, manage media and the rest) are granted to roles in code, not by the importer.

## 7. Rich-text normalisation

`posts.content` and `pages.content` are rewritten once, at import, into the exact form the new editor stores (decision 19, ADR 0017). The legacy bodies come from CKEditor 4.12.1, whose `entities` plugin encoded accented letters and typographic punctuation as named entities, plus markup saved in source mode that the new editor's schema does not know (research: SYNTHESIS §4.7, gap-05 §2, gap-06 §1-4). The first save in any DOM-based editor would rewrite these bytes anyway, and would silently drop the embeds. One deterministic, diffed conversion is the only form that can be verified.

### 7.1 Steps

The normaliser is one pure function in `src/domain/` (ADR 0008): legacy HTML in, normalised HTML plus a list of rule hits and extracted images out. It parses with html5ever and works on the DOM, not on strings.

| Step | Rule | Expected hits |
|---|---|---|
| N0 Parse | Parse as an HTML fragment in a `body` context. This decodes every character reference, lower-cases tag names (posts 520, 524 and 532 have `<P STYLE=…>`), normalises `\r\n` to `\n` and void elements (`<img … />` to `<img …>`). | every non-NULL body |
| N1 Legacy oEmbed wrappers | `div[data-oembed-url]` (CKEditor's `embed` plugin through the Iframely demo endpoint, at most 320 px wide today) becomes the editor's YouTube node for a YouTube iframe and the editor's padlet node for a padlet iframe. Any other provider becomes a paragraph with a link to the embedded URL, because iframes are allowed only from YouTube and padlet (decision 15). The Iframely comment, the 320 px wrapper and the 16:9 box are removed. The wrappers' YouTube iframes have no title, so each YouTube node gets the title „Videó” ([ADR 0043](adr/0043-youtube-embed-input-playlists-ids-minimum-height-and-referer.md), D135). A video id that is not 11 characters fails the embed rule ([editor.md](editor.md) §3.6), and the import reports the embed (D132). | 8 wrappers in 7 published posts (107, 118, 140 ×2, 156, 167, 176, 207) and 1 draft; post 118 is a Facebook video and becomes a link |
| N2 Bare YouTube iframes | `<p><iframe src="…youtube.com/embed/…">` becomes the editor's YouTube node, in the content-v1 embed markup ([editor.md](editor.md) §3.7). The video id, `start`, `list` and `title` are kept, and the host becomes `www.youtube-nocookie.com`. Only a missing or blank title, or exactly "YouTube video player" after trimming, is replaced, by „Videó” ([ADR 0043](adr/0043-youtube-embed-input-playlists-ids-minimum-height-and-referer.md), D135). Every legacy attribute (the fixed 560-853 px `width` and `height`, `frameborder`, `allow`, `referrerpolicy`, `style`) is replaced by the canonical markup, whose `allow`, `allowfullscreen` and `referrerpolicy` each have one fixed value and whose style carries the locked `min-height: 200px` (D133), because content-v1 keeps no legacy node variants (decision 15b). As in N1, a video id that is not 11 characters is reported (D132); all 19 YouTube ids of the harvest **[500]** have 11. | 13 posts **[500]** (214, 263, 398, 402, 418, 423, 427, 444, 477, 484, 552, 557, 605) |
| N3 `<center>` | Unwrapped. The contained blocks get `text-align: center`. When the only child is an embed, the wrapper is just removed. | 9 posts **[500]** (398, 402, 418, 423, 427, 444, 477, 484, 532) |
| N4 Inline `data:` images | Each `img[src^="data:image/"]` is decoded. An image of at most 1×1 px is removed, together with a paragraph left empty. Every other image is written to `/storage/uploads/posts/<id>/inline-<n>.<ext>` and its `src` is replaced by that root-relative URL; `alt` and `style` are kept. `<n>` counts the written images of that body from 1 in document order. `<ext>` follows the MIME type (`image/jpeg` → `jpg`, `png`, `gif`, `webp`); any other type stops the run. Files are written byte-for-byte as decoded; the upload optimiser of ADR 0015 does not run on them. Page bodies use `/storage/uploads/pages/<id>/` (open item 3). | 16 images in 6 posts (91 ×7, 92 ×5, 335, 408, 473, 538): 14 files written, the two 1×1 px images of 473 and 538 removed, bodies about 1.63 MB smaller; 1 image in page 50 |
| N5 Legacy presentational markup | Mapped onto content-v1 (ADR 0013, [editor.md](editor.md) §9): `p[align]` → `text-align` style; `table[align]` dropped, because both sites centre every content-v1 table; `img.left` / `img.right` handled by N5b; `span.marker` → `<span style="background-color: #ffff00">`; `big` unwrapped; `ins` → `u`; `b` → `strong`, `i` → `em`, `strike` and `del` → `s`; `h5` and `h6` → `h4`; an `h2` with an italic style → `<h2><em>`; `span[dir]` unwrapped; `p[lang]` stripped. | **[500]**: 23 `p[align]`, 19 `table[align]`, 3 `img.left` (to N5b), 1 `span.marker`, 1 `big`, 1 `ins`, 23 `span[dir]`, 5 `p[lang]`, 2 italic `h2` |
| N5b content-v1 layout (decision 15b) | Old and new content get one content-v1 shape ([editor.md](editor.md) §3.7, §9), and no legacy editor node variants are kept. (1) Every image becomes the content-v1 image paragraph. A floated image (`img.left`, `img.right` or a `float` style) loses its float and is centred. Its `width` stays, also above 700 px. A legacy `height: Npx` becomes `height: auto`, with `width` and `height` attributes from the stored ratio. An image inside a heading, a link, inline formatting, a cell, a list item or a quote moves into its own paragraph before the enclosing top-level block, and a link around it is dropped and reported. (2) Every YouTube and padlet node from N1 and N2 gets the content-v1 embed markup; YouTube becomes the responsive player of at most 640 px. An embed inside a cell, list item, quote, heading or link moves into its own top-level paragraph before the enclosing block, as [editor.md](editor.md) §5.2 step 11a says. (3) Every table gets the content-v1 table shape: no table width, the locked `border-collapse`, `max-width` and `tbody` declarations, `thead` rows moved into `tbody`, ragged rows padded with empty cells, `box-sizing: content-box` on sized cells, and a `caption` becomes a paragraph before its table. No table stretches to the 1000 px content width any more, and cell layout (borders, fills, widths, alignment) is kept (decision 15c). (4) A lone paragraph in a cell, list item or quote is stored without its `<p>`, except an empty first paragraph of a list item in front of the item's lists, which is kept (`<li><p></p><ul>…</ul></li>`), and a legacy `<li><ul>` or `<li><ol>`, also one with only whitespace before the nested list, gets that empty paragraph, the only fixed point that keeps the nesting ([editor.md](editor.md) §3.7, "Tightening", which quotes the question and the consequence; the maintainer's answer of 2026-11-15 to item D148, "Keep the empty paragraph (as built); amend §3.7 and N5b", research: DECISIONS R31). Empty links are dropped, loose inline content is wrapped in a paragraph, and whitespace follows the editor's rule. Marks get the structure that the editor gives them on load (§7.4, D147): a mark inside the same mark is unwrapped, and a span inside a span becomes one span, the inner span's value winning where both set the same property; marks nest in the editor's order, outermost first `a`, `span`, `strong`, `em`, `s`, `u`, then `sub` or `sup`, and an outer mark is split where that order needs it; equal marks side by side are joined; empty marks are dropped. These mark rules run after the image and embed moves of (1) and (2), which can leave a mark empty, and each change counts as a rule hit, as V5(b) needs for a changed row. N5b reproduces every load-time normalisation of the editor, so that N9 holds, except the editor's split of a legacy `<li><ul>` into an item and a sibling list: it writes that shape in the kept form instead. | **[500]**: 7 floated images (4 `float:left` styles and 3 `img.left` classes; research: gap-06 §4.2-§4.3), about 11 images inside headings, inline formatting or links, 19 YouTube nodes (6 from N1, 13 from N2), 114 tables (102 in posts, 12 in pages), 82 table widths, 8 `thead`, 1 ragged table, empty links in 3 bodies. The full dump may hold more tables in pages (research: QUESTIONS Q15); the first dry run reports the real counts (open item 4). |
| N6 Dead-host links | In `src` and `href` attributes, the prefixes `https://backend.verseghy-gimnazium.net/storage/` and `http://backend.verseghy-gimnazium.net/storage/` become `/storage/`. The host has no DNS record, so these images and links are broken today. Other occurrences of the host are reported, not changed. | **[500]** 15 attributes (14 `img src`, 1 `href`) in posts 76, 85, 137, 203, 208, plus 1 page `href`; at row level the host appears in 11 published posts, 4 drafts and 1 page |
| N7 Sanitise | The result goes through the same sanitiser that every API write uses (ADR 0013, [editor.md](editor.md) §7). Comments and everything outside content-v1 are removed: any `script` element left after N1, iframe `tabindex`, `scrolling` and `style`, and the table attributes that content-v1 drops (`border`, `cellspacing`, `cellpadding`). Style values are re-spelled canonically ([editor.md](editor.md) §3.1): named colours become hex, border sides without a style become `none`, `cm` and `pt` become px, and invalid values such as `100%px` are dropped. These are planned N7 changes, counted per rule. Any removal that no earlier step planned is listed per row and stops the run, unless the resolution file accepts that row (key `N7-accept`). | **[500]** iframe `tabindex` 7, `scrolling` 6, `style` 7; `cellspacing` 99, `cellpadding` 14, `table[border]` 14; about 11,500 `black` colours; 123 `100%px` widths |
| N8 Serialise | html5ever serialisation, which is the same algorithm a browser's `innerHTML` uses: in text only `&`, `<`, `>` and U+00A0 are escaped (`&amp;`, `&lt;`, `&gt;`, `&nbsp;`), in attributes `&`, `"`, `<`, `>` and U+00A0 (`&amp;`, `&quot;`, `&lt;`, `&gt;`, `&nbsp;`; html5ever@0.40.1:`src/serialize/mod.rs`, `write_escaped`). Every other named entity disappears; `&quot;` and `&#39;` in text become `"` and `'`. Whitespace follows the editor's form ([editor.md](editor.md) §3.8). | entity-encoded bodies: 485 of 541 published posts, 27 of 35 drafts, 38 of 39 pages |
| N9 Fixed point | Assertions on every output body: `normalise(out) == out`, `sanitise(out) == out`, and the editor round trip (load into the admin editor, serialise) returns `out` byte for byte (§10, V13). | 0 failures |
| Missing alt (no change) | Images without `alt` are imported without one. The importer never invents alt text. Each is listed in the report for editors. The editor accepts a missing alt on load without changing bytes, and requires it before the next save (decision 15, ADR 0013). Images with an empty `alt` are listed too: ADR 0013 treats an empty or whitespace-only `alt` as missing, so they also need alt text before the next save. | **[500]**: of 110 images, 9 have no `alt` and 14 an empty one (research: gap-06 §4.2) |

Not changed on purpose:

- table cell styles. Pasted Word and Excel tables must keep their styling (decision 15); there is no cell-style compaction. Cell layout (borders, fills, widths, alignment) is always kept (decision 15c). N7 only re-spells the declarations canonically, which does not change how they render ([editor.md](editor.md) §3.1);
- `text-align`, image widths (N5b removes the float of floated images and turns a fixed height into `height: auto` with the same aspect ratio, and images wider than 700 px keep their width, decision 15b), and root-relative `/storage/uploads/…` links;
- plain-text columns (`title`, `description` and the others), which contain no entities.

### 7.2 What the normaliser records

| Record | Where |
|---|---|
| The original body bytes | the row's `import_original` revision (§5.4) |
| Rule hits per row (N1-N7, N5b included), and the images without `alt` | the run report |
| A readable diff per changed row | the dry-run report: both sides pretty-printed one element per line, then a unified diff |
| Extracted images: path, size, SHA-256, pixel size | the run report |

### 7.3 Re-running over backend3's own data

The same function is exposed as a second Loco task, `content_normalize` (`mode:dry-run` by default). It runs over backend3's `posts` and `pages` and changes only rows that are not already at the fixed point. Right after an import it must report zero changes (V13). It is the tool for any later re-normalisation, for example after an editor schema change, and it records a revision for every row it changes. It also selects rows whose integrity tag does not verify or whose `content_policy` is not the current policy; a row whose bytes are already at the fixed point only gets a new tag, without a revision (ADR 0013, [editor.md](editor.md) §8.3).

### 7.4 How the normaliser reads the cases §7.1 leaves open

The normaliser is `normalise` in `src/domain/normalise/`, with the N9 checks as `check_fixed_point`. It holds N0, N3, N5 (its block rules and its inline rules), N6, N8 and N9 so far; the other steps join it in their own slices, in the order of §7.1. Where §7.1 leaves a case open, it reads it as follows; the module documentation gives the reasons, and `tests/fixtures/normalise/` holds a before-and-after fixture for each case.

- **N0.** The parse is the sanitiser's own: an HTML fragment in a `div` context, which builds the same tree as the `body` context named above (both start the tokenizer in the data state and the tree builder in "in body"), so N7 sees the tree N0 built.
- **N3.** A browser centres everything inside a `center` that sets no alignment of its own (WHATWG HTML, "Rendering", §15.3.3), so N3 gives that content `text-align: center` on the blocks that carry it in content-v1 (`p`, `h1`-`h4` and the `h5` and `h6` that N5 maps to `h4`, `td`, `th`), written in front of their `style`, and then removes the `center`:
  - An embed alone in the `center` (an `iframe`, an element with `data-youtube-video`, `data-padlet-embed` or `data-oembed-url`, or a `p` or `div` that holds nothing visible but one) loses only the wrapper. An embed next to other content is kept as it is.
  - A `center` that is the only visible content of a paragraph, heading or cell aligns that block instead (`<h2><center>…</center></h2>`; `<td><center>…</center></td>` becomes `<td style="text-align: center">`, the form the editor stores, [editor.md](editor.md) §3.7). A block that is centred already needs nothing; an `align` attribute gives way to the new declaration, since everything the block shows was centred; an own `text-align` in its `style` does not, and the content is handled as below.
  - A paragraph, heading or cell gets the alignment unless it sets its own. It sets its own when its `style` holds a `text-align` that content-v1 keeps, as the sanitiser reads it (`left`, `center`, `right` or `justify`; the last one wins), or, without one, an `align` attribute that a browser maps to `text-align` on that element (`left`, `right`, `center`, `justify` on `p` and the headings; also `middle` on `div` and the table parts, and `absmiddle` on the table parts; §15.3.3, §15.3.8). A block's own alignment wins over the inherited one, as in CSS.
  - Text and inline elements are wrapped, run by run, in `<p style="text-align: center">` where a paragraph may stand (the top level, a list item, a quote, a cell, a `div` and the other containers).
  - Lists, quotes, `div`s and the other containers, and tables with their sections and rows, are gone into, and their content follows the same rules: `<center><ul><li>pont</li></ul></center>` becomes `<ul><li><p style="text-align: center">pont</p></li></ul>`, and every cell of a centred table without its own alignment gets `text-align: center`. A container that sets another alignment keeps its content as it is; a `caption`, which a browser centres by itself, is left to N5b.
  - A `style` whose `text-align` content-v1 does not keep (`inherit`, `start`, an invalid value, one the sanitiser's reader cannot read) is unclear: the block is treated as setting none, because CSS's own keywords and invalid values leave the centring in force and N7 drops the declaration, and it counts as `center_alignment_unclear`, because a value such as `start` showed another alignment.
  - Other content keeps its place without the centring: inline content where no paragraph may stand and no block around it can carry the alignment (in a heading that also holds other text, in a link, directly in a list), an inline element around a block (a paragraph around it would not parse back), and blocks without an aligned form in content-v1 (`pre`, `legend`, `summary`). Each such child or run with visible text counts as `center_alignment_dropped`, so that the body appears in the report for review.
- **N5, the block rules.** They run after N3, which reads a paragraph's `align` as its own alignment, counts `h5` and `h6` among the aligned headings, and aligns an italic `h2` whose only content is a `center` before N5 puts its text in an `em` (the other way round, the `em` would stand around the `center`, and N3 would drop its centring). Only HTML elements are read, as in N3. The maintainer confirmed the readings below as built (D142, below).
  - `p[align]`: a value that a browser maps on a paragraph (`left`, `right`, `center`, `justify`, matched whole and ASCII case-insensitively, as N3 reads it) becomes that `text-align`, in lower case, written in front of the paragraph's `style` as N3 writes its centring (`paragraph_align_mapped`). A paragraph whose `style` already sets a `text-align` that content-v1 keeps only loses the attribute, because the declaration wins over it in a browser, and so does one whose value maps to nothing (`paragraph_align_dropped`). A `style` whose `text-align` content-v1 does not keep is unclear, as in N3: the attribute's alignment is written, and the paragraph also counts as `paragraph_align_unclear` for review.
  - `table[align]` is removed whatever its value (`table_align_dropped`); a `left` or `right` table, which floated, is centred like the others.
  - `h5` and `h6` become `h4` with their attributes (`heading_to_h4`).
  - An `h2` is italic when its `style` makes it italic in a browser's cascade: of the `font-style` declarations a browser accepts, read with the sanitiser's declaration reader, the last one marked `!important` wins, and the last one of all when none is; a value a browser rejects is skipped. The reader strips `!important` without saying so, so N5 looks for the marker itself. Its `font-style` declarations are cut out whole, so the rest of the `style` stays byte for byte for N7, and its content, from the first child that is not white space or a comment to the last, is wrapped in one `em` (`italic_heading_to_em`). An `em` or `i` inside it with no attribute is unwrapped, because the editor merges italic inside italic into one mark. One with an attribute (a `font-style: normal` that kept its text upright, a `class`) stays for N5's inline rules and N7, which report what they remove. One that holds a block stays too: it is tag soup, not a mark, and unwrapping it can let the next parse close a heading that tag soup nested in another one, so the body would not settle; for the same reason, an italic heading that tag soup nests in another one gets its own `em` only when it holds a block. In tag soup that also fosters a link out of a table into another link around the heading, the new `em` can still make the parser's repairs take one pass more, so such a body can be refused where it settled without N5; like every refused body, it stops `apply` (D140, below), and the dry run names it before that. The maintainer accepted this rare refusal over an `em` around each inline run (D143, below). `oblique`, a CSS-wide keyword, the `font` shorthand and a `font-style` on another element are left to N7: the corpus has no `font-style` but the two on `h2` (research: gap-06 §4.3). Whether the editor writes the `em` back in the same place when the heading also holds `strong` or a link is open (§13, item 10).
  - `p[lang]` is removed (`paragraph_lang_stripped`); `xml:lang` and a `lang` on another element are left to N7.
- **N5, the inline rules** (`marks.rs`). They run right after the block rules, in the same pass, so an `i` that the italic-heading rule unwraps (above) is gone before them and is not counted as `i_to_em`. Each rule hit counts one element. Only elements in the HTML namespace are mapped; an `ins` or `del` inside `svg` is an SVG element, which N7 removes with the `svg`. The readings below that go beyond §7.1 (the attributes left on the renamed marks, the marker's place in front of the span's own `style`, a `span[dir]` with a `style` and a marker span with a `dir`) are confirmed as built, and the structure of the marks is N5b's (§7.1, N5b (4)): the maintainer's answers D144 to D147 of 2026-11-15, quoted below.
  - `b`, `i`, `ins`, `strike` and `del` are renamed to `strong`, `em`, `u`, `s` and `s`, and keep their place, their children and their attributes. content-v1 keeps no attribute on these marks ([editor.md](editor.md) §3.2), so N7 removes what the legacy element carried and lists it as a removal that no step planned, which stops `apply` until review accepts it. That includes a `style` that changed what the tag showed: `<b style="font-weight: normal">` showed no bold, and N5 does not read the style to guess what the reviewer decides. Google Docs wraps a paste in such a `b` (with an `id` of `docs-internal-guid-…`), which the editor's paste filter drops because the explicit weight overrides the tag ([editor.md](editor.md) §5.2 step 9); a body that holds one reaches review this way. The maintainer confirmed this reading (D146, below).
  - `big` is unwrapped together with its attributes, as N7 would unwrap it; its larger size is not kept.
  - `span.marker`: the class `marker` is removed from the span's class list, which is split at ASCII white space and matched exactly, as a class selector matches in a document without quirks (`Marker` and `markers` are other classes); the `class` attribute goes when nothing is left in it, and other classes stay for N7. `background-color: #ffff00`, the yellow of CKEditor's `.marker` rule, is written in front of the span's `style`. In front, because the class rule had the lowest precedence: a `background-color` or `background` of the span's own showed instead, and as the later declaration it also wins in N7's cascade (editor.md §3.1 rule 5) when content-v1 keeps its value. A value that neither reads (`nope`) leaves the yellow in force in both. A value that is valid CSS but that content-v1 does not keep (`none`, `inherit`, `initial`, a `url(…)`) cleared the yellow in a browser, but N7 drops it and the yellow shows after the import; N7 lists the dropped declaration as a removal that no step planned, so the row reaches review. The public sites have no rule for the class (research: backend1-media-editor-i18n §3.4), so readers now see the highlight that the old editor showed its authors: a one-time visible change, in 1 body **[500]**. The maintainer confirmed the yellow in front (D145, below).
  - `span[dir]` is unwrapped, as §7.1 says, whatever the value of its `dir` (content-v1 keeps no `dir`; the bodies hold only `ltr` and `auto` **[500]**), and its other attributes go with it. A span without a `style`, or with one that holds only white space and `;`, counts as `span_dir_unwrapped`: nothing a reader saw is lost, and the editor's serialiser unwraps a span without attributes too ([editor.md](editor.md) §6, step 2.2). A span with any other `style` counts as `span_dir_style_dropped`, so that the body appears in the report for review, because the style may have shown a colour, highlight or size that content-v1 keeps on a span. The **[500]** corpus has no span with a `style`. The maintainer confirmed this reading and the next one (D144, below).
  - A marker span with a `dir` is both. The marker rule runs first and its output is a span, so the span stays with its highlight and loses only its `dir` (`span_dir_removed`): `<span class="marker" dir="ltr">` keeps its highlight.
  - Other inline elements outside content-v1 (the Styles combo's `small`, `tt`, `code`, `kbd`, `samp`, `var`, `cite` and `q`, or `mark` and `font`) are not in N5's list: N7 unwraps them and lists each as an unplanned removal. The **[500]** element inventory lists none of them, and no `b`, `i`, `strike` or `del` either (research: gap-06 §4.1).
  - N5 maps each element on its own, never the structure of the marks around it: it does not merge a mark into the same mark around it (`<strong><b>` becomes `<strong><strong>`), order nested marks, join adjacent equal marks or drop empty ones. The editor does each of these on load, to legacy `strong` and `em` as much as to the renamed marks, so N5b does them, as the maintainer decided (D147, below): §7.1's N5b row (4) gives the four rules, which run after N5b's image and embed moves, since those can leave a mark empty. N5b's slice builds them; until then, the editor round trip of N9 (V13) finds a body where they would change the bytes.
  - N5 runs after N3, so N3 meets a `big` or `span[dir]` around a block as an inline element around a block, which keeps its place without the centring, and counts it as `center_alignment_dropped`; N5 then unwraps it in the same pass. `<center><big><h2>x</h2></big></center>` becomes `<h2>x</h2>`, without the centring a browser showed.
  - An unwrap can leave markup that the next parse reads differently. Line feeds at the start of an unwrapped `big` or `span[dir]` that was the first child of a `pre` or `listing` become that element's first text, where the parser drops one at each parse: one is lost (N7 unwraps the `pre` anyway), and two or more keep the body from settling (see "Fixed point"). A heading in an unwrapped `big` or `span[dir]` inside another heading ends the outer heading when parsed again: `<h2><span dir="ltr"><h3>x</h3></span></h2>` becomes `<h2></h2><h3>x</h3>`. The text is kept in both.
- **N6.** The scheme and the host are compared ASCII case-insensitively, `/storage/` exactly. Every other occurrence of the host in an attribute value or a text node (not in a comment, which N7 removes) is reported with its place and an excerpt; these are the occurrences that editors review (§13 item 4).
- **N8.** html5ever's serialiser writes the attributes of a content-v1 element in the order of [editor.md](editor.md) §3.2, and any other attribute after them. scraper's tree keeps attributes sorted by name, so their source order is not known; this order makes a content-v1 body come out unchanged.
- **Fixed point.** One pass is N0, the rules and N8. The pass repeats until it returns its own input, at most three times, as the sanitiser's does (ADR 0013 §5), so `normalise(normalise(x)) == normalise(x)` holds for every accepted body. A body still changing after the third pass (a `plaintext` element, a `pre` whose content starts with three or more line feeds, a `pre` or `listing` where N5 unwraps a `big` or `span[dir]` that starts it with two or more, tag soup that leaves a link open around deeply nested lists and headings with another link inside them, or tag soup that fosters a link out of a table into another link) is refused as not normalisable. The import handles it like a failed N9 check (§4.2), as the maintainer confirmed (D140, below): the body is not written, and `apply` stops, rolls back and names the row. `content_normalize` reports such a row and leaves its stored bytes as they are.
- **What it records** (§7.2). Rule hits count changes, added up over the passes; N0 and N8 are not rule hits. The dead-host occurrences and the images without alt text come from the final output: an image is listed when its `alt` is missing, or empty or white space only as the admin write check judges it (`image_alt_missing`, ADR 0013 §2), with its position among the body's images and its `src`, a `data:` URL cut after its header.

**The maintainer's answers.** The build read §7.1 beyond its words in seven places, and it had to choose what the import does with a body the normaliser refuses. The register asked the maintainer about both (research: third decision register D139 and D140), and the maintainer answered both on 2026-11-01 (research: DECISIONS R29), with the recommended option. The register's questions, the answers, and the consequences the register gave for those options:

- **D139**, "data-migration.md §7.1's N3 row reads: 'Unwrapped. The contained blocks get `text-align: center`. When the only child is an embed, the wrapper is just removed.' normaliser-core's new §7.4 goes beyond §7.1's words for N3, N6 and N8 in seven places, numbered 1 to 7 below. Each can be dropped on its own, except 7, and dropping 3 drops 4. Do you confirm all seven as built, confirm only some, or keep only what §7.1 says word for word?": "Confirm all seven as built". Its consequence: "No code change. §7.4 cites this answer and the PR leaves draft as built. Centred lists, quotes, divs, tables and cells keep their centring, in the forms the editor stores. Own alignment is read the way the sanitiser and browsers read it. Unclear and dropped centring put the body on the report for editor review. N6 matches case-insensitively, and N8 writes §3.2's order. normaliser-blocks and normaliser-marks land on it unchanged."
- **D140**, "`normalise` repeats its pass up to three times and refuses a body that is still changing (`NotNormalisable`). When the importer meets such a legacy body, should `apply` stop and roll back, as data-migration.md §7.4 reads now? Or should the row only be reported while the run goes on, as ADR 0013 §5 reads for `content_not_normalisable` ('report these cases instead of failing on them')?": "Stop `apply`, as §7.4 reads (as built)". Its consequence: "No normaliser change. `apply` stops at a refused body, rolls back and names the row. The nightly rehearsal stays red until the body is fixed, and the proposed 7-night exit criterion (cutover.md §5.4) waits. There are two fixes: a normaliser or sanitiser fix, made in place because nothing is stored yet (D87, D112), or an editor fixing the post in the current admin before the final dump, as D61 has editors do for dead-host links, alt texts and post 70's index image. If a post is edited into such a shape after the same-day rehearsal's dump (cutover.md §5.4), the window's import stops: it is re-run once, then the window rolls back for free (cutover.md §6 phase 4, §9.1). posts-import-bodies adds a §4.2 row for a refused body. The table names none today, and V13 runs only after the commit. ADR 0013 §5's sentence gets a correction that cites this answer: a refusal, by the normaliser or the sanitiser, leaves no body, so the importer stops on it and `content_normalize` reports it. A correction that follows your newer answer is an editorial fix (ADR 0001 §5-§6), not an amending ADR." [ADR 0013](adr/0013-rich-text-editor-and-html-sanitiser.md) §5 carries that correction.

The seven readings of D139, as the register numbered them, and where this section records each. Wrapping loose text in a centred paragraph (N3, the fourth item) predates them and stays too: only the option that kept §7.1's literal wording would have dropped it.

| Reading | As the register gave it | Recorded in |
|---|---|---|
| 1. Containers | Lists, quotes, `div`s and tables are gone into: `<center><ul><li>a</li></ul></center>` becomes `<ul><li><p style="text-align: center">a</p></li></ul>`, and a centred table's cells get the alignment. | N3, the fifth item |
| 2. The block around it | A `center` that is a heading's or cell's only visible content aligns that block: `<td><center>x</center></td>` becomes `<td style="text-align: center">x</td>`, the editor's form. | N3, the second item |
| 3. Own alignment | Counts only for a `text-align` content-v1 keeps, or an `align` value browsers map on that element. | N3, the third item |
| 4. Unclear | Any other `text-align` (`inherit`, `start`, invalid, unreadable) counts as none: the block is centred and counted as `center_alignment_unclear`. | N3, the sixth item |
| 5. Dropped | Where content-v1 cannot hold centring (a heading with other text, a link, `pre`), it goes and is counted as `center_alignment_dropped`. | N3, the seventh item |
| 6. N6 | Matches scheme and host case-insensitively, `/storage/` exactly; the report skips comments. | N6 |
| 7. N8 | Writes content-v1 attributes in [editor.md](editor.md) §3.2's order; scraper's tree sorts them by name. | N8 |

**The maintainer's answers on N5's block rules.** The block rules of N5 read §7.1 and [editor.md](editor.md) §9 beyond their words in six places, and the `em` of the italic-heading rule can make the normaliser refuse one tag-soup shape that settles without N5. The register asked the maintainer about both (research: third decision register D142 and D143), and the maintainer answered both on 2026-11-15 (research: DECISIONS R31), with the recommended option. The register's questions, the answers, and the consequences the register gave for those options:

- **D142**, "editor.md §9 (and data-migration.md §7.1) give N5's block rules one sentence each: '`p[align]` becomes `text-align`. `table[align]` is dropped: both sites centre every content-v1 table. … `h5` and `h6` become `h4`. An `h2` with `font-style: italic` becomes `<h2><em>`. … `p[lang]` is stripped.' normaliser-blocks' new §7.4 bullet reads those words in six places, numbered 1 to 6 below. 3 is §9's own wording and is listed only because it centres tables that floated. For 2 there are two readings. Do you confirm all six as built, confirm them with the browser's reading of 2, or drop some?": "Confirm all six as built". Its consequence: "No code change. §7.4 cites this answer and the PR leaves draft as built. The alignments a browser maps on a paragraph become `text-align`. A value a browser ignores, or one the style overrides, is dropped and counted as `paragraph_align_dropped`. A paragraph with an unclear style keeps its attribute's alignment and is listed for review. N5 reads the unclear value the way N3 does under D139 reading 4, through the same `style_alignment`. Left and right tables are centred like the others. A heading becomes `<h2><em>` when its `font-style` declarations make it italic in a browser's cascade, and `oblique` and the `font` shorthand are left to N7. An inner `em` or `i` with attributes goes on to the inline rules and N7, and D146 decides what happens to its attributes. normaliser-marks needs no change for it."
- **D143**, "In one tag-soup shape, the `em` that N5 wraps around an italic `h2`'s content makes the normaliser refuse a body that settles without N5. Under D140 a refused body stops `apply`. Do you accept that risk as built? Or should N5 wrap each inline run of an italic heading in its own `em`, so that no new `em` ever holds a block?": "Accept the rare refusal (as built)". Its consequence: "No code change. §7.4 and blocks.rs's module doc already record the shape and cite D140, and §7.4 adds a citation of this answer. The harvest's 2 italic headings come out as `<h2><em>…</em></h2>`. If a dry run ever names such a row, D140's two fixes apply. One is a normaliser change made in place, since nothing is stored before posts-import-bodies (wave 21). The other is an editor fixing the post in the current admin before the final dump. If a post is edited into the shape after the same-day rehearsal's dump, the window's import stops: it is re-run once, then the window rolls back for free (D140). `a_known_soup_is_refused_with_the_em` asserts the refusal. So if N7 running inside the pass makes the shape settle (normaliser-n7-plan, wave 9), that test fails, and that slice updates it and §7.4."

The six readings of D142, as the register numbered them, and where this section records each. Reading 2 had a second option, the browser's reading, which drops the attribute beside a CSS-wide keyword, `start` or `end` and writes it only beside an invalid or unreadable value; the answer keeps the reading as built, which is N3's reading 4 of D139.

| Reading | As the register gave it | Recorded in |
|---|---|---|
| 1. `p[align]` | Maps only `left`, `right`, `center` and `justify` (whole value, any case), in front of the `style`. It is dropped when the `style` sets a `text-align` content-v1 keeps, or the value maps to nothing (`middle`). | N5's block rules, the first item |
| 2. Unclear style | `inherit`, `start`, invalid, unreadable: the attribute's alignment is written and counted as `paragraph_align_unclear`; N7 drops the unclear value. | N5's block rules, the first item |
| 3. `table[align]` | Goes for every value, as §9 says: a `left` table, which a browser floats, is centred. | N5's block rules, the second item |
| 4. Italic `h2` | A browser's cascade decides (`!important` first, then the last accepted `font-style`); `oblique` and the `font` shorthand are left to N7. Every `font-style` is cut out and one `em` wraps the content; nested italic headings end with one `em`. | N5's block rules, the fourth item |
| 5. An inner `em` or `i` | Unwrapped only when it has no attributes and holds no block: `<i style="font-style: normal">` stays for the inline rules and N7 (D146). | N5's block rules, the fourth item |
| 6. Report names | `paragraph_align_mapped`, `_dropped`, `_unclear`, `table_align_dropped`, `heading_to_h4`, `italic_heading_to_em`, `paragraph_lang_stripped`. | N5's block rules, each item |

**The maintainer's answers on N5's inline rules.** The inline rules of N5 read §7.1 beyond its words in the places that the N5 inline-rules item above names. The register asked the maintainer four questions about them (research: third decision register D144 to D147), and the maintainer answered all four on 2026-11-15 (research: DECISIONS R31), each with the recommended option. The register's questions, the answers, and the consequences the register gave for those options:

- **D144**, "data-migration.md §7.1 says '`span[dir]` unwrapped' and editor.md §9 says '`span[dir]` is unwrapped'. As built, N5 unwraps every `span[dir]` that is not a marker. Any `style` that is not blank goes with it and is counted as `span_dir_style_dropped`, so the dry run lists the body. Should that stand? Or should N5 keep the span, without its `dir`, when its style holds a colour, highlight or size that content-v1 keeps? And do you confirm that a marker span with a `dir` keeps its highlight and loses only the `dir`?": "Confirm both as built". Its consequence: "No code change. §7.4 cites this answer, and the PR also waits for D145-D147. A legacy `span[dir]` loses any colour, highlight or size its style showed. Each such body is on the dry-run report as `span_dir_style_dropped`, and `apply` goes on. A style that content-v1 keeps nothing of (`font-family: Arial`, `color: nope`) is listed too, so some rows are false alarms. A marker span with a `dir` keeps its yellow. No [500] body holds either shape. Dry runs store nothing, so if the full dump's dry runs (posts-import-bodies, wave 21; pages-import, wave 22) show styled spans that matter, the rule can still change in place before the first legacy body is stored."
- **D145**, "§7.1 says `span.marker` becomes `<span style="background-color: #ffff00">`. It says nothing about a marker span that has a background of its own. N5 writes the yellow in front of the span's style, so the span's own background wins whenever content-v1 keeps its value. A valid CSS background that content-v1 drops (`none`, `inherit`, `initial`, `url(…)`, or a colour with other parts such as `no-repeat`) leaves the yellow, where the old editor and the public sites showed none or another colour. Do you confirm this as built, skip the yellow in that case, or always force the yellow?": "Keep the yellow in front (as built)". Its consequence: "No code change. §7.4 cites this answer instead of saying the reading awaits confirmation, and the PR leaves draft. Every background content-v1 keeps, `transparent` included, wins as it did before, and a marker with no style turns yellow as §7.1 says. A marker whose own background content-v1 drops ends up yellow. Once normaliser-n7-plan counts that drop as unplanned, its row stops `apply` until review accepts the yellow in an `N7-accept` row, or until an editor fixes the post in the current admin before the final dump, as D61 has editors fix dead-host links."
- **D146**, "N5 renames `b`, `i`, `ins`, `strike` and `del` to `strong`, `em`, `u` and `s`, and the renamed element keeps its attributes. N7 then removes them. No step planned those removals, so `apply` stops on the row until you accept it. That includes a `style` that cancelled the mark, such as `<b style="font-weight: normal">`, which shows bold once the row is accepted. data-migration.md §7.1 names only the new tags ('`b` → `strong`') and says nothing about attributes. Do you confirm this as built? Or should N5 handle these attributes itself, in one of three ways: unwrap a mark whose own style cancels it, drop the attributes as a counted planned removal, or both?": "Confirm as built: leave them to N7's review". Its consequence: "No code change. §7.4 cites this answer and the PR leaves draft. Any attribute on a renamed mark (a style, `id`, `class`, `title`, a `del`'s `datetime`) stops `apply` on its row until one of two things happens. You can accept the row with an `N7-accept` entry (import-resolution-list, wave 26), after the dry run's diff has shown you the change. Or an editor can fix the post in the current admin before the final dump, the way D61 has editors fix other posts. If you accept a cancelled bold, it shows bold. Renamed marks then behave like the `strong` and `em` already in the bodies."
- **D147**, "N5 renames or unwraps each legacy element where it stands. It does not merge a mark into the same mark around it, put nested marks in the editor's order, join equal marks that sit side by side, or drop empty marks. The admin editor does all four when it loads a body. Should the import leave these shapes for the editor round trip to find (as built), add the four rules to N5b in normaliser-tables (wave 22), or add them to N5 in this slice?": "Add the editor's mark rules to N5b (normaliser-tables, wave 22)". Its consequence: "normaliser-marks' code stays as built. This PR changes §7.4's bullet to cite the answer and name N5b, and adds four rules to §7.1's N5b row (4) and editor.md §9's N5b 'Structure' item; then it leaves draft. The rules: (1) a mark inside the same mark is unwrapped, and a span inside a span becomes one span, the inner value winning; (2) marks nest in the editor's order (link, span, strong, em, s, u, then sub or sup), splitting an outer mark where needed; (3) equal marks side by side are joined; (4) empty marks are dropped. Each change counts as a rule hit, which V5(b) needs for a changed row. normaliser-tables runs them after N5b's image and embed moves (built by normaliser-images, wave 10, and normaliser-embeds, wave 21), which can leave a mark empty, and its editor round trip over every output tests them. They also settle normaliser-blocks' §13 item 10. Marks that N7's unwraps leave side by side (`<strong>a</strong><font><strong>b</strong></font>`) are joined in the next pass, as under the third option. normaliser-tables (estimated at 520 lines) grows by one rule set, or a separate slice at wave 22 takes the rules." [editor.md](editor.md) §9 carries the four rules in its N5b "Structure" item.

## 8. Defect resolutions (editor sign-off)

Several defects in the legacy data are visible on the public site or block the new constraints of ADR 0009. They can be fixed only in the data (decision 20). The importer applies a resolution list that editors sign off before the cut-over; then the constraints go on.

### 8.1 The proposed list

Rows are those of the 2026-10-23 dump. Every rehearsal re-detects each defect from its rule, so new cases show up in the nightly report before the window.

| Id | Defect | Rows | Proposed resolution | Unblocks |
|---|---|---|---|---|
| D1 | Post dates one day early. The post date was once a `TIMESTAMP` holding local midnight; backend2's fix 11 cast it to `DATE` without the zone correction. | 181 rows: `created_at < '2022-01-21'`, date not NULL and not `1970-01-01` | +1 day on all 181. This moves their public dates and, for posts now dated on the last day of a month, their archive month. | correct dates and archive buckets |
| D2 | Published posts without an author. These posts do not render on the public site today. | 55, 61, 66 | Editors name the real author. Where nobody knows it, the post gets a generic author `Szerkesztőség`: an existing author of exactly that name if there is one, otherwise one the importer creates. The 4 drafts without an author stay without one. | `CHECK (NOT published OR author_id IS NOT NULL)` |
| D3 | Published post without an index image; its `og:image` points at a directory today. | 70 | Editors choose an index image for post 70 and set it in the current admin before the final dump. This is an ordinary editor edit; decision 32b forbids only engine, charset and clean-up changes. The resolution entry is signed after that edit and records the precondition `index_image` = the chosen `index/<32 hex>.<ext>`. The importer refuses a published post without an index image. | `CHECK (NOT published OR index_image IS NOT NULL)` (ADR 0009) |
| D4 | Posts without a body. | 28 published, 6 drafts | Keep NULL. These are gallery-only or description-only posts. Editors confirm that no body is missing. | none (`content` stays nullable) |
| D5 | Sentinel dates on drafts. backend2's fix 13 set NULL dates to `1970-01-01`. | 11 drafts (65, 77, 162-164, 168-173) | Set to NULL. The 5 drafts that already have a NULL date stay NULL. The admin then requires a date on the next save, defaulting to today (decision 21). | `CHECK (NOT published OR date IS NOT NULL)`, and no latent 1970 archive month |
| D6 | Duplicate canteen days; the public site shows only the first of a pair. | 8 dates, 16 rows (below) | Keep one row per date: the row with more dishes; on a tie, the later id. Links of the dropped row are not imported. Editors confirm each date on the sign-off sheet of §8.2, which shows both menus side by side. | one live day per Budapest date (ADR 0009, ADR 0018 §4.1) |
| D7 | Canteen days that break the 2-3 dish rule. | 16 days (1 with no dish, 15 with one) | Import unchanged. The rule applies when such a day is next saved. | none (the rule is application-level) |
| D8 | Duplicate label names (equal under MySQL's case- and accent-insensitive collation). | `filozófia` ×2, `környezetvédelem` ×3 | Merge each group into one label: the one with most posts; on a tie, the lowest id; its colour is kept. Posts are re-pointed. Detection uses the legacy collation (case and accents ignored); the new uniqueness ignores case only, because accents tell Hungarian words apart. | unique `lower(labels.name)` |
| D9 | Menu items without a position, served first today. | 25; 26, 30, 29, 31 | Keep today's served order (decided, Q23b). The resolution file records the order captured from the live public API (§6.11). | contiguous `position` per parent |
| D10 | Pages linked from nowhere but publicly reachable by URL. | 15 of 39 | Import every page as published (parity). Editors may list pages to import as hidden. | none |

D6 detail (research: gap-02 §1.4, gap-03 §6.13):

| Date | Row ids | Menus |
|---|---|---|
| 2021-11-24 | 307, 310 | identical |
| 2021-11-25 | 308, 311 | differ |
| 2021-11-26 | 309, 312 | identical |
| 2022-04-01 | 393, 398 | differ (393: soup and main; 398: soup, main and dessert) |
| 2022-05-18 | 428, 429 | identical |
| 2023-01-24 | 544, 549 | differ |
| 2024-06-14 | 818, 823 | differ |
| 2024-12-02 | 890, 891 | identical |

Fixes that need no sign-off, because they change nothing an editor or reader can see:

- `page_id` set to NULL on the 3 `internal_link` menu items (§6.11);
- the one role assignment that points at a deleted user dropped (§6.14);
- object-shaped `images` turned into arrays in key order (§6.1);
- the two invisible 1×1 px inline images removed (§7.1 N4).

### 8.2 The resolution file

The resolutions live in a versioned file in the repository, `data/import/resolutions.toml`, which the image ships as `/app/data/import/resolutions.toml`, the default of the `resolutions` argument (research: DECISIONS R9, D12). It holds only ids, dates, enum choices and source values, or the SHA-256 of a source value: no names of people and no content (titles, bodies, slugs, label or dish names). Who signed and when is recorded in the private runbook.

Every entry carries a **precondition**: the source value the editors saw when they signed. If the source row has changed since then, for example because someone corrected a date in the old admin, `apply` stops and names the entry, while a dry run lists it with every other problem, goes on with the defect's proposed default and exits non-zero at its end ([ADR 0047](adr/0047-import-dry-runs-collect-every-defect-and-refusal.md)). The entry is then re-signed. This keeps the list honest without freezing the old admin for weeks.

**Sign-off procedure.** Staging is LAN-only and used only by developers (decision 42), and run reports stay private (§11.3), so editors never see a report. Every run writes the sign-off sheet, `signoff.md`, into its report directory next to the report (§11.3, `src/domain/import_mysql/signoff.rs`), and the maintainer takes it from a staging dry-run report, which lists every instance at once ([ADR 0047](adr/0047-import-dry-runs-collect-every-defect-and-refusal.md)). It holds only defect ids, row ids, the public paths of the affected posts and pages on the live site (`/posts/<id>`, `/information/<slug>`), the resolution the import proposes for each instance (the entry's where a valid one covers it, otherwise the defect's proposed default), the rows that editors fixed in the source as their entry pins (D3), and, once the D6 detector adds them, both menus of each D6 date: the dish names that the public site already shows, the sheet's one column with content, and never a person's name (ADR 0047 §3). The maintainer hands it to the editors outside staging, as the private runbook describes, and records there who signed and when. The PR that changes `resolutions.toml` cites the runbook entry by date only.

**Schema.** `src/domain/import_mysql/resolutions.rs` defines it and its validation (open item 8). The file starts with `version = 1`, the schema version. `signed_off`, a date, is the day of the editors' sign-off; it is absent while the list is a proposal, as in the empty list that the repository starts with (`version = 1` alone). Every other key is a section, an array of tables with one table per entry:

| Section | Defect | Fields | One entry |
|---|---|---|---|
| `[[post_date_shift]]` | D1 | `post_id`, `expect_date` | a post whose date moves one day later; `expect_date` is its source date, never `1970-01-01` |
| `[[post_author]]` | D2 | `post_id`, `expect_author_id`, `set_author` | a published post without an author; `expect_author_id` is always `"none"`, and `set_author` is `"generic"` (`Szerkesztőség`) or an author id |
| `[[post_index_image]]` | D3 | `post_id`, `expect_index_image` | the index image that editors set on a published post in the current admin, `index/<32 hex>.<ext>` |
| `[[post_sentinel_date]]` | D5 | `post_id`, `expect_date` | a draft whose date becomes NULL; `expect_date` is always `1970-01-01` |
| `[[canteen_duplicate]]` | D6 | `date`, `expect_ids`, `keep` | a duplicate canteen date: every row id on it (at least two) and the one kept |
| `[[label_duplicate]]` | D8 | `expect_ids`, `keep` | a group of labels with equal names: every label id (at least two) and the one kept; the names are not written |
| `[[menu_unpositioned]]` | D9 | `parent_id`, `expect_null_lft`, `served_order` | a parent (`"none"` for the root level), its items without `lft`, and the order the live public API serves them in, a permutation of them |
| `[[page_hidden]]` | D10 | `page_id`, `expect_updated_at` | a page imported as hidden; `expect_updated_at` is its source `updated_at`, a UTC instant with whole seconds |
| `[[N7-accept]]` | N7 | `post_id`, `page_id`, `expect_content_sha256` | a body, named by exactly one of `post_id` and `page_id`, whose unplanned sanitiser removals are accepted (§7.1); its precondition is the SHA-256 of the source `content` |

The `expect_*` fields are the preconditions. Ids are the legacy ids, from 1 to 4294967295 (the source's `int unsigned`); dates are TOML local dates and instants TOML date-times in UTC, both written without quotes; TOML has no null, so `"none"` stands for NULL. D2's and D5's preconditions can each hold one value only, and are written out all the same, so that every entry states what the editors saw. D4 and D7 have no section: their rows are imported unchanged (§8.1, ADR 0017 §3), so no entry changes how one is imported. The editors' D4 confirmation that no body is missing is part of the sign-off that `signed_off` dates. `data/import/README.md` says what may be committed next to the file.

```toml
version = 1
signed_off = 2026-11-20                    # absent until the editors sign

[[canteen_duplicate]]                      # D6
date = 2022-04-01
expect_ids = [393, 398]                    # precondition
keep = 398

[[menu_unpositioned]]                      # D9
parent_id = 15
expect_null_lft = [26, 29, 30, 31]         # precondition
served_order = [26, 30, 29, 31]
```

The task reads the file (at most 1 MiB of UTF-8) and validates it after the path checks of §4.1 and before it takes the task lock, so before any read of the source. It reads the file once and keeps the SHA-256 of the bytes it validated, which the run's start log line shows and the `import` activity entries record (§5.4). Validation collects every problem and names the entry and the field of each, for example `canteen_duplicate[1] (D6), field keep: row 400 is not in expect_ids [393, 398]: keep one of the listed rows`, where `canteen_duplicate[1]` is the first `[[canteen_duplicate]]` table of the file. It refuses unknown keys and fields, missing fields, values of the wrong type or out of range, a precondition that cannot hold for its defect, a `keep` that is not among `expect_ids`, a served order that is not a permutation of the unpositioned items, and an entry for a row, date, label group or parent that an earlier entry of its section already covers, or a post in both D1 and D5. A file of another `version` gets that one problem and is not read further. Whether a row exists in the source and still has its defect is the precondition check that each run makes (§4.2). CI validates the committed file (`tests/image_data/`).

## 9. Not migrated

### 9.1 Tables

| Legacy table | Rows | Why it is not migrated |
|---|---:|---|
| `newsletter` | 0 (in every snapshot since 2020) | The feature is retired as a documented parity exception (decision 30). |
| `password_resets` | 6 | Account recovery becomes admin-issued one-time links or temporary passwords (decisions 10a, 10b). |
| `migrations` | 20 | Laravel's migration bookkeeping. backend3 has its own `seaql_migrations`. |
| `permissions` | 9 | Permission names are defined in code (ADR 0007), which keeps 8 of them and drops `edit newsletter`. The importer verifies the 9 legacy names (§6.15). |
| `role_has_permissions` | 20 | The role-to-permission map is defined in code. The importer verifies the 18 links of the kept permissions against it (§6.15). |
| `roles` | 4 | Seeded by a backend3 migration and verified, not copied (§6.13). |

### 9.2 Columns

| Legacy column | Why it is dropped |
|---|---|
| `users.remember_token` | Not portable; backend3 has its own session model (ADR 0006). |
| `pages.template` | Constant `page` (asserted). The public API returns the constant. |
| `pages.extras` | Never filled; the all-NULL triple on every row (asserted). |
| `pages.name` | Becomes `internal_note` (NULL when equal to the title). |
| `pages.deleted_at`, `menu_items.deleted_at` | Never set (asserted). Soft delete is new behaviour with its own `deleted_at` (ADR 0011). |
| `menu_items.lft`, `rgt`, `depth` | The nested set is replaced by `parent_id` + `position`; `lft` is used once to compute `position`. |
| `roles.guard_name`, `permissions.guard_name` | Always `web` (asserted). |
| `model_has_*.model_type` | Always `App\User` (asserted). |

### 9.3 Files on the uploads volume

The importer does not copy media. The media payload (`posts_images/`, `authors_images/`, `colleagues_images/`, `uploads/`) stays in place with unchanged paths and is served as before (ADR 0015). The importer only reads it for V7 and adds the inline images of N4.

| Path on the volume | Fate |
|---|---|
| Laravel `framework/` and `logs/` | not payload; left behind with backend1 |
| other Laravel-era files outside the media folders | not payload; handled by decision 28 as described in the private runbook |
| `sitemap.xml` (written in 2020) | replaced by a sitemap that backend3 generates from published posts and visible pages (decision 28) |
| `.htaccess` (sets `Access-Control-Allow-Origin: *`) | its header moves to the media server (ADR 0015) |

## 10. Verification

Checks V1-V11 run inside the import transaction, before the commit. A failure rolls everything back and the report names the check and the first failing rows. V12-V14 run across runs or after the commit.

Two helper definitions are used below:

- `dom(x)`: parse `x` as in N0, drop whitespace-only text nodes between block elements, serialise as in N8. It removes differences of encoding and formatting, not of content.
- `text(x)`: the text content of `dom(x)` without `script` and `style` content, with runs of whitespace collapsed to one space.

| Id | Check | Pass condition |
|---|---|---|
| V1 | Row counts | Every target table has the source count minus the documented deltas (table below). |
| V2 | Id preservation | The id set of every entity table equals the source id set, minus ids dropped by D6 and D8, plus a generic author created by D2. |
| V3 | Sequences | For every identity sequence, the next value is greater than `max(id)` and not lower than the legacy `AUTO_INCREMENT`. |
| V4 | Column digests | For each table, a SHA-256 over `(id, column values)` in id order. Columns copied verbatim: the digest of the source values equals the digest of the read-back target values. Transformed columns: the digest of the expected values (computed by the pure transforms) equals the digest of the read-back values. |
| V5 | Body checks | For every body: (a) the stored bytes equal the normaliser output (SHA-256 per row); (b) rows with no N1-N7 hit (N5b included) satisfy `dom(source) == dom(target)`, so only encoding and serialisation changed; (c) every row satisfies `text(source) == text(target)`, except rows changed by N1, whose difference must be exactly the inserted link text; (d) no named entity other than `&amp; &lt; &gt; &nbsp; &quot;` remains; (e) the `&nbsp;` count per row is unchanged except on rows with an N1, N3 or N4 hit, which are listed. The report lists, per row, the SHA-256 of the raw source, of `dom(source)` and of the target. |
| V6 | Legacy markup gone | 0 `data:` image sources, 0 dead-host `/storage/` URLs, 0 `div[data-oembed-url]`, 0 `center`, 0 `script`, 0 iframes outside the YouTube and padlet nodes, 0 floated images, 0 YouTube nodes with legacy sizes, 0 tables at the 1000 px stretch (decision 15b). |
| V7 | Files exist | Every `posts.index_image`, gallery entry, author image, colleague image and extracted inline image exists under `storage_root` (0 missing in the 2026-10-23 cross-check). Every extracted file's SHA-256 equals the decoded `data:` bytes. |
| V8 | Constraints and invariants | All foreign keys, unique indexes and checks hold. Explicit queries, for readable failures: published posts without an author or a date = 0; published posts without an index image = 0; duplicate live canteen dates = 0; duplicate live `lower(label name)` = 0; menu items deeper than two levels = 0; `page_link` items whose page is missing = 0. |
| V9 | Menu order | Per parent, the ordered list of child ids equals the served order captured from the live public API (§6.11), and the roots are 7, then 15. |
| V10 | Accounts | 40 users; every `password_hash` is a 60-character `$2y$` string; 43 `user_roles` rows whose per-role counts equal the source's minus the dropped orphan; 4 `user_permissions` rows. Only counts go to the report. |
| V11 | Times | Every event's `starts_at` and `ends_at`, converted back to Europe/Budapest, equals the source `DATETIME` to the second. UTC columns equal their source values as instants. Canteen audit values follow the configured rule. Every `posts.date` and `canteen_days.date` is a Europe/Budapest midnight, and its Budapest calendar day equals the source `DATE` after D1 and D5. |
| V12 | Determinism | Two runs over the same source and resolution file produce reports with equal digests, apart from the generated values listed in §11.2. Checked in CI with two fresh databases (§12), and in every nightly rehearsal, whose `apply` run follows a dry run over the same dump and must match its digests. |
| V13 | Fixed point | After the commit, `content_normalize mode:dry-run` reports 0 changes. In rehearsals, the admin editor round trip over every normalised body returns the stored bytes. |
| V14 | Public behaviour | The read-only smoke suite of ADR 0003 (checked against the REST spec, no diff against backend2, decision 25b) passes against staging's `/api/v1` after every `apply` rehearsal, and against production `/api/v1` as G8 in the window ([cutover.md](cutover.md) §7). The REST-spec request tests stay CI-only, on fixtures. |

Reference counts (2026-10-23 dump):

| Target table | Source rows | Expected target rows | Delta |
|---|---:|---:|---|
| `posts` | 576 | 576 | none |
| `labels` | 93 | 90 | D8 merges 5 rows into 2 |
| `post_labels` | 655 | 655 minus collapsed pairs | D8; the count is reported |
| `authors` | 49 | 49 or 50 | D2 may add the generic author |
| `colleagues` | 70 | 70 | none |
| `dishes` | 624 | 624 | none |
| `canteen_days` | 1,187 | 1,179 | D6 drops 8 rows |
| `canteen_day_dishes` | 2,921 | 2,921 minus the dropped days' links | D6; the count is reported |
| `events` | 293 | 293 | none |
| `pages` | 39 | 39 | none |
| `menu_items` | 23 | 23 | none |
| `users` | 40 | 40 | none |
| `roles` | 4 | 4 | seeded, verified |
| `user_roles` | 44 | 43 | the orphan row |
| `user_permissions` | 4 | 4 | none |

## 11. Idempotence, dry run and determinism

### 11.1 Dry run

Dry run is the default mode. It does everything `apply` does, inside the same single transaction, so every database constraint is exercised. Then it rolls back. Inline images go to a temporary directory and are discarded. The report is complete.

Dry run is the default for manual checks. The nightly rehearsal and the dress rehearsal on staging run `mode:apply` into a freshly recreated staging database and the staging copy of the uploads ([cutover.md](cutover.md) §5.2); the nightly rehearsal runs a dry run over the same dump first (V12). The smoke suite then runs against staging's `/api/v1` (V14). The cut-over runs the same `apply` path once (ADR 0003), so the commit and the inline-image writes to `storage_root` are rehearsed every night. The staging copy of the uploads keeps the inline images of earlier nights; write-if-absent accepts them when their bytes are equal (§11.2).

Where `apply` stops at the first defect or constraint refusal of §4.2, a dry run goes on with the defect's proposed default (or the source value) and without the refused row, lists every problem, and exits non-zero at its end ([ADR 0047](adr/0047-import-dry-runs-collect-every-defect-and-refusal.md)). So the nightly's dry run shows every open defect in one report, its `signoff.md` lists every instance for the editors, and both steps of the nightly stay red until the resolution list covers every instance that needs an entry (all but D10's, which are optional, and D4's and D7's, which have none). Rows that took a default are listed in the Defects part; the row counts, digests and checks of such a dry run describe the defaults, not a signed list.

### 11.2 Idempotence and determinism

| Property | How it holds |
|---|---|
| The import never runs twice into the same database | The target must be empty (§4.2). "Re-running" means a fresh database each time. |
| Same inputs, same result | Rows are sorted by primary key before they are transformed, and no transform reads the clock (`snapshot_at` is an argument). The only order that cannot be derived from keys, the unpositioned menu items, comes from the resolution file. |
| Generated values | `users.pid`, the new preview tokens of the 3 drafts, and the creation times of the `import` revisions and activity-log entries are random or real-time. They are excluded from digests and comparisons. |
| Normaliser | `normalise(normalise(x)) == normalise(x)` for every input the normaliser accepts, by a property test and by V13; a body still changing after three passes is refused as not normalisable and stops `apply` (§7.4, D140). `content_normalize` on normalised data changes nothing. |
| Files | Writing an inline image is write-if-absent. An existing file with identical bytes is accepted; different bytes stop the run. |

### 11.3 The run report

Every run that gets past the checks of §4.1 and the task lock writes a report to the `report` directory, also when it fails: `report.md` for people, then `signoff.md`, the editors' sign-off sheet of §8.2, then `summary.json`, the machine-readable part that the nightly rehearsal saves (cutover.md §5.2). A directory with a `summary.json` holds a complete report, and no file ever overwrites an existing one. All three files are created readable by the task's own user only (mode `0600`), because the report stays private (below). `summary.json` records the outcome (`completed` or `failed`, with the error), `started_at` and `finished_at` (two readings of the app clock, the first at the start of the run and the second just before the report is written) and `duration_ms`, their difference, which is the run time of ADR 0003; every argument of the run, defaults included; the source session's part, `source` (the fingerprint result, the latest timestamp of §4.1 with its column, and every source table's `AUTO_INCREMENT`; `null` when the run stopped before the session ended), which `report.md` shows in its Source section; the defect detectors' part, `defects` (whether the run reached its end, the defects checked, the counts of uncovered instances and of entries whose precondition does not hold, and one record per instance and per entry whose row the rule no longer finds), which `report.md` shows in its Defects section; and the not-migrated tables of §9.1 with their reasons. A run that fails keeps the parts of the steps that ended before it failed. Its `report_format` changes only when a field is renamed or removed. Two runs of the same dump with the same other arguments give the same report (ADR 0003) except in the fields that describe the run itself: `started_at`, `finished_at`, `duration_ms` and the `report` argument; a comparison of two runs leaves those out. Each import step adds its part:

| Part | Content |
|---|---|
| Summary | mode, `snapshot_at`, resolution-file SHA-256, row counts per table, result of every check V1-V12 |
| Source | the source fingerprint result, the latest timestamp in the source with its table and column (§4.1), and every source table's `AUTO_INCREMENT` (§3.3, §5.1) |
| Defects | every detected instance of D1-D10 and N7 with its outcome, its entry, the entry's precondition, the source value and the resolution the import proposes (the valid entry's, otherwise the defect's default, or none where the source value is kept), which a dry run applies, and every entry for a row that the rule no longer finds (a D3 entry whose post has the signed index image holds; the others are stale); legacy ids, calendar days, instants, SHA-256 digests, index image paths and fixed words only |
| Rule hits | per row, the hits of N1-N7 (N5b included), the images without `alt`, the rows exempted from a save-time rule (§4.3) and the values over a validation limit |
| Diffs | for each changed body, a unified diff of the pretty-printed source and target |
| Files | every extracted image: target path, size, SHA-256, pixel size |
| Accounts | counts only: users, roles per role, grants, users without a role, dropped orphans |

The report contains draft content, so it is kept privately, with the rehearsal tooling (ADR 0003). It is never attached to an issue, PR or public CI run. `signoff.md`, the sign-off sheet of §8.2, is the only extract made from it for people outside staging: it holds ids, public paths and proposed resolutions, never a person's name, and no content but D6's two menus once the D6 detector adds them (dish names that the public site already shows; ADR 0047 §3). CI runs use only synthetic fixtures, so their reports contain no real data.

## 12. Testing the importer

The importer meets the same bar as the rest of backend3 (ADR 0008, ADR 0022):

| Layer | What is tested |
|---|---|
| Unit tests of the pure transforms (`src/domain/`) | every rule of §5-§7 with small focused fixtures: JSON object to array; nested set and resolution order to `position`; spatie to `user_roles` and `user_permissions`, including the orphan; every time kind including the spring gap and the autumn overlap; each of D1-D10, including a precondition mismatch and a published post without an index image, which is refused (D3); the defect detector in both modes: an uncovered instance, a precondition mismatch and an entry for a row that no longer has its defect, each stopping `apply` and collected by a dry run, under each entry policy of ADR 0047 §4 (an unlisted D10 page takes the default without a problem; a D3 entry holds once its post has the signed index image), and a sign-off sheet that holds only its allowed fields; each of N0-N9 and N5b with a before and after fixture; the exemption flags of §4.3 |
| Property tests (proptest) | normaliser idempotence and the sanitiser fixed point over generated HTML; the Budapest round trip of ADR 0010 |
| Resolution file | parsing and validation, asserting the exact error message for each invalid case |
| Integration test in CI | GitHub Actions starts MySQL 8.4 loaded with a synthetic legacy fixture whose DDL matches the source fingerprint, and Postgres. The test runs `import_mysql` in dry-run and apply mode, asserts V1-V13, asserts that a second run into the same database is refused, that two fresh databases end with equal digests, and that a fixture variant with a published post without an index image is refused. The source is the `mysql` service of `compose.yaml` (profile `import`), which the `Test` job starts next to Postgres: production's MySQL image and tag, loaded at every start with `tests/fixtures/legacy/schema.sql`, the expected legacy DDL of the 20 tables, and an account with exactly the grants of §3.3. The synthetic rows join it with the steps that read them. The same job also builds and tests once without the `import-mysql` feature (§3.3): that build contains no MySQL driver, and the rest of the suite passes. |
| Admin editor round trip | the admin test suite loads the normaliser's fixture outputs into the editor and asserts byte equality (ADR 0013), including a body with an image without `alt` |

The synthetic fixture reproduces every production shape that the REST-spec fixtures also need (decision 25b): entity-encoded bodies, a published post with a NULL author, NULL content, object-shaped `images`, unpositioned menu items whose served order is not id order, a duplicate canteen date, a duplicate label, a 1970 sentinel, `data:` images including a 1×1 px one, a dead-host link, a legacy oEmbed wrapper, a bare iframe, a `center`, a floated image, a table that stretches to 1000 px, an image without `alt`, a dangling menu `page_id`, an orphan role row, an event inside a daylight-saving transition, and a bcrypt `$2y$` hash generated for the test. Fixtures are written by hand; they never copy rows from a production dump.

## 13. Open items

| # | Open item | How it is settled |
|---|---|---|
| 1 | Which `canteen_data` audit values are Budapest wall time. backend2's fix 03 shifted only the 322 rows that existed in January 2022 (website_backend2@7ba71da:`migrations/03-canteen_data_add_time.sql`; research: SYNTHESIS §4.4.2); later rows were written by Laravel in UTC. | `canteen_audit_local_before` defaults to `2022-01-21T00:00:00Z`. The first rehearsal compares the hour-of-day distribution of `created_at` before and after the cut-off and confirms or moves it. At most two hours of admin-only metadata are at stake (ADR 0010). |
| 2 | Settled: the type of the two calendar-date columns. | Decision 9 ("Everything `timestamptz`") covers them. They are `timestamptz` values at the Europe/Budapest midnight of the day (ADR 0010 point 2, ADR 0018 §1.3, §5.2 above). The row stays so that the other items keep their numbers. |
| 3 | The path of inline images extracted from page bodies (page 50). Decision 19 names only `/storage/uploads/posts/<id>/`. | Proposed `/storage/uploads/pages/<id>/inline-<n>.<ext>`. The first dry run checks that no existing folder under `uploads/pages/` collides; the maintainer confirms. |
| 4 | Link-level detail over the full dump. Attribute counts for N5-N7 and the dead-host links exist only for the 500-post harvest (research: SYNTHESIS §8, O48). | The first dry run over the full dump reports every occurrence; editors review the dead-host occurrences that N6 does not rewrite. |
| 5 | Settled: the exact content-v1 markup (the editor's YouTube and padlet nodes, the centred image, the table shape, the style serialisation and the whitespace handling). | ADR 0013 (Accepted) and [editor.md](editor.md) §3 define it, and [editor.md](editor.md) §9 lists the rules that N2, N5, N5b and N7 apply. N9 and V13 check that every output is the editor's fixed point. The row stays so that the other items keep their numbers. |
| 6 | Public links to the 3 label ids that D8 merges away (`/search/label/<id>`). | The maintainer decides, before the PR that implements `/search/label/{id}` merges, whether the public API answers them with the surviving label or with 404, and [public-api.md](public-api.md) records the answer. Until then the importer reports the 3 merged ids and their survivors. Low traffic is expected. |
| 7 | How the release-1 seed of ADR 0019 (partner banners, site settings, one Diáktoll issue) runs at the cut-over. `import_mysql` does not write those tables. | Proposed: a separate idempotent task that reads versioned seed files from the repository (`data/seed/`, which the image ships as `/app/data/seed/` with the rest of `data/`) and runs right after the import in the window, rehearsed nightly together with it. Settled with ADR 0019 and [cutover.md](cutover.md) before the first dress rehearsal. |
| 8 | Settled: the schema of the resolution file. Its path is `data/import/resolutions.toml`, shipped in the image as `/app/data/import/resolutions.toml` (research: DECISIONS R9, D12). | `src/domain/import_mysql/resolutions.rs` defines the schema and its validation, and §8.2 lists it. The row stays so that the other items keep their numbers. |
| 9 | Removal of the importer. | After the maintainer decommissions the legacy stack (backend1, MySQL, backend2 and its Redis; decision 41, [ADR 0026](adr/0026-backend2-unchanged-and-running-until-decommissioning.md)), a dedicated PR removes the `import-mysql` feature and the `import_mysql` task. `content_normalize` stays. |
| 10 | Where the editor puts N5's `em` in an italic `h2` that also holds other marks. ProseMirror's serialiser nests a text's marks in the order of the schema's marks and keeps an open mark only while that order allows, so `J` may write `<h2><em><strong>a</strong> b</em></h2>` as `<h2><strong><em>a</em></strong><em> b</em></h2>` (editor.md §6, §9). N5 wraps the heading's content in one `em`, as editor.md §9 reads, and the corpus has 2 italic headings **[500]**. | The N9 check `J(out) == out` (editor.md §9) over the N5 fixtures, which the admin suite runs on the importer's fixture outputs (editor.md §13.4) once the editor library is in. If the order differs, N5b splits the `em` along the editor's mark order: D147 (§7.4) gives the editor's mark structure to N5b, whose mark rule (2) nests marks in the editor's order and splits an outer mark where that order needs it (§7.1, N5b (4)). |

## 14. References

- research: DECISIONS 8, 9, 10a, 10e, 15, 15b, 15c, 19, 20, 21, 22, 23b, 25b, 28, 30, 32b, 33, 34a, 39, 41, 42
- research: DECISIONS R29 (D139 and D140, answers of 2026-11-01); research: third decision register D139 and D140 (the questions, their context and the options)
- research: DECISIONS R31 (D142 to D148, answers of 2026-11-15); research: third decision register D142 to D148 (the questions, their context and the options)
- research: SYNTHESIS §4.1-§4.10, §4.12.3, §4.13, §4.14, §5.6, §5.7
- research: gap-02 §1.1, §1.4, §1.5, §4.2, §4.5; gap-03 §4, §5, §6, §8, §9, §10; gap-05 §2-§6; gap-06 §4, §5, §7; gap-07 §3, §4, §7.4; gap-09 §3, §5, §8
- website_backend@0dead98: `app/Models/Posts.php`, `app/PageTemplates.php`, `database/migrations/`
- website_backend2@7ba71da: `migrations/03-canteen_data_add_time.sql`, `migrations/11-posts_data_fix_types.sql`, `migrations/12-menu_items_fix_types.sql`, `migrations/13-posts_fix_null_date.sql`
- loco-rs@1.2.0: `src/schema.rs`, `src/cli.rs`, `src/task.rs`; sea-query@1.0.2: `src/backend/postgres/table.rs`; html5ever@0.40.1: `src/serialize/mod.rs`
- ADR 0002, ADR 0003, ADR 0006, ADR 0007, ADR 0008, ADR 0009, ADR 0010, ADR 0011, ADR 0013, ADR 0015, ADR 0017, ADR 0018, ADR 0019, ADR 0020, ADR 0022, ADR 0024, ADR 0026, ADR 0043 (N1 and N2 titles and ids)
