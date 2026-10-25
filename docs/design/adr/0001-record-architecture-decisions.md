# 0001. Record architecture decisions

- Status: Amended by [0034](0034-test-hashes-and-throwaway-keys-refused-in-production.md) (2026-10-25)
- Decision questions: Q12 (the "design document first" part; PR slicing and review are recorded in [ADR 0012](0012-delivery-review-commits-and-licensing.md)), Q24

## Context

website_backend3 replaces two systems at once: the Laravel 7 + Backpack 4.1 admin (`website_backend`, "backend1") and the Rust GraphQL read API (`website_backend2`, "backend2"). Before any code was written, a research phase described the current state in detail. It produced per-topic research files, fourteen gap files that correct them, one synthesis that merges both, and 56 candidate design questions from four lenses (product parity, data migration, ops and security, AI maintainability). After duplicates were merged, 44 questions remained, 14 of them blocking (research: QUESTIONS, introduction and Appendix A).

The maintainer answered every question on 2026-10-24 (research: DECISIONS). Many answers took the recommended default. Several did not: for example, all event times are stored as `timestamptz` (Q9), there is one host for admin and public API (Q16), the full safety net applies to every entity (Q11), and backend1 stays up read-only after the cut-over (Q41). Those overrides are exactly the decisions that a later reader, human or agent, is most likely to "correct" back to the default if the reason is not written down.

Four more facts shape how the decisions must be recorded:

1. **The repository is public** (AGPL-3.0-only). The research mixes durable design inputs (parity IDs, API semantics, data anomalies, migration rules) with operational and security facts about the running system that must not be published (research: QUESTIONS 24).
2. **Most of the work is done by one maintainer plus AI agents.** Agents read code well, but code does not say why a choice was made. The vendored Loco skill also carries its own doctrine, which agents follow unless the project says otherwise (research: QUESTIONS 7, 43).
3. **Deep stacks have stalled before.** backend2's last round of quality work arrived as 14 deeply stacked pull requests (`Verseghy/website_backend2#246` to `#259`). The first of them has waited for review since 2026-09-14, and none had merged when this repository was created (research: QUESTIONS 12, 13). The recommended answer to Q12 gives the reason for design first: ADRs give agents the "why" that the code cannot (research: QUESTIONS 12).
4. **The maintainer chose "design document first"** (research: DECISIONS 12): one change adds `docs/design/` with an architecture overview and ADRs before any code lands. The maintainer also chose to split the knowledge (research: DECISIONS 24): sanitised, durable specs go into this repository's `docs/`, while the full synthesis and the security and operations findings stay in the private `Verseghy/runbooks` repository under `verseghy-website-rewrite/`.

## Decision

### 1. Every architecture decision is an ADR

We record each architecture decision as an Architecture Decision Record (ADR) in `docs/design/adr/`, in the style Michael Nygard described (title, context, decision, status, consequences), extended for this project. A decision is "architectural" when reversing it would change the schema, the public or admin API contract, the security model, the deployment shape, the delivery process, or what agents are allowed to do. Smaller choices live in code comments, specs and PR descriptions.

### 2. Format

