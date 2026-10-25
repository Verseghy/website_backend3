#!/usr/bin/env bash
# Tests for scripts/ci/check-workflow-policy.sh (ADR 0022 sections 1 and 3,
# ADR 0029).
#
# Each fixture workflow breaks one rule; a compliant fixture and the real
# workflows in .github/workflows/ must pass. The last cases check that the job
# names across the real workflows are exactly the branch-protection contexts
# in ADR 0022 section 3's table, so a renamed job fails here before it
# silently drops out of branch protection, and that the script treats as
# required exactly the contexts that table marks "yes". Run from the
# repository root:
#   bash scripts/tests/test-check-workflow-policy.sh
# Needs mikefarah yq v4 and jq; no network access.
# Single-quoted fixtures and sed/jq programs hold a literal `$` on purpose.
# shellcheck disable=SC2016
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT="$ROOT/scripts/ci/check-workflow-policy.sh"
ADR="$ROOT/docs/design/adr/0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md"

tests_run=0
failures=0

# run_case NAME EXPECTED_EXIT [--contains TEXT] -- COMMAND...
# Runs COMMAND, compares its exit code and, optionally, looks for TEXT in its
# combined output. Prints one TAP-style line per case.
run_case() {
  local name=$1 expected=$2 needle=""
  shift 2
  if [[ ${1-} == --contains ]]; then
    needle=$2
    shift 2
  fi
  [[ ${1-} == -- ]] && shift
  local output status=0
  output=$("$@" 2>&1) || status=$?
  tests_run=$((tests_run + 1))
  if [[ $status -ne $expected ]]; then
    failures=$((failures + 1))
    printf 'not ok %d - %s (exit %d, expected %d)\n%s\n' "$tests_run" "$name" "$status" "$expected" "$output" | sed '2,$s/^/    # /'
  elif [[ -n $needle && $output != *"$needle"* ]]; then
    failures=$((failures + 1))
    printf 'not ok %d - %s (output lacks "%s")\n%s\n' "$tests_run" "$name" "$needle" "$output" | sed '2,$s/^/    # /'
  else
    printf 'ok %d - %s\n' "$tests_run" "$name"
  fi
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

SHA=3d3c42e5aac5ba805825da76410c181273ba90b1
# A compliant workflow's top level: its `on:` block, then the rest.
HEADER_TRIGGERS='on:
  pull_request:
  push:
    branches: [master]'
HEADER_REST='permissions:
  contents: read
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true'
HEADER="$HEADER_TRIGGERS
$HEADER_REST"

# fixture NAME CONTENT: write CONTENT as the only workflow of a fresh
# directory and print that directory.
fixture() {
  local dir="$WORK/$1"
  mkdir -p "$dir"
  printf '%s\n' "$2" >"$dir/workflow.yml"
  printf '%s' "$dir"
}

# job_with_on TRIGGERS STEPS: print a compliant workflow with the `on:` block
# TRIGGERS whose single job, `Check`, has STEPS (YAML list items indented by
# six spaces).
job_with_on() {
  printf '%s\n%s\njobs:\n  check:\n    name: Check\n    runs-on: ubuntu-24.04\n    steps:\n%s\n' "$1" "$HEADER_REST" "$2"
}

# job_with STEPS: the same, with the compliant `on:` block.
job_with() {
  job_with_on "$HEADER_TRIGGERS" "$1"
}

GOOD_STEPS="      - uses: actions/checkout@$SHA # v7.0.1
        with:
          persist-credentials: false
      - uses: ./.github/actions/local
      - uses: docker://registry.example/tool@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
      - name: Use the token, not a secret
        run: echo \"\${{ github.token }}\" > /dev/null"

# Compliant workflows.
run_case "a compliant workflow passes" 0 --contains "comply with ADR 0022" -- \
  bash "$SCRIPT" "$(fixture good "$(job_with "$GOOD_STEPS")")"
run_case "the real workflows pass" 0 -- bash "$SCRIPT" "$ROOT/.github/workflows"

# Rule 1: top-level permissions.
run_case "missing permissions fail" 1 --contains "must be exactly \`contents: read\`" -- \
  bash "$SCRIPT" "$(fixture no-perms "$(job_with "$GOOD_STEPS" | sed '/^permissions:/,/^  contents: read/d')")"
run_case "contents: write fails" 1 --contains "must be exactly \`contents: read\`" -- \
  bash "$SCRIPT" "$(fixture contents-write "$(job_with "$GOOD_STEPS" | sed 's/^  contents: read/  contents: write/')")"
run_case "read-all fails" 1 --contains "must be exactly" -- \
  bash "$SCRIPT" "$(fixture read-all "$(job_with "$GOOD_STEPS" | sed -e '/^  contents: read/d' -e 's/^permissions:$/permissions: read-all/')")"

# Rule 2: pull_request_target in each syntax.
run_case "pull_request_target (mapping) fails" 1 --contains "pull_request_target" -- \
  bash "$SCRIPT" "$(fixture prt-map "$(job_with "$GOOD_STEPS" | sed 's/^  pull_request:$/  pull_request_target:/')")"
run_case "pull_request_target (list) fails" 1 --contains "pull_request_target" -- \
  bash "$SCRIPT" "$(fixture prt-list "$(job_with "$GOOD_STEPS" | sed -e '/^  pull_request:$/d' -e '/^  push:$/d' -e '/^    branches:/d' -e 's/^on:$/on: [push, pull_request_target]/')")"
run_case "pull_request_target (string) fails" 1 --contains "pull_request_target" -- \
  bash "$SCRIPT" "$(fixture prt-string "$(job_with "$GOOD_STEPS" | sed -e '/^  pull_request:$/d' -e '/^  push:$/d' -e '/^    branches:/d' -e 's/^on:$/on: pull_request_target/')")"

# Rule 3: write grants that ADR 0022 forbids.
job_perms() {
  printf '%s\njobs:\n  check:\n    runs-on: ubuntu-24.04\n    permissions:\n%s\n    steps:\n      - run: "true"\n' "$HEADER" "$1"
}
run_case "job packages: write fails" 1 --contains "grants \`packages: write\`" -- \
  bash "$SCRIPT" "$(fixture packages "$(job_perms '      packages: write')")"
run_case "job id-token: write fails" 1 --contains "grants \`id-token: write\`" -- \
  bash "$SCRIPT" "$(fixture id-token "$(job_perms '      id-token: write')")"
run_case "job write-all fails" 1 --contains "write-all" -- \
  bash "$SCRIPT" "$(fixture write-all "$(printf '%s\njobs:\n  check:\n    runs-on: ubuntu-24.04\n    permissions: write-all\n    steps:\n      - run: "true"\n' "$HEADER")")"
run_case "job pull-requests: write is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture pr-write "$(job_perms '      pull-requests: write')")"

# Rule 4: registry logins.
run_case "docker/login-action fails" 1 --contains "logs in to a registry" -- \
  bash "$SCRIPT" "$(fixture login-action "$(job_with "      - uses: docker/login-action@$SHA # v3.0.0")")"
run_case "redhat-actions/podman-login fails" 1 --contains "logs in to a registry" -- \
  bash "$SCRIPT" "$(fixture podman-login-action "$(job_with "      - uses: redhat-actions/podman-login@$SHA # v1.7")")"
for cmd in "docker login ghcr.io" "podman login -u x ghcr.io" "echo x | buildah login --password-stdin quay.io" \
  "skopeo login ghcr.io" "helm registry login ghcr.io" "crane auth login ghcr.io" "oras login ghcr.io"; do
  run_case "run '$cmd' fails" 1 --contains "logs in to a container registry" -- \
    bash "$SCRIPT" "$(fixture "run-${cmd%% *}-$RANDOM" "$(job_with "      - run: $cmd")")"
done
run_case "a plain docker command passes" 0 -- \
  bash "$SCRIPT" "$(fixture docker-compose "$(job_with "      - run: docker compose up --detach --wait postgres")")"

# Rule 5: secrets.
run_case "a secrets expression fails" 1 --contains "uses a secret" -- \
  bash "$SCRIPT" "$(fixture secret-expr "$(job_with '      - run: echo "${{ secrets.TOKEN }}"')")"
run_case "a secret in env fails" 1 --contains "uses a secret" -- \
  bash "$SCRIPT" "$(fixture secret-env "$(job_with '      - run: "true"
        env:
          TOKEN: ${{ secrets.GITHUB_TOKEN }}')")"
run_case "secrets: inherit fails" 1 --contains "passes \`secrets:\`" -- \
  bash "$SCRIPT" "$(fixture secrets-inherit "$(printf '%s\njobs:\n  call:\n    uses: org/repo/.github/workflows/x.yml@%s # v1.0.0\n    secrets: inherit\n' "$HEADER" "$SHA")")"

# Rule 6: action pins.
run_case "a tag pin fails" 1 --contains "not pinned to a full commit SHA" -- \
  bash "$SCRIPT" "$(fixture tag "$(job_with "      - uses: actions/checkout@v7")")"
run_case "a short SHA fails" 1 --contains "not pinned to a full commit SHA" -- \
  bash "$SCRIPT" "$(fixture short-sha "$(job_with "      - uses: actions/checkout@${SHA:0:12} # v7.0.1")")"
run_case "a branch pin fails" 1 --contains "not pinned to a full commit SHA" -- \
  bash "$SCRIPT" "$(fixture branch "$(job_with "      - uses: actions/checkout@main")")"
run_case "a missing version comment fails" 1 --contains "has no version comment" -- \
  bash "$SCRIPT" "$(fixture no-comment "$(job_with "      - uses: actions/checkout@$SHA")")"
run_case "a non-version comment fails" 1 --contains "has no version comment" -- \
  bash "$SCRIPT" "$(fixture bad-comment "$(job_with "      - uses: actions/checkout@$SHA # pinned")")"
run_case "a docker:// tag fails" 1 --contains "not pinned by digest" -- \
  bash "$SCRIPT" "$(fixture docker-tag "$(job_with "      - uses: docker://alpine:3")")"
run_case "an unpinned reusable workflow fails" 1 --contains "not pinned to a full commit SHA" -- \
  bash "$SCRIPT" "$(fixture reusable "$(printf '%s\njobs:\n  call:\n    uses: org/repo/.github/workflows/x.yml@main\n' "$HEADER")")"
# The script resolves aliases before rule 6; the copy keeps the version comment.
run_case "an aliased step keeps its version comment" 0 -- \
  bash "$SCRIPT" "$(fixture alias-step "$(job_with "      - &checkout
        uses: actions/checkout@$SHA # v7.0.1
        with:
          persist-credentials: false
      - *checkout")")"
run_case "an aliased unpinned step fails" 1 --contains "step 2: \`actions/checkout@v7\` is not pinned" -- \
  bash "$SCRIPT" "$(fixture alias-step-tag "$(job_with "      - &checkout
        uses: actions/checkout@v7
      - *checkout")")"

# Rule 7: concurrency.
run_case "missing concurrency fails" 1 --contains "needs \`concurrency:\`" -- \
  bash "$SCRIPT" "$(fixture no-concurrency "$(job_with "$GOOD_STEPS" | sed '/^concurrency:/,/^  cancel-in-progress:/d')")"
run_case "concurrency without cancel-in-progress fails" 1 --contains "needs \`concurrency:\`" -- \
  bash "$SCRIPT" "$(fixture no-cancel "$(job_with "$GOOD_STEPS" | sed '/^  cancel-in-progress:/d')")"

# Rule 8 (ADR 0029): no filter may skip a workflow that runs a required job.
# The compliant fixtures above already keep `push: branches: [master]`.
# pr_filter LINE: a compliant workflow with LINE added under `pull_request:`.
pr_filter() {
  job_with "$GOOD_STEPS" | sed "s|^  pull_request:\$|  pull_request:\\n    $1|"
}
# push_filter LINE: the same, with LINE added under `push:`.
push_filter() {
  job_with "$GOOD_STEPS" | sed "s|^    branches: \[master\]\$|    branches: [master]\\n    $1|"
}
for filter in paths paths-ignore branches branches-ignore; do
  run_case "pull_request $filter fails" 1 --contains "trigger \`pull_request\`: has a \`$filter\` filter" -- \
    bash "$SCRIPT" "$(fixture "pr-$filter" "$(pr_filter "$filter: [\"src/**\"]")")"
done
for filter in paths paths-ignore; do
  run_case "push $filter fails" 1 --contains "trigger \`push\`: has a \`$filter\` filter" -- \
    bash "$SCRIPT" "$(fixture "push-$filter" "$(push_filter "$filter: [\"docs/**\"]")")"
done
run_case "the filter message names the required jobs" 1 --contains "runs required jobs (\`Check\`)" -- \
  bash "$SCRIPT" "$WORK/pr-paths"
run_case "push branches-ignore is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture push-branches-ignore "$(job_with "$GOOD_STEPS" | sed 's/^    branches: \[master\]$/    branches-ignore: [gh-pages]/')")"
run_case "pull_request types is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture pr-types "$(pr_filter "types: [opened, edited, reopened, synchronize]")")"
run_case "a branches filter on another trigger fails" 1 --contains "trigger \`workflow_run\`: has a \`branches\` filter" -- \
  bash "$SCRIPT" "$(fixture workflow-run "$(job_with "$GOOD_STEPS" | sed 's|^  pull_request:$|  pull_request:\n  workflow_run:\n    workflows: [CI]\n    branches: [master]|')")"
optional_only=$(printf 'on:\n  pull_request:\n    paths: ["Cargo.lock"]\n  push:\n    branches: [master]\n%s\njobs:\n  advisories:\n    name: cargo-deny check advisories\n    if: github.event_name == '"'"'schedule'"'"'\n    runs-on: ubuntu-24.04\n    steps:\n      - run: "true"\n' "$HEADER_REST")
run_case "a workflow without required jobs may filter and condition" 0 -- \
  bash "$SCRIPT" "$(fixture optional-only "$optional_only")"

# Rule 8 sees through YAML anchors and aliases, which GitHub Actions accepts.
run_case "an aliased pull_request filter fails" 1 --contains "trigger \`pull_request\`: has a \`branches\` filter" -- \
  bash "$SCRIPT" "$(fixture alias-flow "$(job_with_on 'on: {push: &f {branches: [master]}, pull_request: *f}' "$GOOD_STEPS")")"
run_case "a filter merged into pull_request fails" 1 --contains "trigger \`pull_request\`: has a \`branches\` filter" -- \
  bash "$SCRIPT" "$(fixture alias-merge "$(job_with_on 'on:
  push: &f
    branches: [master]
  pull_request:
    <<: *f
    types: [opened, synchronize]' "$GOOD_STEPS")")"

# Rule 9 (ADR 0029): no job-level `if:` on a required job but the schedule guard.
# job_if CONDITION: a compliant workflow whose required job has `if: CONDITION`.
# printf rather than sed, because conditions hold `&`, `|` and `[`.
job_if() {
  printf '%s\njobs:\n  check:\n    name: Check\n    if: %s\n    runs-on: ubuntu-24.04\n    steps:\n%s\n' \
    "$HEADER" "$1" "$GOOD_STEPS"
}
# The banned forms: other conditions, the inverted guard, and the guard joined
# to another term on either side (a guard-first or guard-last match alone must
# not pass).
for cond in "github.event_name == 'pull_request'" "false" "\${{ false }}" "github.event_name != 'push'" \
  "github.event_name == 'schedule'" "\${{ !(github.event_name != 'schedule') }}" \
  "github.event_name != 'schedule' && github.actor != 'dependabot[bot]'" \
  "\${{ github.event_name != 'schedule' }} || false" \
  "false && github.event_name != 'schedule'" \
  "github.actor != 'dependabot[bot]' && github.event_name != 'schedule'" \
  "false || github.event_name != 'schedule'" \
  "\${{ false }} || \${{ github.event_name != 'schedule' }}"; do
  run_case "job-level if: $cond fails" 1 --contains "has the job-level condition \`if: $cond\`" -- \
    bash "$SCRIPT" "$(fixture "if-$RANDOM$RANDOM" "$(job_if "$cond")")"
done
# A block scalar puts the guard on its own line; only the whole value counts.
run_case "a multi-line condition ending in the guard fails" 1 --contains "has the job-level condition \`if: false &&" -- \
  bash "$SCRIPT" "$(fixture if-multiline "$(job_if "|
      false &&
      github.event_name != 'schedule'")")"
run_case "a multi-line condition starting with the guard fails" 1 --contains "has the job-level condition \`if: github.event_name != 'schedule' || false\`" -- \
  bash "$SCRIPT" "$(fixture if-multiline-first "$(job_if "|
      github.event_name != 'schedule'
      || false")")"
# The report prints one line per violation, so the condition is put on one.
multiline_out=$(bash "$SCRIPT" "$WORK/if-multiline-first" 2>&1 || true)
run_case "a multi-line condition is reported on one line" 0 -- test "$(grep -c '^  - ' <<<"$multiline_out")" -eq 1
run_case "an aliased condition fails" 1 --contains "has the job-level condition \`if: false\`" -- \
  bash "$SCRIPT" "$(fixture if-alias "$(job_if '*skip' | sed 's/^  cancel-in-progress: true$/  cancel-in-progress: true\nenv:\n  SKIP: \&skip "false"/')")"
for cond in "github.event_name != 'schedule'" "\${{ github.event_name != 'schedule' }}" \
  "github.event_name!='schedule'" "\${{github.event_name  !=  'schedule'}}"; do
  run_case "the schedule guard $cond is allowed" 0 -- \
    bash "$SCRIPT" "$(fixture "guard-$RANDOM$RANDOM" "$(job_if "$cond")")"
done
run_case "the condition message names the job and its context" 1 --contains "job check (\`Check\`): has the job-level condition" -- \
  bash "$SCRIPT" "$(fixture if-named "$(job_if "false")")"
run_case "a job without a name is required under its id" 1 --contains "job check (\`check\`)" -- \
  bash "$SCRIPT" "$(fixture if-unnamed "$(job_if "false" | sed '/^    name: Check$/d')")"
run_case "the not-required context may have a condition" 0 -- \
  bash "$SCRIPT" "$(fixture if-optional "$(job_if "github.event_name == 'schedule'" | sed 's/^    name: Check$/    name: cargo-deny check advisories/')")"
run_case "a reusable-workflow job with a condition fails" 1 --contains "has the job-level condition" -- \
  bash "$SCRIPT" "$(fixture if-reusable "$(printf '%s\njobs:\n  call:\n    if: github.event_name == '"'"'push'"'"'\n    uses: org/repo/.github/workflows/x.yml@%s # v1.0.0\n' "$HEADER" "$SHA")")"
run_case "a step-level condition is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture step-if "$(job_with "$GOOD_STEPS
      - name: Log on failure
        if: failure()
        run: \"true\"")")"

# Rule 10 (ADR 0029 section 4): a required job needs only required jobs with
# no job-level `if:` and no `continue-on-error`.
# needs_wf DEP_NAME DEP_LINES NEEDS: a compliant workflow with two jobs:
# `dep`, named DEP_NAME, with the extra job-level lines DEP_LINES (indented by
# four spaces; may be empty), and the required job `check` (`Check`) with
# `needs: NEEDS`.
needs_wf() {
  printf '%s\njobs:\n  dep:\n    name: %s\n%s    runs-on: ubuntu-24.04\n    steps:\n      - run: "true"\n  check:\n    name: Check\n    needs: %s\n    runs-on: ubuntu-24.04\n    steps:\n      - run: "true"\n' \
    "$HEADER" "$1" "${2:+$2$'\n'}" "$3"
}
run_case "needs: on a required job (string) is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture needs-string "$(needs_wf Dep "" dep)")"
run_case "needs: on a required job (list) is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture needs-list "$(needs_wf Dep "" "[dep]")")"
run_case "needs: with continue-on-error: false on the target is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture needs-coe-false "$(needs_wf Dep "    continue-on-error: false" dep)")"
run_case "needs: on the not-required context fails" 1 \
  --contains "job check (\`Check\`): needs job dep (\`cargo-deny check advisories\`), which is not a required context" -- \
  bash "$SCRIPT" "$(fixture needs-optional "$(needs_wf "cargo-deny check advisories" "" dep)")"
run_case "needs: on a job with a condition fails" 1 \
  --contains "needs job dep (\`Dep\`), which has the job-level condition \`if: false\`" -- \
  bash "$SCRIPT" "$(fixture needs-if "$(needs_wf Dep "    if: false" dep)")"
# Rule 9 allows the schedule guard on `dep` itself, so rule 10 is the only
# violation: the guard can skip `dep`, and `check` with it.
run_case "needs: on a job with the schedule guard fails" 1 \
  --contains "needs job dep (\`Dep\`), which has the job-level condition \`if: github.event_name != 'schedule'\`" -- \
  bash "$SCRIPT" "$(fixture needs-guard "$(needs_wf Dep "    if: github.event_name != 'schedule'" dep)")"
guard_out=$(bash "$SCRIPT" "$WORK/needs-guard" 2>&1 || true)
run_case "needs: on the schedule guard is one violation" 0 -- test "$(grep -c '^  - ' <<<"$guard_out")" -eq 1
run_case "needs: on a job with continue-on-error fails" 1 \
  --contains "needs job dep (\`Dep\`), which has \`continue-on-error: true\`" -- \
  bash "$SCRIPT" "$(fixture needs-coe "$(needs_wf Dep "    continue-on-error: true" "[dep]")")"
# The not-required context with a condition: two reasons, one line each.
both_out=$(bash "$SCRIPT" "$(fixture needs-two-reasons "$(needs_wf "cargo-deny check advisories" "    if: github.event_name == 'schedule'" dep)")" 2>&1 || true)
run_case "needs: reports each reason on its own line" 0 -- \
  test "$(grep -c '^  - .*: job check (`Check`): needs job dep' <<<"$both_out")" -eq 2
run_case "needs: on a job name instead of its id fails" 1 \
  --contains "needs \`Dep\`, which is not a job id of this workflow" -- \
  bash "$SCRIPT" "$(fixture needs-name "$(needs_wf Dep "" "[Dep]")")"
run_case "needs: as a mapping fails" 1 --contains "has a \`needs:\` that is neither a job id nor a list of job ids" -- \
  bash "$SCRIPT" "$(fixture needs-map "$(needs_wf Dep "" "{dep: true}")")"
run_case "an aliased needs: list is checked" 1 \
  --contains "needs job dep (\`cargo-deny check advisories\`), which is not a required context" -- \
  bash "$SCRIPT" "$(fixture needs-alias "$(needs_wf "cargo-deny check advisories" "" "*deps" | sed 's/^  cancel-in-progress: true$/  cancel-in-progress: true\nenv:\n  DEPS: \&deps [dep]/')")"
# Every entry of a `needs:` list is checked, not only the first or the last.
# needs_many NEEDS: a compliant workflow with four jobs besides `check`
# (`Check`, with `needs: NEEDS`): `a` and `b` (`A`, `B`), required and clean;
# `adv`, the not-required context; and `guarded`, which carries only the
# schedule guard. Each of the four complies on its own (rule 9 allows the
# guard on `guarded`), so the only violations are rule 10's on `check`.
needs_many() {
  local job id name extra out="$HEADER"$'\njobs:'
  for job in "a|A|" "adv|cargo-deny check advisories|" "b|B|" "guarded|Guarded|    if: github.event_name != 'schedule'"; do
    IFS='|' read -r id name extra <<<"$job"
    out+=$'\n'"  $id:"$'\n'"    name: $name"$'\n'"${extra:+$extra$'\n'}    runs-on: ubuntu-24.04"$'\n'"    steps:"$'\n'"      - run: \"true\""
  done
  printf '%s\n  check:\n    name: Check\n    needs: %s\n    runs-on: ubuntu-24.04\n    steps:\n      - run: "true"\n' "$out" "$1"
}
run_case "needs: on several clean required jobs is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture needs-many-clean "$(needs_many "[a, b]")")"
# The not-required context first, in the middle and last of the list.
for list in "[adv, a, b]" "[a, adv, b]" "[a, b, adv]"; do
  run_case "needs: $list fails on its not-required entry" 1 \
    --contains "job check (\`Check\`): needs job adv (\`cargo-deny check advisories\`), which is not a required context" -- \
    bash "$SCRIPT" "$(fixture "needs-many-${list//[^a-z]/}" "$(needs_many "$list")")"
done
# Two bad entries around a clean one: both are reported, one line each, and
# the clean entry is not.
two_bad_out=$(bash "$SCRIPT" "$(fixture needs-many-two-bad "$(needs_many "[adv, a, guarded]")")" 2>&1 || true)
run_case "needs: reports every bad entry of a list" 0 -- \
  test "$(grep -cE '^  - .*: job check \(`Check`\): needs job (adv \(`cargo-deny check advisories`\), which is not a required context|guarded \(`Guarded`\), which has the job-level condition)' <<<"$two_bad_out")-$(grep -c '^  - ' <<<"$two_bad_out")" = "2-2"
# A chain: check needs mid, which is required and clean, and mid needs the
# not-required context. Rule 10 holds for mid as well, so the chain fails.
chain=$(printf '%s\njobs:\n  adv:\n    name: cargo-deny check advisories\n    runs-on: ubuntu-24.04\n    steps:\n      - run: "true"\n  mid:\n    name: Mid\n    needs: adv\n    runs-on: ubuntu-24.04\n    steps:\n      - run: "true"\n  check:\n    name: Check\n    needs: [mid]\n    runs-on: ubuntu-24.04\n    steps:\n      - run: "true"\n' "$HEADER")
run_case "a chain of needs: ending in the not-required context fails" 1 \
  --contains "job mid (\`Mid\`): needs job adv (\`cargo-deny check advisories\`), which is not a required context" -- \
  bash "$SCRIPT" "$(fixture needs-chain "$chain")"
run_case "the not-required context may need anything" 0 -- \
  bash "$SCRIPT" "$(fixture needs-from-optional "$(needs_wf Dep "    if: github.event_name != 'schedule'" dep | sed 's/^    name: Check$/    name: cargo-deny check advisories/')")"
run_case "a reusable-workflow job's needs: is checked" 1 \
  --contains "job call (\`call\`): needs job dep (\`Dep\`), which has \`continue-on-error: true\`" -- \
  bash "$SCRIPT" "$(fixture needs-reusable "$(printf '%s\njobs:\n  dep:\n    name: Dep\n    continue-on-error: true\n    runs-on: ubuntu-24.04\n    steps:\n      - run: "true"\n  call:\n    needs: dep\n    uses: org/repo/.github/workflows/x.yml@%s # v1.0.0\n' "$HEADER" "$SHA")")"

# Rule 11 (ADR 0029 section 4): no continue-on-error, other than the literal
# false, on a required job or its steps.
# job_coe VALUE: a compliant workflow whose required job has
# `continue-on-error: VALUE`.
job_coe() {
  printf '%s\njobs:\n  check:\n    name: Check\n    continue-on-error: %s\n    runs-on: ubuntu-24.04\n    steps:\n%s\n' \
    "$HEADER" "$1" "$GOOD_STEPS"
}
# step_coe VALUE: the same with `continue-on-error: VALUE` on a named step.
step_coe() {
  job_with "$GOOD_STEPS
      - name: Lint
        run: \"true\"
        continue-on-error: $1"
}
# A quoted 'false' is a string, not the allowed boolean, and is reported in
# double quotes so that the message does not read as the allowed value.
for value in true "\${{ matrix.experimental }}" "'false'"; do
  shown=$value
  [[ $value == "'false'" ]] && shown='"false"'
  run_case "job-level continue-on-error: $value fails" 1 \
    --contains "job check (\`Check\`): has \`continue-on-error: $shown\`" -- \
    bash "$SCRIPT" "$(fixture "coe-$RANDOM$RANDOM" "$(job_coe "$value")")"
done
# A block scalar spreads an expression over several lines; the report prints
# one line per violation, so the value is put on one, as for rule 9.
run_case "a multi-line continue-on-error expression fails" 1 \
  --contains "job check (\`Check\`): has \`continue-on-error: \${{ matrix.x == 1 }}\`, which lets" -- \
  bash "$SCRIPT" "$(fixture coe-multiline "$(job_coe "|
      \${{ matrix.x ==
      1 }}")")"
coe_multiline_out=$(bash "$SCRIPT" "$WORK/coe-multiline" 2>&1 || true)
run_case "a multi-line continue-on-error is reported on one line" 0 -- \
  test "$(grep -c '^  - ' <<<"$coe_multiline_out")" -eq 1
run_case "job-level continue-on-error: false is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture coe-false "$(job_coe false)")"
run_case "step-level continue-on-error: true fails" 1 \
  --contains "job check (\`Check\`), step 5 (Lint): has \`continue-on-error: true\`" -- \
  bash "$SCRIPT" "$(fixture step-coe "$(step_coe true)")"
run_case "an unnamed step's continue-on-error fails" 1 --contains "job check (\`Check\`), step 1: has \`continue-on-error: true\`" -- \
  bash "$SCRIPT" "$(fixture step-coe-unnamed "$(job_with '      - run: "true"
        continue-on-error: true')")"
run_case "step-level continue-on-error: false is allowed" 0 -- \
  bash "$SCRIPT" "$(fixture step-coe-false "$(step_coe false)")"
run_case "an aliased continue-on-error fails" 1 --contains "has \`continue-on-error: true\`" -- \
  bash "$SCRIPT" "$(fixture coe-alias "$(job_coe '*soft' | sed 's/^  cancel-in-progress: true$/  cancel-in-progress: true\nenv:\n  SOFT: \&soft true/')")"
run_case "the not-required context may continue on error" 0 -- \
  bash "$SCRIPT" "$(fixture coe-optional "$(job_coe true | sed -e 's/^    name: Check$/    name: cargo-deny check advisories/' \
    -e 's/^        with:$/        continue-on-error: true\n        with:/')")"

# Broken files and several problems at once.
run_case "invalid YAML fails" 1 --contains "Fix the YAML syntax" -- \
  bash "$SCRIPT" "$(fixture invalid 'on: [push')"
run_case "missing jobs fail" 1 --contains "\`jobs\` is missing" -- bash "$SCRIPT" "$(fixture no-jobs "$HEADER")"
multi=$(fixture multi "$(job_with "      - uses: actions/checkout@v7
      - run: docker login ghcr.io" | sed 's/^  contents: read/  contents: write/')")
multi_out=$(bash "$SCRIPT" "$multi" 2>&1 || true)
run_case "every violation is reported" 0 -- test "$(grep -c '^  - ' <<<"$multi_out")" -eq 3
mkdir -p "$WORK/yaml-ext"
cp "$WORK/tag/workflow.yml" "$WORK/yaml-ext/workflow.yaml"
run_case "*.yaml files are checked too" 1 --contains "workflow.yaml" -- bash "$SCRIPT" "$WORK/yaml-ext"

# Usage errors.
mkdir -p "$WORK/empty"
run_case "a directory without workflows is a usage error" 2 --contains "no *.yml" -- bash "$SCRIPT" "$WORK/empty"
run_case "a missing directory is a usage error" 2 --contains "does not exist" -- bash "$SCRIPT" "$WORK/nope"

# The real job names are exactly ADR 0022's branch-protection contexts.
adr_jobs=$(sed -n '/^### 3\. PR checks/,/^### 4\./p' "$ADR" | sed -n 's/^| `\([^`]*\)` |.*/\1/p' | sort)
real_jobs=$(yq -N '.jobs | to_entries | .[] | (.value.name // .key)' "$ROOT"/.github/workflows/*.yml | sort)
run_case "ADR 0022 lists 16 contexts" 0 -- test "$(wc -l <<<"$adr_jobs")" -eq 16
if [[ $adr_jobs == "$real_jobs" ]]; then
  run_case "job names equal ADR 0022's contexts" 0 -- true
else
  run_case "job names equal ADR 0022's contexts" 0 -- \
    bash -c 'echo "ADR only / workflows only:"; diff <(printf "%s\n" "$1") <(printf "%s\n" "$2"); exit 1' _ "$adr_jobs" "$real_jobs"
fi

# Rules 8 and 9 treat a job as required exactly when ADR 0022's table says
# "yes": one fixture has a job for each of the table's contexts, each with a
# condition that skips it on some pull requests, and the script must flag the
# required ones and only those. A context added to or dropped from the
# script's NOT_REQUIRED_CONTEXTS without the table, or the other way round,
# fails here. The Required column is read from ADR 0022's own table. An ADR
# that amends that column leaves ADR 0022's body as it is (ADR 0001 section
# 6), so the PR that adds it also changes these lines to read the column as
# amended (ADR 0029 section 2).
adr_required=$(sed -n '/^### 3\. PR checks/,/^### 4\./p' "$ADR" | sed -n 's/^| `\([^`]*\)` |.*| yes |$/\1/p' | sort)
adr_optional=$(sed -n '/^### 3\. PR checks/,/^### 4\./p' "$ADR" | sed -n 's/^| `\([^`]*\)` |.*| no |$/\1/p' | sort)
run_case "ADR 0022 marks 15 contexts required and 1 not" 0 -- \
  test "$(wc -l <<<"$adr_required")-$(wc -l <<<"$adr_optional")" = "15-1"
every_context="$HEADER
jobs:"
n=0
while IFS= read -r ctx; do
  n=$((n + 1))
  every_context+="
  job$n:
    name: \"$ctx\"
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-24.04
    steps:
      - run: \"true\""
done <<<"$adr_jobs"
every_out=$(bash "$SCRIPT" "$(fixture every-context "$every_context")" 2>&1 || true)
flagged=$(sed -n 's/.*: job job[0-9]* (`\(.*\)`): has the job-level condition.*/\1/p' <<<"$every_out" | sort)
if [[ $flagged == "$adr_required" ]]; then
  run_case "rule 9 covers exactly ADR 0022's required contexts" 0 -- true
else
  run_case "rule 9 covers exactly ADR 0022's required contexts" 0 -- \
    bash -c 'echo "required in ADR 0022 / flagged by the script:"; diff <(printf "%s\n" "$1") <(printf "%s\n" "$2"); exit 1' _ "$adr_required" "$flagged"
fi
# The same for rule 11, with `continue-on-error: true` on every job.
every_coe=$(sed "s/^    if: github.event_name == 'pull_request'\$/    continue-on-error: true/" <<<"$every_context")
every_coe_out=$(bash "$SCRIPT" "$(fixture every-context-coe "$every_coe")" 2>&1 || true)
flagged_coe=$(sed -n 's/.*: job job[0-9]* (`\(.*\)`): has `continue-on-error: true`.*/\1/p' <<<"$every_coe_out" | sort)
if [[ $flagged_coe == "$adr_required" ]]; then
  run_case "rule 11 covers exactly ADR 0022's required contexts" 0 -- true
else
  run_case "rule 11 covers exactly ADR 0022's required contexts" 0 -- \
    bash -c 'echo "required in ADR 0022 / flagged by the script:"; diff <(printf "%s\n" "$1") <(printf "%s\n" "$2"); exit 1' _ "$adr_required" "$flagged_coe"
fi

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
