#!/usr/bin/env bash
# Tests for scripts/ci/check-contract-drift.sh (ADR 0005 section 4).
#
# Uses a throwaway git repository with committed bindings and snapshots, then
# changes them the ways a stale PR would: an edited binding, a new untracked
# binding, an edited snapshot, an untracked pending snapshot. Changes outside
# the contract paths must pass. Run from the repository root:
#   bash scripts/tests/test-check-contract-drift.sh
# No network access is needed.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT="$ROOT/scripts/ci/check-contract-drift.sh"

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
REPO="$WORK/repo"

# git_fixture: run git in the fixture repository, isolated from the user's
# and the system's configuration (no hooks, no signing, fixed identity).
git_fixture() {
  GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
    GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid \
    GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid \
    git -C "$REPO" -c commit.gpgsign=false -c init.defaultBranch=master "$@"
}

# reset_fixture: recreate the repository with a clean committed contract.
reset_fixture() {
  rm -rf "$REPO"
  mkdir -p "$REPO/admin/src/bindings" "$REPO/tests/requests/snapshots" "$REPO/src/domain/snapshots" "$REPO/src"
  printf 'export type Label = { id: number };\n' >"$REPO/admin/src/bindings/Label.ts"
  printf -- '---\nsource: tests/requests/openapi.rs\n---\n{"openapi":"3.1.0"}\n' >"$REPO/tests/requests/snapshots/openapi__document.snap"
  printf -- '---\nsource: src/domain/slug.rs\n---\nalma\n' >"$REPO/src/domain/snapshots/slug__basic.snap"
  printf 'pub fn f() {}\n' >"$REPO/src/lib.rs"
  git_fixture init -q
  git_fixture add -A
  git_fixture commit -q -m base
}

reset_fixture
run_case "a clean tree passes" 0 --contains "Contract OK" -- bash "$SCRIPT" --repo "$REPO"

printf 'pub fn g() {}\n' >>"$REPO/src/lib.rs"
printf 'notes\n' >"$REPO/notes.md"
run_case "changes outside the contract paths pass" 0 -- bash "$SCRIPT" --repo "$REPO"

reset_fixture
printf 'export type Label = { id: number, name: string };\n' >"$REPO/admin/src/bindings/Label.ts"
run_case "an edited binding fails" 1 --contains "admin/src/bindings/Label.ts" -- bash "$SCRIPT" --repo "$REPO"
run_case "the failure explains the fix" 1 --contains "cargo insta review" -- bash "$SCRIPT" --repo "$REPO"

reset_fixture
printf 'export type Author = { id: number };\n' >"$REPO/admin/src/bindings/Author.ts"
run_case "a new untracked binding fails" 1 --contains "?? admin/src/bindings/Author.ts" -- bash "$SCRIPT" --repo "$REPO"

reset_fixture
mkdir -p "$REPO/admin/src/bindings/nested"
printf 'export type X = string;\n' >"$REPO/admin/src/bindings/nested/X.ts"
run_case "an untracked binding in a subdirectory fails" 1 --contains "admin/src/bindings/nested/X.ts" -- bash "$SCRIPT" --repo "$REPO"

reset_fixture
rm "$REPO/admin/src/bindings/Label.ts"
run_case "a deleted binding fails" 1 --contains "admin/src/bindings/Label.ts" -- bash "$SCRIPT" --repo "$REPO"

reset_fixture
printf -- '---\nsource: tests/requests/openapi.rs\n---\n{"openapi":"3.1.1"}\n' >"$REPO/tests/requests/snapshots/openapi__document.snap"
run_case "an edited OpenAPI snapshot fails" 1 --contains "openapi__document.snap" -- bash "$SCRIPT" --repo "$REPO"

reset_fixture
printf 'pending\n' >"$REPO/src/domain/snapshots/slug__basic.snap.new"
run_case "an untracked pending snapshot fails" 1 --contains "slug__basic.snap.new" -- bash "$SCRIPT" --repo "$REPO"

reset_fixture
run_case "it works from a subdirectory of the repository" 0 -- bash "$SCRIPT" --repo "$REPO/src"

mkdir -p "$WORK/not-git"
run_case "a directory outside git is a usage error" 2 --contains "not inside a git work tree" -- \
  env GIT_CEILING_DIRECTORIES="$WORK" bash "$SCRIPT" --repo "$WORK/not-git"
run_case "an unknown option is a usage error" 2 -- bash "$SCRIPT" --bogus

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
