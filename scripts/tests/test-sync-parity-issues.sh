#!/usr/bin/env bash
#
# test-sync-parity-issues.sh - tests for scripts/sync-parity-issues.sh.
#
# WHY IT EXISTS
#   ADR 0019 section 1 requires the sync script to be tested against a stub gh
#   on PATH: a first run, a second run with no change, a changed disposition,
#   an added ID, --closed and --dry-run. This file runs those cases and the
#   argument, board and parity-file failures, with no network and no token.
#
# HOW IT WORKS
#   - scripts/tests/stubs/gh is put first on PATH. It keeps a repository and a
#     project in JSON files under a temporary directory and logs every call as
#     READ or WRITE, so a case can run the script several times and count the
#     writes each run made.
#   - Most cases use scripts/tests/fixtures/parity-small.md, or a copy that the
#     case changes. One case only parses the real docs/design/parity.md, in a
#     dry run against an empty stub, and checks that it finds as many IDs as
#     the Total of the file's Counts table.
#   - Every case prints PASS or FAIL with the reasons. The exit status is 1 if
#     any case failed.
#
# USAGE (from the repository root)
#   bash scripts/tests/test-sync-parity-issues.sh

# No -e: a failed assertion records the failure and the runner goes on.
set -uo pipefail

TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$TESTS_DIR/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/sync-parity-issues.sh"
STUBS_DIR="$TESTS_DIR/stubs"
FIXTURE="$TESTS_DIR/fixtures/parity-small.md"
readonly TESTS_DIR REPO_ROOT SCRIPT STUBS_DIR FIXTURE

readonly REPO="Verseghy/website_backend3"
readonly PROJECT=7
readonly PR_MERGED="https://github.com/Verseghy/website_backend3/pull/1"
readonly PR_MERGED_K8S="https://github.com/Verseghy/website_k8s/pull/50"
readonly PR_OPEN="https://github.com/Verseghy/website_backend3/pull/2"
readonly BLOB="https://github.com/Verseghy/website_backend3/blob/master"

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/test-sync-parity.XXXXXX")"
trap 'rm -rf "$TMP_ROOT"' EXIT

PASSED=0
FAILED=0
CASE_ERRORS=""
CASE_COUNT=0
# Set by new_state for each case:
GH_STUB_DIR=""  # stub state directory
PARITY=""       # the case's own copy of the fixture
OUT=""          # stdout of the last run
ERR=""          # stderr of the last run
STATUS=0        # exit status of the last run

# ---------------------------------------------------------------------------
# Setup and running.

# new_state: create an empty stub state for the current case (no label, no
# issues, a project with the Status, Disposition, ADR and Area fields and no
# items) and copy the fixture to $PARITY. No parameters.
new_state() {
  CASE_COUNT=$((CASE_COUNT + 1))
  GH_STUB_DIR="$TMP_ROOT/case-$CASE_COUNT"
  mkdir -p "$GH_STUB_DIR"
  PARITY="$GH_STUB_DIR/parity.md"
  OUT="$GH_STUB_DIR/stdout"
  ERR="$GH_STUB_DIR/stderr"
  cp "$FIXTURE" "$PARITY"
  printf '{"nameWithOwner": "%s", "next_number": 2}\n' "$REPO" >"$GH_STUB_DIR/repo.json"
  printf '[]\n' >"$GH_STUB_DIR/labels.json"
  printf '[]\n' >"$GH_STUB_DIR/issues.json"
  printf '{"%s": "MERGED", "%s": "MERGED", "%s": "OPEN"}\n' "$PR_MERGED" "$PR_MERGED_K8S" "$PR_OPEN" >"$GH_STUB_DIR/prs.json"
  cat >"$GH_STUB_DIR/project.json" <<'EOF'
{ "id": "PVT_stub", "number": 7, "owner": "Verseghy", "title": "website_backend3 parity",
  "fields": [
    { "id": "PVTF_title", "name": "Title", "dataType": "TITLE" },
    { "id": "PVTSSF_status", "name": "Status", "dataType": "SINGLE_SELECT",
      "options": [ { "id": "s_todo", "name": "Todo" }, { "id": "s_prog", "name": "In progress" }, { "id": "s_done", "name": "Done" } ] },
    { "id": "PVTSSF_disp", "name": "Disposition", "dataType": "SINGLE_SELECT",
      "options": [ { "id": "o_port", "name": "port" }, { "id": "o_fix", "name": "port-with-fix" }, { "id": "o_repl", "name": "replaced" },
                   { "id": "o_ret", "name": "retired" }, { "id": "o_new", "name": "new" } ] },
    { "id": "PVTF_adr", "name": "ADR", "dataType": "TEXT" },
    { "id": "PVTF_area", "name": "Area", "dataType": "TEXT" } ],
  "items": [] }
EOF
}

