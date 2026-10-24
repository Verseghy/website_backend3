# 0003. Cut-over: one rehearsed flip, then a read-only parallel run

- Status: Accepted (2026-10-24)
- Decision questions: Q2, Q41, Q42, and the cut-over parts of Q31, Q32b and Q33

The step-by-step plan is in [cutover.md](../cutover.md). The operator runbook, with commands and credential handling, lives in the private `Verseghy/runbooks` repository.

## Context

backend3 replaces two running systems that share one MySQL database:

| System | Role today | Reads or writes |
|---|---|---|
| backend1 (`website_backend`, Laravel 7 + Backpack 4.1) | the only admin | writes the `verseghy` database and the uploads volume |
| backend2 (`website_backend2`, Rust GraphQL, 3 replicas) | the public read API | reads live rows on every request; no response cache |
| website_frontend (Angular) | the public site | calls backend2 at a URL compiled into the bundle |

These facts set the shape of the cut-over:

1. **backend3 has its own database.** backend3 runs on PostgreSQL with a clean schema ([ADR 0002](0002-postgresql-as-the-database.md), [ADR 0009](0009-clean-domain-schema-with-preserved-integer-ids.md)). backend2 serves whatever MySQL holds at the moment of the request. Once editors save in backend3, the public site is stale until the Angular site stops calling backend2. The old and new admins cannot both be live on real data. (research: SYNTHESIS §4.11.1, gap-02 §0)
2. **The public site's API URL is compiled in.** `environment.prod.ts` sets `gqlURL: 'https://backend2.microshift.verseghy-gimnazium.net/graphql'`. Moving the site to backend3's REST API means a new frontend image. Building and promoting it are separate steps: PAC builds the image on a push to `master` (about 17 min at 4 CPU / 6 GiB), then argocd-image-updater opens a `website_k8s` PR that a human merges. So the image can be built weeks ahead and promoted in minutes. (research: SYNTHESIS §5.17.5; website_frontend@f74c765:apps/frontend/src/environments/environment.prod.ts)
3. **The frontend change is small.** The REST switch is about −450/+300 lines in 10 service and bootstrap files, 3 environment files and `package.json`. Routes, components, templates and NgRx state do not change. (research: SYNTHESIS §5.17.1)
4. **The data is small.** 20 tables, 6,677 rows, about 6.1 MiB. The import ([ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md)) runs in seconds, so most of the window goes to checking, not copying. (research: gap-02 §1.1, §4.5)
5. **Writes are rare.** In the 56 days before the research, editors created 28 posts (about 3.5 a week), 38 canteen days (one school week per week), 1 page and 1 user. Events have not changed since 2022-08-31. An evening without saves loses nothing measurable. (research: gap-02 §1.3)
6. **MySQL can be frozen without changing backend1's code.** With `super_read_only = ON` the server refuses every client write. `SET PERSIST` keeps the setting across a pod restart. Setting `super_read_only` to ON also forces `read_only` to ON. Only setting `read_only` to OFF forces both off, so lifting the freeze turns off both variables. backend2 only reads, so the public site keeps working. backend1 keeps its sessions in files, so editors can still log in and read, but every save fails. The "remember me" login option and lazy preview-token creation also stop, because both write to the database. (research: SYNTHESIS §4.11.3, gap-02 §4.5; MySQL 8.4 manual, `read_only` and `super_read_only`)
7. **The legacy tables are not transactional.** All 20 tables use MyISAM and have no foreign keys, so a dump is consistent only if it holds one global read lock for its whole run, which `mysqldump --lock-all-tables` does. (research: gap-02 §1.1, SYNTHESIS §4.13 row 16)
8. **Deleting the wrong object is final.** Every Argo CD Application in `website_k8s` carries the `resources-finalizer.argocd.argoproj.io` finalizer. Deleting an Application therefore deletes its PVC, and the volume goes too unless the PV's reclaim policy is `Retain`. Argo CD syncs with prune and self-heal off, so a live `oc scale` survives only until the next sync from Git, and removing a manifest from Git does not delete the object. (research: SYNTHESIS §1.8.5, §8 O25)
9. **Rehearsals need real data, and the real data is personal.** The dumps contain 40 accounts with e-mail addresses and password hashes. The hub cluster is LAN-only, so only developers can reach it. The production node has very little spare capacity ([ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md), Q36). (research: SYNTHESIS §1.2, QUESTIONS Q42)
10. **Two approved backend2 fixes were not merged when the research was done.** website_backend2#251 pages through posts by date and id, so back-dated posts are no longer skipped (without it, 60 of 541 published posts cannot be reached through the home page's "Több betöltése"). website_backend2#258 returns an error instead of HTTP 500 for an ISO week that does not exist, which the canteen page asks for from 2026-12-28 to 2027-01-03. backend2 serves the public site until the flip and is the public site's rollback target after it. (research: SYNTHESIS §5.5.3, §5.11.2)

