# 0026. backend2 unchanged and running until decommissioning

- Status: Accepted (2026-10-25)
- Decision questions: none: records the maintainer's answers of 2026-10-25 (research: DECISIONS R6, R7), which change the backend2 parts of the Q2 answer (decision 2) and extend the Q41 answer (decision 41) to backend2. It amends [ADR 0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md), which records Q2 and Q41, [ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md), which records Q25 and Q31, and two consequences in [ADR 0012](0012-delivery-review-commits-and-licensing.md).

## Context

backend2 (`website_backend2`) is the public read API that the Angular site calls today. It runs 3 replicas, reads the shared MySQL database on every request, and has a Redis that holds only its persisted queries ([ADR 0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md), Context; [ADR 0016](0016-no-redis-in-memory-cache-postgres-queue-cronjob-tasks.md)).

The design accepted on 2026-10-24 changed backend2 before the cut-over and switched it off in the window:

| What was written | Where | Source |
|---|---|---|
| Merge website_backend2 #251 and #258 now, independent of the window date; the maintainer merges them | ADR 0003 §1 and §4; ADR 0012, Consequences; [cutover.md](../cutover.md) P5; [parity.md](../parity.md) PLT-29 | research: DECISIONS 2; QUESTIONS 2 A |
| Scale backend2 and its Redis to 0 replicas at T+30, after the go decision, and scale the website_frontend2 hub beta down with them | ADR 0003 §1 and §7; ADR 0019 §2; cutover.md phase 8; parity.md PLT-33 | research: QUESTIONS 2 A ("backend2 off at T+30"), the option that decision 2 chose; DECISIONS 31 |
| A rollback scales backend2 and its Redis up again before it reverts the frontend tag | ADR 0003 §8; cutover.md §9 | research: QUESTIONS 41 |
| The read-only parallel run keeps backend1 and MySQL | ADR 0003 §7 | research: DECISIONS 41 |

On 2026-10-25 the maintainer answered twice:

- Asked about the plan unit that would merge website_backend2 #251 and #258 (decision 2, PLT-29), the maintainer answered: "Why do we need backend2 changes? I do not want to touch backend2 here" (research: DECISIONS R6).
- Then the maintainer said: "So the backend2 is supposed to run until the old backend is decommissioned and backend2 is going to shut down. That was the decision orginally." Asked to confirm, the maintainer chose to keep backend2 until decommissioning: backend2 and its Redis stay up after the cut-over, reading the frozen MySQL, through the read-only parallel run, and shut down together with backend1 and MySQL when the maintainer decides to decommission (research: DECISIONS R7).

These facts bear on the change:

1. **backend3 carries both fixes itself.** Its `/api/v1` pages through posts with a keyset that returns every published post once ([public-api.md](../public-api.md) §5.3, PUB-01), and it answers `400 invalid_week` for a week that does not exist (§7.8, PUB-08). Neither depends on a change in backend2.
2. **The Angular site still leaves backend2 in the window.** The promoted REST build reads backend3 ([ADR 0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md) §1). After the flip, backend2 has three callers left: the website_frontend2 hub beta, which speaks only GraphQL to backend2 ([ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md), Context; research: QUESTIONS 31); browsers that still run the old Angular bundle from its service worker ([cutover.md](../cutover.md) open item 5); and the rollback path.
3. **backend2 only reads.** From the freeze on, MySQL refuses every write (ADR 0003, Context item 6), so backend2 serves the state of the freeze.
4. **The node is small.** The edge node has very little spare capacity, and its RAM is unknown ([ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)). Memory limits already commit 2 GiB on it ([ADR 0016](0016-no-redis-in-memory-cache-postgres-queue-cronjob-tasks.md)). backend2's three replicas at 128 MiB each and its Redis at 128 MiB are 512 MiB of that (research: SYNTHESIS §1.5.1, §8 O13). The scale-down at T+30 would have freed them for the parallel run.

## Decision

### 1. The rewrite changes nothing in website_backend2

