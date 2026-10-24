#!/usr/bin/env bash
# Tests for scripts/ci/check-session-links.sh (ADR 0012 section 3).
#
# Covers a link in the PR title, the body, a commit message and an added diff
# line (also one whose text starts with "++"); prose without a URL; near
# misses that are not session links; and that neither the script nor this file
# matches itself. Fixture links are assembled
# from parts at run time, so this file never contains one. Run from the
# repository root:
#   bash scripts/tests/test-check-session-links.sh
# Uses a throwaway local git repository; no network access is needed.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT="$ROOT/scripts/ci/check-session-links.sh"

tests_run=0
failures=0

# run_case NAME EXPECTED_EXIT [--contains TEXT] [--lacks TEXT] -- COMMAND...
# Runs COMMAND, compares its exit code and checks its combined output.
run_case() {
  local name=$1 expected=$2 needle="" forbidden=""
  shift 2
  while [[ ${1-} == --contains || ${1-} == --lacks ]]; do
    if [[ $1 == --contains ]]; then needle=$2; else forbidden=$2; fi
    shift 2
  done
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
  elif [[ -n $forbidden && $output == *"$forbidden"* ]]; then
    failures=$((failures + 1))
    printf 'not ok %d - %s (output contains "%s")\n' "$tests_run" "$name" "$forbidden"
  else
    printf 'ok %d - %s\n' "$tests_run" "$name"
  fi
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Fixture links, built from parts.
host_name='claude'
host_tld='ai'
host="${host_name}.${host_tld}"
path_prefix="/code/session_"
LINK="https://${host}${path_prefix}01AbCdEf"
LINK_HTTP="http://${host}${path_prefix}x"
LINK_UPPER="HTTPS://CLAUDE.AI${path_prefix}x"
LINK_SUBDOMAIN="https://www.${host}${path_prefix}x"
LINK_PORT="https://${host}:443${path_prefix}x"
LINK_USERINFO="https://user@${host}${path_prefix}x"

# write_event FILE TITLE BODY: write a minimal pull_request event payload.
# BODY "null" writes a JSON null, as GitHub does for an empty body.
write_event() {
  if [[ $3 == null ]]; then
    jq -n --arg t "$2" '{pull_request: {title: $t, body: null}}' >"$1"
  else
    jq -n --arg t "$2" --arg b "$3" '{pull_request: {title: $t, body: $b}}' >"$1"
  fi
}

# git_fixture: run git in the fixture repository, isolated from the user's
# and the system's configuration (no hooks, no signing, fixed identity).
git_fixture() {
  GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
    GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid \
    GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid \
    git -C "$REPO" -c commit.gpgsign=false -c init.defaultBranch=master "$@"
}

# The fixture repository: a base commit that already contains a link (which
# the PR then removes), and four PR branches.
REPO="$WORK/repo"
mkdir -p "$REPO"
git_fixture init -q
printf 'line one\nold %s\n' "$LINK" >"$REPO/legacy.md"
printf 'readme\n' >"$REPO/README.md"
git_fixture add -A
git_fixture commit -q -m "base"
BASE=$(git_fixture rev-parse HEAD)

git_fixture checkout -q -b clean
printf 'readme\nThe rule bans %s links (no URL here).\nThe path prefix is %s.\n' "$host" "$path_prefix" >"$REPO/README.md"
git_fixture rm -q legacy.md
git_fixture commit -q -m "docs: explain the rule" -m "Session links (host ${host}) are banned."
CLEAN=$(git_fixture rev-parse HEAD)

git_fixture checkout -q -b in-message "$BASE"
printf 'more\n' >>"$REPO/README.md"
git_fixture commit -q -a -m "feat: add more" -m "See ${LINK} for context."
IN_MESSAGE=$(git_fixture rev-parse HEAD)

git_fixture checkout -q -b in-diff "$BASE"
printf 'first\nsecond\nsee %s\n' "$LINK" >"$REPO/notes.md"
git_fixture add notes.md
git_fixture commit -q -m "docs: add notes"
IN_DIFF=$(git_fixture rev-parse HEAD)

git_fixture checkout -q -b plus-plus "$BASE"
# With --unified=0 an added line "++ <link>" appears as "+++ <link>", the
# shape of a file header.
printf 'first\n++ %s\n' "$LINK" >"$REPO/plus.md"
git_fixture add plus.md
git_fixture commit -q -m "docs: add a list"
PLUS_PLUS=$(git_fixture rev-parse HEAD)

git_fixture checkout -q -b moved "$BASE"
git_fixture mv legacy.md moved.md
git_fixture commit -q -m "chore: move a file"
MOVED=$(git_fixture rev-parse HEAD)

EV="$WORK/event.json"
LINKED_FILE="$WORK/linked.md"
printf 'clean line\nsee %s\n' "$LINK" >"$LINKED_FILE"

# Title and body.
write_event "$EV" "fix(api): clean" "Nothing to see."
run_case "clean title and body pass" 0 --contains "No session links found" -- bash "$SCRIPT" --event "$EV"
write_event "$EV" "fix(api): clean" null
run_case "a null body passes" 0 -- bash "$SCRIPT" --event "$EV"
write_event "$EV" "fix(api): see $LINK" "body"
run_case "a link in the title is caught" 1 --contains "PR title" -- bash "$SCRIPT" --event "$EV"
write_event "$EV" "fix(api): clean" $'first line\r\nsecond line with '"$LINK"$'\r\n'
run_case "a link in the body is caught with its line" 1 --contains "PR body, line 2" -- bash "$SCRIPT" --event "$EV"
run_case "findings never print the link" 1 --lacks "$path_prefix" -- bash "$SCRIPT" --event "$EV"
write_event "$EV" "docs(docs): explain" "Links to ${host} whose path starts with ${path_prefix} are banned."
run_case "prose without a URL passes" 0 -- bash "$SCRIPT" --event "$EV"

# URL variants that are session links.
for variant in "$LINK_HTTP" "$LINK_UPPER" "$LINK_SUBDOMAIN" "$LINK_PORT" "$LINK_USERINFO" "<${LINK}>" "[here](${LINK})"; do
  write_event "$EV" "fix(api): clean" "x ${variant} y"
  run_case "caught: ${variant//$path_prefix/<prefix>}" 1 --contains "PR body, line 1" -- bash "$SCRIPT" --event "$EV"
done

# Near misses that are not session links.
for near in "https://${host}/code/project_1" "https://${host}/chat/1" "https://example.com${path_prefix}1" \
  "https://not${host}${path_prefix}1" "${host}${path_prefix}1" "https://${host}/code"; do
  write_event "$EV" "fix(api): clean" "x ${near} y"
  run_case "not a session link: ${near//$path_prefix/<prefix>}" 0 -- bash "$SCRIPT" --event "$EV"
done

# Commit messages and added diff lines.
run_case "a clean range passes, a removed link is fine" 0 -- bash "$SCRIPT" --repo "$REPO" --base "$BASE" --head "$CLEAN"
run_case "a link in a commit message is caught" 1 --contains "commit ${IN_MESSAGE:0:12} message, line 3" -- \
  bash "$SCRIPT" --repo "$REPO" --base "$BASE" --head "$IN_MESSAGE"
run_case "a link in an added line is caught with file and line" 1 --contains "added line notes.md:3" -- \
  bash "$SCRIPT" --repo "$REPO" --base "$BASE" --head "$IN_DIFF"
run_case "an added line starting with ++ is scanned, not taken for a header" 1 --contains "added line plus.md:2" -- \
  bash "$SCRIPT" --repo "$REPO" --base "$BASE" --head "$PLUS_PLUS"
run_case "moving a file with an old link adds nothing new" 0 -- bash "$SCRIPT" --repo "$REPO" --base "$BASE" --head "$MOVED"
write_event "$EV" "fix(api): see $LINK" "body"
run_case "all sources are scanned together" 1 --contains "Found 3 claude.ai session link(s)" -- \
  bash "$SCRIPT" --event "$EV" --repo "$REPO" --base "$BASE" --head "$IN_DIFF" --file "$LINKED_FILE"

# Files, and the self-match guarantee.
run_case "a file with a link is caught" 1 --contains "line 2" -- bash "$SCRIPT" --file "$LINKED_FILE"
run_case "the script does not match itself" 0 -- bash "$SCRIPT" --file "$SCRIPT"
run_case "this test file does not match itself" 0 -- bash "$SCRIPT" --file "${BASH_SOURCE[0]}"

# Usage and input errors.
run_case "nothing to scan is a usage error" 2 --contains "nothing to scan" -- bash "$SCRIPT"
run_case "--base without --head is a usage error" 2 --contains "together" -- bash "$SCRIPT" --base "$BASE"
run_case "an unknown commit is an input error" 2 --contains "fetch-depth: 0" -- \
  bash "$SCRIPT" --repo "$REPO" --base 0000000000000000000000000000000000000000 --head "$CLEAN"
run_case "a missing event file is an input error" 2 --contains "not a readable file" -- bash "$SCRIPT" --event "$WORK/missing.json"
printf '{"push": {}}' >"$WORK/push.json"
run_case "a non-PR payload is an input error" 2 --contains "pull_request" -- bash "$SCRIPT" --event "$WORK/push.json"
run_case "an unknown option is a usage error" 2 -- bash "$SCRIPT" --bogus

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