## Decision

### 1. One rehearsed flip

- backend3 goes live in **one evening window**. In that window the old admin is frozen, the data is imported once, the Angular REST build is promoted, the new admin opens and backend2 is scaled down.
- In the window the import reads the **frozen production MySQL directly**, with a read-only account under `LOCK TABLES ... READ` ([data-migration.md](../data-migration.md) §3.3). The final dump is the reference copy of the same state.
- There is **no** backend2-compatible GraphQL layer in backend3, **no** public-REST-first phase with a scheduled re-import, and **no** replication between the databases.
- The window is scheduled only when backend3 is at parity: every parity issue is closed by a merged PR that references it, or closed as a documented parity exception (Logs, Backups, newsletter; [ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md)).
- website_backend2#251 and #258 are merged and deployed **now**, independent of the window date (decision 2). The maintainer merges them (decision R2). Agents do not.
- The admin write freeze is budgeted at **1 to 3 hours**. The nominal sequence, measured from the freeze at T0, is: final dump at T+1 min, import at T+2, Angular promotion at T+10, new admin open at T+20, backend2 scaled down at T+30 (research: gap-02 §4.5). The rest of the budget is reserve for checks.

### 2. Rehearsals on hub staging (Q42)

- Staging is a namespace on the LAN-only hub cluster. Only developers use it. Editors first use the new admin after go-live. The read-only parallel run (section 7) gives them a reference view.
- **Nightly rehearsal:** the newest production dump, taken with `--lock-all-tables`, is loaded into a throwaway MySQL on staging. The import runs into an empty, migrated PostgreSQL. It produces a machine-readable report. A read-only smoke suite then runs against staging's `/api/v1`.
- **Dress rehearsal:** at least one run of the whole window on staging, including the freeze, a restart of the frozen MySQL pod, a rollback before the first content change with the unfreeze checked across a restart, and a second run through to go.
- **Same-day rehearsal:** on the day of the window, a rehearsal runs against that day's dump and must pass.
- The number of consecutive passing nightly runs required before scheduling is set by the maintainer ([cutover.md](../cutover.md) open item 7).
- Production data stays on staging. It is never committed, attached to an issue or pasted into a PR. CI uses synthetic fixtures that include the production-only data shapes (Q25b).

### 3. Production MySQL stays as it is until it retires (Q32b)

- Production MySQL gets no engine conversion, no charset change and no data fixes before the cut-over. Every data fix lives in the importer ([ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md)).
- Every dump that feeds a rehearsal, and the final reference dump, is taken with `mysqldump --lock-all-tables`. The backup runbook takes its MySQL dumps the same way ([ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)).
- backend2's hand-run SQL fix 12 (website_backend2@7ba71da:migrations/12-menu_items_fix_types.sql) is **never** applied. It fails on today's data with `ERROR 1265`. After a back-fill it would break the old admin's menu create form instead (research: SYNTHESIS §4.4.2).

### 4. Prerequisites

