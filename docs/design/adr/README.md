# Architecture decision records

This directory holds the architecture decisions for website_backend3. Each record (ADR) states one decision, the facts behind it, what follows from it and what else was considered. [ADR 0001](0001-record-architecture-decisions.md) defines the format, the numbering, the statuses and how a decision is changed.

ADRs 0001 to 0024 record the maintainer's answers of 2026-10-24 to the 44 design questions (research: DECISIONS, research: QUESTIONS). Where an answer overrode the recommended default, the ADR states the chosen option as the decision. The rejected default appears only under "Alternatives considered".

For the architecture as a whole, start with the [design overview](../README.md).

## Index

| ADR | Title | Status | Decision in one line |
|---|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted (2026-10-24) | Decisions are numbered ADRs in this directory; the maintainer's answers of 2026-10-24 are the source; an Accepted ADR changes only through a new ADR that a maintainer accepts. |
| [0002](0002-postgresql-as-the-database.md) | PostgreSQL as the database | Accepted (2026-10-24) | backend3 owns a new PostgreSQL Deployment and PVC with a clean schema; the MySQL data arrives through an import task. |
| [0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md) | Cut-over: one rehearsed flip, then a read-only parallel run | Accepted (2026-10-24) | Nightly import rehearsals, then one evening admin write freeze of about 1-3 h with the Angular REST build promoted in the window and no GraphQL shim; afterwards backend1 and MySQL stay up read-only until the maintainer decides to decommission them; production MySQL is not altered before the cut-over and is dumped with `--lock-all-tables`; every credential is rotated once at the start of the window, before the freeze. |
| [0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md) | Admin UI lives in admin/ and ships in the backend3 image | Accepted (2026-10-24) | The ra-core + shadcn-admin-kit UI lives in `admin/`, is built by a Node stage of the Containerfile and is served same-origin at `/admin` by a tower-http `ServeDir`; ts-rs bindings are generated into `admin/`. |
| [0005](0005-rust-dtos-are-the-api-contract-ts-rs-utoipa-openapi.md) | Rust DTOs are the API contract (ts-rs + utoipa OpenAPI) | Accepted (2026-10-24) | Rust DTOs are the source of truth; ts-rs emits the admin's TypeScript types; code-first utoipa 6 emits OpenAPI 3.1, snapshot-tested and served at `/api/v1/openapi.json`; a one-day utoipa x Loco spike comes first. |
| [0006](0006-admin-authentication-and-account-lifecycle.md) | Admin authentication and account lifecycle | Accepted (2026-10-24) | Loco JWT in an httpOnly, Secure, SameSite=Strict cookie (about 2 h, refreshed on activity) with a per-user `token_version`; Bearer for scripts; the 40 accounts are imported and rehashed from bcrypt to Argon2id at login; admin-issued reset links or temporary passwords, no SMTP; 12-character minimum; optional TOTP for everyone; active flag and last login. |
| [0007](0007-authorization-four-roles-enforced-on-every-operation.md) | Authorization: four roles, enforced on every operation | Accepted (2026-10-24) | The 4 roles and backend1's permission names are kept (`edit newsletter` retires with the newsletter) and checked on list, read, create, update and delete; new permissions `edit pages`, `edit menu` and `manage media`, plus four for the release-1 screens (16 in all); `view logs` gates the activity log and the trash that replace the retired log viewer; `manage backups` is not created, because the Backups screen is retired (research: DECISIONS 28); per-user extra grants stay; roles are defined in code; every user-management operation requires `edit users`. |
| [0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md) | Code layout: a pure src/domain/ beside Loco models | Accepted (2026-10-24) | A narrow override of Loco's doctrine: `src/domain/` is pure (no I/O, no `AppContext`, unit and property tests); external-system code takes explicit connection and config arguments; only controllers, workers, tasks and initializers touch `AppContext`; models stay fat Active Record. |
| [0009](0009-clean-domain-schema-with-preserved-integer-ids.md) | Clean domain schema with preserved integer ids | Accepted (2026-10-24) | snake_case domain tables (posts, labels, post_labels, authors, canteen_days, dishes, canteen_day_dishes, events, colleagues, pages, menu_items, roles, user_roles, user_permissions); dead tables dropped; integer identity ids preserved with sequences reset; a UUID `pid` only on users. |
| [0010](0010-all-instants-stored-as-timestamptz.md) | All instants stored as timestamptz | Accepted (2026-10-24) | Every instant is `timestamptz`, event times included; the importer converts event wall times from Europe/Budapest to UTC, and the public API renders them back as naive Europe/Budapest `YYYY-MM-DD HH:MM:SS`. |
| [0011](0011-safety-net-soft-delete-revisions-and-an-activity-log-for-eve.md) | Safety net: soft delete, revisions and an activity log for every entity | Accepted (2026-10-24) | Every admin-editable entity gets audit columns, soft delete, JSON-snapshot revisions and an append-only activity log, with self-built trash, history and audit screens written clean-room. |
| [0012](0012-delivery-review-commits-and-licensing.md) | Delivery, review, commits and licensing | Accepted (2026-10-24) | Design docs first, then a bootstrap PR, then one vertical-slice PR per entity or feature (about 500 hand-written lines); agents open and merge their own PRs here and a human reviews everything at the end; conventional commits with fixed scopes; AGPL-3.0-only, with vendored code keeping its own licence, indexed in `THIRD_PARTY.md`. |
| [0013](0013-rich-text-editor-and-html-sanitiser.md) | Rich-text editor and HTML sanitiser | Accepted (2026-10-24) | content-v1 is the one stored HTML format, defined once as a generated policy file; extended TipTap 3 with MIT packages only (tables and cell styles, alignment, YouTube, padlet, image sizes, real uploads, mandatory alt text) and no raw HTML view; Windows desktop Word and Excel paste keeps styling; pictures from outside sources are never pasted, while images copied inside the editor are; the ammonia sanitiser with a CSS grammar and a structural pass runs on every write; stored bytes are served verbatim only when their integrity tag verifies; strict image uploads and `/storage` response headers; revisions and previews are never rendered raw; the editor is styled like website_frontend2's PageRenderer, with an Angular preview; the import normalises every body to content-v1. |
| [0014](0014-one-host-backend3-microshift-verseghy-gimnazium-net.md) | One host: backend3.microshift.verseghy-gimnazium.net | Accepted (2026-10-24) | `backend3.microshift.verseghy-gimnazium.net` serves `/admin`, `/api/admin` and `/api/v1`. |
| [0015](0015-media-separate-static-server-media-library-image-optimisatio.md) | Media: separate static server, media library, image optimisation | Accepted (2026-10-24) | An unprivileged `ubi9/nginx-124` serves `/storage` on both public hosts (and on the backend3 host for the admin) from the existing uploads volume, which backend3 co-mounts for uploads (if the co-mount test fails, backend3 serves `/storage` itself at 1 replica); a minimal media library with a MIME allow-list and a limit of about 128 MiB; images get a visually lossless re-encode and metadata strip, never a resize. |
| [0016](0016-no-redis-in-memory-cache-postgres-queue-cronjob-tasks.md) | No Redis: in-memory cache, Postgres queue, CronJob tasks | Accepted (2026-10-24) | Loco's in-memory cache, Loco's Postgres queue for durable jobs, and scheduled work as Kubernetes CronJobs that run backend3 tasks. |
| [0017](0017-legacy-data-import-normalisation-and-defect-resolution.md) | Legacy data import: normalisation and defect resolution | Accepted (2026-10-24) | An idempotent import that normalises stored HTML to the editor's fixed point (with a dry-run diff and the original kept as a revision) and applies an editor-signed defect resolution list before constraints are switched on. |
| [0018](0018-content-model-publishing-pages-menu-tree-canteen-week-editor.md) | Content model: publishing, pages, menu tree, canteen week editor | Accepted (2026-10-24) | Draft/published posts with fixes; one page type with a published/hidden flag; a clean-room drag-and-drop tree for the two-level menu that keeps today's order; a canteen week editor beside the single-day form. |
| [0019](0019-scope-parity-tracking-retired-features-release-1-additions.md) | Scope: parity tracking, retired features, release-1 additions | Accepted (2026-10-24) | One GitHub issue per parity ID; backend2's retirement proven by new tests written from the REST spec; Logs, Backups and newsletter retired as documented exceptions; sitemap kept; notices, partner banners, site settings and Diáktoll issues added; website_frontend2 out of scope; the dashboard shows usage statistics and self-hosted API read counts. |
| [0020](0020-search-and-hungarian-ordering.md) | Search and Hungarian ordering | Accepted (2026-10-24) | A derived plain-text column maintained on write, matched with `unaccent(lower(...)) LIKE` and escaped wildcards; explicit ICU `hu` collation for name ordering, with "Dr." stripped for colleagues. |
| [0021](0021-hungarian-only-admin-ui.md) | Hungarian-only admin UI | Accepted (2026-10-24) | One complete Hungarian message file in the repository; English only as a missing-key fallback. |
| [0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) | Builds and CI: PAC for production images, GitHub Actions for PR checks | Accepted (2026-10-24) | Production images come only from Pipelines-as-Code and Shipwright on the hub (larger build step, explicit nested timeouts, master only); GitHub Actions runs the PR checks. |
| [0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) | Operations: hardening, rate limits, monitoring, backups, secrets | Accepted (2026-10-24) | The admin stays internet-reachable, hardened in code and by router-level limits from day one; `/metrics` is exposed and monitoring runs only on the hub; the existing backup process stays as it is, and only the new PostgreSQL database and PVCs are added to it; backend3's keys live in the shared `backend-secrets` ExternalSecret. |
| [0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) | AI maintainability: instruction files, the Loco skill, guardrails | Accepted (2026-10-24) | Tracked `AGENTS.md` and `CLAUDE.md` files, the Loco skill vendored verbatim from the exactly pinned `loco-rs` 1.2.0 with a CI equality check, and committed deny rules plus a PreToolUse hook against destructive or non-local commands. |