# run_script ARGS...: run the sync script with the stub first on PATH, after
# emptying the call log. Sets OUT, ERR (files) and STATUS.
#   ARGS  the script's arguments
run_script() {
  : >"$GH_STUB_DIR/calls.log"
  PATH="$STUBS_DIR:$PATH" GH_STUB_DIR="$GH_STUB_DIR" bash "$SCRIPT" "$@" >"$OUT" 2>"$ERR"
  STATUS=$?
}

# sync ARGS...: run_script with --repo, --project and the case's parity file.
#   ARGS  extra arguments (--closed, --dry-run, ...)
sync() {
  run_script --repo "$REPO" --project "$PROJECT" --parity-file "$PARITY" "$@"
}

# edit_parity AWK_PROGRAM: rewrite the case's parity file with an awk program.
#   AWK_PROGRAM  program that prints the new file
edit_parity() {
  awk "$1" "$PARITY" >"$PARITY.new" && mv "$PARITY.new" "$PARITY"
}

# edit_state FILE JQ_ARGS...: rewrite a stub state file with jq, to simulate
# a change made on GitHub by hand.
#   FILE     state file name (issues.json, project.json, ...)
#   JQ_ARGS  jq options (such as --arg) and the filter that makes the change
edit_state() {
  local file="$GH_STUB_DIR/$1"
  shift
  jq "$@" "$file" >"$file.new" && mv "$file.new" "$file"
}

# issue ID FILTER: print FILTER applied to the issue whose title starts with [ID].
#   ID      parity ID
#   FILTER  jq filter on the issue object, for example .state
issue() {
  jq -r --arg id "$1" ".[] | select(.title | startswith(\"[\" + \$id + \"] \")) | $2" "$GH_STUB_DIR/issues.json"
}

# board_value ID FIELD: print the board value (text or option name) of field
# FIELD on the item of ID's issue, or nothing.
#   ID     parity ID
#   FIELD  field name (Disposition, ADR, Area)
board_value() {
  local url
  url="$(issue "$1" .url)"
  jq -r --arg u "$url" --arg f "$2" \
    '(.fields[] | select(.name == $f) | .id) as $fid | .items[] | select(.issue_url == $u) | .values[$fid] | (.text // .name) // empty' \
    "$GH_STUB_DIR/project.json"
}

# ---------------------------------------------------------------------------
# Assertions. Each records a reason in CASE_ERRORS when it fails.

# fail_case REASON: record a failed assertion of the current case.
#   REASON  what was expected and what happened
fail_case() {
  CASE_ERRORS+="    - $1"$'\n'
}

# expect_status CODE: the last run exited with CODE.
#   CODE  expected exit status
expect_status() {
  [[ "$STATUS" == "$1" ]] || fail_case "exit status $STATUS, expected $1; stderr: $(head -c 600 "$ERR")"
}

# expect_eq ACTUAL EXPECTED WHAT: two strings are equal.
#   ACTUAL    the value found
#   EXPECTED  the value wanted
#   WHAT      what the value is, for the message
expect_eq() {
  [[ "$1" == "$2" ]] || fail_case "$3: got '$1', expected '$2'"
}

# expect_contains FILE TEXT: FILE contains TEXT (a fixed string).
#   FILE  file to search (for example "$OUT" or "$ERR")
#   TEXT  text that must appear
expect_contains() {
  grep -qF -- "$2" "$1" || fail_case "${1##*/} does not contain: $2"
}

# expect_text_contains VALUE TEXT: the string VALUE contains TEXT.
#   VALUE  the string to search
#   TEXT   text that must appear
expect_text_contains() {
  [[ "$1" == *"$2"* ]] || fail_case "text does not contain: $2"
}

# expect_text_lacks VALUE TEXT: the string VALUE does not contain TEXT.
#   VALUE  the string to search
#   TEXT   text that must not appear
expect_text_lacks() {
  [[ "$1" != *"$2"* ]] || fail_case "text unexpectedly contains: $2"
}