**Retain first.** Before any `website_k8s` change that touches the `mysql` or `backend` Applications or the uploads PVC, the MySQL and uploads PVs are set to `reclaimPolicy: Retain`. This includes the media-server handover and backend3's co-mount. The PostgreSQL PV is set to `Retain` as soon as it is bound, before backend3's first production deploy. The pre-flight checks all three again. (research: SYNTHESIS §8 O25; decision 41)

The window does not start until all of these hold. [cutover.md](../cutover.md) section 4 lists how each one is checked.

- Parity reached (section 1); import task complete; the editors' sign-off on the defect resolution list is recorded (ADR 0017).
- Rehearsal exit criteria met (section 2).
- The Angular REST change is merged in `website_frontend`, built by PAC, and its Image Updater PR is **open and held** (`/hold`). The rollback target, the frontend tag before the flip, is recorded.
- website_backend2#251 and #258 are still deployed. They were merged earlier (section 1). The pre-flight checks them again because backend2 is the rollback target.
- The three PVs still have `reclaimPolicy: Retain`.
- The backup runbook covers the PostgreSQL database and the new PVCs, and a PostgreSQL restore has been rehearsed on staging ([ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)).
- backend3 runs in production on its host ([ADR 0014](0014-one-host-backend3-microshift-verseghy-gimnazium-net.md)) against an empty, migrated database with no accounts. Its probes pass, and the frontend pod can reach it (server-side rendering calls the public host).
- backend3's three production Routes (`backend3`, `backend3-api-admin`, `backend3-api-v1`; ADR 0014) carry the HAProxy connection and request-limit annotations of [ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) from their first deploy. The annotations are verified against the MicroShift docs for the node's confirmed version. The rest of ADR 0023's hardening is live as well.
- The release-1 seed of [ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md) is ready and rehearsed with the import: the three partner banners, the site settings and one Diáktoll issue. No notice is seeded.
- The media variant from [ADR 0015](0015-media-separate-static-server-media-library-image-optimisatio.md) is settled by the co-mount test. If the separate media server is used, it already serves `/storage/**` on both hosts.
- The node's spare capacity covers the parallel run. If it does not, the window is not scheduled. Scaling down or removing backend1 or MySQL is never the remedy (section 7).
- Every cluster change of the window is a pre-opened, held `website_k8s` PR, and so is every rollback change.

### 5. The point of no return

- The point of no return is the **first write through the admin API**, apart from the exceptions below. This includes every create, update or delete of an admin-editable entity, and every media-library upload, rename, move and delete.
- These do not count: logins and logouts with their bookkeeping (throttle counters, `token_version`, last-login time), bcrypt-to-Argon2id rehashes, and changes to one's own account (password change, TOTP enrolment). Public read counters are not admin writes and do not count either. A rollback loses all of these. Losing logins and rehashes costs nothing, because MySQL still holds the old password hashes. Account changes made before the first content change, such as a new password or TOTP enrolment, are lost too. Editors are told this.
- Nobody edits imported data by hand in the window. A data problem is a no-go. It is fixed in the importer or the resolution list and rehearsed again.

### 6. Go/no-go gate

- The maintainer decides go or no-go after the import and the public flip, and before the new admin opens. An agent never makes this call.
- The checks cover the import report, the API smoke suite, the public pages, admin login and a fresh PostgreSQL backup. [cutover.md](../cutover.md) section 7 lists them with pass criteria.
- Any failed check means no-go and the free rollback in section 8.

### 7. Read-only parallel run (Q41)

