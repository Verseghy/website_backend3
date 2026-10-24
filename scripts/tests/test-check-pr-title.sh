#!/usr/bin/env bash
# Tests for scripts/ci/check-pr-title.sh (ADR 0012 section 3).
#
# Covers accepted and rejected sample titles, the 90-character limit (counted
# in characters, not bytes), and that the script's regex is exactly the one
# printed in ADR 0012. Run from the repository root:
#   bash scripts/tests/test-check-pr-title.sh
# No network access is needed.
# Single-quoted fixtures and sed/jq programs hold a literal `$` on purpose.
# shellcheck disable=SC2016
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT="$ROOT/scripts/ci/check-pr-title.sh"
ADR="$ROOT/docs/design/adr/0012-delivery-review-commits-and-licensing.md"

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

# title_of_length N: print a valid title of exactly N characters whose
# subject is padded with 'ő' (a two-byte UTF-8 character), so the test fails
# if the script counted bytes instead of characters.
title_of_length() {
  # The prefix is 10 ASCII characters; count the padding with a loop counter
  # rather than ${#title}, which counts bytes under the C locale.
  local title='fix(api): ' i
  for ((i = 10; i < $1; i++)); do
    title+='ő'
  done
  printf '%s' "$title"
}

# The regex must be byte-identical to the one in ADR 0012 section 3.
adr_regex=$(sed -n 's/^.*The check regex is `\([^`]*\)`.*$/\1/p' "$ADR")
run_case "ADR 0012 states a check regex" 0 -- test -n "$adr_regex"
run_case "script regex equals the ADR 0012 regex" 0 --contains "$adr_regex" -- bash "$SCRIPT" --print-regex
script_regex=$(bash "$SCRIPT" --print-regex)
run_case "printed regex is exactly the ADR's" 0 -- test "$script_regex" = "$adr_regex"

# Accepted titles: the ADR's examples, Dependabot's forms and edge cases.
accepted=(
  'feat(db,api,admin): add labels with trash and history'
  'fix(api): keep backdated posts in keyset paging'
  'build(api): bump loco-rs to 1.3.0'
  'build(api,admin,deploy,docs): bootstrap the Loco app, admin, guardrails and CI'
  'feat(api)!: drop the legacy cursor parameter'
  'revert(admin): restore the old menu editor'
  'build(admin): bump vite from 8.0.1 to 8.0.2 in /admin'
  'ci(deploy): bump actions/checkout from 7.0.0 to 7.0.1'
  'docs(docs): fix a typo in ADR 0019'
  'test(import): cover NULL authors'
  'chore(db): rename an index'
  'perf(api): cache the menu tree for 60 seconds'
  'refactor(api): extract the slug helper'
  'style(admin): format the data provider'
  'fix(api): javítsa a hibás ékezetes keresést'
)
for t in "${accepted[@]}"; do
  run_case "accepts: $t" 0 -- bash "$SCRIPT" "$t"
done
run_case "accepts exactly 90 characters (multi-byte subject)" 0 -- bash "$SCRIPT" "$(title_of_length 90)"

# Rejected titles, each with the hint the author should see.
run_case "rejects 91 characters" 1 --contains "91 characters" -- bash "$SCRIPT" "$(title_of_length 91)"
run_case "rejects a missing scope" 1 --contains "A scope is required" -- bash "$SCRIPT" 'feat: add labels'
run_case "rejects an unknown type" 1 --contains "Type 'feature' is not allowed" -- bash "$SCRIPT" 'feature(api): add labels'
run_case "rejects an upper-case type" 1 --contains "Type 'Feat' is not allowed" -- bash "$SCRIPT" 'Feat(api): add labels'
run_case "rejects an unknown scope" 1 --contains "Scope 'web' is not allowed" -- bash "$SCRIPT" 'feat(web): add labels'
run_case "rejects a space after a scope comma" 1 --contains "Scope ' admin' is not allowed" -- bash "$SCRIPT" 'feat(api, admin): add labels'
run_case "rejects a trailing scope comma" 1 --contains "stray comma" -- bash "$SCRIPT" 'feat(api,): add labels'
run_case "rejects an empty scope" 1 --contains "empty" -- bash "$SCRIPT" 'feat(): add labels'
run_case "rejects a space before the colon" 1 --contains "space before the colon" -- bash "$SCRIPT" 'feat(api) : add labels'
run_case "rejects a missing space after the colon" 1 --contains "one space after the colon" -- bash "$SCRIPT" 'feat(api):add labels'
run_case "rejects two spaces after the colon" 1 --contains "non-space character" -- bash "$SCRIPT" 'feat(api):  add labels'
run_case "rejects an empty subject" 1 -- bash "$SCRIPT" 'feat(api): '
run_case "rejects a trailing period" 1 --contains "ends with a period" -- bash "$SCRIPT" 'fix(api): keep backdated posts in keyset paging.'
run_case "rejects GitHub's revert title" 1 --contains "revert(<scope>)" -- bash "$SCRIPT" 'Revert "feat(api): add labels"'
run_case "rejects a plain sentence" 1 --contains "no '<type>(<scope>): ' prefix" -- bash "$SCRIPT" 'Add labels with trash and history'
run_case "rejects an empty title" 1 --contains "empty" -- bash "$SCRIPT" ''
run_case "rejects a second line" 1 --contains "line break" -- bash "$SCRIPT" $'feat(api): add labels\nsecond line'
run_case "rejects a tab" 1 --contains "control character" -- bash "$SCRIPT" $'feat(api): add\tlabels'
run_case "failure output shows the format" 1 --contains "Format:  <type>(<scope>" -- bash "$SCRIPT" 'nope'

# Usage errors.
run_case "no argument is a usage error" 2 --contains "exactly one argument" -- bash "$SCRIPT"
run_case "two arguments are a usage error" 2 -- bash "$SCRIPT" 'feat(api): a' 'extra'
run_case "--help exits 0" 0 --contains "Usage:" -- bash "$SCRIPT" --help

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