# expect_writes N: the last run made exactly N write calls.
#   N  expected number of WRITE lines in the call log
expect_writes() {
  local count
  count="$(grep -c '^WRITE' "$GH_STUB_DIR/calls.log")"
  [[ "$count" == "$1" ]] || fail_case "$count write calls, expected $1:"$'\n'"$(grep '^WRITE' "$GH_STUB_DIR/calls.log" | head -20 | sed 's/^/        /')"
}

# expect_calls PATTERN N: exactly N logged calls match the extended regex PATTERN.
#   PATTERN  grep -E pattern over the call log
#   N        expected number of matching calls
expect_calls() {
  local count
  count="$(grep -cE -- "$1" "$GH_STUB_DIR/calls.log")"
  [[ "$count" == "$2" ]] || fail_case "$count calls match /$1/, expected $2"
}

# run_case NAME FUNCTION: run one test case and print PASS or FAIL.
#   NAME      description printed with the result
#   FUNCTION  the function that sets up, runs and asserts
run_case() {
  CASE_ERRORS=""
  "$2"
  if [[ -z "$CASE_ERRORS" ]]; then
    PASSED=$((PASSED + 1))
    printf 'PASS  %s\n' "$1"
  else
    FAILED=$((FAILED + 1))
    printf 'FAIL  %s\n%s' "$1" "$CASE_ERRORS"
  fi
}

# ---------------------------------------------------------------------------
# Cases: the sync behaviour.

# counts_table FILE: print "<sum of the section rows> <Total>" for the Counts
# table of a parity file (the section headed "## <n>. Counts"), or nothing when
# that section has no section row or no "| **Total** | **<n>** |" row. Header
# and separator rows are skipped because their count cell is not a number.
#   FILE  the parity file to read
counts_table() {
  awk -F'|' '
    /^## / { in_counts = ($0 ~ /^## [0-9]+\. Counts[[:space:]]*$/); next }
    !in_counts || NF != 4 { next }
    {
      label = $2; value = $3
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", label)
      gsub(/^[[:space:]*]+|[[:space:]*]+$/, "", value)
      if (value !~ /^[0-9]+$/) next
      if (label == "**Total**") { total = value; have_total = 1 } else { sum += value; rows++ }
    }
    END { if (have_total && rows > 0) print sum, total }
  ' "$1"
}

case_real_parity_file() {
  # The expected number of IDs is the Total of the Counts table (section 22)
  # of parity.md, and the table's section rows must add up to that Total. A PR
  # that adds an ID updates the table; this case then checks that the parser
  # finds exactly that many IDs, so neither the table nor a row can drift.
  local sum="" total=""
  read -r sum total < <(counts_table "$REPO_ROOT/docs/design/parity.md")
  if [[ -z "$total" ]]; then
    fail_case "docs/design/parity.md has no Counts table with section rows and a '| **Total** | **<n>** |' row"
    return
  fi
  expect_eq "$sum" "$total" "sum of the Counts rows against the table's Total"
  new_state
  # The default --parity-file is docs/design/parity.md of the checkout.
  run_script --repo "$REPO" --project "$PROJECT" --dry-run
  expect_status 0
  expect_writes 0
  expect_eq "$(grep -c '^create issue \[' "$OUT")" "$total" "create lines"
  expect_eq "$(grep -o '^create issue \[[^]]*\]' "$OUT" | sort -u | wc -l | tr -d ' ')" "$total" "distinct IDs"
  # A first run sets three board fields per ID: Disposition, ADR and Area.
  expect_contains "$OUT" "summary: created $total, updated 0, closed 0, added to the project $total, fields set $((3 * total)), unchanged 0 (dry run: nothing was changed)"
}

case_counts_table_reader() {
  # counts_table reads the section rows and the Total of section 22 only, and
  # prints nothing when the Total row is missing.
  new_state
  cat >"$PARITY" <<'EOF'
## 21. Platform items (PLT)

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| PLT-01 | A row whose count-like cell must not count | new | none | 7 |

## 22. Counts

| Section | IDs |
|---|---|
| Shared behaviour (SHR) | 18 |
| Platform features of backend1 (AUTH, I18N) | 33 |
| **Total** | **51** |

## 23. Open items

| Item | Rows |
|---|---|
| Something | 5 |
EOF
  expect_eq "$(counts_table "$PARITY")" "51 51" "counts of a well-formed table"
  edit_parity '$0 == "| Shared behaviour (SHR) | 18 |" { $0 = "| Shared behaviour (SHR) | 19 |" } { print }'
  expect_eq "$(counts_table "$PARITY")" "52 51" "counts after one section row changed"
  edit_parity '!/^\| \*\*Total\*\* \|/ { print }'
  expect_eq "$(counts_table "$PARITY")" "" "counts without a Total row"
}

