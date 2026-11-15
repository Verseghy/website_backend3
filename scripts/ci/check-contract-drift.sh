#!/usr/bin/env bash
# check-contract-drift.sh: fail when regenerated API contract files differ
# from what is committed.
#
# Why this exists: the Rust DTOs are the API contract (ADR 0005). The ts-rs
# export tests rewrite admin/src/bindings/, and the insta snapshot tests pin
# the OpenAPI document (and any other snapshot) under */snapshots/. The
# rich-text policy is a contract with the admin too: the rich_text_manifest
# task writes admin/src/rich-text/policy.json and css-vectors.jsonl from it
# (docs/design/editor.md §3.9). In the other direction, the admin's editor
# writes its golden files, tests/fixtures/e1/*.html, which the server's P2
# and P3 tests read (editor.md §13.6; npm run test:e1 in admin/). The
# "Contract" CI job (a required context, ADR 0022 section 3) runs those
# tests and the task, and then this script. A DTO, policy or editor change
# without the regenerated files, or a generated file edited by hand,
# therefore fails the PR instead of drifting.
#
# It uses `git status --porcelain --untracked-files=all` rather than
# `git diff --exit-code`, because a new DTO produces a new, untracked binding
# file that `git diff` would not report (ADR 0005 section 4).
#
# Usage: check-contract-drift.sh [--repo DIR]   (default: current directory)
#
# Exit codes: 0 no drift, 1 drift found, 2 usage error (not a git work tree).
set -euo pipefail

# The generated paths, as git pathspecs relative to the repository root. Of
# admin/src/rich-text/, only the two manifest files are generated: the
# admin's own rich-text code lives beside them. tests/fixtures/e1 is wholly
# generated, apart from its README.md, which the admin's test never writes.
readonly -a CONTRACT_PATHS=(
  'admin/src/bindings'
  ':(glob)**/snapshots/**'
  'admin/src/rich-text/policy.json'
  'admin/src/rich-text/css-vectors.jsonl'
  'tests/fixtures/e1'
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
    echo "Contract OK: admin/src/bindings/, the rich-text manifest, the editor's golden files and the snapshots match the committed files."
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
    echo "For admin/src/rich-text/, run 'cargo loco task rich_text_manifest' from the"
    echo "repository root and commit both files (docs/design/editor.md §3.9)."
    echo "For tests/fixtures/e1/, run 'npm run test:e1' (or 'npm test') in admin/ and"
    echo "commit what it writes (docs/design/editor.md §13.6, P2 and P3)."
    echo "Never edit admin/src/bindings/, the manifest files or the golden files by hand. A changed OpenAPI"
    echo "snapshot is an API change: say so in the PR and use the 'api' scope (ADR 0012 section 3)."
  } >&2
  return 1
}

main "$@"
