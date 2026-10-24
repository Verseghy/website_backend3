#!/usr/bin/env bash
# check-contract-drift.sh: fail when regenerated API contract files differ
# from what is committed.
#
# Why this exists: the Rust DTOs are the API contract (ADR 0005). The ts-rs
# export tests rewrite admin/src/bindings/, and the insta snapshot tests pin
# the OpenAPI document (and any other snapshot) under */snapshots/. The
# "Contract" CI job (a required context, ADR 0022 section 3) runs those tests
# and then this script. A DTO change without the regenerated bindings, or a
# binding edited by hand, therefore fails the PR instead of drifting.
#
# It uses `git status --porcelain --untracked-files=all` rather than
# `git diff --exit-code`, because a new DTO produces a new, untracked binding
# file that `git diff` would not report (ADR 0005 section 4).
#
# Usage: check-contract-drift.sh [--repo DIR]   (default: current directory)
#
# Exit codes: 0 no drift, 1 drift found, 2 usage error (not a git work tree).
set -euo pipefail

# The generated paths, as git pathspecs relative to the repository root.
readonly -a CONTRACT_PATHS=(
  'admin/src/bindings'
  ':(glob)**/snapshots/**'
)

usage() {
  echo "Usage: check-contract-drift.sh [--repo DIR]   (default: current directory)"
}

die_usage() {
  echo "error: $*" >&2
  exit 2
}

main() {
  local repo=.
  while (($# > 0)); do
    case $1 in
      --repo) [[ $# -ge 2 ]] || die_usage "--repo needs a directory"; repo=$2; shift 2 ;;
      -h | --help) usage; return 0 ;;
      *) die_usage "unknown argument '$1'" ;;
    esac
  done
  command -v git >/dev/null || die_usage "git is required but not installed."
  git -C "$repo" rev-parse --is-inside-work-tree >/dev/null 2>&1 ||
    die_usage "'$repo' is not inside a git work tree."
  local top
  top=$(git -C "$repo" rev-parse --show-toplevel)

  local status
  status=$(git -C "$top" status --porcelain=v1 --untracked-files=all -- "${CONTRACT_PATHS[@]}")
  if [[ -z $status ]]; then
    echo "Contract OK: admin/src/bindings/ and the snapshots match the committed files."
    return 0
  fi
  {
    echo "The generated API contract differs from the committed files (ADR 0005 section 4):"
    printf '%s\n' "$status" | sed 's/^/  /'
    echo
    echo "Changes to tracked files:"
    git -C "$top" --no-pager diff --stat -- "${CONTRACT_PATHS[@]}" | sed 's/^/  /'
    # head may close the pipe early; the resulting SIGPIPE is not an error.
    git -C "$top" --no-pager diff --no-color -- "${CONTRACT_PATHS[@]}" | head -n 200 || true
    echo
    echo "Fix: run 'cargo test' locally (the ts-rs export tests rewrite admin/src/bindings/),"
    echo "review snapshot changes with 'cargo insta review', and commit the results."
    echo "Never edit admin/src/bindings/ by hand. A changed OpenAPI snapshot is an API change:"
    echo "say so in the PR and use the 'api' scope (ADR 0012 section 3)."
  } >&2
  return 1
}

main "$@"
