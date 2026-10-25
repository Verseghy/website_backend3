#!/usr/bin/env bash
#
# sync-parity-issues.sh - keep one GitHub issue and one project-board item per
# parity ID in step with docs/design/parity.md.
#
# WHY THIS EXISTS
#   docs/design/parity.md lists every feature backend3 must account for, each
#   with a stable ID (SHR-01, POS-H03, I18N-01, PLT-39, ...). ADR 0019 section 1
#   tracks the status of those IDs on GitHub instead of in the file: one issue
#   per ID, all on the org-level project "website_backend3 parity". A PR
#   finishes an ID by closing its issue ("Closes #N"). This script carries the
#   file to the issues and the board, so parity.md stays the only place where
#   IDs, features, dispositions and notes are edited (PLT-39, ADR 0012 section 1).
#
# CONVENTIONS IT EMBODIES
#   - parity.md owns the issue title "[<ID>] <feature>", the issue body and the
#     board fields Disposition, ADR and Area. The board owns Status: the script
#     never sets it, the project's workflows do.
#   - An issue belongs to an ID when it has the label "parity" and its title
#     starts with "[<ID>] ". IDs are never renumbered or reused, so that prefix
#     is a stable key. The label is a plain GitHub label, outside Prow's label
#     configuration.
#   - Idempotent: a run changes only what differs. A second run over an
#     unchanged parity.md makes no write call, and a run that stopped halfway
#     (an error, a rate limit) is finished by running the same command again.
#   - It never opens or closes an issue, with one exception: --closed creates
#     the issue of work that merged before its issue existed, then closes it
#     with a comment that links the merged PR (ADR 0019 section 1, closing rule 6).
#   - It talks to GitHub only through gh. Reads use `gh api graphql`, because
#     `gh issue list --label` goes through the search index, which lags behind
#     writes and could make a quick re-run create duplicates. Writes use the
#     gh issue, label and project subcommands.
#   - Every relative link in a row (and every reference-style link, such as
#     [0019] or [api]) is rewritten to an absolute link to docs/design/ on
#     master, so the issue's links work on GitHub.
#
# USAGE
#   scripts/sync-parity-issues.sh --repo OWNER/REPO --project NUMBER
#       [--closed ID=PR_URL]... [--dry-run] [--parity-file PATH]
#
#   --repo OWNER/REPO   repository that holds the issues. OWNER also owns the project.
#   --project NUMBER    number of the "website_backend3 parity" project of OWNER.
#   --closed ID=PR_URL  ID's work merged in PR_URL before its issue existed:
#                       create the issue closed, with a comment linking PR_URL.
#                       Repeat the flag for more IDs; repeat it with the same
#                       ID to link more than one PR. The PR must be merged.
#   --dry-run           print every change the run would make; write nothing.
#   --parity-file PATH  parity checklist to read (default: docs/design/parity.md
#                       of the checkout that holds this script).
#
#   Each change is printed as one line (create, add to project, set field,
#   update, comment, close), followed by a summary. Warnings and errors go to
#   stderr. Exit status: 0 on success, 1 on an error, 2 on a usage error.
#   Needs gh 2.x, jq 1.6 or later, awk and bash.
#
# ONE-TIME BOARD SETUP (needs a token with the project scope)
#   gh auth refresh -s project
#   gh project create --owner Verseghy --title "website_backend3 parity" --format json --jq .number
#   N=<the number printed above>
#   gh project field-create "$N" --owner Verseghy --name Disposition --data-type SINGLE_SELECT \
#       --single-select-options "port,port-with-fix,replaced,retired,new"
#   gh project field-create "$N" --owner Verseghy --name ADR --data-type TEXT
#   gh project field-create "$N" --owner Verseghy --name Area --data-type TEXT
#   gh project link "$N" --owner Verseghy --repo Verseghy/website_backend3
#
#   Status is the project's built-in single-select field (ADR 0019: Todo, In
#   progress, Done); the script never sets it. In the project's Workflows
#   settings, keep "Item closed" on (it sets Done, and it is on by default) and
#   turn on "Item added to project" (it sets Todo, and it is off by default).
#   Workflows are turned on in the web UI only: neither gh nor the GraphQL API
#   can enable one.
#
# EXAMPLES
#   # Preview the first sync, then run it. PLT-11, PLT-02 and PLT-39 merged
#   # before their issues existed (ADR 0019 section 1, closing rule 6).
#   scripts/sync-parity-issues.sh --repo Verseghy/website_backend3 --project "$N" --dry-run \
#       --closed PLT-11=https://github.com/Verseghy/website_backend3/pull/1 \
#       --closed PLT-02=https://github.com/Verseghy/website_k8s/pull/50 \
#       --closed PLT-39=https://github.com/Verseghy/website_backend3/pull/<this PR>
#
#   # After a PR changes parity.md: carry the change to the issues and the board.
#   scripts/sync-parity-issues.sh --repo Verseghy/website_backend3 --project "$N"
#
# NOTES
#   - The first run creates one issue per ID of parity.md and makes about five
#     write calls per ID. If GitHub's secondary rate limit stops it, wait and
#     run the same command again; it continues where it stopped.
#   - If a run stops between creating a --closed issue and closing it, the next
#     run warns that the issue is open and leaves it alone. The warning prints
#     the command that finishes the job by hand.
#
# Tests: scripts/tests/test-sync-parity-issues.sh runs this script against a
# stateful gh stub (scripts/tests/stubs/gh), with no network.

set -euo pipefail

