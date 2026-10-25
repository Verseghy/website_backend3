# 0031. Slice size as guidance; slices shipped as built

- Status: Accepted (2026-10-25)
- Decision questions: none: records the maintainer's answers of 2026-10-25 on how the slice size rule counts (research: DECISIONS R10) and on who may ask for a split, where this ADR lands and the follow-up to re-measure the estimates (research: DECISIONS R15 (D76)). It amends [ADR 0012](0012-delivery-review-commits-and-licensing.md) §1 item 5, and §1 item 1 for this ADR only, which record Q12, and five parts of [ADR 0025](0025-delivery-exceptions-for-the-backend3-slice-plan.md), which records the answer R5: §2 rule 1, the §1 rows for ADR 0012 §1 items 5 and 1, the first positive consequence, and the follow-up to re-measure the estimates.

## Context

The maintainer's answer to Q12 sets the delivery shape: "one vertical-slice PR per entity/feature (~500 hand-written lines, no deep stacks)" (research: DECISIONS 12). Two rules carry the figure:

| Rule | Where | What it says |
|---|---|---|
| The size | [ADR 0012](0012-delivery-review-commits-and-licensing.md) §1 item 5 | "A slice aims for about 500 hand-written lines or fewer (added plus changed)." Generated files, lock files and vendored files do not count. "A feature that needs more is split into smaller vertical slices by feature", and never by layer. |
| The size of the planned slices | [ADR 0025](0025-delivery-exceptions-for-the-backend3-slice-plan.md) §2 rule 1 | Every slice of the six classes is "about 500 hand-written lines or fewer, counted as ADR 0012 §1 item 5 counts them". ADR 0025 §1 adds: "The limit of about 500 hand-written lines and its counting rule do not change." |

ADR 0025 records a plan of 223 slices, estimated at about 435 hand-written lines each on average, and a follow-up: "The estimates are re-measured after the first three slices merge." The plan layers its slices into waves by their dependencies, so the slices of one wave depend only on slices of earlier waves. Wave 2 was the first wave of code slices built under it. Nine of its website_backend3 slices opened as draft PRs on 2026-10-25, each built to the full testing bar. Their bodies report these sizes, counted by the rule of ADR 0012 §1 item 5:

| PR | Slice | Hand-written lines |
|---|---|---|
| `Verseghy/website_backend3#206` | `budapest-time` | 527 |
| `Verseghy/website_backend3#208` | `task-lock` | 648 |
| `Verseghy/website_backend3#205` | `openapi-spike` | 705 |
| `Verseghy/website_backend3#207` | `app-clock` | 793 |
| `Verseghy/website_backend3#209` | `metrics-endpoint` | 1,172 |
| `Verseghy/website_backend3#210` | `auth-users` | 1,304 |
| `Verseghy/website_backend3#213` | `clipboard-recorder` | 1,459 |
| `Verseghy/website_backend3#211` | `image-sniff-budget` | 2,428 |
| `Verseghy/website_backend3#212` | `sanitizer-policy-core` | 3,172 |

These facts bear on the rule:

1. **Every one is over the figure.** The nine come to 12,208 hand-written lines, about 1,360 each on average. The plan estimated about 3,800 for the same nine (research: slice plan).
2. **The size is mostly tests.** The slices carry 1.1 to 2 lines of tests for every line of code (research: DECISIONS R10). The tests are the ones the project requires: unit tests for every public function of `src/domain/` and a proptest property for each one there with a non-trivial input space ([ADR 0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md) rule 1), plus tests at every layer a slice touches (ADR 0012 §1 item 4, ADR 0025 §2 rule 4). The bootstrap already showed the same shape (ADR 0025, Context).
3. **The rules as written split the slices.** Under ADR 0012 §1 item 5 and ADR 0025 §2 rule 1, most of the nine would have had to be split by feature into several PRs before they merged, and the later slices would have been planned again at the measured ratio.

Asked how the size rule should count, the maintainer answered on 2026-10-25: "Ship as built" (research: DECISIONS R10). With that option, about 500 lines is guidance only, slices go in as single PRs as built, and later slices are not split for size unless a reviewer asks. All nine merged on 2026-10-25 after that answer and before this ADR, each citing the answer in its body. The maintainer's most recent explicit answer wins over an Accepted ADR ([ADR 0001](0001-record-architecture-decisions.md) §5); this ADR writes the answer down.

That answer left three points open:

