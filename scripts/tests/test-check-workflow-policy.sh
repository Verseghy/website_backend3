#!/usr/bin/env bash
# Tests for scripts/ci/check-workflow-policy.sh (ADR 0022 sections 1 and 3).
#
# Each fixture workflow breaks one rule; a compliant fixture and the real
# workflows in .github/workflows/ must pass. The last case checks that the job
# names across the real workflows are exactly the branch-protection contexts
# in ADR 0022 section 3's table, so a renamed job fails here before it
# silently drops out of branch protection. Run from the repository root:
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
HEADER='on:
  pull_request:
  push:
    branches: [master]
permissions:
  contents: read
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true'

# fixture NAME CONTENT: write CONTENT as the only workflow of a fresh
# directory and print that directory.
fixture() {
  local dir="$WORK/$1"
  mkdir -p "$dir"
  printf '%s\n' "$2" >"$dir/workflow.yml"
  printf '%s' "$dir"
}

# job_with STEPS: print a compliant workflow whose single job has STEPS
# (YAML list items indented by six spaces).
job_with() {
  printf '%s\njobs:\n  check:\n    name: Check\n    runs-on: ubuntu-24.04\n    steps:\n%s\n' "$HEADER" "$1"
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

# Rule 7: concurrency.
run_case "missing concurrency fails" 1 --contains "needs \`concurrency:\`" -- \
  bash "$SCRIPT" "$(fixture no-concurrency "$(job_with "$GOOD_STEPS" | sed '/^concurrency:/,/^  cancel-in-progress:/d')")"
run_case "concurrency without cancel-in-progress fails" 1 --contains "needs \`concurrency:\`" -- \
  bash "$SCRIPT" "$(fixture no-cancel "$(job_with "$GOOD_STEPS" | sed '/^  cancel-in-progress:/d')")"

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

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