- **Retain first.** The PVs are set to `Retain` before any change that could delete them (section 4).
- After go-live, **backend1 and MySQL stay up, read-only**, as a reference view for editors. MySQL keeps `super_read_only = ON`, set with `SET PERSIST`. backend1 keeps its URL, `https://backend.microshift.verseghy-gimnazium.net/admin`, and loses write access to the uploads volume. Its scratch files (sessions, cache, compiled views, logs) move to an `emptyDir`.
- backend2 and its Redis are scaled to 0 replicas in Git. Their manifests stay.
- The website_frontend2 hub beta is scaled down when backend2 goes to 0 (decision 31).
- The parallel run lasts **until the maintainer decides to decommission**. No timer, CronJob, TTL or scheduled PR scales down or deletes backend1, MySQL, their PVCs or their Applications. Agents never propose decommissioning on their own initiative. The committed guardrails deny `oc delete pvc`, `oc delete pv` and `oc delete application` ([ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md)).
- **One writable admin at a time.** At every point in the plan, at most one of the two admins accepts writes. This includes rollback: backend3 is closed before MySQL is unfrozen.

### 8. Rollback

**Before the first content change: free.** The maintainer reverts the promotion and unfreezes, in this order:

1. If backend2 is already scaled down, scale it and its Redis up. Then revert the frontend tag to the recorded one.
2. Close the new admin. Reset backend3's database to the empty, migrated schema, so no account can log in.
3. Undo any media change made in the window, and give backend1 back its write access to the uploads volume.
4. Unfreeze MySQL with `SET PERSIST super_read_only = OFF; SET PERSIST read_only = OFF;`. `SELECT @@GLOBAL.read_only, @@GLOBAL.super_read_only;` must return `0 0`.

This takes minutes and loses no content. Rotated credentials stay rotated.

**After the first content change: fix forward.** PostgreSQL is now ahead of MySQL. The default is to fix backend3 through normal PRs. The maintainer may still order a manual reverse move. It works because backend1 and MySQL are still there:

1. Make backend3's database read-only with `ALTER DATABASE <db> SET default_transaction_read_only = on`, then restart backend3 so every connection picks up the setting.
2. Scale backend2 and its Redis up, then revert the frontend tag.
3. List the changes made since go-live from the activity log ([ADR 0011](0011-safety-net-soft-delete-revisions-and-an-activity-log-for-eve.md)). Also list the media operations (`media_upload`, `media_rename`, `media_move`, `media_delete`). Undo renames and moves on the volume, and restore deleted files from the media trash, before re-entering content.
4. Give backend1 back its write access to the uploads volume, unfreeze MySQL (the two statements and the check above), and re-enter those changes by hand in the old admin.

No reverse-sync code is written.

### 9. Credentials (cut-over part of Q33)

- The cut-over includes a full rotation of every credential the website's workloads use: database account passwords, application keys, and backend3's production keys. backend3's production keys are new values that were never used on staging or in rehearsals. The full inventory is in the private runbook, and every item in it is rotated.
- Rotation is the **first phase of the window, before the freeze**. A rotation failure is then cheap: nothing is frozen yet, and the window can stop. Once set, the freeze is never lifted except by a rollback.
- backend3's keys go into the shared `backend-secrets` ExternalSecret, next to the existing apps' keys ([ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)). The rotation forces an ExternalSecret refresh instead of waiting for the 24 h refresh interval.
- A rollback does not undo the rotation. After a no-go, the rescheduled window does not rotate again the credentials that phase 1 already rotated.

### 10. Who does what

- The maintainer runs the window, reviews and merges every `website_k8s`, `website_frontend` and `website_backend2` PR (decision R2), and decides go/no-go, rollback, a reverse move and decommissioning.
- One editor is on call during the window to check content and make the first content change.
- Agents build the importer and the smoke suite, open and hold the PRs, and report. They run rehearsals only by triggering and reading the staging CronJob and its report. The dress rehearsal's database steps (the import run, the freeze and unfreeze, the reset, PostgreSQL read-only) are run by the maintainer by hand, because the committed guardrails (decision 37, [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md)) keep agents' database commands to local databases. Agents never merge outside `website_backend3` and `Verseghy/runbooks`.
- `website_backend3` `master` is frozen from the same-day rehearsal to the end of the window, so production runs the image that was rehearsed.

## Consequences

**Positive**

