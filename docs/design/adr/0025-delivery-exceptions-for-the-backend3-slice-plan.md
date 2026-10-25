# 0025. Delivery exceptions for the backend3 slice plan

- Status: Accepted (2026-10-25)
- Decision questions: none: amends [ADR 0012](0012-delivery-review-commits-and-licensing.md) §1, which records Q12, and one sentence of [ADR 0005](0005-rust-dtos-are-the-api-contract-ts-rs-utoipa-openapi.md) §5, which records Q4. It records the maintainer's answer of 2026-10-25 to this proposal (research: DECISIONS R5).

## Context

The maintainer's answer to Q12 sets the delivery shape: "one vertical-slice PR per entity/feature (~500 hand-written lines, no deep stacks)" (research: DECISIONS 12). [ADR 0012](0012-delivery-review-commits-and-licensing.md) §1 turns that into rules. Four of them, and one sentence of [ADR 0005](0005-rust-dtos-are-the-api-contract-ts-rs-utoipa-openapi.md), are what this ADR proposes to amend:

| Rule | Where | What it says |
|---|---|---|
| Where an amending ADR lands | ADR 0012 §1 item 1 | A later change adds the amending ADR "in the same PR as the code that needs it". That PR merges only after a maintainer accepts the new ADR. |
| The slice shape | ADR 0012 §1 item 4 | One PR per entity or feature group. A slice carries one feature end to end: the migration and the model, the admin DTOs and handlers plus the public endpoint, the admin screen, the importer step for its tables, tests at each layer, the doc updates and its `Closes #N` lines. |
| The size | ADR 0012 §1 item 5 | About 500 hand-written lines or fewer. A feature that needs more is split into smaller vertical slices by feature, and never split by layer. |
| The order | ADR 0012 §1, last paragraph | Authentication, users and roles first; then labels and authors, which settle the shared patterns; then the other entities. "Each entity's public endpoint ships in the same slice as the entity." The project board can change the order. |
| The spike endpoints | ADR 0005 §5 | The spike PR removes its two endpoints before it merges, "and the first `/api/v1` slice adds them again with full tests". |

The slice plan of 2026-10-25 (research: slice plan) breaks the remaining work into slices. Measured against the bootstrap code, items 4 and 5 cannot both hold for most entities:

| Fact | Value | Source |
|---|---|---|
| A pure module is mostly tests | Under the project's testing bar, a `src/domain/` module carries unit tests and proptests. `src/domain/admin_path.rs` is 375 lines: 135 of code and 240 of tests. `src/domain/database_target.rs` is 727 lines: 338 and 389. Item 5 counts tests as hand-written lines. | `website_backend3@4e7fb76:src/domain/admin_path.rs`, `src/domain/database_target.rs` |
| One small route end to end | The JSON 404 answer for unknown API paths reads no data, yet it takes about 544 lines in four files: `src/controllers/not_found.rs` (150), `src/dtos/admin/error.rs` (158), `src/domain/api_path.rs` (121) and `tests/requests/not_found.rs` (115). More than half of them are tests. | `website_backend3@4e7fb76` |
| Legacy size is no guide | backend1's whole labels CRUD is 166 lines: controller 81, request 54, model 31. The backend3 feature adds the safety net ([ADR 0011](0011-safety-net-soft-delete-revisions-and-an-activity-log-for-eve.md)), search ([ADR 0020](0020-search-and-hungarian-ordering.md)), a Hungarian screen and tests at every layer. | `website_backend@0dead98:app/Http/Controllers/Admin/LabelsCrudController.php`, `app/Http/Requests/LabelsRequest.php`, `app/Models/Posts/Labels.php` |
| Even the first draft was twice the limit | The plan's first draft had 119 slices, estimated at about 420 hand-written lines each. It already kept most of the shared libraries, the public endpoints and the importer steps in slices of their own. Counted bottom-up against the bootstrap, its slices came to about 1,000 hand-written lines each. Slices that carry every layer of item 4 would be larger still. | research: slice plan, first draft and first critique round |
| The plan that keeps the limit | 223 website_backend3 slices, about 97,000 hand-written lines, averaging about 435 and at most about 560. 116 of them are named in section 3. The estimates are re-measured after the first three slices merge. | research: slice plan |
| Shared parts have no screen | The activity log and revisions ([ADR 0011](0011-safety-net-soft-delete-revisions-and-an-activity-log-for-eve.md)), the sanitiser ([ADR 0013](0013-rich-text-editor-and-html-sanitiser.md)), the image optimiser ([ADR 0015](0015-media-separate-static-server-media-library-image-optimisatio.md)), the importer framework and the normaliser ([ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md)) and the `/api/v1` conventions ([public-api.md](../public-api.md)) serve several entities. Item 4 has room for them only inside the first entity that needs them, which would put that slice far over the limit. | the ADRs named; research: slice plan |
| The users screens reuse the labels contracts | The users list, forms and trash use the list, search, write and trash contracts that the labels slices settle. Built first, users would settle those contracts on the entity with the most guards (roles, extra grants, the ADR 0007 §6 guards), or build them twice. | [ADR 0007](0007-authorization-four-roles-enforced-on-every-operation.md) §6; research: slice plan |
| The first `/api/v1` slice has no posts | In the plan, the first `/api/v1` slice is the router, CORS and error-schema slice. `GET /api/v1/posts/{id}` needs the posts schema, the content-serving rule of ADR 0013 and the public test fixtures, which land many slices later. | research: slice plan; [public-api.md](../public-api.md) §7.1, §7.3 |

