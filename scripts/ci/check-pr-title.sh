#!/usr/bin/env bash
# check-pr-title.sh: check a pull request title against the commit convention.
#
# Why this exists: Tide squash-merges every pull request, so the PR title
# becomes the commit subject on master (ADR 0012 section 3). The convention is
# therefore checked on the title, not on branch commits, which are squashed
# away. The "PR title" CI job runs this script and is a required context
# (ADR 0022 section 3).
#
# The rules, all from ADR 0012 section 3:
#   - format `<type>(<scope>[,<scope>...])[!]: <subject>`, checked with the
#     exact regex of the ADR (TITLE_REGEX below);
#   - the whole title is at most 90 characters, so the squash subject plus
#     Tide's " (#NNNN)" suffix stays within 100;
#   - the subject has no trailing period;
#   - the title is a single line.
# scripts/tests/test-check-pr-title.sh compares TITLE_REGEX with the regex
# printed in the ADR, so the two cannot drift apart silently.
#
# Usage:
#   check-pr-title.sh TITLE      check TITLE; exit 0 if valid, 1 if not
#   check-pr-title.sh --print-regex
#                                print the regex the check uses and exit 0
#   check-pr-title.sh --help
#
# CI passes the title from the event payload through an environment variable
# ("$PR_TITLE") and never interpolates `${{ }}` into the shell, so a crafted
# title cannot inject commands.
#
# Exit codes: 0 valid, 1 invalid title, 2 usage error.
set -euo pipefail

# Copied verbatim from ADR 0012 section 3. Bash's `=~` uses the platform's
# POSIX ERE engine; glibc's supports `\S`, as GitHub's Ubuntu runners do.
readonly TITLE_REGEX='^(build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)\((api|admin|db|import|deploy|docs)(,(api|admin|db|import|deploy|docs))*\)!?: \S.*$'
readonly MAX_LENGTH=90
readonly TYPES='build chore ci docs feat fix perf refactor revert style test'
readonly SCOPES='api admin db import deploy docs'

usage() {
  cat <<'EOF'
Usage: check-pr-title.sh TITLE
       check-pr-title.sh --print-regex

Checks TITLE against the PR title convention of ADR 0012 section 3:
  <type>(<scope>[,<scope>...])[!]: <subject>
at most 90 characters, no trailing period, one line.
EOF
}

# char_count TEXT: print the number of Unicode code points in TEXT.
# Counting UTF-8 lead bytes (every byte except 0x80-0xBF continuation bytes)
# gives the code-point count without depending on the runner's locale, so a
# Hungarian title such as "javítsa a ő betűt" is measured in characters.
char_count() {
  printf '%s' "$1" | LC_ALL=C tr -d '\200-\277' | wc -c | tr -d '[:space:]'
}

# in_list WORD LIST: succeed when WORD is one of the space-separated LIST.
in_list() {
  local word=$1 item
  for item in $2; do
    [[ $item == "$word" ]] && return 0
  done
  return 1
}

# explain_mismatch TITLE: print the most specific reason why TITLE does not
# match TITLE_REGEX. Only called after the regex has failed.
explain_mismatch() {
  local title=$1
  local head_re='^([^(:!]*)(\(([^)]*)\))?(!)?:(.*)$'
  if [[ $title =~ ^Revert\ \" ]]; then
    echo "GitHub's revert button produced this title. Rename it to"
    echo "'revert(<scope>): <subject>', for example 'revert(api): restore keyset paging'."
    return
  fi
  if [[ $title =~ ^[^:\ ]+[[:space:]]+: ]]; then
    echo "Remove the space before the colon."
    return
  fi
  if ! [[ $title =~ $head_re ]]; then
    echo "The title has no '<type>(<scope>): ' prefix."
    return
  fi
  local type=${BASH_REMATCH[1]} has_scope=${BASH_REMATCH[2]}
  local scopes=${BASH_REMATCH[3]} rest=${BASH_REMATCH[5]}
  if ! in_list "$type" "$TYPES"; then
    echo "Type '$type' is not allowed. Use one of: $TYPES."
    return
  fi
  if [[ -z $has_scope ]]; then
    echo "A scope is required, for example '$type(api): ...'. Scopes: $SCOPES."
    return
  fi
  local scope
  local -a scope_list
  IFS=',' read -r -a scope_list <<<"$scopes"
  if [[ -z $scopes || $scopes == *, || $scopes == ,* ]]; then
    echo "The scope list '($scopes)' is empty or has a stray comma."
    return
  fi
  for scope in "${scope_list[@]}"; do
    if ! in_list "$scope" "$SCOPES"; then
      echo "Scope '$scope' is not allowed (no spaces around commas). Use one or more of: $SCOPES."
      return
    fi
  done
  if [[ $rest != ' '* ]]; then
    echo "Put exactly one space after the colon."
    return
  fi
  echo "The subject must start with a non-space character right after ': '."
}

main() {
  if [[ $# -eq 1 && $1 == --print-regex ]]; then
    printf '%s\n' "$TITLE_REGEX"
    return 0
  fi
  if [[ $# -eq 1 && ($1 == --help || $1 == -h) ]]; then
    usage
    return 0
  fi
  if [[ $# -ne 1 ]]; then
    usage >&2
    echo "error: expected exactly one argument, the PR title (got $#)." >&2
    return 2
  fi

  local title=$1
  local -a problems=()

  if [[ -z $title ]]; then
    problems+=("The title is empty.")
  fi
  # A newline would let `.*$` in the regex accept a second line, so reject
  # every control character explicitly.
  if [[ $title == *[[:cntrl:]]* ]]; then
    problems+=("The title contains a line break or another control character; use one line.")
  fi
  local length
  length=$(char_count "$title")
  if ((length > MAX_LENGTH)); then
    problems+=("The title is $length characters long; the limit is $MAX_LENGTH (Tide appends ' (#NNNN)' and the subject must stay within 100). Shorten the subject.")
  fi
  if [[ -n $title ]] && ! [[ $title =~ $TITLE_REGEX ]]; then
    problems+=("$(explain_mismatch "$title")")
  fi
  if [[ $title == *. ]]; then
    problems+=("The subject ends with a period; drop it.")
  fi

  if ((${#problems[@]} == 0)); then
    echo "PR title OK: $title"
    return 0
  fi

  {
    echo "PR title does not follow the convention of ADR 0012 section 3:"
    echo "  title: $title"
    local p
    for p in "${problems[@]}"; do
      echo "  - $p"
    done
    echo
    echo "Format:  <type>(<scope>[,<scope>...])[!]: <subject>"
    echo "Types:   $TYPES"
    echo "Scopes:  $SCOPES"
    echo "Example: feat(db,api,admin): add labels with trash and history"
    echo "Regex:   $TITLE_REGEX"
    echo "Edit the PR title on GitHub; the check runs again on the 'edited' event."
  } >&2
  return 1
}

main "$@"
