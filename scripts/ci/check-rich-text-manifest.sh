#!/usr/bin/env bash
# check-rich-text-manifest.sh: fail when the rich-text policy manifest lets a
# forbidden tag into content-v1.
#
# Why this exists: backend3 stores one HTML dialect, content-v1, and the
# public sites insert it as trusted markup (ADR 0013). Thirteen tags must
# never be part of it: their parsing depends on the scripting flag, on raw
# text or on quirks mode, which is exactly what differs along the sites'
# sink chain (docs/design/editor.md §3.5, "Never in the tag list"). The
# `rich_text_manifest` task writes admin/src/rich-text/policy.json from the
# Rust policy and refuses such a tag itself; this check reads the committed
# file with jq and its own copy of the list, so a change to the Rust side
# that dropped the task's refusal still fails CI (editor.md §13.6,
# "Manifest"). The "Contract" CI job runs it after the drift check, which
# makes sure the committed file is the task's output.
#
# Rules:
#   1. The file is a JSON object with a `tags` array of strings, an
#      `elements` object and a `forbidden_tags` array of strings.
#   2. Neither `tags` nor the keys of `elements` holds a forbidden tag, in
#      any ASCII case (the HTML parser lower-cases tag names).
#   3. `forbidden_tags` lists every forbidden tag, so the admin, which reads
#      the list from the file, refuses all of them too.
#
# Usage: check-rich-text-manifest.sh [--file PATH]
#        (default: admin/src/rich-text/policy.json in the current directory)
#
# Exit codes: 0 no violation, 1 at least one violation or an unreadable
# manifest, 2 usage error.
set -euo pipefail

# editor.md §3.5. A change here needs an ADR that amends ADR 0013.
readonly FORBIDDEN_TAGS='["noscript","template","style","script","xmp","noembed","noframes","plaintext","textarea","title","listing","select","option"]'

readonly FIX="The tag list comes from src/domain/rich_text/policy.rs (ELEMENTS); \
remove the tag there, run 'cargo loco task rich_text_manifest' and commit \
admin/src/rich-text/ (docs/design/editor.md §3.5, §3.9)."

usage() {
  echo "Usage: check-rich-text-manifest.sh [--file PATH]   (default: admin/src/rich-text/policy.json)"
}

die_usage() {
  echo "error: $*" >&2
  exit 2
}

fail() {
  echo "error: $*" >&2
  exit 1
}

main() {
  local file=admin/src/rich-text/policy.json
  while (($# > 0)); do
    case $1 in
      --file) [[ $# -ge 2 ]] || die_usage "--file needs a path"; file=$2; shift 2 ;;
      -h | --help) usage; return 0 ;;
      *) die_usage "unknown argument '$1'" ;;
    esac
  done
  command -v jq >/dev/null || die_usage "jq is required but not installed."
  [[ -f $file ]] || fail "$file does not exist. Run 'cargo loco task rich_text_manifest' from the repository root."
  jq -e . "$file" >/dev/null 2>&1 || fail "$file is not valid JSON. Run 'cargo loco task rich_text_manifest' again."

  # Rule 1.
  local shape
  shape=$(jq -r '
    if type != "object" then "the manifest is not a JSON object"
    elif (.tags | type) != "array" or any(.tags[]; type != "string") then "`tags` is not an array of strings"
    elif (.elements | type) != "object" then "`elements` is not an object"
    elif (.forbidden_tags | type) != "array" or any(.forbidden_tags[]; type != "string") then "`forbidden_tags` is not an array of strings"
    else "" end' "$file")
  [[ -z $shape ]] || fail "$file: $shape. Run 'cargo loco task rich_text_manifest' again."

  local problems=()
  # Rule 2.
  local found
  found=$(jq -r --argjson forbidden "$FORBIDDEN_TAGS" '
    [ (.tags[] | {where: "tags", tag: .}), (.elements | keys[] | {where: "elements", tag: .}) ]
    | map(select((.tag | ascii_downcase) as $t | $forbidden | index($t)))
    | .[] | "\(.where) holds the forbidden tag `\(.tag)`"' "$file")
  if [[ -n $found ]]; then
    while IFS= read -r line; do problems+=("$line"); done <<<"$found"
  fi
  # Rule 3.
  local missing
  missing=$(jq -r --argjson forbidden "$FORBIDDEN_TAGS" '
    (.forbidden_tags | map(ascii_downcase)) as $listed
    | $forbidden[] | select(. as $t | $listed | index($t) | not)
    | "forbidden_tags does not list `\(.)`"' "$file")
  if [[ -n $missing ]]; then
    while IFS= read -r line; do problems+=("$line"); done <<<"$missing"
  fi

  if ((${#problems[@]} == 0)); then
    echo "Rich-text manifest OK: $file lets no forbidden tag into content-v1."
    return 0
  fi
  {
    echo "$file breaks the content-v1 tag rule (docs/design/editor.md §3.5):"
    printf '  %s\n' "${problems[@]}"
    echo
    echo "$FIX"
  } >&2
  return 1
}

main "$@"