The plan keeps what the Q12 answer is about: about 500 hand-written lines per PR, no deep stacks, and no PR that layers several entities. It departs from item 4's "every layer in one PR" in named, bounded ways, and from the order paragraph in two places.

An agent may not read those departures into an Accepted ADR. One that finds an Accepted decision unworkable writes a Proposed ADR with the evidence and asks the maintainer ([ADR 0001](0001-record-architecture-decisions.md) §6). The plan's critique routed every departure here, so the maintainer answers once for the whole plan instead of slice by slice. ADR 0012 §1 item 1 places an amending ADR in the PR of the code that needs it. This ADR is a PR of its own, because it shapes the whole plan rather than the code of one slice, so section 1 asks the maintainer to accept that departure as well.

The maintainer's answer, on 2026-10-25: "Accept as planned (Recommended)" (research: DECISIONS R5). The question named the six slice classes, the users screens after the labels slices, and the resulting plan of 223 PRs of about 500 hand-written lines each.

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0012 §1 item 1: the amending ADR goes "in the same PR as the code that needs it" | For this ADR only: it lands in a docs-only PR of its own, held until the maintainer answers. On rejection that PR merges with the ADR Rejected, which changes no decision ([ADR 0001](0001-record-architecture-decisions.md) §4). Every other amending ADR, including the ones that later slices carry, still lands with the code that needs it. | 6 |
| ADR 0012 §1 item 4: one PR per entity or feature group, carrying every layer | The slices named in section 3 may carry part of a feature instead of every layer of it. | 2, 3 |
| ADR 0012 §1 item 5: "never split by layer" | The shapes in section 3 stay within one entity or one shared component, so they are not the banned layer split. Section 2 rule 2 names the only two slices that write several tables. A layer split across entities stays banned. The limit of about 500 hand-written lines and its counting rule do not change. | 2 |
| ADR 0012 §1, the order paragraph | The users screens follow the labels slices. Each entity's public endpoints are slices of their own. | 4 |
| ADR 0005 §5: "the first `/api/v1` slice adds them again with full tests" | Two named posts slices re-add the endpoints. | 5 |

Everything else in both ADRs holds as written. That includes the rest of ADR 0012 §1 item 1 (the PR that needs an amending ADR merges only after a maintainer accepts it), items 2, 3, 6 (branch from `master`, at most one open base PR) and 7 (drafts), ADR 0012 §2-§4, and ADR 0005's spike scope, pass criteria and failure path.

### 2. Rules for every slice in section 3

1. **Size.** About 500 hand-written lines or fewer, counted as ADR 0012 §1 item 5 counts them.
2. **One entity or one shared component.** No class batches several entities into one PR. No PR carries the migrations, the endpoints, the screens or the importer steps of several entities. An entity's link tables belong to it, for example `post_labels` to posts and `user_roles` to users. Two class (d) slices write several tables by design, and this ADR accepts them by name:
   - `canteen-import-days` imports dishes, canteen days and the links between them. They are one canteen workflow: one permission, `edit canteens`, covers dishes and days, and the week editor of [ADR 0018](0018-content-model-publishing-pages-menu-tree-canteen-week-editor.md) §4 writes the days and their links and creates dishes inline. The links cannot be imported without both tables.
   - `release1-seed` is one seeding task. It writes the reviewed release-1 rows that have no legacy source: the partner banners, the site settings and one Diáktoll issue. Each of those entities gets its schema, its screens and its public endpoints in earlier slices of its own chain; the seed adds only rows.