case_first_run() {
  local body
  new_state
  sync
  expect_status 0
  expect_calls '^WRITE label create parity ' 1
  expect_calls '^WRITE issue create ' 6
  expect_calls '^WRITE project item-add ' 6
  expect_calls '^WRITE project item-edit ' 18
  expect_writes 31
  expect_eq "$(jq -r '[.[] | select(.labels == ["parity"] and .state == "OPEN")] | length' "$GH_STUB_DIR/issues.json")" 6 "open parity issues"
  expect_eq "$(issue SHR-01 .title)" "[SHR-01] Admin access for signed-in accounts" "title"
  expect_eq "$(board_value SHR-01 Disposition)" "port-with-fix" "SHR-01 Disposition"
  expect_eq "$(board_value SHR-01 ADR)" "0006, 0007" "SHR-01 ADR"
  expect_eq "$(board_value SHR-03 ADR)" "none" "SHR-03 ADR"
  expect_eq "$(board_value SHR-01 Area)" "1. Shared behaviour of the content screens (SHR)" "SHR-01 Area"
  expect_eq "$(board_value POS-H18 Area)" "2.1 Publishing workflow (POS-H)" "POS-H18 Area"
  expect_eq "$(board_value POS-H18 ADR)" "api" "POS-H18 ADR"
  expect_contains "$OUT" "summary: created 6, updated 0, closed 0, added to the project 6, fields set 18, unchanged 0"

  body="$(issue SHR-01 .body)"
  expect_text_contains "$body" "- **Disposition:** \`port-with-fix\`"
  expect_text_contains "$body" "- **ADR:** [0006], [0007]"
  expect_text_contains "$body" "- **Area:** 1. Shared behaviour of the content screens (SHR)"
  expect_text_contains "$body" "- **Notes:** Every admin request needs an active, signed-in account (ADR 0006)."
  expect_text_contains "$body" "**Done when:** the PR that closes this issue is merged, and it contains the feature and a test that pins the fixed behaviour named in the notes."
  expect_text_contains "$body" "_This issue is generated from [docs/design/parity.md]($BLOB/docs/design/parity.md)"
  expect_text_contains "$body" "[0006]: $BLOB/docs/design/adr/0006-authentication.md"
  expect_text_contains "$body" "[0007]: $BLOB/docs/design/adr/0007-authorization.md"
  expect_text_lacks "$body" "[0004]:"
  # A relative inline link and a full reference link become absolute links.
  expect_text_contains "$(issue SHR-02 .body)" "[cutover.md]($BLOB/docs/design/cutover.md)"
  expect_text_contains "$(issue POS-H18 .body)" "[api]: $BLOB/docs/design/public-api.md"
  expect_text_contains "$(issue SHR-03 .body)" "- **Notes:** none"
  expect_text_contains "$(issue LOG-01 .body)" "plus the documented exception in ADR 0019 section 3"
  # The definition no row uses reaches no issue.
  expect_eq "$(jq '[.[] | select(.body | contains("[0099]"))] | length' "$GH_STUB_DIR/issues.json")" 0 "bodies with [0099]"
}

case_second_run_no_change() {
  new_state
  sync
  sync
  expect_status 0
  expect_writes 0
  expect_contains "$OUT" "summary: created 0, updated 0, closed 0, added to the project 0, fields set 0, unchanged 6"
}

case_changed_disposition() {
  local number
  new_state
  sync
  number="$(issue SHR-03 .number)"
  edit_parity '/^\| SHR-03 / { sub(/\| port \|/, "| port-with-fix |") } { print }'
  sync
  expect_status 0
  expect_writes 2
  expect_calls "^WRITE issue edit $number --repo $REPO --body-file " 1
  expect_calls '^WRITE project item-edit .*--field-id PVTSSF_disp --single-select-option-id o_fix$' 1
  expect_contains "$OUT" "update issue [SHR-03] #$number: body"
  expect_contains "$OUT" "set field [SHR-03] #$number Disposition = port-with-fix"
  expect_eq "$(board_value SHR-03 Disposition)" "port-with-fix" "SHR-03 Disposition"
  expect_text_contains "$(issue SHR-03 .body)" "- **Disposition:** \`port-with-fix\`"
}

