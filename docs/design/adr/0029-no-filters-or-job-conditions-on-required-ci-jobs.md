# 0029. No filters or job conditions on required CI jobs

- Status: Accepted (2026-10-25)
- Decision questions: none: amends two rows of the table in [ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) §3, which records Q34b. It records the maintainer's answers of 2026-10-25 to decision-register items D31 and D66 (research: DECISIONS R9) and to the second register's items D70 and D83 (research: DECISIONS R15).

## Context

Agents merge their own PRs in this repository. Tide merges an approved PR as soon as every required context passes, and no second person reviews each PR before it merges ([ADR 0012](0012-delivery-review-commits-and-licensing.md) §2; research: DECISIONS 13). The required PR checks of [ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) §3 are therefore the only gate in front of `master`. A required check that can be skipped into a pass is no gate.

| Fact | Value | Source |
|---|---|---|
| A job skipped by a condition passes | GitHub reports a job that its `if:` skipped as "Success". It does not prevent a pull request from merging, even if it is a required check. A job that depends on a failed job (`needs:`) is skipped and "may not block merging". | GitHub Docs, "Using conditions to control job execution"; "Troubleshooting required status checks", Handling skipped but required checks |
| A job whose `needs:` failed or was skipped is skipped | "If a job fails or is skipped, all jobs that need it are skipped unless the jobs use a conditional expression that causes the job to continue." The skipped job then passes, as in the row above. | GitHub Docs, "Workflow syntax for GitHub Actions", `jobs.<job_id>.needs` |
| `continue-on-error` lets a failure pass | At job level it "Prevents a workflow run from failing when a job fails"; at step level it "Prevents a job from failing when a step fails", so the job's check passes with the step failed. Its value may be an expression, such as `${{ matrix.experimental }}`. | GitHub Docs, "Workflow syntax for GitHub Actions", `jobs.<job_id>.continue-on-error` and `jobs.<job_id>.steps[*].continue-on-error` |
| Tide counts a skipped check as passing | Tide maps a completed check run with the conclusion `success`, `skipped` or `neutral` to a passing context. A required context with no check run keeps the PR pending. | `kubernetes-sigs/prow@6691f5aff:pkg/tide/tide.go` (`checkRunToContext`); Verseghy/website_k8s#53 |
| A workflow skipped by a filter never reports | When path filtering, branch filtering or a commit message skips a workflow, its checks stay "Pending" and block merging. GitHub advises: "Avoid requiring workflows that can be skipped." | GitHub Docs, "Troubleshooting required status checks", Handling skipped but required checks |
| backend3's contexts are required for every base branch | Prow lists the 15 required contexts at repository level, so Tide requires them for a PR to any base branch, including a PR stacked on another open PR's branch. The maintainer kept that placement. | `website_k8s@f583aec:verseghy-website/prow/configmaps.yaml`; Verseghy/website_k8s#53; research: DECISIONS R9 (D28) |
| A job's context name | The context of a GitHub Actions job is its `name:`, or its job id when it has none. ADR 0022 §3's table lists 16 contexts; 15 are required, and `cargo-deny check advisories` is not. | ADR 0022 §3 |
| `if:` syntax | In a job's `if:`, the `${{ }}` delimiters may be omitted; GitHub evaluates the value as an expression either way. | GitHub Docs, "Workflow syntax for GitHub Actions", `jobs.<job_id>.if` |
| YAML anchors and aliases | GitHub Actions accepts them in workflows since 2025-09-18, so a trigger's settings or a job's `if:` can be an alias of a node written elsewhere in the file. | GitHub Changelog, 2025-09-18, "Actions: YAML anchors and non-public workflow templates" |
| The workflows today | No workflow has a `paths` or `paths-ignore` filter. `ci.yml` and `cargo-deny.yml` limit their `push` runs with `branches: [master]`; `pull_request` has no filter except `types` in `pr.yml`. The one job-level `if:` is `github.event_name != 'schedule'`, on `cargo-deny check bans licenses sources`: `cargo-deny.yml` also runs weekly from a schedule, and that run is for advisories only. No job uses `needs:` or `continue-on-error`. | `website_backend3@ad496da:.github/workflows/ci.yml`, `cargo-deny.yml`, `pr.yml` |
| What `Workflow policy` checks | Its row in ADR 0022 §3 names `packages: write`, `id-token: write`, registry logins and `pull_request_target`. The check also enforces the other workflow rules of ADR 0022 §1 and §3 (top-level permissions, no secrets, pinned actions, concurrency). It did not look at filters or job-level conditions. | ADR 0022 §3; `website_backend3@ad496da:scripts/ci/check-workflow-policy.sh` |
| What `Toolchain` runs | Its row in ADR 0022 §3 names three items, all from `scripts/ci/check-toolchain.sh`. It leaves out part of that script's rule 1: no legacy `rust-toolchain` file may shadow `rust-toolchain.toml`, which ADR 0022 §3 makes the single source of the toolchain. The job also runs that script's rule 4, the base-image rules of ADR 0022 §2, since the bootstrap (Verseghy/website_backend3#186), and `scripts/ci/check-workspace-deps.sh` since Verseghy/website_backend3#200. Both script headers said they went beyond the row. | `website_backend3@ad496da:.github/workflows/ci.yml`, `scripts/ci/check-toolchain.sh`, `scripts/ci/check-workspace-deps.sh`; Verseghy/website_backend3#195 item 10 |

The `Workflow policy` suggestion came from the website_k8s follow-ups (Verseghy/website_backend3#195 item 9). The Toolchain gap was raised after #200 merged (#195 item 10). The rules of D31 left `needs:`, `continue-on-error` and step-level conditions outside, and that gap was put to the maintainer as well (#195 item 13).

The maintainer answered D31 and D66 on 2026-10-25 (research: DECISIONS R9):

- **D31.** Asked "Should the `Workflow policy` check also reject `paths`, `paths-ignore` and `branches` filters and job-level `if:` on required jobs?", the maintainer answered: "Adopt, with an amendment of the ADR 0022 §3 row".
- **D66.** Asked "Do you accept #200's additions as within the existing decisions, and should ADR 0022 §3's Toolchain row and ADR 0024 §2's list of project-skill recipes be updated in a docs PR?", the maintainer answered: "Accept, and update both tables in a docs PR".

The maintainer answered D70 and D83 on 2026-10-25, in the second decision register (research: DECISIONS R15):

- **D70.** The question was how D66's table updates are applied: this ADR records ADR 0022 §3's Toolchain row as an amendment of ADR 0022 §3, and ADR 0028 §2 carries ADR 0024 §2's recipe list. Asked "Do you accept that way of applying D66? Or should both rows be in-place edits of ADR 0022 §3 and ADR 0024 §2 in a separate docs PR, as D66's wording says?", the maintainer answered: "As recorded in ADR 0028 §2 and ADR 0029".
- **D83.** Asked "Should the Workflow policy check also reject any of these on required jobs? 1. A `needs:` on a job that can fail or be skipped. 2. Job- or step-level `continue-on-error`. 3. A step-level `if:` that skips the real work. 4. `pull_request.types` without `synchronize`. 5. A `push` trigger limited to `tags`.", the maintainer answered: "Reject items 1 and 2 mechanically; leave 3-5 to review".

ADR 0022 is Accepted, so its body is not rewritten ([ADR 0001](0001-record-architecture-decisions.md) §6). This ADR amends its §3 table for D31, and it carries the up-to-date Toolchain row of D66 as well, so that both changes to that table live in one place. The maintainer confirmed that way of applying D66 (D70). The ADR 0024 §2 part of D66 is not part of this ADR: [ADR 0028](0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md) §2 carries it.

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0022 §3, the `Workflow policy` row: "fails if any workflow grants `packages: write` or `id-token: write`, logs in to a container registry, or uses `pull_request_target` (section 1)" | The check also fails when a filter could skip a workflow that runs a required job, or when a required job has a job-level condition (D31); and when a required job `needs:` a job that can fail or be skipped without failing it, or has `continue-on-error` (D83). | 2, 4 |
| ADR 0022 §3, the `Toolchain` row | The row names everything the job runs: `check-toolchain.sh` rules 1-4 and `check-workspace-deps.sh`. Rule 4 and `check-workspace-deps.sh` are the additions that the maintainer accepted as within the existing decisions (D66). The row also names rule 1's check that no legacy `rust-toolchain` file shadows `rust-toolchain.toml`. That check applies ADR 0022 §3's own rule that `rust-toolchain.toml` at the repo root is the single source, so naming it completes the row's description and is not part of D66. None of this changes a decision. | 3 |

Everything else in ADR 0022 holds as written: the job names, which contexts are required, the other rows of §3 and the rest of the ADR.

### 2. Required jobs run on every pull request

**Which jobs are required.** A job's context is its `name:`, or its job id when it has none. For rules 8 to 11 (rules 10 and 11 are section 4), a job counts as required unless ADR 0022 §3's table marks its context "no". `scripts/ci/check-workflow-policy.sh` holds the contexts that the table marks "no", today only `cargo-deny check advisories`, and treats every other job as required. A new job is therefore covered from its first PR, before it is added to the table or to Prow. For rules 8, 9 and 11 that is the strict default. For rule 10 it is the lenient one, because being required is what lets a job be a `needs:` target; section 4 covers the window until Prow lists a new job. Making a job optional changes ADR 0022 §3's Required column, so it needs an amending ADR. The same PR adds the context to the script's list and changes `scripts/tests/test-check-workflow-policy.sh` to read the Required column as amended. The test reads that column from ADR 0022 §3's own table, which the amending ADR does not edit ([ADR 0001](0001-record-architecture-decisions.md) §6), so without that change the test fails.

**Rule 8: no filter may skip a workflow that runs a required job.** In a workflow with at least one required job:

- no trigger has a `paths` or `paths-ignore` filter;
- no trigger other than `push` has a `branches` or `branches-ignore` filter.

Rule 8 does not limit `branches` or `branches-ignore` on `push`. `ci.yml` and `cargo-deny.yml` keep `branches: [master]`, which is how the workflows run on `push` to `master` only (ADR 0022 §3). A PR gets its required contexts from its `pull_request` run, which rule 8 keeps unfiltered. `paths` and `paths-ignore` are banned on `push` too, so that one list of filters holds for every trigger; no workflow uses them. Other trigger settings stay allowed, for example `types` on `pull_request` (`pr.yml` lists `opened, edited, reopened, synchronize`) and `schedule`. A workflow whose jobs are all not required may use filters; there is none today.

The check reads each workflow with its YAML anchors and aliases resolved, so a trigger written as an alias, such as `pull_request: *push_settings`, is checked as if it were written out. The same holds for a job's `if:`.

**Rule 9: no job-level condition on a required job.** A required job has no job-level `if:`, with one exception: exactly `github.event_name != 'schedule'`. It may be written bare or inside `${{ }}`, with any spacing around `!=` and inside the delimiters. That condition is true on every `pull_request` and `push` event, so it never skips a PR check. It skips the job only in a scheduled run, which reports on a `master` commit that no PR waits for. `cargo-deny check bans licenses sources` carries it, because `cargo-deny.yml`'s weekly schedule runs the advisories only.

Every other job-level condition on a required job fails the check. That includes `if: false`, a condition on another event such as `github.event_name != 'push'`, the inverted guard (`github.event_name == 'schedule'`, or `${{ !(github.event_name != 'schedule') }}`), and the schedule guard joined to another term with `&&` or `||`, before or after it, on one line or on several. It applies to a reusable-workflow job (`uses:`) too. A job that must not run on every PR cannot be a required context. Rule 9 covers job-level conditions only. A step-level `if:`, such as `if: failure()` on a log step, is not checked; section 4 leaves it to review.

**The amended row.** ADR 0022 §3's `Workflow policy` row reads:

| Job (context) | What it runs | Required |
|---|---|---|
| `Workflow policy` | fails if any workflow grants `packages: write` or `id-token: write`, logs in to a container registry, or uses `pull_request_target` (ADR 0022 §1); or if a workflow that runs a required job has a `paths` or `paths-ignore` filter on any trigger, or a `branches` or `branches-ignore` filter on a trigger other than `push`; or if a required job has a job-level `if:` other than `github.event_name != 'schedule'` ([ADR 0029](0029-no-filters-or-job-conditions-on-required-ci-jobs.md) §2); or if a required job `needs:` a job that is not required or that has a job-level `if:` or `continue-on-error`; or if a required job, or one of its steps, has `continue-on-error` other than `false` ([ADR 0029](0029-no-filters-or-job-conditions-on-required-ci-jobs.md) §4) | yes |

The script's header lists rules 1-11 in full. Rules 1-7 are the rules of ADR 0022 sections 1 and 3, unchanged.

### 3. The Toolchain row

`check-toolchain.sh` rule 4 checks the base-image rules of ADR 0022 §2 before the post-merge PAC build, the only other place a wrong base image would show. `check-workspace-deps.sh` keeps every Cargo requirement where Dependabot rewrites it (`AGENTS.md`, "Rust dependencies"). Neither needs a Rust toolchain, so both run in the `Toolchain` job. ADR 0022 §3's `Toolchain` row reads:

| Job (context) | What it runs | Required |
|---|---|---|
| `Toolchain` | `scripts/ci/check-toolchain.sh`: `rust-toolchain.toml` names an exact release at or above 1.94, and no legacy `rust-toolchain` file shadows it; the Containerfile's `RUST_VERSION` equals it; the Containerfile's Node ARG equals `admin/.node-version` ([ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md)); every base image is UBI 9 from `registry.access.redhat.com`, pinned by digest, and the admin stage's `ubi9/nodejs-<major>` image matches `admin/.node-version` (rule 4; ADR 0022 §2). `scripts/ci/check-workspace-deps.sh`: every Cargo dependency requirement is declared once, in the root `Cargo.toml`'s `[workspace.dependencies]`, in a form that Dependabot rewrites; each crate inherits it with `workspace = true`, adding only `features` or `optional` | yes |

### 4. A required job cannot pass over a failure

D83 adds rules 10 and 11 to the `Workflow policy` check, for items 1 and 2 of its question. Both close a way for a required job to report success although work it stands for failed. No job uses `needs:` or `continue-on-error` today, so no workflow changes.

**Rule 10: a required job needs only jobs that fail the gate themselves.** Every job that a required job names in `needs:` is a job of the same workflow that:

- is itself required, as section 2 defines it;
- has no job-level `if:`, the schedule guard of rule 9 included;
- has no job-level `continue-on-error` other than the literal `false`.

GitHub skips every job whose `needs:` failed or was skipped, and the skipped required job passes (Context). A required target that fails reports its own failing context, so the PR still cannot merge. That holds once website_k8s's Prow configuration lists the target's context. Section 2 counts a new job as required from its first PR, before Prow lists it, so rule 10 accepts a `needs:` on it; until Prow lists it, its failure would skip the job that needs it into a pass while nothing that Tide waits for fails. An existing required job therefore does not name a new job in `needs:` until website_k8s's Prow configuration lists the new job's context. The check cannot read that configuration (Alternatives), so this part is left to review; no job uses `needs:` today. A target that is not required can fail with nothing else failing, and a target with a condition can be skipped; either takes the required job that needs it into a pass. The schedule guard counts as a condition here. It skips nothing on a pull request, but rule 10 decides "can be skipped" from the job's text alone, without reasoning about events, and no job needs a guarded job today. A target is required, so rule 10 holds for it too, and therefore along every chain of `needs:`. A `needs:` entry that is not a job id of the same workflow, for example a job's `name:`, fails the check with a message that says so, because the check cannot show such a target to be required. So does a `needs:` that is neither a job id nor a list of job ids.

**Rule 11: no `continue-on-error` on a required job.** A required job has no `continue-on-error` other than the literal `false`, at job level or on any of its steps. At job level it lets the workflow run pass when the job fails; at step level it lets the job pass when the step fails, so the required context passes with the step failed (Context). An expression such as `${{ matrix.experimental }}` counts as set, because it can be true; so does the quoted string `'false'`, because only the YAML boolean is certain to be false. `continue-on-error: false` lets no failure pass, so it is allowed. Rule 11 covers diagnostics steps too. A step that only collects logs after a failure, such as `PostgreSQL log` with `if: failure()` in `ci.yml`, needs no `continue-on-error`, because the job has already failed when it runs.

**Left to review.** D83's items 3 to 5 are not checked by the script. Only review can catch them: the maintainer can hold any PR with `/hold`, and the human review of the whole codebase (PLT-36 in [parity.md](../parity.md); [ADR 0012](0012-delivery-review-commits-and-licensing.md) §2) covers the workflows as well. No second person reviews each PR before it merges (ADR 0012), so until that review a PR that uses one of them can merge.

| D83 item | Why it is left to review |
|---|---|
| 3. A step-level `if:` that skips the real work of a required job | Telling it apart from a step condition that belongs there, such as `if: failure()` on a log step, needs judgement; a mechanical rule would have false positives. |
| 4. `pull_request.types` without `synchronize` | A push to the PR then starts no run, so the new head's required contexts have no check run and stay pending, which blocks merging (Context). It fails safe. |
| 5. A `push` trigger limited to `tags` | It changes only push runs, and a push run gates no PR. |

## Consequences

**Positive**

- A required context cannot be skipped into a pass by its own job-level condition, or by a job it `needs:` once Prow requires that job's context (section 4). Every PR runs every required job, whatever it changes and whatever its base branch, stacked PRs included.
- A required context cannot turn a failure into a pass with `continue-on-error`, on the job or on a step.
- A filter cannot leave a PR waiting on a context that never reports.
- The `Toolchain` row names what the job runs, so the final review (PLT-36 in [parity.md](../parity.md)) reads the code against a current table.

**Negative**

- No PR saves CI time by skipping work: a docs-only PR runs the full Rust and admin jobs, as it does today.
- The rules cover workflow filters and job-level conditions, as D31 asked, and `needs:` and `continue-on-error`, as D83 asked. D83's items 3 to 5 are left to review (section 4): a step-level `if:` that skips a required job's real work, `pull_request.types` without `synchronize`, and a `push` trigger limited to tags pass the check, and only a reviewer finds them.
- Rule 10 accepts a `needs:` on a new job that Prow does not require yet, because the check counts every job as required unless ADR 0022 §3 marks it "no" (section 2). Until website_k8s's Prow configuration lists the new context, a failure of that job would skip the required job that needs it into a pass. Only review keeps an existing required job from needing a new job in that window (section 4).
- Rule 10 also refuses the schedule guard on a job that a required job needs, although the guard skips nothing on a pull request. A workflow with a scheduled run that wants a required job after another puts the guard on the later job only, and the earlier job then runs in the scheduled run as well.
- A step that is allowed to fail cannot be part of a required job. Such a step belongs in a job that is not required, which needs an amending ADR (section 2).
- The script keeps its own copy of the "not required" contexts. Its test compares that copy with ADR 0022 §3's table on every run.

**What must be true in code and tests**

- `scripts/ci/check-workflow-policy.sh` implements rules 8 and 9 as section 2 states them and rules 10 and 11 as section 4 states them, and its header documents them and names D83's items 3 to 5 as left to review.
- `scripts/tests/test-check-workflow-policy.sh` has a failing fixture for each banned filter (`paths`, `paths-ignore`, `branches` and `branches-ignore` on `pull_request`; `paths` and `paths-ignore` on `push`; `branches` on another trigger; a filter that reaches `pull_request` through an alias or a merge key) and for each kind of banned condition (another condition, the inverted guard, the guard joined to another term before or after it, a multi-line condition with the guard first or last, an aliased condition), and passing fixtures for `push: branches`, `types`, each spelling of the schedule guard, the not-required context and a step-level condition. One fixture gives each of ADR 0022 §3's 16 contexts a condition and checks that exactly the 15 required ones fail.
- For rule 10, the same test has failing fixtures for a `needs:` on the not-required context, on a job with a condition, on a job with only the schedule guard, on a job with `continue-on-error`, on a job's `name:` instead of its id, through an alias, at the end of a chain, from a reusable-workflow job, as the first, a middle and the last entry of a list, and for a `needs:` that is a mapping, with one line per reason when a target breaks two and one line per bad entry when a list has two; and passing fixtures for a `needs:` written as a string, as a list and as a list of several jobs, a target with `continue-on-error: false`, and a job that is not required needing a job with a condition.
- For rule 11, it has failing fixtures for job-level `continue-on-error` set to `true`, to an expression, to an expression over several lines (reported on one line) and to the quoted string `'false'`, for step-level `continue-on-error: true` on a named and on an unnamed step, and for an aliased value; and passing fixtures for `false` at job and at step level and for the not-required context. A fixture gives each of the 16 contexts `continue-on-error: true` and checks that exactly the 15 required ones fail.
- The current workflows pass.
- The comments in `ci.yml` and `cargo-deny.yml` name this ADR, the schedule guard's comment says why it is allowed, and `ci.yml`'s ground rules list rules 8 to 11.
- The headers of `check-toolchain.sh` and `check-workspace-deps.sh` point to section 3.
- ADR 0022's Status line names 0029 beside [ADR 0027](0027-per-package-licence-exceptions-and-the-loco-advisory-ignores.md), which amends its §4: `Amended by 0027 and 0029`.

**Follow-up work**

None in website_k8s: the job names and the required contexts do not change.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Decline: keep the `Workflow policy` row as it is | research: DECISIONS R9 (D31, the other option) | Required checks would stay skippable through a future job-level condition, and with self-merge they are the only gate. The maintainer adopted the rule. |
| Accept the Toolchain additions without editing the table | research: DECISIONS R9 (D66, the other option) | The table would keep lagging behind the code, and the final review would have to know why. The maintainer chose the update. |
| Edit ADR 0022 §3's Toolchain row in place, in a separate docs PR | research: DECISIONS R15 (D70, the other option) | This ADR amends ADR 0022 §3, and an amended ADR keeps its body as it was ([ADR 0001](0001-record-architecture-decisions.md) §6), so the amending ADR is where the row belongs, as ADR 0028 §2 does for ADR 0024 §2. The maintainer chose to keep the row here. |
| Ban every job-level condition, the schedule guard included | not offered; considered while writing this ADR | The guard skips nothing on a PR or a push. Without it, the bans, licences and sources check would also run in the weekly schedule, which exists for the advisories. |
| Ban `branches` on `push` as well | not offered; considered while writing this ADR | Push runs would then run on every branch push and repeat the `pull_request` runs. ADR 0022 §3 runs the workflows on `push` to `master` only, and a push run gates no PR. |
| Also require a `branches` filter on `push` to keep `master` | not offered; considered while writing this ADR | A push run gates no PR, so it is outside what D31 asked. ADR 0022 §3 already runs the workflows on `push` to `master`; checking that needs a further maintainer decision. |
| Allow any condition that is true on every `pull_request` and `push` event | not offered; considered while writing this ADR | Telling such conditions apart needs an evaluator for GitHub's expression language. One exact allowed form is simple to check and to review. |
| Keep path filters, and add a second workflow with the same job names that reports success whenever the filtered one is skipped | not offered; considered while writing this ADR | It reports a required context as passed without running the check, which is what this ADR rules out. |
| Read the required contexts from website_k8s's Prow configuration | not offered; considered while writing this ADR | The PR checks here cannot read website_k8s. ADR 0022 §3 is the source that both repositories follow, and the test holds the script to it. The cost is the window of section 4, in which rule 10 accepts a `needs:` on a job that Prow does not require yet. |
| Reject all five of D83's items | research: DECISIONS R15 (D83, another option) | Item 3 needs a heuristic with false positives, and items 4 and 5 would block harmless setups; item 4 already fails safe. The maintainer chose items 1 and 2. |
| Leave this ADR's rules at filters and job-level conditions | research: DECISIONS R15 (D83, the third option) | `needs:` and `continue-on-error` are the two items that can turn a failing required job green, and with self-merge only review would catch them. The maintainer chose to reject them mechanically. |
| Allow the schedule guard on a job that a required job needs | not offered; considered while amending this ADR for D83 | The guard skips nothing on a PR, but allowing it means deciding which events each target runs on. Rule 10 reads "can be skipped" from the job's text alone, and no job needs a guarded job today. |
| Ban the key `continue-on-error` outright, `false` included | not offered; considered while amending this ADR for D83 | `false` lets no failure pass, so refusing it would be a false positive. |
| Allow `continue-on-error` on a step that runs only after a failure (`if: failure()`) | not offered; considered while amending this ADR for D83 | The job has already failed when such a step runs, so the step needs no `continue-on-error`. The exception would also make the check read step-level conditions, which D83 leaves to review. |

## References

- research: DECISIONS R9 (D31, D66; D28 for the context placement), DECISIONS R15 (D70, D83), DECISIONS 13, 34b
- Verseghy/website_backend3#195 items 9, 10 and 13; Verseghy/website_backend3#186 (bootstrap), Verseghy/website_backend3#200 (`check-workspace-deps.sh`)
- Verseghy/website_k8s#53, "prow: require website_backend3's PR checks in branch protection"; `website_k8s@f583aec:verseghy-website/prow/configmaps.yaml`
- `kubernetes-sigs/prow@6691f5aff:pkg/tide/tide.go` (`checkRunToContext`)
- GitHub Docs, "Troubleshooting required status checks", Handling skipped but required checks: <https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/collaborating-on-repositories-with-code-quality-features/troubleshooting-required-status-checks#handling-skipped-but-required-checks>
- GitHub Docs, "Using conditions to control job execution": <https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-jobs-with-conditions>
- GitHub Changelog, 2025-09-18, "Actions: YAML anchors and non-public workflow templates": <https://github.blog/changelog/2025-09-18-actions-yaml-anchors-and-non-public-workflow-templates/>
- GitHub Docs, "Workflow syntax for GitHub Actions", `on.<push|pull_request|pull_request_target>.<paths|paths-ignore>`, `on.<pull_request|pull_request_target>.<branches|branches-ignore>`, `jobs.<job_id>.if`, `jobs.<job_id>.needs`, `jobs.<job_id>.continue-on-error` and `jobs.<job_id>.steps[*].continue-on-error`: <https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax>
- `website_backend3@ad496da`: `.github/workflows/ci.yml`, `.github/workflows/cargo-deny.yml`, `.github/workflows/pr.yml`, `scripts/ci/check-workflow-policy.sh`, `scripts/ci/check-toolchain.sh`, `scripts/ci/check-workspace-deps.sh`
- Related: [ADR 0001](0001-record-architecture-decisions.md), [ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md), [ADR 0012](0012-delivery-review-commits-and-licensing.md), [ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md), [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md), [ADR 0027](0027-per-package-licence-exceptions-and-the-loco-advisory-ignores.md), [ADR 0028](0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md)
