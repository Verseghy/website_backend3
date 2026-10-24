#!/usr/bin/env bash
# check-session-links.sh: fail when a Claude Code session link appears in a PR.
#
# Why this exists: website_backend3 is public, and no claude.ai session link
# may appear anywhere in it (ADR 0012 section 3, ADR 0024 section 1). A
# session link is an http or https URL whose host is claude.ai and whose path
# starts with /code/session_. The "No session links" CI job (a required
# context, ADR 0022 section 3) runs this script on every pull_request event,
# including `edited`, so a body edit is checked while the PR is open.
#
# What it scans:
#   --event FILE   the PR title and body from a GitHub event payload
#                  (`$GITHUB_EVENT_PATH`), read with jq, so the text is never
#                  interpolated into a shell command;
#   --base SHA --head SHA
#                  every commit message in BASE..HEAD, and every added line of
#                  the diff BASE...HEAD (merge-base to head, renames detected,
#                  so moving an existing file adds nothing new);
#   --file PATH    the contents of a file (repeatable; for local use and tests).
# It matches only real links: scheme, host (claude.ai or a subdomain, any
# letter case, optional userinfo and port) and the path prefix. Prose that
# names the rule without a URL passes.
#
# The pattern is assembled from parts at run time (build_pattern), so this
# file never contains text that the pattern matches and the script can scan
# itself; scripts/tests/test-check-session-links.sh does exactly that.
#
# Findings name the place (title, body line, commit and line, file and line)
# but never print the link itself, because CI logs of a public repository are
# public too.
#
# Usage:
#   check-session-links.sh [--event FILE] [--base SHA --head SHA]
#                          [--repo DIR] [--file PATH]...
#
# Exit codes: 0 no link found, 1 at least one link found, 2 usage or input
# error (missing file, unknown commit, malformed payload).
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: check-session-links.sh [--event FILE] [--base SHA --head SHA]
                              [--repo DIR] [--file PATH]...

  --event FILE  GitHub pull_request event payload; scans the PR title and body
  --base SHA    base commit of the PR (needs --head)
  --head SHA    head commit of the PR; scans commit messages in BASE..HEAD and
                the lines added by BASE...HEAD
  --repo DIR    git repository for --base/--head (default: current directory)
  --file PATH   also scan this file (repeatable)
EOF
}

die_usage() {
  echo "error: $*" >&2
  echo "Run with --help for usage." >&2
  exit 2
}

# build_pattern: print the extended regex for a session link.
# Each piece is kept separate on purpose (see the header); grep runs it with
# -i because scheme and host are case-insensitive in URLs.
build_pattern() {
  local scheme='https?'
  local userinfo='([^/?#@[:space:]]*@)?'
  local subdomains='([a-z0-9-]+\.)*'
  local host_name='claude'
  local host_tld='ai'
  local port='(:[0-9]+)?'
  local path_dir='code'
  local path_prefix='session_'
  printf '%s' "${scheme}://${userinfo}${subdomains}${host_name}\\.${host_tld}${port}/${path_dir}/${path_prefix}"
}

PATTERN=$(build_pattern)
readonly PATTERN
findings=()

# matching_lines: read text on stdin and print the 1-based numbers of the
# lines that contain a session link, one per line.
matching_lines() {
  grep -n -i -E -e "$PATTERN" | cut -d: -f1 || true
}

# scan_text LABEL TEXT: record a finding for every line of TEXT with a link.
scan_text() {
  local label=$1 text=$2 n
  while IFS= read -r n; do
    [[ -n $n ]] && findings+=("$label, line $n")
  done < <(printf '%s\n' "$text" | matching_lines)
}

# scan_event FILE: scan the PR title and body of a pull_request event payload.
scan_event() {
  local event=$1
  [[ -r $event ]] || die_usage "event payload '$event' is not a readable file."
  if ! jq -e '(.pull_request | type) == "object"' "$event" >/dev/null 2>&1; then
    die_usage "'$event' is not a pull_request event payload (no .pull_request object). This check runs on pull_request events only."
  fi
  local title body
  title=$(jq -r '.pull_request.title // ""' "$event")
  body=$(jq -r '.pull_request.body // ""' "$event")
  if [[ -n $(printf '%s\n' "$title" | matching_lines) ]]; then
    findings+=("PR title")
  fi
  scan_text "PR body" "$body"
}

# resolve_commit REPO REV: print the full SHA of REV or exit 2 with a hint.
resolve_commit() {
  local repo=$1 rev=$2
  git -C "$repo" rev-parse --verify --quiet "${rev}^{commit}" ||
    die_usage "commit '$rev' is not in the repository at '$repo'. In CI, check out with fetch-depth: 0 so the base and head commits are present."
}