case_added_id() {
  new_state
  sync
  edit_parity '{ print } /^\| SHR-03 / { print "| SHR-04 | Only declared fields are saved | port | [0004] | Typed request DTOs |" }'
  sync
  expect_status 0
  expect_calls '^WRITE issue create ' 1
  expect_calls '^WRITE issue create .*--title \[SHR-04\] Only declared fields are saved ' 1
  expect_calls '^WRITE project item-add ' 1
  expect_calls '^WRITE project item-edit ' 3
  expect_writes 5
  expect_eq "$(board_value SHR-04 Area)" "1. Shared behaviour of the content screens (SHR)" "SHR-04 Area"
}

case_closed() {
  new_state
  sync --closed "PLT-39=$PR_MERGED"
  expect_status 0
  expect_eq "$(issue PLT-39 .state)" "CLOSED" "PLT-39 state"
  expect_eq "$(issue PLT-39 '.comments | length')" 1 "PLT-39 comments"
  expect_text_contains "$(issue PLT-39 '.comments[0]')" "Done in $PR_MERGED, which merged before this issue existed."
  expect_eq "$(board_value PLT-39 Disposition)" "new" "PLT-39 Disposition"
  # The item is on the board, with its fields, before the issue closes, so
  # the board's "Item closed" workflow moves it to Done.
  expect_eq "$(grep '^WRITE' "$GH_STUB_DIR/calls.log" | tail -7 | awk '{print $2 " " $3}' | tr '\n' ',')" \
    "issue create,project item-add,project item-edit,project item-edit,project item-edit,issue comment,issue close," "PLT-39 write order"
  expect_eq "$(jq '[.[] | select(.state == "CLOSED")] | length' "$GH_STUB_DIR/issues.json")" 1 "closed issues"

  # Again with --closed: already closed, so nothing is written.
  sync --closed "PLT-39=$PR_MERGED"
  expect_status 0
  expect_writes 0
  # Without --closed: the issue is never reopened.
  sync
  expect_status 0
  expect_writes 0
  expect_eq "$(issue PLT-39 .state)" "CLOSED" "PLT-39 state after later runs"
  expect_eq "$(issue PLT-39 '.comments | length')" 1 "PLT-39 comments after later runs"
}

case_closed_two_prs() {
  new_state
  sync --closed "PLT-39=$PR_MERGED" --closed "PLT-39=$PR_MERGED_K8S"
  expect_status 0
  expect_text_contains "$(issue PLT-39 '.comments[0]')" "Done in $PR_MERGED and $PR_MERGED_K8S, which merged"
  expect_calls '^WRITE issue comment ' 1
}

case_closed_existing_open_issue() {
  new_state
  sync
  sync --closed "PLT-39=$PR_MERGED"
  expect_status 0
  expect_writes 0
  expect_contains "$ERR" "warning: --closed PLT-39: issue #"
  expect_contains "$ERR" "already exists and is open, so it is left open"
  expect_eq "$(issue PLT-39 .state)" "OPEN" "PLT-39 state"
}

case_closed_unmerged_pr() {
  new_state
  sync --closed "PLT-39=$PR_OPEN"
  expect_status 1
  expect_contains "$ERR" "--closed: $PR_OPEN is OPEN, not merged"
  expect_writes 0
}

case_dry_run() {
  local dry_lines
  new_state
  sync --dry-run --closed "PLT-39=$PR_MERGED"
  expect_status 0
  expect_writes 0
  expect_eq "$(jq length "$GH_STUB_DIR/issues.json")" 0 "issues after a dry run"
  expect_contains "$OUT" "create label parity"
  expect_contains "$OUT" "create issue [SHR-01] Admin access for signed-in accounts"
  expect_contains "$OUT" "add to project [SHR-01]"
  expect_contains "$OUT" "set field [SHR-01] ADR = 0006, 0007"
  expect_contains "$OUT" "comment on issue [PLT-39]: $PR_MERGED"
  expect_contains "$OUT" "close issue [PLT-39]"
  expect_contains "$OUT" "(dry run: nothing was changed)"
  # A real run prints the same change lines as the dry run.
  dry_lines="$(grep -v '^summary:' "$OUT")"
  sync --closed "PLT-39=$PR_MERGED"
  expect_eq "$(grep -v '^summary:' "$OUT")" "$dry_lines" "real-run lines"

  # A dry run over pending changes prints them and still writes nothing.
  edit_parity '/^\| SHR-03 / { sub(/\| port \|/, "| replaced |") } { print }'
  sync --dry-run
  expect_status 0
  expect_writes 0
  expect_contains "$OUT" "update issue [SHR-03] #"
  expect_contains "$OUT" "Disposition = replaced"
  expect_eq "$(board_value SHR-03 Disposition)" "port" "SHR-03 Disposition after a dry run"
}