# Resolve the checkout from this file's location with shell builtins only, so
# the script needs no dirname or readlink.
script_path="${BASH_SOURCE[0]}"
[[ "$script_path" == */* ]] || script_path="./$script_path"
SCRIPT_DIR="$(cd "${script_path%/*}" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
readonly SCRIPT_DIR REPO_ROOT

readonly LABEL_NAME="parity"
readonly LABEL_COLOR="5319E7"
readonly LABEL_DESCRIPTION="One parity ID of docs/design/parity.md (ADR 0019)"
# Issue links point at master, where parity.md lives in docs/design/. Relative
# links resolve against that directory even when --parity-file points at a
# test fixture somewhere else.
readonly DOCS_BASE="docs/design"
readonly PARITY_PATH_ON_MASTER="docs/design/parity.md"
readonly ADR_0019_PATH="docs/design/adr/0019-scope-parity-tracking-retired-features-release-1-additions.md"
readonly DISPOSITIONS="port port-with-fix replaced retired new"
# A parity ID: SHR-01, POS-H03, I18N-01, PLT-39.
readonly ID_REGEX='^[A-Z][A-Z0-9]*-[A-Z]*[0-9]+$'
readonly PR_URL_REGEX='^https://github\.com/[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9._-]+/pull/[1-9][0-9]*$'
# Field separator of the awk output and of the action list (ASCII unit
# separator): table cells never contain it, and bash `read` keeps empty fields
# between non-whitespace separators.
readonly US=$'\037'

REPO=""     # OWNER/REPO; replaced by GitHub's spelling once read
OWNER=""    # owner of the repository and of the project
PROJECT=""  # project number
PROJECT_ID=""  # project node ID, for gh project item-edit
PARITY_FILE="$REPO_ROOT/docs/design/parity.md"
DRY_RUN=0
CLOSED_ARGS=()
WORK_DIR=""

# ---------------------------------------------------------------------------
# GraphQL reads. Each query was checked against GitHub's public schema
# (Repository.issues, ProjectV2Owner.projectV2, ProjectV2.fields and items).
# `gh api graphql --paginate` needs the $endCursor variable and exactly one
# pageInfo per page, so each paginated query pages one connection only.

# The repository's canonical name, whether the label exists, and every issue
# with the label, open or closed. The labels: filter of Repository.issues is
# read from the database, not the search index.
readonly ISSUES_QUERY='
query ParityIssues($owner: String!, $name: String!, $endCursor: String) {
  repository(owner: $owner, name: $name) {
    nameWithOwner
    label(name: "parity") { name }
    issues(first: 100, after: $endCursor, labels: ["parity"], states: [OPEN, CLOSED],
           orderBy: {field: CREATED_AT, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes { number title body state url }
    }
  }
}'

# The project's node ID and its fields with their data types and options.
# repositoryOwner plus "... on ProjectV2Owner" works for an organization and for
# a user. A project has at most 50 fields, so one page is enough.
readonly BOARD_QUERY='
query ParityBoard($owner: String!, $number: Int!) {
  repositoryOwner(login: $owner) {
    ... on ProjectV2Owner {
      projectV2(number: $number) {
        id
        title
        fields(first: 100) {
          nodes {
            ... on ProjectV2FieldCommon { id name dataType }
            ... on ProjectV2SingleSelectField { options { id name } }
          }
        }
      }
    }
  }
}'

# Every item on the board with its issue and its text and single-select values.
# archivedStates includes ARCHIVED, because the default (NOT_ARCHIVED) would
# make an archived item look missing and the script would add it again.
readonly ITEMS_QUERY='
query ParityBoardItems($owner: String!, $number: Int!, $endCursor: String) {
  repositoryOwner(login: $owner) {
    ... on ProjectV2Owner {
      projectV2(number: $number) {
        items(first: 100, after: $endCursor, archivedStates: [ARCHIVED, NOT_ARCHIVED]) {
          pageInfo { hasNextPage endCursor }
          nodes {
            id
            content { ... on Issue { number repository { nameWithOwner } } }
            fieldValues(first: 100) {
              nodes {
                ... on ProjectV2ItemFieldTextValue { text field { ... on ProjectV2FieldCommon { id } } }
                ... on ProjectV2ItemFieldSingleSelectValue { optionId name field { ... on ProjectV2FieldCommon { id } } }
              }
            }
          }
        }
      }
    }
  }
}'

# ---------------------------------------------------------------------------
# Messages.

# usage: print the short usage text to stdout. No parameters.
usage() {
  cat <<EOF
Usage: scripts/sync-parity-issues.sh --repo OWNER/REPO --project NUMBER
           [--closed ID=PR_URL]... [--dry-run] [--parity-file PATH]

Creates or updates one GitHub issue (label "parity") per ID in
docs/design/parity.md and its item on project NUMBER of OWNER (ADR 0019).

  --repo OWNER/REPO   repository that holds the issues; OWNER owns the project
  --project NUMBER    number of the "website_backend3 parity" project
  --closed ID=PR_URL  create ID's issue closed, linking the merged PR_URL (repeatable)
  --dry-run           print every change, write nothing
  --parity-file PATH  parity checklist (default: docs/design/parity.md)
  -h, --help          show this help

The header of this script documents the one-time board setup.
EOF
}

# die MESSAGE: print "error: MESSAGE" to stderr and exit 1.
#   MESSAGE  what went wrong and what the operator should do about it
die() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

# usage_error MESSAGE: print "error: MESSAGE" and a --help hint, exit 2.
#   MESSAGE  which argument is wrong and how to fix it
usage_error() {
  printf 'error: %s\nRun with --help for usage.\n' "$1" >&2
  exit 2
}

# warn MESSAGE: print "warning: MESSAGE" to stderr and continue.
#   MESSAGE  the condition the operator should look at
warn() {
  printf 'warning: %s\n' "$1" >&2
}

# ---------------------------------------------------------------------------
# Arguments and tools.

# parse_args ARGS...: fill REPO, PROJECT, CLOSED_ARGS, DRY_RUN and PARITY_FILE
# from the command line; exit 2 on an unknown flag, a missing value or a
# stray argument. Both "--flag value" and "--flag=value" are accepted.
#   ARGS  the script's command-line arguments
parse_args() {
  while (($# > 0)); do
    case "$1" in
      --repo=* | --project=* | --closed=* | --parity-file=*)
        # Split "--flag=value" into "--flag" "value" and parse it again.
        set -- "${1%%=*}" "${1#*=}" "${@:2}"
        continue
        ;;
      --repo | --project | --closed | --parity-file)
        (($# >= 2)) || usage_error "$1 needs a value"
        case "$1" in
          --repo) REPO="$2" ;;
          --project) PROJECT="$2" ;;
          --closed) CLOSED_ARGS+=("$2") ;;
          --parity-file) PARITY_FILE="$2" ;;
        esac
        shift 2
        ;;
      --dry-run)
        DRY_RUN=1
        shift
        ;;
      -h | --help)
        usage
        exit 0
        ;;
      -*) usage_error "unknown option '$1'" ;;
      *) usage_error "unexpected argument '$1'" ;;
    esac
  done
}

# validate_args: check the values parse_args collected, so that a wrong
# argument fails before anything is read from GitHub. Exits 2 with a message
# that names the argument. No parameters; reads the globals.
validate_args() {
  local owner pair id url seen
  [[ -n "$REPO" ]] || usage_error "--repo is required, for example --repo Verseghy/website_backend3"
  [[ "$REPO" =~ ^[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9._-]+$ ]] ||
    usage_error "--repo must be OWNER/REPO, for example Verseghy/website_backend3 (got '$REPO')"
  owner="${REPO%%/*}"
  [[ -n "$PROJECT" ]] ||
    usage_error "--project is required: the number of the project (list them with: gh project list --owner $owner)"
  [[ "$PROJECT" =~ ^[1-9][0-9]*$ ]] ||
    usage_error "--project must be the project's number, a positive integer (got '$PROJECT'); list them with: gh project list --owner $owner"

  # CLOSED_ARGS may be empty; "${a[@]+...}" keeps bash < 4.4 from treating an
  # empty array as unbound under set -u.
  seen=" "
  for pair in ${CLOSED_ARGS[@]+"${CLOSED_ARGS[@]}"}; do
    [[ "$pair" == *=* ]] ||
      usage_error "--closed expects ID=PR_URL, for example --closed PLT-11=https://github.com/Verseghy/website_backend3/pull/1 (got '$pair')"
    id="${pair%%=*}"
    url="${pair#*=}"
    [[ "$id" =~ $ID_REGEX ]] || usage_error "--closed: '$id' is not a parity ID such as SHR-01 or PLT-39"
    [[ "$url" =~ $PR_URL_REGEX ]] ||
      usage_error "--closed $id: '$url' is not a GitHub pull-request URL such as https://github.com/OWNER/REPO/pull/123"
    [[ "$seen" != *" $pair "* ]] || usage_error "--closed $pair is given twice"
    seen+="$pair "
  done

  [[ -f "$PARITY_FILE" && -r "$PARITY_FILE" ]] ||
    usage_error "parity file '$PARITY_FILE' does not exist or cannot be read; pass --parity-file PATH"
}

# require_tools: exit 1 with install hints if gh or jq is missing. awk is
# POSIX and assumed present. No parameters.
require_tools() {
  command -v gh >/dev/null 2>&1 ||
    die "gh (GitHub CLI) is not installed or not on PATH; install it from https://cli.github.com and run: gh auth login"
  command -v jq >/dev/null 2>&1 ||
    die "jq is not installed or not on PATH; install jq 1.6 or later (https://jqlang.org)"
}

# ---------------------------------------------------------------------------
# Parsing parity.md.

# parse_parity_file PATH OUT: parse the parity checklist at PATH into OUT as
# {"rows": [{line, id, feature, disposition, adr, notes, area}],
#  "refs": [{key, url}]}. awk writes ROW, REF and ERR records separated by
# $US; jq turns them into JSON. Exits 1 and lists every problem (duplicate ID,
# unknown disposition, malformed row, undefined reference) with its line.
#   PATH  the parity checklist (parity.md or a test fixture)
#   OUT   file to write the JSON to
parse_parity_file() {
  local path="$1" out="$2" raw="$WORK_DIR/parity.raw"

  # LC_ALL=C: the regexes are ASCII and must not follow locale collation;
  # UTF-8 text passes through untouched. Only POSIX awk features are used, so
  # gawk, mawk and busybox awk all work.
  if ! LC_ALL=C awk -v file="$path" -v US="$US" -v allowed_list="$DISPOSITIONS" -v id_re="$ID_REGEX" '
    function trim(s) { gsub(/^[ \t]+|[ \t]+$/, "", s); return s }
    # Problems go to stdout as ERR records, not to "/dev/stderr": not every awk
    # treats that name specially, and reopening it truncates a redirected stderr.
    function fail(line, msg) { print "ERR" US line US msg; errors++ }
    # split_cells(line, cells): the cells of a table row, trimmed. An escaped
    # pipe (\|) is cell text, not a separator, and is unescaped.
    function split_cells(line, cells,    s, raw, n, i) {
      split("", cells)
      s = trim(line)
      sub(/^\|/, "", s)
      if (s ~ /\|$/ && s !~ /\\\|$/) sub(/\|$/, "", s)
      gsub(/\\\|/, "\001", s)
      n = split(s, raw, "|")
      for (i = 1; i <= n; i++) { cells[i] = trim(raw[i]); gsub(/\001/, "|", cells[i]) }
      return n
    }
    BEGIN {
      n = split(allowed_list, list, " ")
      for (i = 1; i <= n; i++) allowed[list[i]] = 1
      allowed_text = allowed_list; gsub(/ /, ", ", allowed_text)
    }
    { sub(/\r$/, "") }
    /^```/ { in_fence = !in_fence; next }
    in_fence { next }
    # The Area of a row is its nearest ## or ### heading.
    /^###? / { area = $0; sub(/^#+[ \t]+/, "", area); sub(/[ \t]+#+$/, "", area); area = trim(area); in_table = 0; next }
    /^\[[^]]+\]:[ \t]/ {
      key = substr($0, 2, index($0, "]:") - 2)
      url = trim(substr($0, index($0, "]:") + 2)); sub(/[ \t].*$/, "", url)
      if (!(key in refdef)) { refdef[key] = url; print "REF" US key US url }
      in_table = 0; next
    }
    /^[ \t]*\|/ {
      n = split_cells($0, cells)
      # The first row of a table is its header. A table whose header is the
      # parity header must contain parity rows only.
      if (!in_table) {
        in_table = 1
        parity_table = (n == 5 && cells[1] == "ID" && cells[2] == "Feature" && cells[3] == "Disposition" && cells[4] == "ADR" && cells[5] == "Notes")
        next
      }
      separator = 1
      for (i = 1; i <= n; i++) if (cells[i] !~ /^:?-+:?$/) separator = 0
      if (separator) next
      id = cells[1]
      if (id !~ id_re) {
        if (parity_table) fail(NR, "malformed row: the first cell \"" id "\" is not an ID such as SHR-01, POS-H03 or I18N-01")
        next
      }
      if (n != 5) { fail(NR, "malformed row " id ": it has " n " cells, but a parity row has 5 (ID | Feature | Disposition | ADR | Notes)"); next }
      if (id in seen) { fail(NR, "duplicate ID " id " (first used on line " seen[id] "); IDs are never reused, give this row a new ID"); next }
      seen[id] = NR
      if (cells[2] == "") fail(NR, "malformed row " id ": the Feature cell is empty")
      if (!(cells[3] in allowed)) fail(NR, "row " id " has the unknown disposition \"" cells[3] "\"; use one of: " allowed_text)
      if (cells[4] != "none" && cells[4] !~ /^\[[^]]+\](, *\[[^]]+\])*$/)
        fail(NR, "malformed row " id ": the ADR cell must be none or reference links such as [0019], [api] (got \"" cells[4] "\")")
      if (area == "") fail(NR, "row " id " comes before any ## or ### heading, so it has no area")
      rows++
      row_line[rows] = NR; row_id[rows] = id; row_adr[rows] = cells[4]; row_text[rows] = cells[2] " " cells[5]
      print "ROW" US NR US id US cells[2] US cells[3] US cells[4] US cells[5] US area
      next
    }
    { in_table = 0 }
    END {
      # Reference definitions sit at the end of the file, so the references
      # are checked here: every [key] of an ADR cell and every [text][key] in
      # a Feature or Notes cell needs a "[key]: target" line.
      for (r = 1; r <= rows; r++) {
        s = row_adr[r]
        while (match(s, /\[[^]]+\]/)) {
          key = substr(s, RSTART + 1, RLENGTH - 2); s = substr(s, RSTART + RLENGTH)
          if (!(key in refdef)) fail(row_line[r], "row " row_id[r] " uses [" key "], but " file " has no \"[" key "]: <target>\" definition")
        }
        s = row_text[r]
        while (match(s, /\]\[[^]]+\]/)) {
          key = substr(s, RSTART + 2, RLENGTH - 3); s = substr(s, RSTART + RLENGTH)
          if (!(key in refdef)) fail(row_line[r], "row " row_id[r] " uses [" key "], but " file " has no \"[" key "]: <target>\" definition")
        }
      }
      if (rows == 0 && errors == 0) fail(0, "no parity rows found: a parity row is a table row whose first cell is an ID such as SHR-01")
      exit (errors > 0)
    }
  ' "$path" >"$raw"; then
    jq -R -r --arg file "$path" 'split("\u001f") | select(.[0] == "ERR") | if .[1] == "0" then "\($file): \(.[2])" else "\($file):\(.[1]): \(.[2])" end' "$raw" >&2
    die "$path has errors (listed above); fix them and run again"
  fi

  jq -R -s '
    split("\n") | map(select(length > 0) | split("\u001f")) as $records
    | { rows: [ $records[] | select(.[0] == "ROW")
                | {line: (.[1] | tonumber), id: .[2], feature: .[3], disposition: .[4], adr: .[5], notes: .[6], area: .[7]} ],
        refs: [ $records[] | select(.[0] == "REF") | {key: .[1], url: .[2]} ] }
  ' "$raw" >"$out"
}

# validate_closed_ids PARITY_JSON: exit 2 if a --closed ID has no row in the
# parity file; otherwise write {"<ID>": ["<PR URL>", ...]} to
# $WORK_DIR/closed.json.
#   PARITY_JSON  output of parse_parity_file
validate_closed_ids() {
  local parity_json="$1" pair id
  for pair in ${CLOSED_ARGS[@]+"${CLOSED_ARGS[@]}"}; do
    id="${pair%%=*}"
    jq -e --arg id "$id" 'any(.rows[]; .id == $id)' "$parity_json" >/dev/null ||
      usage_error "--closed: $id is not an ID in $PARITY_FILE; check the spelling"
  done
  # One "ID=URL" pair per line -> {"ID": [URL, ...]}, in command-line order.
  printf '%s\n' ${CLOSED_ARGS[@]+"${CLOSED_ARGS[@]}"} | jq -R -s '
    split("\n") | map(select(length > 0) | capture("^(?<id>[^=]+)=(?<url>.+)$"))
    | reduce .[] as $p ({}; .[$p.id] += [$p.url])
  ' >"$WORK_DIR/closed.json"
}

# ---------------------------------------------------------------------------
# GitHub access, only through gh.

# print_gh_error: copy the stderr of the last failed gh call to stderr, if
# it said anything. No parameters.
print_gh_error() {
  if [[ -s "$WORK_DIR/gh.err" ]]; then
    printf '%s\n' "$(<"$WORK_DIR/gh.err")" >&2
  fi
}

# gh_read OUT WHAT HINT ARGS...: run a read-only gh command and write its
# stdout to OUT. On failure exit 1 with gh's own message, what was being
# read and a hint.
#   OUT   file for gh's stdout
#   WHAT  what was being read, for the error message ("read project 3 of Verseghy")
#   HINT  what the operator should check if it fails
#   ARGS  the gh arguments
gh_read() {
  local out="$1" what="$2" hint="$3"
  shift 3
  if ! gh "$@" </dev/null >"$out" 2>"$WORK_DIR/gh.err"; then
    print_gh_error
    die "could not $what (gh's message is above); $hint"
  fi
}

# gh_write ID ARGS...: run a gh command that changes GitHub. Its stdout lands
# in $WORK_DIR/gh.out. On failure exit 1 and say how to resume. stdin comes
# from /dev/null, so gh can never consume the action list that the caller's
# loop is reading.
#   ID    the parity ID the write is for (empty for the label), for the message
#   ARGS  the gh arguments
gh_write() {
  local id="$1"
  shift
  if ! gh "$@" </dev/null >"$WORK_DIR/gh.out" 2>"$WORK_DIR/gh.err"; then
    print_gh_error
    die "gh $1 $2 failed${id:+ for $id} (gh's message is above). Changes made before it are kept; fix the cause and run the same command again to continue"
  fi
}

# fetch_issues OUT: read the repository's canonical name, whether the parity
# label exists, and every parity issue. Writes
# {"repo": "Owner/Name", "label": true|false, "issues": [{number, title, body, state, url}]}.
#   OUT  file to write the JSON to
fetch_issues() {
  local out="$1" pages="$WORK_DIR/issues.pages.json"
  gh_read "$pages" "read the issues of $REPO" "check --repo and that 'gh auth status' shows a valid login" \
    api graphql --paginate --slurp -f query="$ISSUES_QUERY" -f owner="${REPO%%/*}" -f name="${REPO#*/}"
  jq '{ repo: .[0].data.repository.nameWithOwner,
        label: (.[0].data.repository.label != null),
        issues: [ .[].data.repository.issues.nodes[] ] }' "$pages" >"$out"
}

