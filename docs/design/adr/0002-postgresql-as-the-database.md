# 0002. PostgreSQL as the database

- Status: Accepted (2026-10-24)
- Decision questions: Q1

## Context

backend3 replaces two applications that share one MySQL 8.4 database named `verseghy`: backend1 (Laravel 7 admin) writes it, and backend2 (async-graphql public API) reads it. backend3 needs a database it owns.

The facts that force the choice:

| Fact | Value | Source |
|---|---|---|
| Size of the legacy data | 6.05 MiB data and indexes, 6,677 rows, 20 tables | research: SYNTHESIS §4.1 |
| Integrity in the legacy database | 0 foreign keys, primary keys plus one unique index only, non-transactional MyISAM tables, database default charset `latin1` | research: SYNTHESIS §4.1, §4.6.1 |
| Loco 1.2.0 database support | Postgres and SQLite only. No MySQL feature; a MySQL driver arrives only through app-side Cargo feature unification | research: SYNTHESIS §6.3.1, §6.3.4, §4.11.2 |
| Loco on MySQL, concretely | `cargo loco db create` returns "Only Postgres databases are supported for table creation"; per-test databases fall back to a no-op `Any` backend, so tests get no isolation; the DB-backed job queue exists only for Redis, Postgres and SQLite; `db seed --reset` and `db schema` return `BackendNotSupported`; the boot-time `verify_access()` check is skipped | research: SYNTHESIS §4.11.2, gap-02 §1.8 |
| Loco on Postgres | fully on the supported path: `queue.kind: Postgres`, per-test databases (`_loco_test_<rand>_<ts>`, needs `CREATEDB`), `db create`, the vendored agent skill's recipes | research: SYNTHESIS §6.3.4, §6.8, §6.9 |
| Two drivers in one binary | `sqlx-mysql` and `sqlx-postgres` compile together with loco-rs 1.2.0 (executed in a probe) | research: SYNTHESIS §6.24, §6.26 |
| Search and ordering needs | Hungarian accent-insensitive search and Hungarian name ordering (ADR 0020) need `unaccent` and an ICU `hu` collation | research: SYNTHESIS §4.10 R6 |
| Platform | production is a single-node MicroShift with LVMS storage. Its only operator catalog has no Postgres operator, so a Postgres must be a plain Deployment | research: SYNTHESIS §4.11.3, gap-02 §4.3 |
| House precedent | the iam and matverseny backends already run PostgreSQL 16 (on another cluster, through an operator) | research: SYNTHESIS §4.11.3 |
| Coexistence | backend2 reads the MySQL rows directly. Once backend3 writes to another database, the public site is stale until the Angular site reads backend3. A separate database therefore implies a coordinated cut-over (ADR 0003) | research: SYNTHESIS §4.11.1 |

At 6 MiB the one-off data migration costs little. Permanent workarounds for an unsupported engine would cost something in every later change.

## Decision

backend3 owns a new **PostgreSQL** database. It is the only database backend3 reads or writes at run time. The legacy MySQL data is copied into it by the import task of ADR 0017, whose mapping is specified in [data-migration.md](../data-migration.md).

### What runs

| Item | Decision |
|---|---|
| Workload | One plain Deployment `postgres` with `strategy: Recreate`, one RWO PVC, one Service `postgres:5432`, in website_k8s under `verseghy-website/postgres/`. It mirrors the existing `mysql` component and its Argo CD Application. No operator. |
| Volume | The PV of the Postgres PVC uses `persistentVolumeReclaimPolicy: Retain`, set before the first production import (ADR 0003, ADR 0023). |
| Security context | Runs under the default `restricted-v2` SCC with an arbitrary UID. If the official image is chosen, `PGDATA` is a sub-directory of the mount, so `initdb` owns it: PostgreSQL 18 images keep it in `/var/lib/postgresql/18/docker`, below a volume mounted at `/var/lib/postgresql` (website_k8s@f0286dc:verseghy-website/postgres/deployment.yaml). Verify SCC behaviour against the MicroShift documentation for the live version before the manifests merge. |
| Database and roles | One database `verseghy`. The container's bootstrap superuser has its own secret key. An init script creates the application role `website_backend3` (`LOGIN`, no `SUPERUSER`, no `CREATEDB`), which owns the `verseghy` database and runs backend3's migrations. |
| Secrets | backend3's database keys go into the shared `backend-secrets` ExternalSecret (decision 33, ADR 0023). backend3 reads `DATABASE_URL` from its environment in `config/production.yaml`; no module below the Loco boundary reads the environment (ADR 0008). |
| Extensions and collations | backend3 migrations create the `unaccent` extension (a trusted extension, so the owning role may create it) and the ICU collation `hu_icu` that name ordering uses through an explicit `COLLATE` clause (ADR 0020). The database's default collation stays deterministic. |
| Session settings | backend3 sets the session `TimeZone` to `UTC`. No query depends on the session zone (ADR 0010). |
| Queue and cache | Durable jobs use Loco's Postgres queue (`queue.kind: Postgres`). There is no Redis (ADR 0016). |
| Starting resources | Request 64Mi memory, limit 256Mi; `shared_buffers=32MB`, `max_wal_size=256MB`. These are research proposals for a dataset of tens of MiB (research: gap-02 §4.3) and are adjusted once the node capacity is known. |
| Backups | The new database and its PVC are added to the existing backup procedure before the first production import (decision 32a). The procedure and the restore drill are documented in the private runbook (ADR 0023). |
| CI and tests | GitHub Actions runs the tests against a throwaway Postgres started by `compose.yaml`, whose role has `CREATEDB`, so Loco's per-test databases work (ADR 0022). Tests never touch a non-local database (ADR 0024). |