case_hand_edits_restored() {
  local number
  new_state
  sync
  number="$(issue SHR-01 .number)"
  edit_state issues.json 'map(if (.title | startswith("[SHR-01] ")) then .title = "[SHR-01] renamed by hand" | .body = "edited" else . end)'
  edit_state project.json '.items[0].values.PVTF_area = {text: "moved by hand"}'
  sync
  expect_status 0
  expect_writes 2
  expect_calls "^WRITE issue edit $number --repo $REPO --title \\[SHR-01\\] Admin access for signed-in accounts --body-file " 1
  expect_calls '^WRITE project item-edit .*--field-id PVTF_area --text 1\. Shared behaviour' 1
  expect_contains "$OUT" "update issue [SHR-01] #$number: title, body"
}

case_crlf_body_is_unchanged() {
  new_state
  sync
  # A body saved in the browser comes back with CRLF line ends.
  edit_state issues.json 'map(.body |= (gsub("\n"; "\r\n") + "\r\n"))'
  sync
  expect_status 0
  expect_writes 0
}

case_missing_board_item_added() {
  new_state
  sync
  edit_state project.json --arg u "$(issue SHR-02 .url)" '.items |= map(select(.issue_url != $u))'
  sync
  expect_status 0
  expect_calls '^WRITE issue create ' 0
  expect_calls '^WRITE project item-add ' 1
  expect_calls '^WRITE project item-edit ' 3
  expect_writes 4
  expect_eq "$(board_value SHR-02 Disposition)" "replaced" "SHR-02 Disposition"
}

case_repo_case_is_canonical() {
  new_state
  run_script --repo verseghy/WEBSITE_backend3 --project "$PROJECT" --parity-file "$PARITY"
  expect_status 0
  expect_text_contains "$(issue SHR-02 .body)" "$BLOB/docs/design/cutover.md"
  sync
  expect_writes 0
}

case_long_title_truncated() {
  local feature
  new_state
  feature="Feature text that is far too long for a title $(printf 'and goes on %.0s' {1..30})end"
  edit_parity '{ print } /^\| SHR-03 / { print "| SHR-04 | '"$feature"' | new | none | |" }'
  sync
  expect_status 0
  expect_eq "$(issue SHR-04 '.title | length')" 256 "title length"
  expect_eq "$(issue SHR-04 '.title | endswith("…")')" true "title ends with an ellipsis"
  expect_text_contains "$(issue SHR-04 .body)" "- **Feature:** $feature"
  sync
  expect_writes 0
}

case_duplicate_issues_fail() {
  new_state
  sync
  edit_state issues.json '. + [(.[0] | .number = 99 | .url = "https://github.com/Verseghy/website_backend3/issues/99")]'
  sync
  expect_status 1
  expect_contains "$ERR" "issues #2 and #99 all have the title prefix [SHR-01]"
  expect_writes 0
}

case_unknown_issue_warns() {
  new_state
  sync
  edit_state issues.json '. + [{number: 50, title: "[ZZZ-01] Not in the file", body: "", state: "OPEN",
    url: "https://github.com/Verseghy/website_backend3/issues/50", labels: ["parity"], comments: []}]'
  sync
  expect_status 0
  expect_writes 0
  expect_contains "$ERR" "warning: issue #50 [ZZZ-01] has no row in parity.md"
}

# ---------------------------------------------------------------------------
# Cases: arguments, tools and the board.

# usage_failure CODE MESSAGE ARGS...: the script fails with CODE, prints
# MESSAGE on stderr and calls gh not at all.
#   CODE     expected exit status
#   MESSAGE  text expected on stderr
#   ARGS     the script's arguments
usage_failure() {
  local code="$1" message="$2"
  shift 2
  new_state
  run_script "$@"
  expect_status "$code"
  expect_contains "$ERR" "$message"
  expect_eq "$(wc -l <"$GH_STUB_DIR/calls.log" | tr -d ' ')" 0 "gh calls"
}