- **Who counts as a reviewer.** The answer does not say whether an automated review agent may ask for a split, or only a person. In wave 3, the automated review agents raised the size of almost every slice as a defect. Some slices declined it under the answer; others turned it into a question for the maintainer (research: DECISIONS R15 (D76)).
- **Where this ADR lands.** ADR 0012 §1 item 1 puts an amending ADR "in the same PR as the code that needs it", and ADR 0025 §1 keeps the rule: "Every other amending ADR, including the ones that later slices carry, still lands with the code that needs it." No code implements a line count. What carries this ADR are the instruction files that agents follow: the "Shape" rule in `CLAUDE.md` and recipe 1 of the backend3 skill. [ADR 0026](0026-backend2-unchanged-and-running-until-decommissioning.md) also landed in a docs-only PR (`Verseghy/website_backend3#204`).
- **The re-measure follow-up.** With the estimates no longer a limit, it was unclear whether ADR 0025's follow-up to re-measure them still applied.

Asked these three points, the maintainer answered on 2026-10-25: "Humans only; docs-only landing is fine; retire the re-measure follow-up" (research: DECISIONS R15 (D76)).

## Decision

### 1. The size is guidance

- **About 500 hand-written lines is guidance, not a limit.** It stays the size a slice is planned around. A slice that comes out larger is not held, failed, planned again or split because of its size.
- **A slice goes in as one PR, as built.** The PR carries the whole slice: its code, the tests of every layer it touches and its doc updates.
- **A slice is split for size only when a human reviewer asks** (section 2). The split is by feature, never by layer (ADR 0012 §1 item 5). A part of a slice listed in ADR 0025 §3 keeps that slice's class and meets ADR 0025 §2 rules 2 to 5. A part of any other slice is itself a vertical slice under ADR 0012 §1 item 4.
- **The plan's line estimates are not a limit.** They are planning figures. A slice may come out above or below its estimate.
- **The counting rule does not change.** Hand-written lines are the added plus changed lines, without generated files (ts-rs bindings, the OpenAPI snapshot, insta snapshots, generator output committed without edits), lock files and vendored files (ADR 0012 §1 item 5). A PR that reports its size counts it this way.
- **The testing bar does not change.** A slice is never made smaller by leaving out tests, proptests or docs that [ADR 0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md), ADR 0012 §1 item 4 and ADR 0025 §2 rule 4 require.

### 2. Only a human may ask for a split

- **The maintainer, or another person who reviews the PR, may ask for a split for size.** That request is the only reason a slice is split for size.
- **An automated review agent may not.** It does not raise a slice's size as a defect, and a size remark from one is not a split request: the slice is not split, held or planned again because of it, and the agent that built the slice does not turn it into a question for the maintainer. An agent that reviews a slice still checks everything else, including the testing bar of section 1 and the counting rule when the PR reports its size.

### 3. The re-measure follow-up is retired

ADR 0025's follow-up "The estimates are re-measured after the first three slices merge" no longer applies. The estimates are planning figures, not limits (section 1), so no rule depends on re-measuring them. The rest of that bullet holds: a listed slice that is split further by feature keeps its class, and every part meets ADR 0025 §2 rules 2 to 5.

### 4. Where this ADR lands

This ADR lands in a docs-only PR, as ADR 0026 did. Its follow-up work (below) changes the instruction files that carry it, in the same PR. ADR 0012 §1 item 1 and ADR 0025 §1 still hold for every other amending ADR: it lands with the code that needs it.

### 5. What this amends

| Amended text | Change |
|---|---|
| ADR 0012 §1 item 5: "A slice aims for about 500 hand-written lines or fewer" and "A feature that needs more is split into smaller vertical slices by feature" | About 500 lines is guidance (section 1). A slice that comes out larger goes in as one PR, as built. It is split by feature only when a human reviewer asks (section 2). The counting rule and "never split by layer" hold. |
| ADR 0012 §1 item 1: the amending ADR goes "in the same PR as the code that needs it" | For this ADR only: it lands in a docs-only PR (section 4). |
| ADR 0025 §2 rule 1: "About 500 hand-written lines or fewer, counted as ADR 0012 §1 item 5 counts them" | The figure is guidance, counted the same way (section 1). A slice of a class in ADR 0025 §3 is not split for size unless a human reviewer asks. |
| ADR 0025 §1, the row for ADR 0012 §1 item 5: "The limit of about 500 hand-written lines and its counting rule do not change" | The counting rule does not change. The figure is no longer a limit. |
| ADR 0025 §1, the row for ADR 0012 §1 item 1: "Every other amending ADR, including the ones that later slices carry, still lands with the code that needs it" | This ADR is an exception as well (section 4). The rule holds for every other amending ADR. |
| ADR 0025, Consequences, Positive, bullet 1: "Every PR stays near 500 hand-written lines" | No longer holds. A PR is as large as its slice comes out (Consequences below). |
| ADR 0025, Consequences, Follow-up work, bullet 3: "The estimates are re-measured after the first three slices merge" | Retired (section 3). The rest of the bullet holds. |