Every ADR uses this structure. The template is in [the ADR index](README.md#template).

| Part | Content |
|---|---|
| `# NNNN. Title` | A short noun phrase that names the decision, not the problem. |
| `- Status:` | One of the statuses in section 4, with the date. |
| `- Decision questions:` | The Q-numbers from research: QUESTIONS that this ADR records, including sub-parts (for example `Q10a, Q10b`). An ADR that records no question says `none` and says whose answer it records. |
| `## Context` | The facts that force the decision, with numbers, each with a source. |
| `## Decision` | What we do, precise enough to implement: names, settings, limits. |
| `## Consequences` | Positive and negative effects, follow-up work, and what must be true in code and tests. |
| `## Alternatives considered` | The other options that were offered, one line each, with the reason they were not chosen. |
| `## References` | Sources, in the citation forms of section 7. |

Write in plain English, with short sentences and active voice. Use tables for structured facts and mermaid diagrams where a picture helps.

### 3. Numbers and file names

- Numbers have four digits, are assigned in order, and are never reused, not even for a rejected ADR.
- The file name is `NNNN-<slug>.md`. The slug is the title in lower case, with every run of characters other than `a-z` and `0-9` replaced by one hyphen, leading and trailing hyphens removed, and the result cut to at most 60 characters. Example: "Media: separate static server, media library, image optimisation" becomes `0015-media-separate-static-server-media-library-image-optimisatio.md`.
- The file name never changes after the ADR is merged, even if the title is later reworded, so links stay stable.

### 4. Statuses

| Status | Meaning |
|---|---|
| `Proposed (date)` | Written, not yet answered by a maintainer. Code must not depend on it. |
| `Accepted (date)` | Records a maintainer's answer. Code follows it. |
| `Rejected (date)` | A maintainer turned the proposal down. The file stays so the reasoning is not repeated. |
| `Superseded by NNNN (date)` | A later ADR replaces this decision completely. |
| `Amended by NNNN (date)` | A later ADR replaces named parts of this decision; the rest still holds. |
| `Deprecated (date)` | The decision no longer applies and nothing replaces it (for example, the component it governs was removed). |

[ADR 0013](0013-rich-text-editor-and-html-sanitiser.md) was accepted on 2026-10-24, after the maintainer answered Q15 (research: DECISIONS 15) and the follow-up questions on the detailed editor design (research: DECISIONS 15a-15g).

### 5. The source of truth and precedence

The maintainer's answers of 2026-10-24 (research: DECISIONS) are binding. ADRs 0001 to 0024 record them. Where an answer overrides the recommended default ("user override" or "user choice" in DECISIONS), the ADR states the chosen option as the decision and lists the default only under "Alternatives considered". An ADR never argues for a rejected option.

When documents disagree, this order wins:

1. the maintainer's most recent explicit answer;
2. Accepted ADRs, with a later ADR winning over an earlier one it supersedes or amends;
3. the architecture overview ([docs/design/README.md](../README.md)) and the specs in `docs/design/` ([public-api.md](../public-api.md), [parity.md](../parity.md), [data-migration.md](../data-migration.md), [cutover.md](../cutover.md), [editor.md](../editor.md));
4. the private research, where a `gap-NN` file wins over the file it corrects.

If an Accepted ADR misstates the answer it records, that is a documentation bug. Fix the ADR text in a docs PR; do not write a superseding ADR for it.

### 6. Changing a decision

- **Accepted ADRs are not rewritten.** To change a decision, write a new ADR. Its Context says what changed and quotes the maintainer's answer with its date. Its Decision says which ADR it supersedes or which parts it amends.
- **The old ADR changes in one place only:** its Status line becomes `Superseded by NNNN (date)` or `Amended by NNNN (date)`, with a link to the new ADR. The body stays as it was, so the history is readable.
- **Editorial fixes are allowed** without a new ADR: typos, broken links, a clearer sentence that does not change the meaning, or a correction under section 5.
- **Only a maintainer's answer accepts, rejects or supersedes an ADR.** An agent that finds an Accepted decision wrong or unworkable stops, writes a Proposed ADR or an issue that states the problem with evidence, and asks the maintainer. Under the merge policy of [ADR 0012](0012-delivery-review-commits-and-licensing.md) an agent may merge a Proposed ADR, but it stays Proposed until a maintainer answers.
- **A PR that would contradict an Accepted ADR is not merged** until a superseding or amending ADR is Accepted.
- **Every new or changed ADR updates [the ADR index](README.md)** in the same PR, and the index in [docs/design/README.md](../README.md#architecture-decision-records).

### 7. Citations

| Source | Form |
|---|---|
| The maintainer's answers | `research: DECISIONS 16` (row number or label, such as `10a` or `R1`) |
| The offered options | `research: QUESTIONS 16` |
| The research synthesis | `research: SYNTHESIS §4.12` |
| A gap file | `research: gap-02 §4.5` |
| Public code | `website_backend@0dead98:app/Models/Posts.php` (repository, commit, path) |
| Public pull requests and issues | `Verseghy/website_backend2#251` |
| Another ADR or spec in this repository | a relative Markdown link |

The research is private. A citation tells a reader with access where to look; the ADR text itself must be understandable without it. Never cite local file system paths, temporary directories or chat or session URLs.

### 8. What a public ADR may contain

This repository is public. Every ADR and spec in `docs/` follows these rules, and a violation is a defect:

1. No credentials, password hashes, tokens, keys, or any value taken from a secret.
2. No description of live, unfixed weaknesses of the running systems. This applies to every area of the running systems: do not state what they lack or how they are configured, only what backend3 enforces. Describe the target design instead ("backend3 enforces X"). Neutral history that is already public in merged PRs is fine.
3. No personal data: no names or e-mail addresses of editors or users and no per-person facts. Aggregate counts are fine. Public school contact details that the site itself shows are fine.
4. No `claude.ai` session links.

Material that cannot meet these rules goes to the private runbooks repository, and the ADR says only what the target design does.

### 9. Where things live

| What | Where |
|---|---|
| Decisions and their reasons | `docs/design/adr/` (this set) |
| The architecture overview | [`docs/design/README.md`](../README.md) |
| Specs that implement the decisions | `docs/design/*.md` |
| Evidence, the full synthesis, security and operations findings | private `Verseghy/runbooks`, under `verseghy-website-rewrite/` |
| Parity progress | one GitHub issue per parity ID on a GitHub Project board ([ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md)) |

The root `CLAUDE.md` links both the design docs and the private research, the latter marked "if available" (research: DECISIONS 24, 43a).

## Consequences

**Positive**

- Agents and reviewers can find why the code is the way it is, including why it departs from a framework default or a recommended option.
- The maintainer's overrides are protected: a later change has to go through a new ADR and a maintainer answer.
- The public repository carries enough design to work from, while the security-sensitive evidence stays private.

**Negative**

- Every decision change costs a written ADR and a maintainer answer, even when the change looks small.
- Readers without access to the private research see the conclusions and the key numbers, not the full evidence. Each ADR's Context must therefore carry the numbers that matter, not only a citation.
- Two indexes (the ADR index and the overview) must be kept in step.

**Follow-up work and what must be true**

- The bootstrap PR adds a root `CLAUDE.md` that points agents to `docs/design/`, states the precedence in section 5 and the rule in section 6 that agents do not change Accepted decisions on their own.
- Every file in `docs/design/adr/` appears in both indexes, with the same status as its own Status line.
- Every numbered row of research: DECISIONS (1-44 and their sub-parts) is recorded by at least one ADR. Rows R1-R3 are recorded where they bear on this repository (licence, merge policy), without local paths; R4 is not recorded. Every ADR names its decision questions.
- The CI check that bans session links (research: DECISIONS 44; [ADR 0012](0012-delivery-review-commits-and-licensing.md)) also covers `docs/`.
- [ADR 0013](0013-rich-text-editor-and-html-sanitiser.md) is Accepted (2026-10-24); its detailed design is specified in [editor.md](../editor.md).

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Spike the code first and write the design afterwards | QUESTIONS 12 B | The early choices (schema, API contract, auth, hosting) are the expensive ones to change, and agents need the reasons before they write code, not after. The maintainer chose design first. |
| Layered PRs or one long-lived branch instead of vertical slices | QUESTIONS 12 C, D | These are PR-slicing options; [ADR 0012](0012-delivery-review-commits-and-licensing.md) records why vertical slices were chosen. |
| Commit the full research synthesis to the public repository | QUESTIONS 24 B | It contains security-sensitive operational facts about the running system. |
| Keep all knowledge in the private runbooks repository only | QUESTIONS 24 C | Agents and contributors working in this repository would have no durable specs to work from. |
| Keep no written record; research again when needed | QUESTIONS 24 D | The research took a full phase of work; repeating it for every question is slower than reading an ADR, and the reasons behind overrides would be lost. |
| One long design document without ADRs | not offered; considered while writing this ADR | A single document has no per-decision status, so a changed decision silently rewrites history. |
| Decisions only in PR descriptions or issues | not offered; considered while writing this ADR | Hard to find, not versioned with the code, and not loaded by agents working in the repository. |

## References

- research: DECISIONS 12, 13, 15, 24, 43a, 44, R1-R3 (answers of 2026-10-24)
- research: QUESTIONS 7, 12, 13, 24, 43 and Appendix A
- Michael Nygard, "Documenting Architecture Decisions", 2011-11-15, <https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions>
- Stacked backend2 PRs: `Verseghy/website_backend2#246` to `#259`
- [ADR index](README.md) and [architecture overview](../README.md)