3. **Consumed.** The next slice of its own chain depends on it and uses what it adds. A chain is the run of slices that one entity or one shared component goes through in the plan, linked by their dependencies; for example `auth-users`, `auth-login-api`, `auth-session-me`, then `admin-sign-in`. A test-only slice, or the last slice of a chain, is consumed by the gate that runs it: a CI job, the nightly rehearsal ([ADR 0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md)), the cut-over runbook or the final human review (ADR 0012 §2).
4. **Complete for what it carries.** It is not a stub. It has tests at every layer it touches, as ADR 0012 §1 item 4 lists them: unit tests and proptests for `src/domain/` ([ADR 0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md)), request tests for endpoints, admin unit tests and an E2E test for a screen, and importer tests for a step. It updates the docs it changes.
5. **Closes lines.** It carries `Closes #N` only for a parity issue it finishes ([ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md) §1). A slice that finishes none carries none.

### 3. The accepted classes

The slices are named by their keys in the plan. Every class keeps the aim of item 4: each of its slices belongs to one chain, and the chain as a whole still delivers the entity or feature end to end.

| Class | What it is | Slices |
|---|---|---|
| (a) Library and shared infrastructure | Pure libraries or shared infrastructure with no screen. | `task-lock`, `app-clock`, `metrics-endpoint`, `metrics-runtime`, `openapi-spike`, `budapest-time`, `date-newtypes`, `totp-crypto`, `schema-snapshot`, `activity-log-table`, `revisions-core`, `image-sniff-budget`, `image-optimise-jpeg`, `image-optimise-png`, `image-optimise-webp-gif`, `media-upload-core`, `owned-files-worker`, `media-nginx-contract`, `media-fallback-router`, `sanitizer-policy-core`, `sanitizer-structural-pass`, `sanitizer-css-values`, `sanitizer-css-canonical`, `rich-text-manifest`, `rich-text-validate-search`, `content-write-service`, `content-integrity-mac`, `content-serve`, `v1-router-cors`, `v1-etag-params`, `v1-test-harness` |
| (b) Server-first pairs | The server half merges before the screen that uses it, because the server half alone is about 500 lines. In the plan, each screen depends on its server half, directly or through its chain, so the order holds when a slice starts as soon as its own dependencies have merged. The pairs are listed as server half, then screen. | `auth-users`, `auth-login-api` and `auth-session-me`, then `admin-sign-in`; `labels-list-api`, then `labels-list-screen`; `labels-write-contract`, then `labels-forms`; `trash-convention`, then `trash-ui`; `posts-schema`, then `posts-list`; `login-throttle-account`, `login-throttle-address` and `login-throttle-ops`, then `users-unlock`; `activity-sign-in-entries`, then `activity-log-screen` |
| (c) Public endpoints | An entity's public endpoints are slices of their own in that entity's chain, right after its core slices. | `public-post-detail`, `public-post-preview`, `public-posts-list`, `public-archive`, `public-search`, `public-authors`, `public-labels`, `public-caps-pin`, `pages-public`, `menu-public`, `colleagues-public`, `canteen-public`, `canteen-public-current`, `events-public`, `notices-public`, `banners-public`, `diaktoll-public`, `site-settings-public` |
| (d) Importer steps | The importer framework comes first. Each importer step is a slice of its own in the entity's chain, after the entity's revision DTO exists ([ADR 0011](0011-safety-net-soft-delete-revisions-and-an-activity-log-for-eve.md) §4, [ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md)). The nightly rehearsals still grow with each step, which is item 4's reason for the importer step. | Framework: `import-task-args`, `import-mysql-ci`, `import-source-session`, `import-resolution-schema`, `import-defect-detector`. Steps: `labels-import`, `labels-d8-merge`, `authors-import`, `import-users`, `posts-import-rows`, `posts-import-dates`, `posts-import-bodies`, `posts-import-defects`, `pages-import`, `menu-import`, `colleagues-import`, `canteen-import-days`, `canteen-import-d6-d7`, `events-import`, `release1-seed`. Normaliser: `normaliser-core`, `normaliser-marks`, `normaliser-blocks`, `normaliser-images`, `normaliser-embeds`, `normaliser-tables`, `normaliser-inline-images`, `normaliser-n7-plan`, `content-normalize-task` |
| (e) Editor and paste libraries | Editor and paste library slices built ahead of the rich-text input that consumes them ([ADR 0013](0013-rich-text-editor-and-html-sanitiser.md), [editor.md](../editor.md)). | `clipboard-recorder`, `editor-schema`, `editor-serializer`, `editor-tables`, `editor-images-embeds`, `paste-generic`, `paste-pictures`, `paste-word-lists`, `paste-word-runs`, `paste-excel-borders` |
| (f) Test-only slices | Conformance and verification slices that add tests and checks, not features. | `safety-net-conformance`, `form-conventions-conformance`, `read-only-db-test`, `rich-text-sink-chain`, `editor-corpus-job`, `smoke-suite`, `import-verify-digests`, `import-verify-invariants`, `import-verify-bodies`, `import-verify-api` |