# fetch_board OUT: read the project and check its fields. Writes
# {"project_id": ..., "fields": {"Disposition": {id, options: {name: id}},
#  "ADR": {id}, "Area": {id}}}. Exits 1 if the project is missing, or if a
# field or a Disposition option is missing or has the wrong type; the message
# names what to create.
#   OUT  file to write the JSON to
fetch_board() {
  local out="$1" raw="$WORK_DIR/board.raw.json" errors
  gh_read "$raw" "read project $PROJECT of $OWNER" \
    "check the number (gh project list --owner $OWNER) and that the token has the project scope (gh auth refresh -s project)" \
    api graphql -f query="$BOARD_QUERY" -f owner="$OWNER" -F number="$PROJECT"
  jq -e '.data.repositoryOwner.projectV2 != null' "$raw" >/dev/null ||
    die "$OWNER has no project number $PROJECT; list its projects with: gh project list --owner $OWNER"

  errors="$(jq -r --arg n "$PROJECT" --arg owner "$OWNER" --arg dispositions "$DISPOSITIONS" '
    def create($name; $type):
      "gh project field-create \($n) --owner \($owner) --name \($name) --data-type \($type)"
      + (if $type == "SINGLE_SELECT" then " --single-select-options \"\($dispositions | split(" ") | join(","))\"" else "" end);
    def check($name; $type):
      (.[$name]) as $f
      | if $f == null then "the project has no \"\($name)\" field; create it with: \(create($name; $type))"
        elif $f.dataType != $type then
          "the project'"'"'s \"\($name)\" field is \($f.dataType), but it must be \($type); delete it in the project settings, then create it with: \(create($name; $type))"
        else empty end;
    (.data.repositoryOwner.projectV2.fields.nodes | map(select(.name != null) | {key: .name, value: .}) | from_entries) as $fields
    | ($fields | check("Disposition"; "SINGLE_SELECT"), check("ADR"; "TEXT"), check("Area"; "TEXT")),
      ( $fields.Disposition | select(. != null and .dataType == "SINGLE_SELECT")
        | [.options[].name] as $have
        | $dispositions | split(" ")[] | select(. as $o | $have | index([$o]) | not)
        | "the project'"'"'s \"Disposition\" field has no option \"\(.)\"; add the option \"\(.)\" to the field in the project settings (Settings, Fields, Disposition)" )
  ' "$raw")"
  [[ -z "$errors" ]] || die "project $PROJECT of $OWNER is not set up for the parity board:"$'\n'"  ${errors//$'\n'/$'\n'  }"

  jq '.data.repositoryOwner.projectV2 as $p
      | ($p.fields.nodes | map(select(.name != null) | {key: .name, value: .}) | from_entries) as $f
      | { project_id: $p.id,
          fields: { Disposition: { id: $f.Disposition.id, options: ($f.Disposition.options | map({key: .name, value: .id}) | from_entries) },
                    ADR: { id: $f.ADR.id },
                    Area: { id: $f.Area.id } } }' "$raw" >"$out"
}