case_arg_missing_repo() { usage_failure 2 "--repo is required" --project 7; }
case_arg_bad_repo() { usage_failure 2 "--repo must be OWNER/REPO, for example Verseghy/website_backend3 (got 'website_backend3')" --repo website_backend3 --project 7; }
case_arg_bad_project() { usage_failure 2 "--project must be the project's number, a positive integer (got 'abc')" --repo "$REPO" --project abc; }
case_arg_unknown_flag() { usage_failure 2 "unknown option '--force'" --repo "$REPO" --project 7 --force; }
case_arg_missing_value() { usage_failure 2 "--project needs a value" --repo "$REPO" --project; }
case_arg_closed_format() { usage_failure 2 "--closed expects ID=PR_URL" --repo "$REPO" --project 7 --closed PLT-39; }
case_arg_closed_unknown_id() {
  usage_failure 2 "--closed: PLT-99 is not an ID in" --repo "$REPO" --project 7 --parity-file "$FIXTURE" --closed "PLT-99=$PR_MERGED"
}
case_arg_closed_not_pr_url() {
  usage_failure 2 "--closed PLT-39: 'https://github.com/Verseghy/website_backend3/issues/3' is not a GitHub pull-request URL" \
    --repo "$REPO" --project 7 --closed PLT-39=https://github.com/Verseghy/website_backend3/issues/3
}
case_arg_missing_parity_file() { usage_failure 2 "parity file '/nonexistent/parity.md' does not exist" --repo "$REPO" --project 7 --parity-file /nonexistent/parity.md; }

# without_tool TOOL: run the script with a PATH that has every tool it uses
# except TOOL, and expect the message that names TOOL.
#   TOOL  gh or jq
without_tool() {
  local bin tool
  new_state
  bin="$GH_STUB_DIR/bin"
  mkdir -p "$bin"
  for tool in jq awk mktemp rm cat; do
    [[ "$tool" == "$1" ]] || ln -s "$(command -v "$tool")" "$bin/$tool"
  done
  [[ "$1" == "gh" ]] || ln -s "$STUBS_DIR/gh" "$bin/gh"
  PATH="$bin" GH_STUB_DIR="$GH_STUB_DIR" "$BASH" "$SCRIPT" --repo "$REPO" --project 7 >"$OUT" 2>"$ERR"
  STATUS=$?
  expect_status 1
  expect_contains "$ERR" "error: $1"
  expect_contains "$ERR" "is not installed or not on PATH"
}
case_missing_gh() { without_tool gh; }
case_missing_jq() { without_tool jq; }

case_board_wrong_number() {
  new_state
  run_script --repo "$REPO" --project 8 --parity-file "$PARITY"
  expect_status 1
  expect_contains "$ERR" "Could not resolve to a ProjectV2 with the number 8."
  expect_contains "$ERR" "could not read project 8 of Verseghy"
  expect_writes 0
}

case_board_missing_field() {
  new_state
  edit_state project.json '.fields |= map(select(.name != "ADR"))'
  sync
  expect_status 1
  expect_contains "$ERR" "the project has no \"ADR\" field; create it with: gh project field-create 7 --owner Verseghy --name ADR --data-type TEXT"
  expect_writes 0
}

case_board_wrong_field_type() {
  new_state
  edit_state project.json '.fields |= map(if .name == "Area" then .dataType = "NUMBER" else . end)'
  sync
  expect_status 1
  expect_contains "$ERR" "the project's \"Area\" field is NUMBER, but it must be TEXT"
  expect_writes 0
}

case_board_missing_option() {
  new_state
  edit_state project.json '.fields |= map(if .name == "Disposition" then .options |= map(select(.name != "new")) else . end)'
  sync
  expect_status 1
  expect_contains "$ERR" "the project's \"Disposition\" field has no option \"new\""
  expect_writes 0
}

# ---------------------------------------------------------------------------
# Cases: errors in the parity file. Parsing comes before any gh call.

# parity_failure AWK_PROGRAM MESSAGE: change the fixture with AWK_PROGRAM and
# expect the run to fail with MESSAGE before calling gh.
#   AWK_PROGRAM  edit applied to the case's parity file
#   MESSAGE      text expected on stderr
parity_failure() {
  new_state
  edit_parity "$1"
  sync
  expect_status 1
  expect_contains "$ERR" "$2"
  expect_contains "$ERR" "has errors (listed above); fix them and run again"
  expect_eq "$(wc -l <"$GH_STUB_DIR/calls.log" | tr -d ' ')" 0 "gh calls"
}