Everything else in ADR 0012 and ADR 0025 holds as written. That includes one PR per slice, the slice classes of ADR 0025 §3 and their closed lists, ADR 0025 §2 rules 2 to 5, the ban on layer splits across entities, the slice order of ADR 0025 §4, branching from `master` with at most one open base PR, drafts, the merge policy and the final human review (ADR 0012 §2).

## Consequences

**Positive**

- The wave 2 PRs went in as built, and every slice after them does too. No chain waits while a finished slice is cut into smaller PRs or planned again.
- No slice is trimmed of tests or docs to meet a number. The testing bar stays whole.
- At the measured sizes, a strict limit would need several PRs per slice. Shipping as built keeps the number of PRs, Tide cycles, rebases and base PRs on the critical path at the plan's count.
- Automated reviews no longer reopen the size question on every slice. Only a person can ask for a split, so a size remark does not start a round of re-planning.
- Each PR stays the unit the plan describes, with the dependencies the plan gives it.

**Negative**

- PRs are larger than the Q12 answer's figure. In wave 2 they average about 1,360 hand-written lines, not about 500. Each self-merged PR (ADR 0012 §2) takes longer to read and is a larger revert.
- ADR 0012 lists small slices among the mitigations for merging without a per-PR review (Consequences, "Negative and risks"). That mitigation is weaker now. The required checks, the testing bar, the ADRs and the maintainer's `/hold` remain.
- No automated check or review limits the size. A slice that grows too large to read well is split only if a person asks.
- The final human review covers the same code in larger PRs. PR bodies that name the parity IDs, the ADRs and, for an ADR 0025 slice, its class and the slice or gate that consumes it, keep it workable.
- The plan's estimates were well below the measured sizes in wave 2, and with the follow-up retired they are not re-measured. Totals and schedules built on them understate the work.

**Follow-up work, in the same PR as this ADR**

- The Status lines of ADR 0012 and ADR 0025, both ADR indexes, and the overview's ADR list and glossary.
- The "Shape" rule in `CLAUDE.md` and step 15 of recipe 1 in `.claude/skills/backend3/SKILL.md` say that about 500 lines is guidance, that only a human reviewer may ask for a split for size, and point to this ADR.

**What must be true**

- No slice PR is held, failed or split only because of its size, unless the maintainer or another person reviewing that PR asks for a split. No CI check enforces a line count.
- A size remark from an automated review agent never splits, holds or re-plans a slice.
- A slice that is split on request is split by feature. Every part of a slice listed in ADR 0025 §3 keeps that slice's class and meets ADR 0025 §2 rules 2 to 5; every part of any other slice is a vertical slice under ADR 0012 §1 item 4.
- Every slice carries the tests of every layer it touches (ADR 0025 §2 rule 4), whatever its size.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Keep about 500 lines as a limit, and split each slice that is larger by feature before it merges | ADR 0012 §1 item 5; ADR 0025 §2 rule 1 | Not chosen: the maintainer chose "Ship as built" (research: DECISIONS R10). |
| Set a new, larger limit that fits the measured slices | considered while writing this ADR | The answer sets no limit at all. Wave 2 spans 527 to 3,172 lines, so a new fixed figure would still split some slices for size. |
| Count only code toward the figure, and leave tests out | considered while writing this ADR | The answer drops the figure as a limit; it does not change what counts. Leaving tests out of the count would also make the figure say less about how much there is to read. |
| Let automated review agents ask for a split too | research: DECISIONS R15 (D76) | Not chosen: the maintainer chose "Humans only". |
| Keep R10's wording as it is, and settle who counts as a reviewer the next time an agent asks for a split | research: DECISIONS R15 (D76) | Not chosen: the maintainer chose "Humans only". |

## References

- research: DECISIONS 12 (answer of 2026-10-24); R5, R10, R15 (D76) (answers of 2026-10-25)
- research: QUESTIONS 12
- research: slice plan (2026-10-25), the wave 2 estimates
- `Verseghy/website_backend3#205` to `#213`: the wave 2 slice PRs, with the sizes their bodies report
- `Verseghy/website_backend3#204`: the docs-only PR of [ADR 0026](0026-backend2-unchanged-and-running-until-decommissioning.md)
- Amended: [ADR 0012](0012-delivery-review-commits-and-licensing.md) §1 item 5, and §1 item 1 for this ADR only; [ADR 0025](0025-delivery-exceptions-for-the-backend3-slice-plan.md) §2 rule 1, the §1 rows for ADR 0012 §1 items 5 and 1, Consequences, Positive, bullet 1, and the follow-up to re-measure the estimates (Consequences, Follow-up work, bullet 3)
- Related: [ADR 0001](0001-record-architecture-decisions.md) §5, §6; [ADR 0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md) rule 1; ADR 0012 §1 item 4, §2; ADR 0025 §2 rules 2 to 5, §3