### 4. Order

- **Authentication and authorization still come first:** the accounts table, sign-in, the session, the roles and the route table.
- **The users screens follow the labels slices.** `users-list`, `users-create-edit`, `users-roles-grants`, `users-lifecycle`, `users-unlock`, `users-trash` and `users-anonymise`, and the read-only roles and permissions screens (`rbac-lists`), come after the labels and trash slices that settle the contracts they reuse: `users-list` after `labels-search`, `users-create-edit` after `labels-forms`, `users-trash` after `global-trash`, and `rbac-lists` after `labels-list-screen`. ADR 0012 §1 lets the project board change the order. This ADR records the change so that it is not "corrected" back.
- **Public endpoints get their own slices.** Each entity's public endpoints are class (c) slices in that entity's chain, right after its core slices. This replaces the sentence "Each entity's public endpoint ships in the same slice as the entity."
- The rest of the paragraph's order holds as a dependency order.

### 5. The spike endpoints of ADR 0005 §5

- The spike PR (`openapi-spike`) removes both spike endpoints before it merges, as ADR 0005 §5 says.
- `public-post-detail` re-adds `GET /api/v1/posts/{id}` for published posts, with the full tests of [public-api.md](../public-api.md) §7.3. `public-post-preview` then adds its optional preview token.
- `public-posts-list` re-adds `GET /api/v1/posts`, with the cursor and the full tests of [public-api.md](../public-api.md) §7.1.
- `v1-router-cors`, the first `/api/v1` slice, re-adds neither.
- Each re-added endpoint merges with its response-shape test, as ADR 0005 already requires.

### 6. Answering this proposal

- **The PR is held.** The PR that adds this ADR carries `/hold` until the maintainer answers. It is not merged as Proposed, because every slice named in section 3, and every other slice that adds or changes a feature, depends on the answer, directly or through its chain. Draft PRs of dependent slices may be prepared meanwhile, but none is marked ready. Only docs-only PRs that record a check or a clean-up, and `ci-postgres-align`, a build-configuration change that aligns the CI and compose PostgreSQL image with production, do not wait for the answer.
- **On acceptance,** the agent sets this ADR's Status to `Accepted (date)` and quotes the answer with its date in the Context. It sets the Status lines of ADR 0012 and ADR 0005 to `Amended by 0025 (date)`, with a link to this ADR, updates both indexes, and merges.
- **On rejection,** the agent sets `Rejected (date)`, records the answer, and merges, so that the reasoning stays ([ADR 0001](0001-record-architecture-decisions.md) §4). The plan is then regrouped before any dependent slice starts.

## Consequences

**Positive**

- Every PR stays near 500 hand-written lines, so each self-merged PR (ADR 0012 §2) stays small enough to test, revert and read in the final review.
- Shared parts are built and tested once, before the entities that use them. They are not grown inside the first entity slice and retrofitted later.
- The users screens reuse list, search, write and trash contracts that labels have already settled.
- One answer settles the shape of the whole plan. No slice waits for an ADR of its own.

**Negative**

- There are about twice as many PRs as in the plan's first draft (223 instead of 119), and more still than whole-entity slices would need: more Tide cycles, more rebases, and longer chains on the critical path.
- A part merges before the slice that uses it. If a chain stops, its merged parts are unused code until the plan is regrouped. Rule 3 of section 2 and the final human review guard against this.
- For a while, server halves expose endpoints that no screen calls yet. Their request tests are what exercises them.
- The nightly rehearsal covers an entity once its importer step merges, not when its schema does. Until then the rehearsal leaves that entity's tables empty.
- The lists are closed. A slice that is not listed, and that is not a split by feature of a listed slice, needs its own amending ADR before it takes one of these shapes.