### Settled in the deployment PR, not here

These were not decided in the design round. The deployment PR records each choice with its evidence:

| Open point | How it is settled |
|---|---|
| Image | `docker.io/library/postgres` (same registry class as today's `mysql` image, semver tags work with Argo CD Image Updater) or `quay.io/sclorg/postgresql-16-c9s` (arbitrary-UID safe by design). Decided by a dry run of each candidate under `restricted-v2` on the node. `registry.redhat.io/rhel9/postgresql-16` needs registry credentials for Image Updater and has non-semver tags. |
| Major version | 16 (parity with iam and matverseny) or 18 (supported until 2030-11-14). When this ADR was written, nothing in backend3's design needed a feature newer than PostgreSQL 15 (`UNIQUE NULLS NOT DISTINCT`). Production's `initdb` now uses the builtin `C.UTF-8` locale (research: DECISIONS R12, R13), and the builtin locale provider needs PostgreSQL 17 or later. The deployment PR runs 18 ([design README](../README.md#open-questions), Open questions). |
| PVC size | 2Gi or 5Gi, after reading the node's volume-group free space. Expected data is tens of MiB plus WAL. |
| Extension availability | `CREATE EXTENSION unaccent` and `CREATE COLLATION hu_icu (provider = icu, locale = 'hu')` must succeed in the chosen image (ICU support compiled in) before the image is pinned. |

## Consequences

Positive:

- Loco runs on its supported path. The vendored skill, `cargo loco db create`, per-test databases and the Postgres queue all work without project overrides.
- The schema is designed once (ADR 0009). The importer is the only code that knows the Laravel shape.
- Real transactions, foreign keys, partial unique indexes (needed by soft delete, ADR 0011), `CHECK` constraints, `jsonb`, `timestamptz` (ADR 0010), `unaccent` and ICU collations are available.

Negative:

- A new stateful workload runs on a node with tight capacity. It needs a PVC, a backup entry and an upgrade path.
- backend3 cannot run next to backend1 against the same data. The switch is one rehearsed flip (ADR 0003), and nothing reaches editors before the admin is at parity.
- Search and ordering semantics change from MySQL's `utf8mb4_unicode_ci` behaviour to explicit Postgres rules. ADR 0020 restates them and tests pin them.
- The binary carries a MySQL client driver for as long as the import task exists (ADR 0017).

Follow-up work:

- website_k8s PR: the `postgres` component, Application, Image Updater entry, ExternalSecret keys, PV `Retain` patch. That PR is opened for human review, not self-merged (decision R2).
- Private runbook: back up the new database and add a restore drill.
- backend3 bootstrap PR: `config/*.yaml` with a Postgres `DATABASE_URL`, `queue.kind: Postgres`, the `unaccent` migration, and CI with a throwaway Postgres (ADR 0022).

What must be true in code and tests:

- The only database URL backend3 connects to at run time is the Postgres one. The MySQL URL is read only by the `import_mysql` task, from `IMPORT_SOURCE_DATABASE_URL`, and passed as an explicit argument to the import code; it is never a command-line argument ([data-migration.md](../data-migration.md) §4.1).
- Tests create their own databases through Loco's per-test helpers, and the test harness refuses a non-local database (ADR 0024).

## Alternatives considered

| Option | Why not |
|---|---|
| B. Keep the shared MySQL 8.4 through an app-added `sqlx-mysql` | Off Loco's supported path (no `db create`, no per-test databases, no DB queue, skill recipes need overrides). The Laravel schema could only be added to while backend1 runs. A later move to Postgres would be a second migration. |
| C. A SQLite file on a PVC | Single writer, no `unaccent` or ICU for Hungarian search and ordering, and it still needs the full data migration. |

## References

- research: DECISIONS 1 (and 2, 32a, 33 for the surrounding operations)
- research: SYNTHESIS §4.1, §4.6.1, §4.10 (R6, R24), §4.11, §4.12.3, §6.3, §6.8, §6.9, §6.24, §6.26
- research: gap-01 §2-3, gap-02 §1.1, §1.8, §4.3
- research: QUESTIONS Q1
- loco-rs@1.2.0: `src/db/connect.rs`, `src/testing/db.rs`, `src/config/queue.rs`
- ADR 0003, ADR 0008, ADR 0009, ADR 0010, ADR 0011, ADR 0016, ADR 0017, ADR 0020, ADR 0022, ADR 0023, ADR 0024
