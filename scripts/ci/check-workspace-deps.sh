#!/usr/bin/env bash
# check-workspace-deps.sh: every Cargo dependency requirement lives in the
# root Cargo.toml's [workspace.dependencies], written so that Dependabot can
# rewrite it.
#
# Why this exists: the root Cargo.toml is both the workspace root and the
# backend3 package. When a root manifest has a [workspace.dependencies] table,
# Dependabot rewrites only that table of it (`workspace_root_manifest?` in
# dependabot-core@77f70df:cargo/lib/dependabot/cargo/file_updater.rb hands the
# root manifest to file_updater/workspace_manifest_updater.rb). A requirement
# in the root package's own [dependencies], [dev-dependencies] or
# [build-dependencies] (or their target forms) is still parsed, but an update
# beyond it arrives as a Cargo.lock-only PR that fails every `--locked` job
# (#188, #190). A member manifest such as migration/Cargo.toml is different:
# Dependabot rewrites its requirements (`ManifestUpdater`). Its crates still
# inherit every requirement, so that each requirement and its reason live in
# one place: a crate that both crates use has one requirement, one Dependabot
# change raises it for both, and the two cannot drift apart. Dependabot's
# parser skips every member entry that says `workspace = true`
# (cargo/lib/dependabot/cargo/file_parser.rb); .github/dependabot.yml explains
# the Dependabot side.
#
# Rules, for every manifest checked (see "Manifests" below):
#   1. Every entry of [dependencies], [dev-dependencies] and
#      [build-dependencies] (also the old dev_dependencies/build_dependencies
#      spellings, and each [target.<cfg>.*] form of the three) inherits from
#      the workspace: it is a table with `workspace = true`, in any TOML form
#      (`{ workspace = true }`, `name.workspace = true`, a
#      [dependencies.<name>] header).
#   2. Next to `workspace = true`, an entry holds only `features` and
#      `optional`. Everything else (`version`, `path`, `git`, `registry`,
#      `package`, `default-features`, ...) belongs in the workspace entry:
#      Cargo ignores or rejects it in a member, and `default-features = false`
#      only takes effect in the workspace entry.
#   3. In the root Cargo.toml, Dependabot's updater finds every requirement of
#      [workspace.dependencies] in the text. The rule runs the updater's own
#      search, its regular expressions ported to jq
#      (`update_version_in_section` and `update_table_header_notation` in
#      dependabot-core@77f70df:cargo/lib/dependabot/cargo/file_updater/workspace_manifest_updater.rb)
#      on the raw file, for each entry that the parser reads: from each literal
#      `[workspace.dependencies]` up to the next line that starts with "[", a
#      line that starts with the bare key, `name = "req"`, `name = 'req'` or
#      `name = { ... version = "req" ... }`; failing that, a
#      [workspace.dependencies.<name>] table with `version = "req"`. Every
#      other spelling is valid TOML that Cargo reads the same way, but the
#      updater does not find it, so its updates would change only Cargo.lock:
#      a dotted key (`name.version = "1"`), a quoted key (`"name" = "1"`,
#      `name = { "version" = "1" }`), `dependencies.name = "1"` under
#      [workspace], or spaces inside the header's brackets. Entries without a
#      requirement (`path` or `git` without `version`) are not checked: the
#      updater has nothing to rewrite there. Nor are renamed entries
#      (`package = "..."`): the parser skips them, so Dependabot proposes no
#      version update for them however they are written.
# Cargo itself validates the rest of [workspace.dependencies].
#
# Manifests: when the root is the top of a git work tree (the CI checkout, a
# clone, a worktree), the Cargo.toml files that git lists as tracked or as
# untracked and not ignored (`git ls-files --cached --others
# --exclude-standard`). That leaves out build output and every other ignored
# path, and nested repositories or worktrees, and still checks a new crate
# before its first `git add`. Elsewhere (the fixtures of the tests),
# every Cargo.toml under the root, skipping target/, node_modules/ and .git/.
# The root Cargo.toml is always checked.
#
# The "Toolchain" CI job (a required context, ADR 0022 section 3) runs this
# script; ADR 0029 section 3 lists it in that job's row of ADR 0022 section
# 3's table, beside rule 4 of check-toolchain.sh. It is a static check of the
# manifests and needs no Rust toolchain.
#
# Usage: check-workspace-deps.sh [--root DIR]   (default: the current directory)
#
# Needs mikefarah yq v4 (TOML to JSON), jq 1.6 or later (the rules) and, for
# the git listing, git. yq's TOML reader turns every form above into plain
# JSON objects; the rules run in jq because yq's own expressions treat
# dotted-key tables and scalars inconsistently.
#
# Exit codes: 0 every requirement is inherited and findable, 1 at least one
# violation, unreadable manifest or failed jq run, 2 usage error.
set -euo pipefail