- No throwaway code: no GraphQL compatibility layer, no sync job and no reverse export.
- Loco stays on its supported PostgreSQL path from the first day ([ADR 0002](0002-postgresql-as-the-database.md)).
- The window runs an import that has already run against real data every night for weeks.
- Rollback is free until the first content change. It needs only reverts of held PRs, a database reset and the two unfreeze statements.
- Editors keep a read-only view of the old admin for as long as the maintainer wants.
- No automatic step can delete old data.

**Negative**

- Delivery is all or nothing: editors get nothing from backend3 until the whole admin is at parity.
- Editors do no acceptance testing before go-live, because staging is LAN-only. The reference view, the on-call editor and the safety net (soft delete and revisions, ADR 0011) limit the cost of early mistakes.
- Editors cannot save for one evening.
- The old stack runs for an open-ended time. It uses memory on a small node, still needs routine image updates, and stays in the backup runbook.
- After the first content change, going back means re-entering changes by hand.
- The window carries the credential rotation, which adds steps to the evening.
- Browsers that hold the old Angular service worker keep running the old bundle until they reload. After backend2 is scaled down, that bundle's API calls fail until the visitor accepts the update prompt. The dress rehearsal measures this (cutover.md, open items).

**What must be true in code, tests and manifests**

- The import task is idempotent and deterministic: the same dump gives the same rows and the same report. It starts from an empty schema, opens the MySQL source read-only and never writes to it. It takes its source and target connection settings as explicit arguments ([ADR 0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md)). It writes a machine-readable report: per-table counts, the expected deltas from the resolution list, integrity checks, a normalisation summary, sequence checks and run time.
- Pure transforms in the importer have unit tests. The import as a whole has an integration test against a synthetic MySQL fixture that contains the production-only shapes (entity-encoded bodies, NULL authors, unpositioned menu items).
- The smoke suite lives in this repository. It is read-only (GET only), takes a base URL, checks backend3 on its own and does not diff against backend2 (Q25b). It runs nightly on staging and once in the window.
- Public read endpoints keep answering when the database refuses writes. The read counters from Q27 are best-effort, so a read-only database during a reverse move does not break the public API.
- backend3 boots, and `/_readiness` returns 200, against a read-only database. Production config sets `DB_AUTO_MIGRATE=false`, so boot runs no migration. Queue workers log refused writes and back off.
- The activity log records every content change with entity, id, user, time and the changed fields, enough to re-enter it by hand. It also records every media operation with the old and new path (ADR 0011).
- backend3 boots in production against an empty database with no accounts. The first accounts come only from the import.
- `website_k8s` gets these PRs ahead of the window, held: the frontend promotion (from Image Updater); backend1's read-only media mount with its scratch files on an `emptyDir`; backend2 and its Redis at 0 replicas; the media handover, if the fallback media variant is used; and the matching revert PRs.

## Alternatives considered