If a status in this table and the Status line of the ADR itself disagree, the ADR file is right and this table is a bug.

## Adding an ADR

1. Take the next free number. Numbers are never reused.
2. Name the file `NNNN-<slug>.md`, where the slug is the lower-case title with every run of characters other than `a-z` and `0-9` turned into one hyphen, trimmed, and cut to 60 characters ([ADR 0001](0001-record-architecture-decisions.md#3-numbers-and-file-names)).
3. Copy the template below. Start with `Status: Proposed`.
4. Add a row to the index above and to the index in the [design overview](../README.md#architecture-decision-records), in the same PR.
5. A maintainer's answer moves the ADR to Accepted or Rejected. Agents do not accept their own proposals.

## Changing a decision

Do not rewrite an Accepted ADR. Write a new ADR that quotes the maintainer's answer and names the ADR it supersedes or amends. Then change only the old ADR's Status line to `Superseded by NNNN (date)` or `Amended by NNNN (date)`, with a link. Typos, broken links and corrections where an ADR misstates the answer it records are fixed in place. The full rules are in [ADR 0001](0001-record-architecture-decisions.md#6-changing-a-decision).

## Template

```markdown
# NNNN. Title as a short noun phrase

- Status: Proposed (YYYY-MM-DD)
- Decision questions: Qn, Qm (or "none: records the maintainer's answer of YYYY-MM-DD on ...")

## Context

The facts that force the decision, with numbers. Give each fact a source:
"research: SYNTHESIS §4.12", "research: gap-02 §4.5", "research: DECISIONS 16",
or public code as "website_backend@0dead98:app/Models/Posts.php".
State what is still unknown and how it will be settled.

## Decision

What we do, precise enough to implement: names, settings, limits, defaults.
Name exact Loco, Sea-ORM or ra-core APIs only when they have been verified;
otherwise describe the behaviour.

## Consequences

- Positive:
- Negative:
- Follow-up work:
- What must be true in code and tests:

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| ... | QUESTIONS n X | ... |

## References

- research: DECISIONS n
- research: QUESTIONS n
- Public code, PRs and documentation
```

Before you merge, check the text against the public-safety rules in [ADR 0001](0001-record-architecture-decisions.md#8-what-a-public-adr-may-contain): no secrets, no live weaknesses of the running systems, no personal data, no session links.