**Follow-up work**

- The maintainer answers; the agent then follows section 6.
- In the acceptance update, the "Shape" rule in `CLAUDE.md`, recipe 1 of the backend3 skill and the "Slice" entry of the overview's glossary each gain a pointer to this ADR, so agents read the accepted classes there too. They are not changed while this ADR is Proposed: code must not depend on a Proposed ADR (ADR 0001 §4), and agents follow those files as rules.
- The estimates are re-measured after the first three slices merge. A listed slice that is split further by feature keeps its class, and every part meets section 2.

**What must be true in code and tests**

- Every slice in section 3 meets the rules of section 2. ADR 0012 asks every PR body to name the ADRs it implements; an exception slice also names its class and the slice or gate that consumes it.
- In the plan, each class (b) screen depends on its server half, directly or through its chain.
- `v1-router-cors` adds no posts route. The route-coverage test of ADR 0005 §4 shows which `/api/v1` routes exist after each merge.
- No slice named in section 3, and no other slice that adds or changes a feature, is marked ready before this ADR is Accepted.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Keep ADR 0012 §1 items 4 and 5 as written | ADR 0012 | Even the first draft, which already kept most shared, public and importer work in slices of their own, came to about 1,000 hand-written lines per slice. Whole-entity slices would be larger still, so most PRs would break item 5. |
| Keep whole-entity slices and raise the limit to about 1,000 lines | not offered; considered while writing this ADR | The Q12 answer names about 500 lines (research: DECISIONS 12), and doubling it changes the answer itself. A limit of about 1,000 would still fit only the first draft's slices, not whole-entity slices, and every self-merged PR would be at least twice as large. |
| Layered PRs across entities (all schema, then all endpoints, then all screens) | QUESTIONS 12 C | No layer PR can be tested end to end. Section 2 rule 2 keeps this ban. |
| Read the exceptions into ADR 0012 without a new ADR | not offered; considered while writing this ADR | Items 4 and 5 and the order paragraph are explicit, and only a maintainer's answer changes an Accepted ADR (ADR 0001 §6). |
| One amending ADR per slice, when the slice arrives | not offered; considered while writing this ADR | 116 slices would each wait for an answer, and every chain would stall on the first one. |
| Re-add the spike endpoints in `v1-router-cors` | ADR 0005 §5 | The posts schema, the content-serving rule and the public fixtures do not exist yet. The router slice would have to pull in posts work or carry a throwaway endpoint. |
| Build the users screens first, as the order paragraph says | ADR 0012 §1 | The users screens would settle the shared contracts on the entity with the most guards, or the contracts would be built twice. |

## References

- research: DECISIONS 12, 13 (answers of 2026-10-24)
- research: QUESTIONS 12
- research: slice plan (2026-10-25), with its first draft and its two critique rounds, in the private `Verseghy/runbooks` repository under `verseghy-website-rewrite/`
- `website_backend3@4e7fb76`: `src/domain/admin_path.rs`, `src/domain/database_target.rs`, `src/controllers/not_found.rs`, `src/dtos/admin/error.rs`, `src/domain/api_path.rs`, `tests/requests/not_found.rs` (the bootstrap, `Verseghy/website_backend3#186`)
- `website_backend@0dead98`: `app/Http/Controllers/Admin/LabelsCrudController.php`, `app/Http/Requests/LabelsRequest.php`, `app/Models/Posts/Labels.php`
- [ADR 0001](0001-record-architecture-decisions.md) §4, §6; [ADR 0005](0005-rust-dtos-are-the-api-contract-ts-rs-utoipa-openapi.md) §4, §5; [ADR 0012](0012-delivery-review-commits-and-licensing.md) §1, §2; [ADR 0018](0018-content-model-publishing-pages-menu-tree-canteen-week-editor.md) §4; [ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md) §1
- Related: [ADR 0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md), [ADR 0007](0007-authorization-four-roles-enforced-on-every-operation.md), [ADR 0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md), [ADR 0011](0011-safety-net-soft-delete-revisions-and-an-activity-log-for-eve.md), [ADR 0013](0013-rich-text-editor-and-html-sanitiser.md), [ADR 0015](0015-media-separate-static-server-media-library-image-optimisatio.md), [ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md), [ADR 0020](0020-search-and-hungarian-ordering.md); [public-api.md](../public-api.md), [editor.md](../editor.md)