| Question | Option | Why not |
|---|---|---|
| Q2 | B. Flip plus a temporary backend2-compatible `POST /graphql` with APQ in backend3 | It would shrink the freeze to minutes, but costs about 800-1,200 lines of throwaway code and a second public contract until the REST switch. |
| Q2 | C. Public REST first, fed by a scheduled MySQL-to-PostgreSQL re-import every 5-15 min | It needs sync code in production for weeks, and public edits lag by up to one interval. |
| Q2 | D. No flag day, with the Angular switch as a normal release | Possible only on a shared MySQL. ADR 0002 chose PostgreSQL. |
| research | Freeze the old admin for the whole development time | Editors would have no working admin for weeks or months. |
| research | Continuous binlog replication into PostgreSQL | More machinery than 6 MiB of data warrants, and it mirrors the old schema shape. |
| Q41 | A. Go/no-go gate, then forward-only: MySQL and backend1 at 0 replicas for 30 days, then deleted, with the final dump kept offline | Not chosen (maintainer's choice). The old admin stays readable as a reference, and retirement is the maintainer's decision, not a timer. The go/no-go gate and `Retain` are kept. |
| Q41 | B. A reversible window with a PostgreSQL-to-MySQL reverse export, so backend1 can resume for N weeks | Throwaway code across a renamed schema. |
| Q41 | C as offered: a parallel run for a fixed N weeks | Chosen without the fixed end. The run lasts until the maintainer decides. |
| Q42 | A. A staging overlay on the production node with a scrubbed nightly copy that editors can reach | Not chosen (maintainer's choice). The node has very little spare capacity (Q36). |
| Q42 | C. No shared staging, only local compose and the restore drill | The window and its rollback need rehearsing on a cluster with the real images and routes. |
| Q32b | A. Convert the tables to InnoDB and set the database default to utf8mb4 now | Production MySQL stays as it is until it retires. `--lock-all-tables` gives consistent dumps without touching it. |
| Q32b | B. A plus a data hygiene script on production | Data fixes live in the importer. The old admin would re-create one of them on its next menu edit. |
| Q32b | D. Leave production untouched and rely only on the cut-over freeze for consistency | Nightly rehearsal dumps are taken while editors work, so they need consistent dumps too. |
| Q33 | A. Rotate independently of the cut-over, with a per-app `backend3-secrets` ExternalSecret | Not chosen (maintainer's choice): the rotation is part of the cut-over, and backend3 shares `backend-secrets`. |
| Q33 | C. Rotate independently of the cut-over, keeping one shared `backend-secrets` | Same reason. |

## References

- research: SYNTHESIS §1.1, §1.2, §1.4, §1.8.4, §1.8.5, §1.11, §1.14 (`DB_AUTO_MIGRATE`), §1.16.7
- research: SYNTHESIS §4.4.2, §4.11.1, §4.11.3, §4.12.3, §4.12.7, §4.12.8, §4.13
- research: SYNTHESIS §5.5.3, §5.11.2, §5.17.1, §5.17.4, §5.17.5; §6.6 (Loco health routes)
- research: SYNTHESIS §8 (O13, O15, O16, O18, O25)
- research: gap-01 §3.2; gap-02 §0, §1.1, §1.3, §4.2, §4.5, §5
- research: QUESTIONS Q2, Q25, Q31, Q32, Q33, Q41, Q42; decisions of 2026-10-24 (2, 25a, 28, 29, 30, 31, 35, 37, 41, R2)
- [data-migration.md](../data-migration.md) §3.3 (how the importer reads the source); ADRs 0011, 0014, 0015, 0019, 0023, 0024
- website_backend2#251 "fix: page through posts by date and id so backdated posts are not skipped"
- website_backend2#258 "fix: report nonexistent canteen weeks as a GraphQL error"
- website_backend2@7ba71da:migrations/12-menu_items_fix_types.sql
- website_frontend@f74c765:apps/frontend/src/environments/environment.prod.ts
- website_k8s@34b8b55:verseghy-website/frontend/kustomization.yaml, verseghy-website/image-updater/imageupdater.yaml, verseghy-website/applications/
- MySQL 8.4 Reference Manual: [`read_only`](https://dev.mysql.com/doc/refman/8.4/en/server-system-variables.html#sysvar_read_only), [`super_read_only`](https://dev.mysql.com/doc/refman/8.4/en/server-system-variables.html#sysvar_super_read_only), [persisted system variables](https://dev.mysql.com/doc/refman/8.4/en/persisted-system-variables.html), [mysqldump `--lock-all-tables`](https://dev.mysql.com/doc/refman/8.4/en/mysqldump.html#option_mysqldump_lock-all-tables)
- PostgreSQL 16 documentation: [`default_transaction_read_only`](https://www.postgresql.org/docs/16/runtime-config-client.html#GUC-DEFAULT-TRANSACTION-READ-ONLY), [`ALTER DATABASE ... SET`](https://www.postgresql.org/docs/16/sql-alterdatabase.html) (unchanged in 18, the other candidate of ADR 0002)
- [cutover.md](../cutover.md)
