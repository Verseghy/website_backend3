#!/usr/bin/env bash
# Tests for scripts/ci/check-rich-text-manifest.sh (docs/design/editor.md
# §3.5, §13.6 "Manifest").
#
# Starts from the committed admin/src/rich-text/policy.json, which must pass,
# and writes changed copies of it with jq: a forbidden tag in the tag list or
# in the element map (in any case), a forbidden tag missing from the
# manifest's own list, members of the wrong shape (a `forbidden_tags` entry
# that is not a string included), a broken file and a missing one. Run from
# the repository root:
#   bash scripts/tests/test-check-rich-text-manifest.sh
# No network access is needed.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT="$ROOT/scripts/ci/check-rich-text-manifest.sh"
COMMITTED="$ROOT/admin/src/rich-text/policy.json"

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

# variant NAME JQ_FILTER: a copy of the committed manifest changed by the
# filter, at $WORK/NAME.json.
variant() {
  jq "$2" "$COMMITTED" >"$WORK/$1.json"
}

run_case "the committed manifest passes" 0 --contains "Rich-text manifest OK" -- bash "$SCRIPT" --file "$COMMITTED"
run_case "the default path is the admin copy" 0 --contains "admin/src/rich-text/policy.json" -- \
  bash -c 'cd "$1" && bash "$2"' _ "$ROOT" "$SCRIPT"

variant script-tag '.tags += ["script"]'
run_case "a forbidden tag in the tag list fails" 1 --contains 'tags holds the forbidden tag `script`' -- \
  bash "$SCRIPT" --file "$WORK/script-tag.json"
run_case "the failure says how to fix it" 1 --contains "cargo loco task rich_text_manifest" -- \
  bash "$SCRIPT" --file "$WORK/script-tag.json"

variant upper-element '.elements.NoScript = {"attributes": [], "style": null}'
run_case "a forbidden tag among the elements fails in any case" 1 --contains 'elements holds the forbidden tag `NoScript`' -- \
  bash "$SCRIPT" --file "$WORK/upper-element.json"

variant every-tag '.tags += ["noscript","template","style","script","xmp","noembed","noframes","plaintext","textarea","title","listing","select","option"]'
for tag in noscript template style script xmp noembed noframes plaintext textarea title listing select option; do
  run_case "the forbidden tag $tag is reported" 1 --contains "\`$tag\`" -- bash "$SCRIPT" --file "$WORK/every-tag.json"
done

variant allowed-extra '.tags += ["optgroup", "scripts", "div"]'
run_case "names that only resemble a forbidden tag pass" 0 -- bash "$SCRIPT" --file "$WORK/allowed-extra.json"

variant unlisted '.forbidden_tags -= ["xmp"]'
run_case "a forbidden tag missing from forbidden_tags fails" 1 --contains 'forbidden_tags does not list `xmp`' -- \
  bash "$SCRIPT" --file "$WORK/unlisted.json"

variant tags-object '.tags = {"p": true}'
run_case "a tag list that is not an array fails" 1 --contains '`tags` is not an array of strings' -- \
  bash "$SCRIPT" --file "$WORK/tags-object.json"

variant no-elements 'del(.elements)'
run_case "a manifest without elements fails" 1 --contains '`elements` is not an object' -- \
  bash "$SCRIPT" --file "$WORK/no-elements.json"

variant forbidden-number '.forbidden_tags += [1]'
run_case "a forbidden_tags entry that is not a string fails" 1 --contains '`forbidden_tags` is not an array of strings' -- \
  bash "$SCRIPT" --file "$WORK/forbidden-number.json"

variant forbidden-object '.forbidden_tags = {"script": true}'
run_case "a forbidden_tags that is not an array fails" 1 --contains '`forbidden_tags` is not an array of strings' -- \
  bash "$SCRIPT" --file "$WORK/forbidden-object.json"

printf '{"tags": [' >"$WORK/broken.json"
run_case "a file that is not JSON fails" 1 --contains "is not valid JSON" -- bash "$SCRIPT" --file "$WORK/broken.json"

printf '[]\n' >"$WORK/array.json"
run_case "a JSON array fails" 1 --contains "not a JSON object" -- bash "$SCRIPT" --file "$WORK/array.json"

run_case "a missing file fails" 1 --contains "does not exist" -- bash "$SCRIPT" --file "$WORK/missing.json"
run_case "an unknown option is a usage error" 2 -- bash "$SCRIPT" --bogus
run_case "--file without a path is a usage error" 2 -- bash "$SCRIPT" --file

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