case_parity_duplicate_id() {
  parity_failure '{ print } /^\| SHR-03 / { print "| SHR-01 | Copied row | port | none | |" }' \
    "duplicate ID SHR-01 (first used on line 13)"
}
case_parity_bad_disposition() {
  parity_failure '/^\| SHR-03 / { sub(/\| port \|/, "| ported |") } { print }' \
    'row SHR-03 has the unknown disposition "ported"; use one of: port, port-with-fix, replaced, retired, new'
}
case_parity_malformed_row() {
  parity_failure '/^\| SHR-03 / { $0 = "| SHR-03 | Inputs trimmed | port | none |" } { print }' \
    "malformed row SHR-03: it has 4 cells, but a parity row has 5"
}
case_parity_bad_id_in_table() {
  parity_failure '{ print } /^\| SHR-03 / { print "| SHR_04 | Bad ID | port | none | |" }' \
    'malformed row: the first cell "SHR_04" is not an ID'
}
case_parity_undefined_reference() {
  parity_failure '/^\| SHR-03 / { sub(/\| none \|/, "| [0042] |") } { print }' \
    'row SHR-03 uses [0042], but'
}

# ---------------------------------------------------------------------------

run_case "real docs/design/parity.md parses to the Total of its Counts table, and the rows add up to it (dry run, empty stub)" case_real_parity_file
run_case "the Counts table reader sums section 22 only and needs its Total row" case_counts_table_reader
run_case "first run creates every issue, adds it to the board and sets its fields" case_first_run
run_case "second run with no change makes no write call" case_second_run_no_change
run_case "changed disposition: one body edit and one field edit" case_changed_disposition
run_case "added ID: one issue created, added and given its fields" case_added_id
run_case "--closed creates, comments and closes; later runs leave it closed" case_closed
run_case "--closed with two PRs for one ID links both in one comment" case_closed_two_prs
run_case "--closed for an existing open issue warns and changes nothing" case_closed_existing_open_issue
run_case "--closed with an unmerged PR fails before any write" case_closed_unmerged_pr
run_case "--dry-run prints every change and writes nothing" case_dry_run
run_case "title, body and field edits made by hand are restored" case_hand_edits_restored
run_case "a body with CRLF line ends counts as unchanged" case_crlf_body_is_unchanged
run_case "an issue missing from the board is added again, not recreated" case_missing_board_item_added
run_case "--repo in another case uses GitHub's spelling, so bodies stay stable" case_repo_case_is_canonical
run_case "a title over 256 characters is truncated; the body keeps the feature" case_long_title_truncated
run_case "two issues for one ID fail the run" case_duplicate_issues_fail
run_case "a parity issue with no row in parity.md is warned about and left alone" case_unknown_issue_warns
run_case "argument: missing --repo" case_arg_missing_repo
run_case "argument: --repo not OWNER/REPO" case_arg_bad_repo
run_case "argument: non-numeric --project" case_arg_bad_project
run_case "argument: unknown flag" case_arg_unknown_flag
run_case "argument: flag without a value" case_arg_missing_value
run_case "argument: --closed without ID=URL" case_arg_closed_format
run_case "argument: --closed ID not in parity.md" case_arg_closed_unknown_id
run_case "argument: --closed URL is not a pull-request URL" case_arg_closed_not_pr_url
run_case "argument: missing parity file" case_arg_missing_parity_file
run_case "tool: missing gh" case_missing_gh
run_case "tool: missing jq" case_missing_jq
run_case "board: unknown project number" case_board_wrong_number
run_case "board: missing field names the command that creates it" case_board_missing_field
run_case "board: field of the wrong type" case_board_wrong_field_type
run_case "board: missing Disposition option names the option" case_board_missing_option
run_case "parity file: duplicate ID" case_parity_duplicate_id
run_case "parity file: unknown disposition" case_parity_bad_disposition
run_case "parity file: row with the wrong number of cells" case_parity_malformed_row
run_case "parity file: row in a parity table without a valid ID" case_parity_bad_id_in_table
run_case "parity file: reference with no definition" case_parity_undefined_reference

printf '\n%d passed, %d failed\n' "$PASSED" "$FAILED"
((FAILED == 0))
