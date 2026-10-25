# 0032. backend1's public REST API retired

- Status: Accepted (2026-10-25)
- Decision questions: none: records the maintainer's answer of 2026-10-25 to D40 of the decision register (research: DECISIONS R9). It amends §3 of [ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md), the table of retired features, which records Q28 and Q30.

## Context

Besides its admin, backend1 (`website_backend`) has a public JSON API. `website_backend@0dead98:routes/api.php` defines these routes, all for `GET`:

| Path | Routes |
|---|---|
| `/api/posts/` | `getPost`, `getPostsByAuthor`, `getPostsByLabel`, `listPosts`, `search`, `listFeaturedPosts`, `getPreview`, `getPostsByYearMonth`, `getCountByMonth` |
| `/api/events/` | `getEventsByMonth` |
| `/api/canteen/` | `getCanteenMenus`, `getCanteenByWeek` |
| `/api/colleagues/` | `listColleagues` |
| `/api/pages/` | `getPageBySlug` |
| `/api/menu/` | `getMenuItems` |
| `/api/newsletter/` | `subscribe`, `unsubscribe` |
| `/api/user` | Laravel's scaffold route for the signed-in API user (`auth:api` middleware) |

| Fact | Value | Source |
|---|---|---|
| No client calls it | The Angular site stopped calling it on 2022-02-26, when it moved to backend2's GraphQL API (`website_frontend@5ea40fd`, `Verseghy/website_frontend#1014`). No other consumer was found. | research: SYNTHESIS §4.8, §5.15.2 |
| Its newsletter routes are already retired | NWS-02 in [parity.md](../parity.md) retires the subscribe and unsubscribe endpoints, and ADR 0019 §3 lists them. | ADR 0019 §3 |
| The other routes had no row | parity.md's PUB rows track the backend2 reads that the Angular site depends on, and its backend1 rows track admin features. So the read routes and `/api/user` had no fate in the list against which the window criteria of ADR 0019 §2 are judged. | [parity.md](../parity.md) §18; research: SYNTHESIS §5.16 |
| What the public site reads after the switch | backend3 serves one public read API, `/api/v1` ([public-api.md](../public-api.md)). The first `/api/v1` slice is `v1-router-cors` ([ADR 0025](0025-delivery-exceptions-for-the-backend3-slice-plan.md) §3 and §5). | [ADR 0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md) §1 |

On 2026-10-25 the maintainer answered D40 of the decision register, "Parity row for backend1's retired public REST API" (research: DECISIONS R9). It asked whether a parity row should record backend1's public read REST API (`routes/api.php`) as retired, through an amendment of ADR 0019 §3. The answer: "Add a retired row". The consequence the option stated: "An amending ADR, a parity issue and a row, closed by v1-router-cors's 404 tests; the final review sees the 404s as intended."

## Decision

### 1. backend3 does not port backend1's public REST API

- **Nothing is ported.** backend3 mounts none of the routes of `routes/api.php`, under their old paths or as aliases of `/api/v1`. Its only public read API is `/api/v1` ([public-api.md](../public-api.md)), which the Angular site reads after the switch (ADR 0003 §1). A request for one of the old paths answers `404`.
- **PUB-14 records it.** [parity.md](../parity.md) §18 gains PUB-14, backend1's public REST API, with the disposition `retired`, citing this ADR. Its newsletter routes stay under NWS-02.
- **How PUB-14 closes.** By ADR 0019 §1, closing rule 5, the PR that implements the retirement closes it. That is `v1-router-cors`: its request tests pin a `404` for a `GET` of every route of `routes/api.php`, the newsletter routes included, and its PR body closes PUB-14's issue next to NWS-02's. This ADR is the documented exception that the closing PR needs (ADR 0019 §1, the `retired` row of the table); it is Accepted before that PR opens.
- **backend1 is not changed.** This ADR changes nothing in website_backend. backend1 stays up read-only until the maintainer decommissions it (ADR 0003 §7).

### 2. What this amends

[ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md):

| Amended text | Change |
|---|---|
| §3, the table of retired features | Gains a row. Feature: backend1's public REST API (`routes/api.php`), apart from its newsletter routes, which the newsletter row covers. Parity ID: PUB-14. Decision: retired; none of its routes is ported. What replaces it: nothing in that shape; the public site reads `/api/v1` (PUB-01 to PUB-13). |

Everything else in ADR 0019 holds, with the amendments of [ADR 0026](0026-backend2-unchanged-and-running-until-decommissioning.md). That includes §2: its first retirement criterion already requires every parity issue to be done before the window may be scheduled, PUB-14 included, and PUB-14 adds no read that the promoted Angular build calls.

## Consequences

**Positive**

- Every public surface of backend1 and backend2 has an explicit fate in [parity.md](../parity.md).
- The final review (PLT-36) sees the `404`s on these paths as intended, not as missing features.

**Negative**

- A client that still called these routes would get `404` from backend3. None is known since 2022-02-26.
- One more parity issue to track.

**Follow-up work, in the same PR as this ADR**

- [parity.md](../parity.md): the PUB-14 row in §18, and the Counts table (14 PUB IDs, 184 in all). The parity-sync test reads the expected number of IDs from that table.
- The Status line of ADR 0019, and both ADR indexes.

**Follow-up work after it merges**

- `scripts/sync-parity-issues.sh` creates PUB-14's issue (ADR 0019 §1).
- Outside this repository: the private slice plan adds PUB-14 to the issues that `v1-router-cors` closes.

**What must be true**

- backend3 mounts no route of `routes/api.php`.
- Request tests pin a `404` for a `GET` of every route of `website_backend@0dead98:routes/api.php`.
- PUB-14's issue closes only with that PR, never by hand (ADR 0019 §1, closing rule 5).

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| No row | DECISIONS R9 (D40, option 2) | Not chosen (maintainer's choice, R9). Only `v1-router-cors`'s tests and its PR body would record the retirement, and the parity list would leave a legacy public surface without a fate. |
| Port the read routes, or alias them to `/api/v1` | not offered; considered while writing this ADR | No client has called them since 2022, and a second public read API would need its own contract and tests. |
| Fold the read routes into NWS-02 | not offered; considered while writing this ADR | NWS-02 is the newsletter's row. The read routes are not newsletter features, and an ID keeps its meaning (ADR 0019 §1). |

## References

- research: DECISIONS R9 (the decision register's answer of 2026-10-25 to D40)
- research: SYNTHESIS §4.8, §5.15.2, §5.16
- `website_backend@0dead98:routes/api.php`; `website_frontend@5ea40fd` (`Verseghy/website_frontend#1014`)
- Amended: [ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md) §3
- Related: [ADR 0001](0001-record-architecture-decisions.md) §6; [ADR 0003](0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md) §1, §7; [ADR 0025](0025-delivery-exceptions-for-the-backend3-slice-plan.md) §3, §5; [ADR 0026](0026-backend2-unchanged-and-running-until-decommissioning.md), which retired PLT-29 the same way
- [parity.md](../parity.md) PUB-14, NWS-02; [public-api.md](../public-api.md)