# scan_range REPO BASE HEAD: scan commit messages and added diff lines.
scan_range() {
  local repo=$1 base head sha message
  base=$(resolve_commit "$repo" "$2")
  head=$(resolve_commit "$repo" "$3")

  while IFS= read -r sha; do
    [[ -n $sha ]] || continue
    message=$(git -C "$repo" log -1 --format=%B "$sha")
    scan_text "commit ${sha:0:12} message" "$message"
  done < <(git -C "$repo" rev-list "$base..$head")

  # --unified=0 leaves only added and removed lines under each hunk header.
  # Explicit prefixes and core.quotePath=false keep the "+++ b/<path>" lines
  # parseable whatever the local git configuration says.
  # A "+++ " line is a file name only in a file's header, between its
  # "diff --git" line and its first "@@" hunk header: inside a hunk, an added
  # line whose text starts with "++ " also begins with "+++ ", and it must be
  # scanned like any other added line.
  local diff_lines
  if ! diff_lines=$(
    git -C "$repo" -c core.quotePath=false diff --no-color --no-ext-diff \
      --find-renames --unified=0 --diff-filter=d \
      --src-prefix=a/ --dst-prefix=b/ "$base...$head" |
      awk '
        /^diff --git / { header = 1; next }
        header && /^\+\+\+ / { file = substr($0, 5); sub(/^"?b\//, "", file); sub(/"$/, "", file); next }
        /^@@ /     { header = 0; match($0, /\+[0-9]+/); line = substr($0, RSTART + 1, RLENGTH - 1) + 0; next }
        !header && /^\+/ { printf "%s\t%d\t%s\n", file, line, substr($0, 2); line++; next }
      '
  ); then
    die_usage "git diff $base...$head failed; the two commits need a common ancestor."
  fi
  local hit
  while IFS= read -r hit; do
    [[ -n $hit ]] || continue
    findings+=("added line $(printf '%s' "$hit" | cut -f1):$(printf '%s' "$hit" | cut -f2)")
  done < <(printf '%s\n' "$diff_lines" | grep -i -E -e "$PATTERN" || true)
}

# scan_file PATH: scan a file's contents.
scan_file() {
  local path=$1 n
  [[ -r $path ]] || die_usage "file '$path' is not readable."
  while IFS= read -r n; do
    [[ -n $n ]] && findings+=("file $path, line $n")
  done < <(matching_lines <"$path")
}

main() {
  local event="" base="" head="" repo="."
  local -a files=()
  while (($# > 0)); do
    case $1 in
      --event) [[ $# -ge 2 ]] || die_usage "--event needs a file"; event=$2; shift 2 ;;
      --base) [[ $# -ge 2 ]] || die_usage "--base needs a commit"; base=$2; shift 2 ;;
      --head) [[ $# -ge 2 ]] || die_usage "--head needs a commit"; head=$2; shift 2 ;;
      --repo) [[ $# -ge 2 ]] || die_usage "--repo needs a directory"; repo=$2; shift 2 ;;
      --file) [[ $# -ge 2 ]] || die_usage "--file needs a path"; files+=("$2"); shift 2 ;;
      -h | --help) usage; return 0 ;;
      *) die_usage "unknown argument '$1'" ;;
    esac
  done
  if [[ -n $base && -z $head ]] || [[ -z $base && -n $head ]]; then
    die_usage "--base and --head must be given together."
  fi
  if [[ -z $event && -z $base && ${#files[@]} -eq 0 ]]; then
    die_usage "nothing to scan; pass --event, --base/--head or --file."
  fi
  command -v jq >/dev/null || die_usage "jq is required but not installed."
  command -v git >/dev/null || die_usage "git is required but not installed."

  [[ -n $event ]] && scan_event "$event"
  [[ -n $base ]] && scan_range "$repo" "$base" "$head"
  local f
  for f in "${files[@]}"; do
    scan_file "$f"
  done

  if ((${#findings[@]} == 0)); then
    echo "No session links found."
    return 0
  fi
  {
    echo "Found ${#findings[@]} claude.ai session link(s). They are banned in this public repository (ADR 0012 section 3):"
    local place
    for place in "${findings[@]}"; do
      echo "  - $place"
    done
    echo
    echo "Remove each one: edit the PR title or body on GitHub, reword commit messages"
    echo "(git rebase, then push the rewritten branch), and delete or rewrite the added lines."
    echo "The check runs again on the 'edited' and 'synchronize' events."
  } >&2
  return 1
}

main "$@"