- The rewrite opens, rebases, merges and deploys no website_backend2 PR. #251 and #258 are not merged, rebased or deployed as part of it. The rewrite does not change backend2's code or its image.
- PLT-29 is retired. Section 3 adds it to the documented exceptions of ADR 0019 §3. The PR that adds this ADR closes its issue (ADR 0019 §1, closing rule 5).
- backend3's fixed behaviours stay as they are: PUB-01, PUB-08, POS-H03 and CAN-05 do not change.
- The maintainer still reviews and merges every PR in another repository (decision R2). This project simply has no website_backend2 PR.

### 2. backend2 and its Redis keep running until decommissioning

- backend2 and its Redis are not scaled down, changed or removed in the cut-over window. Their manifests and replicas stay as they are today, and their routine image updates continue as today.
- From the freeze on, they serve the frozen MySQL data. In the window the Angular site still switches to `/api/v1`, so after the flip backend2 serves only the website_frontend2 hub beta, browsers that still run the old Angular bundle, and the rollback path.
- They stay up through the whole read-only parallel run, under the same rules as backend1 and MySQL (ADR 0003 §7). No timer, CronJob, TTL, scheduled PR or agent scales them down or deletes them, and agents never propose it. Scaling down or removing backend2 or its Redis is never the remedy for missing node capacity.
- They retire together with backend1 and MySQL when the maintainer decides to decommission (decision 41). Their manifests are removed then, before MySQL's, through human-reviewed website_k8s PRs ([cutover.md](../cutover.md) §10). PLT-33 closes with that removal.
- The website_frontend2 hub beta is scaled down when backend2 retires, as decision 31 says. That moment is now the decommissioning, not the window.

### 3. What this amends

[ADR 0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md):

| Amended text | Change |
|---|---|
| §1, bullet 1: in the window "the new admin opens and backend2 is scaled down" | The window's last step is that the new admin opens. backend2 is not scaled down. |
| §1, bullet 5: "website_backend2#251 and #258 are merged and deployed **now**, independent of the window date (decision 2)" | Replaced by section 1: the rewrite makes no change in website_backend2. |
| §1, bullet 6: the nominal sequence ends with "backend2 scaled down at T+30" | The nominal sequence ends with the new admin open at T+20. The rest of the budget is reserve for checks. |
| §4, bullet 4: "website_backend2#251 and #258 are still deployed", checked again in the pre-flight | Removed. Nothing in backend2 is checked. |
| §4, bullet 11: "Scaling down or removing backend1 or MySQL is never the remedy" | Covers backend2 and its Redis too. The capacity reading counts them as running. |
| §7, bullet 3: "backend2 and its Redis are scaled to 0 replicas in Git" | Replaced by section 2: they keep running until decommissioning. |
| §7, bullet 4: the hub beta "is scaled down when backend2 goes to 0" | It is scaled down when backend2 retires, at decommissioning (section 2). |
| §7, bullet 5: no timer, CronJob, TTL or scheduled PR "scales down or deletes backend1, MySQL, their PVCs or their Applications" | Covers backend2 and its Redis too. |
| §8, free rollback, step 1: "If backend2 is already scaled down, scale it and its Redis up. Then revert the frontend tag" | Revert the frontend tag. backend2 never stopped. |
| §8, fix forward, step 2: "Scale backend2 and its Redis up, then revert the frontend tag" | Check that backend2 still answers, because it may have run unattended for months, then revert the frontend tag ([cutover.md](../cutover.md) §9.2). |
| §10, bullet 1: the maintainer "reviews and merges every `website_k8s`, `website_frontend` and `website_backend2` PR (decision R2)" | This project has no website_backend2 PR (section 1). Decision R2 still holds for the website_k8s and website_frontend PRs. |
| Consequences, Negative: "After backend2 is scaled down, that bundle's API calls fail until the visitor accepts the update prompt" | The old bundle keeps getting answers from backend2, with the frozen data, until backend2 retires. Staging has no visitors, so the dress rehearsal does not measure how long old bundles stay; backend2's `query_req_count` during the parallel run shows it ([ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §7; [cutover.md](../cutover.md) open item 5). |
| What must be true in code, tests and manifests, last bullet: a held website_k8s PR for "backend2 and its Redis at 0 replicas" | There is no such PR. The other held PRs and their reverts stay. |

[ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md):

| Amended text | Change |
|---|---|
| §1, closing rule 2: "PLT-29 in website_backend2" as an example | PLT-29 is retired and closes by rule 5 (section 1). |
| §1, closing rule 6: "plus PLT-29 if website_backend2 #251 and #258 merge before the script first runs" | No longer applies. |
| §2, retirement criteria: backend2 and its Redis "are scaled to zero replicas inside the cut-over window, in phase 8", and only after the promoted build serves the site and the go decision is taken | They retire at decommissioning (section 2). The three criteria still decide when the window may be scheduled. PLT-33 may stay open after the window, until decommissioning. |
| §3, the table of retired features | Gains a row: the backend2 fixes #251 and #258, merged now; parity ID PLT-29; retired by this ADR; nothing replaces it in backend2, and backend3's `/api/v1` carries both fixes (PUB-01, PUB-08). |

[ADR 0012](0012-delivery-review-commits-and-licensing.md):

| Amended text | Change |
|---|---|
| Consequences, "Negative and risks", bullet 4: "website_backend2 #251 and #258 already exist, and decision 2 says to merge them now; the maintainer merges them (R2)" | This sentence no longer applies. The rest of the bullet holds. |
| Consequences, "Follow-up work", bullet 1: "The maintainer merges website_backend2 #251 and #258 now (decision 2); cutover.md P5 checks that they are deployed" | Removed. cutover.md has no P5 any more. |

The research decisions:

- **Decision 2.** Its "merge backend2 fixes #251/#258 now" is overridden (R6). The "backend2 off at T+30" step of Q2 option A, which decision 2 chose, is replaced (R7). The rest of decision 2 holds: one rehearsed flip, no GraphQL shim, nightly import rehearsals, the evening write freeze of about 1-3 h, and the Angular REST build staged ahead and promoted in the window.
- **Decision 31** holds. backend2 now retires at decommissioning, so the hub beta's scale-down happens then.
- **Decision 41** holds and now covers backend2 and its Redis: they retire together with backend1 and MySQL when the maintainer decides.

Everything else in ADR 0003, ADR 0012 and ADR 0019 holds as written. That includes the flip, the freeze, the go/no-go gate, the point of no return, one writable admin at a time, the credential rotation, and the REST-contract tests of ADR 0019 §2 that prove backend3 can replace backend2 on the public site.

## Consequences

**Positive**

- No website_backend2 PR waits for review, a rebase or its stack of base PRs before the window can be scheduled.
- The window has one step fewer. A rollback needs no scale-up: reverting the frontend tag puts the public site back on a backend2 that never stopped.
- The website_frontend2 hub beta keeps working until backend2 retires. Decision 31 still applies, at the new retirement time.
- Browsers that still run the old Angular bundle get the frozen data from backend2 instead of failed requests.
- backend1, MySQL, backend2 and its Redis retire in one decommissioning, by one decision of the maintainer.

**Negative**

- Until the flip, the live site keeps backend2's current behaviour without the two fixes that ADR 0003 Context item 10 describes. A rollback to backend2 after the flip does too. Visitors get the fixed paging and week handling from backend3's `/api/v1` at the flip, not earlier.
- After the first content change in backend3, backend2 serves older content than the public site. The hub beta and old bundles show the site as it was at the freeze.
- The edge node's capacity during the parallel run must cover backend2 and its Redis as running, next to backend1, MySQL, PostgreSQL, backend3, the media server and the frontend ([cutover.md](../cutover.md) P16). The window no longer frees their 512 MiB of memory limits.
- The old stack that runs for an open-ended time (ADR 0003, Consequences) is larger: it now includes backend2 and its Redis, which still need their routine image updates (section 2), and backend2 keeps its keys in `backend-secrets` until it retires ([ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)).
- PLT-33 stays open after the window, until the maintainer decommissions.