# fetch_items OUT: read the board's items and keep those whose content is an
# issue of $REPO. Writes {"<issue number>": {"item_id": ..., "values":
# {"<field id>": "<text or option id>"}}}.
#   OUT  file to write the JSON to
fetch_items() {
  local out="$1" pages="$WORK_DIR/items.pages.json"
  gh_read "$pages" "read the items of project $PROJECT of $OWNER" \
    "check that the token has the project scope (gh auth refresh -s project)" \
    api graphql --paginate --slurp -f query="$ITEMS_QUERY" -f owner="$OWNER" -F number="$PROJECT"
  jq --arg repo "$REPO" '
    [ .[].data.repositoryOwner.projectV2.items.nodes[]
      | select(.content.number != null and ((.content.repository.nameWithOwner // "") | ascii_downcase) == ($repo | ascii_downcase)) ]
    | map({ key: (.content.number | tostring),
            value: { item_id: .id,
                     values: ([ .fieldValues.nodes[] | select(.field.id != null) | {key: .field.id, value: (.optionId // .text)} ] | from_entries) } })
    | from_entries' "$pages" >"$out"
}

# check_prs_merged PLAN: exit 1 unless every PR that a planned --closed
# comment links is merged, so no issue is closed for unfinished work.
#   PLAN  output of build_plan
check_prs_merged() {
  local plan="$1" url state
  while IFS= read -r url; do
    [[ -n "$url" ]] || continue
    gh_read "$WORK_DIR/pr.json" "read pull request $url" "check the --closed URL" pr view "$url" --json state
    state="$(jq -r '.state' "$WORK_DIR/pr.json")"
    [[ "$state" == "MERGED" ]] ||
      die "--closed: $url is $state, not merged; pass --closed only for work that has merged (ADR 0019 section 1, closing rule 6)"
  done < <(jq -r '.prs_to_check[]' "$plan")
}

# ---------------------------------------------------------------------------
# Planning: a pure jq function of the parity rows and what GitHub holds now.

# The plan program. Inputs, all through --slurpfile (arrays of one) or --arg:
#   $parity  {rows, refs}            from parse_parity_file
#   $gh      {repo, label, issues}   from fetch_issues
#   $board   {project_id, fields}    from fetch_board
#   $items   {"<number>": {...}}     from fetch_items
#   $closed  {"<ID>": [PR URL]}      from validate_closed_ids
#   $docs_base, $parity_path, $adr_path, $label
# Output: {errors, warnings, actions, texts, prs_to_check, summary}. Each
# action has kind (label, create, add, field, edit, comment, close), id, line
# (the line printed for it) and the values the executor needs.
readonly PLAN_PROGRAM='
$parity[0] as $parity | $gh[0] as $gh | $board[0] as $board | $items[0] as $items | $closed[0] as $closed
| $gh.repo as $repo

# blob(path): absolute URL of a repository path on master.
| def blob($path): "https://github.com/\($repo)/blob/master/\($path)";
  # absolute: a link target from parity.md, resolved against docs/design/ on
  # master. URLs with a scheme, fragments and root-relative paths stay as they are.
  def absolute:
    if test("^[A-Za-z][A-Za-z0-9+.-]*:") or startswith("#") or startswith("/") then .
    else blob($docs_base + "/" + .) end;
  def absolute_inline_links: gsub("\\]\\((?<u>[^()\\s]+)\\)"; "](\(.u | absolute))");
  # uses_ref(key): true if the text uses [key] as a reference link, that is,
  # [key] or [text][key], and not as the text of an inline link [key](...).
  def uses_ref($key): split("[" + $key + "]") | .[1:] | any(startswith("(") | not);
  # The "closing PR contains" column of the table in ADR 0019 section 1.
  def done_when:
    { "port": "the feature and a test that pins today'"'"'s behaviour",
      "port-with-fix": "the feature and a test that pins the fixed behaviour named in the notes",
      "replaced": "the replacement and its tests",
      "retired": "the change that implements the retirement (for example the importer step that skips a table, or the runbook pointer), plus the documented exception in ADR 0019 section 3",
      "new": "the feature and its tests" }[.];
  # GitHub rejects titles over 256 characters. jq counts code points, as GitHub does.
  def issue_title:
    "[\(.id)] \(.feature)" as $t
    | if ($t | length) <= 256 then $t else ($t[0:255] | sub("\\s+$"; "")) + "…" end;
  def issue_body:
    . as $r
    | [ "- **Feature:** \($r.feature)",
        "- **Disposition:** `\($r.disposition)`",
        "- **ADR:** \($r.adr)",
        "- **Area:** \($r.area)",
        "- **Notes:** \(if $r.notes == "" then "none" else $r.notes end)",
        "",
        "**Done when:** the PR that closes this issue is merged, and it contains \($r.disposition | done_when). [ADR 0019 §1](\(blob($adr_path))) gives the closing rule for each kind of ID.",
        "",
        "---",
        "",
        "_This issue is generated from [\($parity_path)](\(blob($parity_path))) by `scripts/sync-parity-issues.sh`. Edit the row there, not this issue: the next sync overwrites the title and the body. The project board tracks the status._" ]
    | join("\n") | absolute_inline_links
    | . as $text
    # Only the reference definitions the body uses, in parity.md order.
    | [ $parity.refs[] | select(.key as $k | $text | uses_ref($k)) | "[\(.key)]: \(.url | absolute)" ] as $defs
    | if ($defs | length) == 0 then $text else $text + "\n\n" + ($defs | join("\n")) end;
  # GitHub returns a body edited in the browser with CRLF line ends and may
  # drop trailing whitespace; neither counts as a change.
  def normalise: gsub("\r"; "") | sub("\\s+$"; "");
  def sentence_list: if length <= 1 then join("") else (.[0:-1] | join(", ")) + " and " + .[-1] end;
  def closing_comment:
    "Done in \(sentence_list), which merged before this issue existed. The sync script created this issue closed, as closing rule 6 of [ADR 0019 §1](\(blob($adr_path))) describes.";

  ($parity.rows | map({key: .id, value: true}) | from_entries) as $known
| ($gh.issues | map(. + {key: ((.title | capture("^\\[(?<id>[^\\]]+)\\] ")) // {} | .id)})) as $keyed
| ($keyed | map(select(.key != null)) | group_by(.key) | map(select(length > 1))) as $dups
| ($keyed | map(select(.key != null) | {key, value: .}) | from_entries) as $by_id

| [ $parity.rows[] as $r
    | $by_id[$r.id] as $issue
    | ($r | issue_title) as $title
    | ($r | issue_body) as $body
    | (if $issue == null then null else $items[$issue.number | tostring] end) as $item
    | ($closed[$r.id] // []) as $prs
    | (if $issue == null then "[\($r.id)]" else "[\($r.id)] #\($issue.number)" end) as $ref
    | ($r.adr | gsub("\\[(?<k>[^\\]]*)\\]"; .k)) as $adr_text
    # The board fields whose current value differs from parity.md (all of
    # them for an item that is not on the board yet).
    | [ {name: "Disposition", field_id: $board.fields.Disposition.id, field_kind: "option",
         value: $board.fields.Disposition.options[$r.disposition], shown: $r.disposition},
        {name: "ADR", field_id: $board.fields.ADR.id, field_kind: "text", value: $adr_text, shown: $adr_text},
        {name: "Area", field_id: $board.fields.Area.id, field_kind: "text", value: $r.area, shown: $r.area} ]
      | map(select(($item.values[.field_id] // null) != .value)) as $fields
    | ($fields | map({kind: "field", number: $issue.number, item_id: $item.item_id, field_id, field_kind, value,
                      line: "set field \($ref) \(.name) = \(.shown)"})) as $field_actions
    | { id: $r.id, title: $title, body: $body, comment: (if ($prs | length) > 0 then $prs | closing_comment else null end),
        warning: (if $issue != null and ($prs | length) > 0 and $issue.state == "OPEN" then
                    "--closed \($r.id): issue #\($issue.number) already exists and is open, so it is left open. If its work is done and a previous run stopped before closing it, finish by hand: gh issue close \($issue.number) --repo \($repo) --reason completed --comment \"Done in \($prs | join(" and "))\""
                  else null end),
        prs: (if $issue == null then $prs else [] end),
        actions: (
          if $issue == null then
            [ {kind: "create", line: "create issue \($title)"},
              {kind: "add", line: "add to project \($ref)"} ]
            + $field_actions
            + (if ($prs | length) > 0 then
                 [ {kind: "comment", line: "comment on issue \($ref): \($prs | join(" "))"},
                   {kind: "close", line: "close issue \($ref)"} ]
               else [] end)
          else
            [ (if $issue.title != $title then "title" else empty end),
              (if ($issue.body | normalise) != $body then "body" else empty end) ] as $changed
            | (if ($changed | length) > 0 then
                 [ {kind: "edit", number: $issue.number, value: ($changed | join(",")),
                    line: "update issue \($ref): \($changed | join(", "))"} ]
               else [] end)
            + (if $item == null then [ {kind: "add", number: $issue.number, url: $issue.url, line: "add to project \($ref)"} ] else [] end)
            + $field_actions
          end
          | map(. + {id: $r.id}) ) } ] as $entries

| ( (if $gh.label then [] else [ {kind: "label", id: "", line: "create label \($label)"} ] end)
    + [ $entries[].actions[] ] ) as $actions
| { errors: [ $dups[] | "issues \(map("#\(.number)") | join(" and ")) all have the title prefix [\(.[0].key)] and the parity label; remove the label from all but one of them, then run again" ],
    warnings: ( [ $keyed[] | select(.key == null)
                  | "issue #\(.number) has the parity label, but its title does not start with \"[<ID>] \"; it is ignored. Restore the prefix, or remove the label" ]
              + [ $keyed[] | select(.key != null and ($known[.key] | not))
                  | "issue #\(.number) [\(.key)] has no row in parity.md; it is left as it is. IDs are never removed: restore the row, with the disposition retired if the feature was dropped" ]
              + [ $entries[] | .warning | select(. != null) ] ),
    actions: $actions,
    texts: ( $entries | map({key: .id, value: {title, body, comment}}) | from_entries ),
    prs_to_check: [ $entries[].prs[] ] | unique,
    summary: { created: ([ $actions[] | select(.kind == "create") ] | length),
               updated: ([ $actions[] | select(.kind == "edit") ] | length),
               closed: ([ $actions[] | select(.kind == "close") ] | length),
               added: ([ $actions[] | select(.kind == "add") ] | length),
               fields: ([ $actions[] | select(.kind == "field") ] | length),
               unchanged: ([ $entries[] | select((.actions | length) == 0) ] | length) } }
'

# build_plan OUT: compute what to change from the files fetched so far and
# write the plan JSON to OUT. Exits 1 on a conflict that needs a person
# (two issues for one ID).
#   OUT  file to write the plan to
build_plan() {
  local out="$1" errors
  jq -n \
    --slurpfile parity "$WORK_DIR/parity.json" \
    --slurpfile gh "$WORK_DIR/issues.json" \
    --slurpfile board "$WORK_DIR/board.json" \
    --slurpfile items "$WORK_DIR/items.json" \
    --slurpfile closed "$WORK_DIR/closed.json" \
    --arg docs_base "$DOCS_BASE" --arg parity_path "$PARITY_PATH_ON_MASTER" \
    --arg adr_path "$ADR_0019_PATH" --arg label "$LABEL_NAME" \
    "$PLAN_PROGRAM" >"$out"
  errors="$(jq -r '.errors[]' "$out")"
  [[ -z "$errors" ]] || die "$errors"
}

# ---------------------------------------------------------------------------
# Executing the plan.

# apply_actions PLAN: print each action's line and, unless --dry-run, carry it
# out through gh. Actions of one ID are contiguous and in order (create, add,
# fields, comment, close), so the number, URL and item ID that a create or an
# add returns are kept in cur_* for the actions that follow it.
#   PLAN  output of build_plan
apply_actions() {
  local plan="$1" list="$WORK_DIR/actions.list" body_file="$WORK_DIR/text.md"
  local kind id number url item_id field_id field_kind value line title
  local cur_id="" cur_number="" cur_url="" cur_item=""
  local -a edit_args

  # One action per line; values are single-line, so a line-based read is safe.
  # Titles and bodies, which can be long, are read from the plan by ID instead.
  jq -r '.actions[] | [.kind, .id, .number, .url, .item_id, .field_id, .field_kind, .value, .line]
                    | map(if . == null then "" else tostring end) | join("\u001f")' "$plan" >"$list"

  while IFS="$US" read -r kind id number url item_id field_id field_kind value line; do
    printf '%s\n' "$line"
    ((DRY_RUN == 0)) || continue

    if [[ "$id" != "$cur_id" ]]; then
      cur_id="$id" cur_number="" cur_url="" cur_item=""
    fi
    [[ -z "$number" ]] || cur_number="$number"
    [[ -z "$url" ]] || cur_url="$url"
    [[ -z "$item_id" ]] || cur_item="$item_id"

    case "$kind" in
      label)
        gh_write "" label create "$LABEL_NAME" --repo "$REPO" --color "$LABEL_COLOR" --description "$LABEL_DESCRIPTION"
        ;;
      create)
        title="$(jq -r --arg id "$id" '.texts[$id].title' "$plan")"
        jq -j --arg id "$id" '.texts[$id].body' "$plan" >"$body_file"
        gh_write "$id" issue create --repo "$REPO" --title "$title" --body-file "$body_file" --label "$LABEL_NAME"
        # gh issue create prints the new issue's URL; the number is its last segment.
        cur_url="$(<"$WORK_DIR/gh.out")"
        cur_number="${cur_url##*/}"
        [[ "$cur_number" =~ ^[0-9]+$ ]] || die "gh issue create for $id printed '$cur_url', not an issue URL; check the issue by hand"
        ;;
      add)
        gh_write "$id" project item-add "$PROJECT" --owner "$OWNER" --url "$cur_url" --format json
        cur_item="$(jq -r '.id' "$WORK_DIR/gh.out")"
        ;;
      field)
        if [[ "$field_kind" == "option" ]]; then
          gh_write "$id" project item-edit --id "$cur_item" --project-id "$PROJECT_ID" --field-id "$field_id" --single-select-option-id "$value"
        else
          gh_write "$id" project item-edit --id "$cur_item" --project-id "$PROJECT_ID" --field-id "$field_id" --text "$value"
        fi
        ;;
      edit)
        edit_args=()
        if [[ ",$value," == *",title,"* ]]; then
          edit_args+=(--title "$(jq -r --arg id "$id" '.texts[$id].title' "$plan")")
        fi
        if [[ ",$value," == *",body,"* ]]; then
          jq -j --arg id "$id" '.texts[$id].body' "$plan" >"$body_file"
          edit_args+=(--body-file "$body_file")
        fi
        gh_write "$id" issue edit "$cur_number" --repo "$REPO" "${edit_args[@]}"
        ;;
      comment)
        jq -j --arg id "$id" '.texts[$id].comment' "$plan" >"$body_file"
        gh_write "$id" issue comment "$cur_number" --repo "$REPO" --body-file "$body_file"
        ;;
      close)
        gh_write "$id" issue close "$cur_number" --repo "$REPO" --reason completed
        ;;
      *) die "internal error: unknown action '$kind'" ;;
    esac
  done <"$list"
}

# print_summary PLAN: print the change counts of the plan, and say so when
# nothing was written because of --dry-run.
#   PLAN  output of build_plan
print_summary() {
  local plan="$1"
  jq -r --argjson dry "$DRY_RUN" '.summary
    | "summary: created \(.created), updated \(.updated), closed \(.closed), added to the project \(.added), fields set \(.fields), unchanged \(.unchanged)"
      + (if $dry == 1 then " (dry run: nothing was changed)" else "" end)' "$plan"
}

# main ARGS...: validate, read, plan, then apply or print.
#   ARGS  the script's command-line arguments
main() {
  parse_args "$@"
  validate_args
  require_tools

  WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sync-parity-issues.XXXXXX")"
  trap 'rm -rf "$WORK_DIR"' EXIT

  parse_parity_file "$PARITY_FILE" "$WORK_DIR/parity.json"
  validate_closed_ids "$WORK_DIR/parity.json"

  fetch_issues "$WORK_DIR/issues.json"
  # Use GitHub's spelling of the repository from here on, so that links in the
  # bodies do not change when --repo is typed in a different case.
  REPO="$(jq -r '.repo' "$WORK_DIR/issues.json")"
  OWNER="${REPO%%/*}"
  fetch_board "$WORK_DIR/board.json"
  PROJECT_ID="$(jq -r '.project_id' "$WORK_DIR/board.json")"
  fetch_items "$WORK_DIR/items.json"

  build_plan "$WORK_DIR/plan.json"
  while IFS= read -r message; do
    [[ -z "$message" ]] || warn "$message"
  done < <(jq -r '.warnings[]' "$WORK_DIR/plan.json")
  check_prs_merged "$WORK_DIR/plan.json"

  apply_actions "$WORK_DIR/plan.json"
  print_summary "$WORK_DIR/plan.json"
}

main "$@"