usage() {
  echo "Usage: check-workspace-deps.sh [--root DIR]   (default: the current directory)"
}

die_usage() {
  echo "error: $*" >&2
  exit 2
}

# The jq program that lists the violations of rules 1 and 2 in one manifest,
# given as JSON on stdin. It prints one tab-separated line per violation:
#   TABLE  NAME  KIND  DECLARATION  EXTRA_KEYS
# KIND is "own" (rule 1: a version string, or a table without
# `workspace = true`) or "extra" (rule 2). DECLARATION is the entry as compact
# JSON; EXTRA_KEYS lists the keys rule 2 forbids, comma-separated.
# shellcheck disable=SC2016 # the program is jq syntax, not shell.
readonly VIOLATIONS_JQ='
  def dep_table: IN("dependencies", "dev-dependencies", "build-dependencies",
                    "dev_dependencies", "build_dependencies");
  def extra_keys: [keys[] | select(IN("workspace", "features", "optional") | not)];
  [
    (to_entries[] | select(.key | dep_table) | {table: .key, deps: .value}),
    ((.target // {}) | to_entries[] | .key as $cfg | .value | to_entries[]
      | select(.key | dep_table) | {table: "target.\($cfg).\(.key)", deps: .value})
  ]
  | .[] | .table as $table | (.deps // {}) | to_entries[]
  | .key as $name | .value as $decl
  | if ($decl | type) != "object" or $decl.workspace != true then
      [$table, $name, "own", ($decl | tojson), ""]
    elif ($decl | extra_keys | length) > 0 then
      [$table, $name, "extra", ($decl | tojson), ($decl | extra_keys | join(", "))]
    else empty end
  | @tsv
'

# The jq program that counts the dependency entries of one manifest.
readonly COUNT_JQ='
  def dep_table: IN("dependencies", "dev-dependencies", "build-dependencies",
                    "dev_dependencies", "build_dependencies");
  [ (to_entries[] | select(.key | dep_table) | .value // {} | length),
    ((.target // {}) | to_entries[] | .value | to_entries[]
      | select(.key | dep_table) | .value // {} | length) ]
  | add // 0
'

# The jq program for rule 3. Input: the root manifest as JSON. Arguments:
# $text, the raw manifest (--rawfile), and $sq, a single quote (the program
# sits in a single-quoted shell string). It prints one tab-separated line per
# entry it checks:
#   STATUS  NAME  REQUIREMENT
# STATUS is "found" or "missing". Which entries it checks, and why the
# requirement is taken as the parser takes it, follow file_parser.rb
# (`manifest_dependencies`, `requirement_from_declaration`,
# `name_from_declaration`).
#
# The patterns are WorkspaceManifestUpdater's Ruby regexps written for jq.
# jq's regexps (Oniguruma, Perl syntax) anchor "^" at the start of the whole
# text only, so `(?<![^\n])` stands for the Ruby "^" (the start of any line),
# and `[\s\S]` for the Ruby "." under the /m flag. The updater's unquoted
# forms (`name = 1`) are left out: such a value is not a TOML string, so it is
# never a requirement.
# shellcheck disable=SC2016 # the program is jq syntax, not shell.
readonly FINDABLE_JQ='
  def re_escape: gsub("(?<c>[\\\\^$.|?*+()\\[\\]{}])"; "\\\(.c)");
  def line_start: "(?<![^\\n])";
  def sections($header):
    [$text | match($header + "[\\s\\S]*?(?=\\n\\[|\\n*\\z)"; "g") | .string];
  def found_in($sections; $patterns):
    any($sections[]; . as $section | any($patterns[]; . as $p | $section | test($p)));
  (.workspace.dependencies // {}) | to_entries[]
  | .key as $name | .value as $decl
  | (if ($decl | type) == "string" then $decl
     elif ($decl | type) == "object" and ($decl.version | type) == "string" then $decl.version
     else "" end) as $req
  | select($req != "")
  | select(($decl | type) != "object" or ($decl.package // $name) == $name)
  | ($name | re_escape) as $n
  | ($req | re_escape) as $r
  | (found_in(sections("\\[workspace\\.dependencies\\]");
        [ line_start + "\\s*" + $n + "\\s*=\\s*\"" + $r + "\"",
          line_start + "\\s*" + $n + "\\s*=\\s*" + $sq + $r + $sq,
          line_start + "\\s*" + $n + "\\s*=\\s*\\{[^}]*version\\s*=\\s*\"" + $r + "\"",
          line_start + "\\s*" + $n + "\\s*=\\s*\\{[^}]*version\\s*=\\s*" + $sq + $r + $sq ])
     or found_in(sections("\\[workspace\\.dependencies\\." + $n + "\\]");
        [ line_start + "\\s*version\\s*=\\s*\"" + $r + "\"",
          line_start + "\\s*version\\s*=\\s*" + $sq + $r + $sq ])) as $found
  | [(if $found then "found" else "missing" end), $name, $req]
  | @tsv
'

problems=()
manifests=0
entries=0
findable=0
files=()
discovery=""

# problem MESSAGE: record one violation.
problem() {
  problems+=("$1")
}

# check_manifest ROOT RELATIVE_PATH: apply the rules to one Cargo.toml.
#   ROOT - the canonical workspace root; RELATIVE_PATH - the manifest's path
#   below it, as the report shows it.
check_manifest() {
  local root=$1 rel=$2 json table name kind decl extra count violations
  if ! json=$(yq -p toml -o json '.' "$root/$rel" 2>&1); then
    problem "$rel is not valid TOML: ${json//$'\n'/ }"
    return
  fi
  manifests=$((manifests + 1))
  # Each jq run is captured before its output is read, so a jq failure is
  # reported instead of reading as "no violations".
  if ! count=$(jq -r "$COUNT_JQ" <<<"$json" 2>&1); then
    problem "$rel could not be checked: jq failed: ${count//$'\n'/ }"
    return
  fi
  if ! violations=$(jq -r "$VIOLATIONS_JQ" <<<"$json" 2>&1); then
    problem "$rel could not be checked: jq failed: ${violations//$'\n'/ }"
    return
  fi
  entries=$((entries + count))
  while IFS=$'\t' read -r table name kind decl extra; do
    [[ -n $table ]] || continue
    case $kind in
      own)
        problem "$rel [$table] $name = $decl declares its own requirement. Move the requirement to [workspace.dependencies] in the root Cargo.toml, with its reason comment, default-features = false and package = \"...\" if it has them. Then write here: $name = { workspace = true }, plus features = [...] or optional = true if only this crate needs them."
        ;;
      extra)
        problem "$rel [$table] $name = $decl sets $extra next to workspace = true. Move $extra to the $name entry in the root Cargo.toml's [workspace.dependencies]; a member entry holds only workspace = true, features and optional."
        ;;
    esac
  done <<<"$violations"
  # Rule 3 applies to the workspace root's own manifest, found by identity
  # rather than by the spelling of its path.
  if [[ $root/$rel -ef $root/Cargo.toml ]]; then
    check_findable_requirements "$root/$rel" "$rel" "$json"
  fi
}

# check_findable_requirements FILE RELATIVE_PATH JSON: apply rule 3.
#   FILE - the root manifest to read the raw text from; RELATIVE_PATH - its
#   name in the report; JSON - the same manifest, converted by yq.
check_findable_requirements() {
  local file=$1 rel=$2 json=$3 status name req verdicts
  if ! verdicts=$(jq -r --rawfile text "$file" --arg sq "'" "$FINDABLE_JQ" <<<"$json" 2>&1); then
    problem "$rel [workspace.dependencies] could not be checked against Dependabot's updater: jq failed (jq 1.6 or later is needed): ${verdicts//$'\n'/ }"
    return
  fi
  while IFS=$'\t' read -r status name req; do
    [[ -n $status ]] || continue
    findable=$((findable + 1))
    if [[ $status == missing ]]; then
      problem "$rel [workspace.dependencies] $name = \"$req\" is written in a form that Dependabot's updater does not find: it looks for a bare key under the literal [workspace.dependencies] header, or for a [workspace.dependencies.$name] table, so updates of $name would change only Cargo.lock. Write it under the [workspace.dependencies] header as $name = \"$req\" or $name = { version = \"$req\", ... }, with no quotes around $name or version and no dotted keys."
    fi
  done <<<"$verdicts"
}

# discover_manifests ROOT: fill `files` with the manifests to check, relative
# to ROOT, the root manifest first and the rest sorted, and set `discovery` to
# how they were found ("Manifests" in the header).
#   ROOT - the canonical workspace root.
discover_manifests() {
  local root=$1 file toplevel=""
  local -a found=()
  if command -v git >/dev/null; then
    toplevel=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null || true)
  fi
  if [[ -n $toplevel && $toplevel -ef $root ]]; then
    discovery="git"
    while IFS= read -r -d '' file; do
      # --cached also lists a tracked file that was deleted from the work tree.
      if [[ $file != Cargo.toml && -f $root/$file ]]; then
        found+=("$file")
      fi
    done < <(git -C "$root" ls-files -z --cached --others --exclude-standard \
      -- 'Cargo.toml' '*/Cargo.toml' | sort -zu)
  else
    discovery="find"
    while IFS= read -r -d '' file; do
      file=${file#"$root"/}
      if [[ $file != Cargo.toml ]]; then
        found+=("$file")
      fi
    done < <(find "$root" \( -name target -o -name node_modules -o -name .git \) -prune \
      -o -name Cargo.toml -type f -print0 | sort -z)
  fi
  files=(Cargo.toml "${found[@]}")
}

main() {
  local root=.
  while (($# > 0)); do
    case $1 in
      --root) [[ $# -ge 2 ]] || die_usage "--root needs a directory"; root=$2; shift 2 ;;
      -h | --help) usage; return 0 ;;
      *) die_usage "unknown argument '$1'" ;;
    esac
  done
  [[ -d $root ]] || die_usage "root directory '$root' does not exist."
  [[ -f $root/Cargo.toml ]] || die_usage "'$root' has no Cargo.toml; pass the workspace root with --root."
  command -v yq >/dev/null || die_usage "yq (mikefarah, v4) is required but not installed: https://github.com/mikefarah/yq"
  local version
  version=$(yq --version 2>&1 || true)
  [[ $version == *mikefarah* ]] || die_usage "this script needs mikefarah yq v4; found: $version"
  command -v jq >/dev/null || die_usage "jq is required but not installed: https://jqlang.org"
  # One canonical spelling of the root ("dir/", "./", a symlink), so that the
  # relative paths in the report and the root manifest's identity do not
  # depend on how --root was written.
  root=$(cd "$root" && pwd -P)

  discover_manifests "$root"
  local file
  for file in "${files[@]}"; do
    check_manifest "$root" "$file"
  done

  if ((${#problems[@]} == 0)); then
    echo "Workspace dependencies OK: all $entries dependency entries in $manifests manifest(s) (${files[*]}; listed by $discovery) inherit from [workspace.dependencies], and Dependabot's updater finds all $findable requirements there."
    return 0
  fi
  {
    echo "Workspace dependency check failed. Every requirement is declared once, in the root Cargo.toml's [workspace.dependencies], where Dependabot can rewrite it: Dependabot rewrites no other table of the root manifest, and one place per requirement keeps both crates on the same one (header of scripts/ci/check-workspace-deps.sh, .github/dependabot.yml)."
    local p
    for p in "${problems[@]}"; do
      echo "  - $p"
    done
  } >&2
  return 1
}

main "$@"