**Follow-up work, in the same PR as this ADR**

- [cutover.md](../cutover.md): P5 and its references go; window phase 8, "Retire the GraphQL path", goes, and the closing phase becomes phase 8; the free rollback loses its scale-up step, and the reverse move checks that backend2 still answers, then reverts only the frontend tag; the Roles row names no website_backend2 PR; P1's check names PLT-31, PLT-32 and PLT-33 as the issues that may stay open; P16 counts backend2 and its Redis as running; the parallel-run table shows them running on the frozen data; open item 5 reads backend2's counter in production instead of the dress rehearsal; the decommissioning outline retires them, and the hub beta, together with backend1 and MySQL, with MySQL removed last.
- [parity.md](../parity.md): PLT-29 becomes `retired`, citing this ADR, and PLT-33 names the new retirement time.
- The overview ([docs/design/README.md](../README.md)), [public-api.md](../public-api.md) and [data-migration.md](../data-migration.md) drop the backend2 fixes and the scale-down after the flip.
- The Status lines of ADR 0003, ADR 0012 and ADR 0019, and both ADR indexes.
- Outside this repository: the private slice plan drops the unit that merges #251 and #258 and the held website_k8s PR that sets backend2 and its Redis to 0 replicas. The private cut-over runbook has no backend2 scale-down step and no backend2 revert PR.

**What must be true**

- No website_backend3 PR, plan unit or runbook step changes website_backend2, or scales backend2 or its Redis down, before the maintainer decides to decommission.
- In the window, G13 still checks that backend2 runs ([cutover.md](../cutover.md) §7), because it is the rollback target. A reverse move checks again that backend2 answers before the frontend tag is restored (cutover.md §9.2).
- In the dress rehearsal, staging's backend2 and its Redis keep running after the promotion, and the rehearsed rollback reverts only the frontend tag.
- The smoke suite and the REST tests keep checking backend3 on its own, not against backend2 (ADR 0019 §2).

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Merge and deploy website_backend2 #251 and #258 now | QUESTIONS 2 A; DECISIONS 2 | Not chosen (maintainer's choice, R6): the rewrite does not touch backend2. backend3 carries both fixes. |
| Scale backend2 and its Redis to 0 replicas at T+30 in the window | QUESTIONS 2 A; ADR 0003 §1, §7 | Not chosen (maintainer's choice, R7): backend2 runs until the old stack is decommissioned. |
| Scale backend2 down at the end of the window or the next day, once the old service-worker bundles are measured | cutover.md open item 5 | Same answer: backend2 runs until decommissioning. |
| Keep backend2 for a fixed number of weeks after the flip | not offered; considered while writing this ADR | Retirement is the maintainer's decision, not a timer, as it is for backend1 and MySQL (ADR 0003 §7). |

## References

- research: DECISIONS 2, 31, 41, R2 (answers of 2026-10-24); R6, R7 (answers of 2026-10-25)
- research: QUESTIONS 2, 31, 41
- research: SYNTHESIS §1.5.1 (memory committed by limits), §8 O13
- Amended: [ADR 0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md) §1, §4, §7, §8, §10 and Consequences; [ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md) §1, §2, §3; [ADR 0012](0012-delivery-review-commits-and-licensing.md) Consequences
- Related: [ADR 0016](0016-no-redis-in-memory-cache-postgres-queue-cronjob-tasks.md), [ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)
- [cutover.md](../cutover.md) §3, P1, P13, P16, §6, §7, §9, §10, open items 5 and 7; [parity.md](../parity.md) PLT-29, PLT-33; [public-api.md](../public-api.md) §5.3, §7.8, §13, §14
- website_backend2#251 "fix: page through posts by date and id so backdated posts are not skipped" and website_backend2#258 "fix: report nonexistent canteen weeks as a GraphQL error" (neither merged as part of this project)
