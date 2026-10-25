#!/usr/bin/env bash
# Tests for scripts/ci/check-workspace-deps.sh (.github/dependabot.yml: every
# Cargo requirement lives in the root [workspace.dependencies]).
#
# Builds fixture workspaces (a root manifest that is both the workspace root
# and a package, as backend3's is, plus a `migration` member) in a temporary
# directory: valid ones in every TOML form Cargo accepts for an inherited
# dependency, and one per broken rule. For rule 3, the forms that Dependabot's
# WorkspaceManifestUpdater finds and the ones it does not; each of those
# outcomes was checked against that updater itself. Also the spellings of
# --root, and manifest discovery with and without git (a few fixtures are
# made git work trees; nothing is committed). The repository itself must pass
# too. Run from the repository root:
#   bash scripts/tests/test-check-workspace-deps.sh
# Needs mikefarah yq v4, jq and git (the git cases are skipped without it);
# no network access.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT="$ROOT/scripts/ci/check-workspace-deps.sh"

tests_run=0
failures=0

# run_case NAME EXPECTED_EXIT [--contains TEXT]... [--lacks TEXT]... -- COMMAND...
# Runs COMMAND, compares its exit code and checks that its combined output
# contains every --contains TEXT and none of the --lacks TEXT. Prints one
# TAP-style line per case.
run_case() {
  local name=$1 expected=$2
  shift 2
  local -a needles=() absent=()
  while (($# > 0)) && [[ $1 != -- ]]; do
    case $1 in
      --contains) needles+=("$2"); shift 2 ;;
      --lacks) absent+=("$2"); shift 2 ;;
      *) echo "run_case: unknown option '$1'" >&2; exit 2 ;;
    esac
  done
  [[ ${1-} == -- ]] && shift
  local output status=0 reason="" needle
  output=$("$@" 2>&1) || status=$?
  tests_run=$((tests_run + 1))
  if [[ $status -ne $expected ]]; then
    reason="exit $status, expected $expected"
  else
    for needle in "${needles[@]}"; do
      [[ $output == *"$needle"* ]] || { reason="output lacks \"$needle\""; break; }
    done
    for needle in "${absent[@]}"; do
      [[ $output != *"$needle"* ]] || { reason="output contains \"$needle\""; break; }
    done
  fi
  if [[ -n $reason ]]; then
    failures=$((failures + 1))
    printf 'not ok %d - %s (%s)\n%s\n' "$tests_run" "$name" "$reason" "$output" | sed '2,$s/^/    # /'
  else
    printf 'ok %d - %s\n' "$tests_run" "$name"
  fi
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The root manifest of a valid workspace, in the shape of the real one.
GOOD_ROOT='[workspace]
resolver = "3"

[workspace.dependencies]
# Comments and plain requirements are fine in the workspace table itself.
loco-rs = { version = "=1.2.0", default-features = false, features = ["auth"] }
migration = { path = "migration" }
tokio = { version = "1.45", default-features = false, features = ["macros"] }
serde = "1"
renamed = { version = "1", package = "real-name" }
cfg-if = "1"
cc = "1"

[package]
name = "app"
version = "0.1.0"

[dependencies]
loco-rs = { workspace = true }
migration = { workspace = true }
tokio = { workspace = true, features = ["net"] }
serde.workspace = true
renamed = { workspace = true, optional = true }

[dependencies.cfg-if]
workspace = true

[target."cfg(unix)".dependencies]
serde = { workspace = true, features = ["derive"] }

[build-dependencies]
cc = { workspace = true }

[dev-dependencies]
loco-rs = { workspace = true, features = ["testing"] }
tokio = { workspace = true }
'

GOOD_MEMBER='[package]
name = "migration"
version = "0.1.0"

[dependencies]
loco-rs = { workspace = true }
'

# root_with_dep LINE: print GOOD_ROOT with LINE added as the first entry of the
# package's [dependencies] table (appending would land in [dev-dependencies]).
root_with_dep() {
  printf '%s' "${GOOD_ROOT//\[dependencies\]$'\n'/[dependencies]$'\n'$1$'\n'}"
}

# workspace NAME ROOT_MANIFEST [MEMBER_MANIFEST]: create a fixture workspace
# and print its path. An empty MEMBER_MANIFEST leaves migration/ out.
workspace() {
  local dir="$WORK/$1"
  mkdir -p "$dir"
  printf '%s' "$2" >"$dir/Cargo.toml"
  if [[ -n ${3-} ]]; then
    mkdir -p "$dir/migration"
    printf '%s' "$3" >"$dir/migration/Cargo.toml"
  fi
  printf '%s' "$dir"
}

# A member manifest that declares its own requirement, for the discovery cases.
OWN_MEMBER='[package]
name = "x"
version = "0.1.0"

[dependencies]
serde = "1"
'

# The repository itself.
run_case "the repository's manifests pass" 0 --contains "Workspace dependencies OK" -- \
  bash "$SCRIPT" --root "$ROOT"

# Valid fixtures. GOOD_ROOT has five requirements for rule 3: migration is a
# path entry and `renamed` a renamed one, which the rule leaves out.
run_case "every inherited form passes (inline, dotted, header, target, optional, build)" 0 \
  --contains "all 11 dependency entries in 2 manifest(s) (Cargo.toml migration/Cargo.toml; listed by find)" \
  --contains "Dependabot's updater finds all 5 requirements there" -- \
  bash "$SCRIPT" --root "$(workspace good "$GOOD_ROOT" "$GOOD_MEMBER")"
run_case "a manifest without dependency tables passes" 0 --contains "all 0 dependency entries in 1 manifest(s)" -- \
  bash "$SCRIPT" --root "$(workspace empty $'[workspace]\nresolver = "3"\n\n[workspace.dependencies]\nserde = "1"\n')"
dir=$(workspace skipped "$GOOD_ROOT" "$GOOD_MEMBER")
for skip in target/package/app-0.1.0 admin/node_modules/x .git/x; do
  mkdir -p "$dir/$skip"
  printf '%s' "$OWN_MEMBER" >"$dir/$skip/Cargo.toml"
done
run_case "outside git, Cargo.toml files under target/, node_modules/ and .git/ are skipped" 0 \
  --contains "in 2 manifest(s) (Cargo.toml migration/Cargo.toml; listed by find)" -- \
  bash "$SCRIPT" --root "$dir"
# shellcheck disable=SC2016 # the bash -c body expands its own arguments.
run_case "the current directory is the default root" 0 --contains "Workspace dependencies OK" -- \
  bash -c 'cd "$1" && bash "$2"' _ "$(workspace default-root "$GOOD_ROOT" "$GOOD_MEMBER")" "$SCRIPT"

# The root is canonicalised, so every spelling of it gives the same report.
run_case "a root with a trailing slash passes (the repository)" 0 \
  --contains "(Cargo.toml migration/Cargo.toml; listed by git)" --lacks "not valid TOML" -- \
  bash "$SCRIPT" --root "$ROOT/"
dir=$(workspace slashes "$GOOD_ROOT" "$GOOD_MEMBER")
run_case "a root with doubled trailing slashes passes" 0 \
  --contains "(Cargo.toml migration/Cargo.toml; listed by find)" -- \
  bash "$SCRIPT" --root "$dir//"
ln -s "$dir" "$WORK/slashes-link"
run_case "a root reached through a symlink passes" 0 \
  --contains "(Cargo.toml migration/Cargo.toml; listed by find)" -- \
  bash "$SCRIPT" --root "$WORK/slashes-link"
dotted_root=${GOOD_ROOT//serde = \"1\"/serde.version = \"1\"}
for spelling in ./ . "../dotted-spelling" "../dotted-spelling/"; do
  # shellcheck disable=SC2016 # the bash -c body expands its own arguments.
  run_case "rule 3 runs whatever the spelling of the root ('--root $spelling')" 1 \
    --contains "Cargo.toml [workspace.dependencies] serde = \"1\" is written in a form that Dependabot's updater does not find" \
    --lacks "Workspace dependencies OK" --lacks "/Cargo.toml [" -- \
    bash -c 'cd "$1" && bash "$2" --root "$3"' _ "$(workspace dotted-spelling "$dotted_root" "$GOOD_MEMBER")" "$SCRIPT" "$spelling"
done

# Manifest discovery in a git work tree: tracked and untracked files, but not
# ignored ones, nested repositories or files deleted from the work tree.
if command -v git >/dev/null; then
  dir=$(workspace git-tree "$GOOD_ROOT" "$GOOD_MEMBER")
  git -C "$dir" init -q
  printf 'vendor/\n' >"$dir/.gitignore"
  mkdir -p "$dir/vendor/x" "$dir/worktrees/copy" "$dir/gone"
  printf '%s' "$OWN_MEMBER" >"$dir/vendor/x/Cargo.toml"
  git -C "$dir/worktrees/copy" init -q
  printf '%s' "$OWN_MEMBER" >"$dir/worktrees/copy/Cargo.toml"
  printf '%s' "$OWN_MEMBER" >"$dir/gone/Cargo.toml"
  git -C "$dir" add Cargo.toml gone/Cargo.toml
  rm "$dir/gone/Cargo.toml"
  run_case "in git, ignored, nested-repository and deleted manifests are skipped; untracked members are checked" 0 \
    --contains "in 2 manifest(s) (Cargo.toml migration/Cargo.toml; listed by git)" -- \
    bash "$SCRIPT" --root "$dir"
  mkdir -p "$dir/newcrate"
  printf '%s' "$OWN_MEMBER" >"$dir/newcrate/Cargo.toml"
  run_case "in git, a new crate is checked before its first git add" 1 \
    --contains 'newcrate/Cargo.toml [dependencies] serde = "1" declares its own requirement' \
    --lacks "vendor/x" --lacks "worktrees/copy" -- \
    bash "$SCRIPT" --root "$dir"
  dir=$(workspace git-outer "$GOOD_ROOT")
  git -C "$dir" init -q
  mkdir -p "$dir/inner/migration"
  printf '%s' "$GOOD_ROOT" >"$dir/inner/Cargo.toml"
  printf '%s' "$GOOD_MEMBER" >"$dir/inner/migration/Cargo.toml"
  run_case "a root below the top of a git work tree is searched with find" 0 \
    --contains "(Cargo.toml migration/Cargo.toml; listed by find)" -- \
    bash "$SCRIPT" --root "$dir/inner"
else
  tests_run=$((tests_run + 1))
  printf 'ok %d - # SKIP git is not installed: the git discovery cases did not run\n' "$tests_run"
fi

# Rule 1: a requirement declared in a member table.
run_case "a version string in the root package's [dependencies] fails" 1 \
  --contains 'Cargo.toml [dependencies] axum = "0.8" declares its own requirement' \
  --contains "Move the requirement to [workspace.dependencies] in the root Cargo.toml" \
  --contains "axum = { workspace = true }" -- \
  bash "$SCRIPT" --root "$(workspace own-string "$(root_with_dep 'axum = "0.8"')" "$GOOD_MEMBER")"
run_case "an inline table with a version in a member fails" 1 \
  --contains 'migration/Cargo.toml [dependencies] sea-orm-migration = {"version":"2.0","features":["sqlx-postgres"]} declares its own requirement' -- \
  bash "$SCRIPT" --root "$(workspace own-table "$GOOD_ROOT" "$GOOD_MEMBER"$'sea-orm-migration = { version = "2.0", features = ["sqlx-postgres"] }\n')"
run_case "a [dependencies.<name>] header with a version fails" 1 \
  --contains 'migration/Cargo.toml [dependencies] sea-orm-migration = {"version":"2.0"} declares its own requirement' -- \
  bash "$SCRIPT" --root "$(workspace own-header "$GOOD_ROOT" "$GOOD_MEMBER"$'\n[dependencies.sea-orm-migration]\nversion = "2.0"\n')"
run_case "a [dev-dependencies] requirement fails" 1 --contains "Cargo.toml [dev-dependencies] proptest" -- \
  bash "$SCRIPT" --root "$(workspace own-dev "$GOOD_ROOT"$'proptest = "1"\n' "$GOOD_MEMBER")"
run_case "a [build-dependencies] requirement fails" 1 --contains "migration/Cargo.toml [build-dependencies] cc" -- \
  bash "$SCRIPT" --root "$(workspace own-build "$GOOD_ROOT" "$GOOD_MEMBER"$'\n[build-dependencies]\ncc = "1"\n')"
run_case "a target-specific requirement fails" 1 --contains "Cargo.toml [target.cfg(windows).dependencies] winapi" -- \
  bash "$SCRIPT" --root "$(workspace own-target "$GOOD_ROOT"$'\n[target.\'cfg(windows)\'.dependencies]\nwinapi = "0.3"\n' "$GOOD_MEMBER")"
run_case "the old dev_dependencies spelling is checked too" 1 --contains "Cargo.toml [dev_dependencies] proptest" -- \
  bash "$SCRIPT" --root "$(workspace own-legacy "$GOOD_ROOT"$'\n[dev_dependencies]\nproptest = "1"\n' "$GOOD_MEMBER")"
run_case "a path dependency in a member fails" 1 --contains 'Cargo.toml [dependencies] helper = {"path":"helper"} declares its own requirement' -- \
  bash "$SCRIPT" --root "$(workspace own-path "$(root_with_dep 'helper = { path = "helper" }')" "$GOOD_MEMBER")"
run_case "workspace = false fails" 1 --contains 'serde = {"workspace":false} declares its own requirement' -- \
  bash "$SCRIPT" --root "$(workspace ws-false "${GOOD_ROOT//serde.workspace = true/serde.workspace = false}" "$GOOD_MEMBER")"
# What `cargo add <crate>` writes when the crate is not in [workspace.dependencies] yet.
run_case "a requirement written by cargo add for a new crate fails" 1 --contains '[dependencies] memchr = "2.8.3"' -- \
  bash "$SCRIPT" --root "$(workspace cargo-add "$GOOD_ROOT" "$GOOD_MEMBER"$'memchr = "2.8.3"\n')"

# `cargo loco generate deployment lambda` (loco-gen 1.2.0,
# templates/deployment/lambda/lambda.t) adds this line after the first line
# of Cargo.toml that matches `\[dependencies\]` (rrgen 0.6.0); simulated on
# copies of the repository's own manifests.
dir="$WORK/lambda"
mkdir -p "$dir/migration"
cp "$ROOT/migration/Cargo.toml" "$dir/migration/Cargo.toml"
awk '{ print } !done && /\[dependencies\]/ { print "lambda_http = { version = \"1.3\" }"; done = 1 }' \
  "$ROOT/Cargo.toml" >"$dir/Cargo.toml"
run_case "the requirement the lambda generator injects is reported with the fix" 1 \
  --contains 'Cargo.toml [dependencies] lambda_http = {"version":"1.3"} declares its own requirement' \
  --contains "lambda_http = { workspace = true }" -- \
  bash "$SCRIPT" --root "$dir"

# Rule 2: keys next to workspace = true that belong in the workspace entry.
run_case "a version next to workspace = true fails" 1 \
  --contains "[dependencies] tokio = {\"workspace\":true,\"version\":\"1.45\"} sets version next to workspace = true" \
  --contains "Move version to the tokio entry in the root Cargo.toml's [workspace.dependencies]" -- \
  bash "$SCRIPT" --root "$(workspace extra-version "$GOOD_ROOT" "$GOOD_MEMBER"$'tokio = { workspace = true, version = "1.45" }\n')"
run_case "default-features next to workspace = true fails" 1 --contains "sets default-features next to workspace = true" -- \
  bash "$SCRIPT" --root "$(workspace extra-default "$GOOD_ROOT" "$GOOD_MEMBER"$'tokio = { workspace = true, default-features = false }\n')"
run_case "a rename next to workspace = true fails" 1 --contains "sets package next to workspace = true" -- \
  bash "$SCRIPT" --root "$(workspace extra-package "$GOOD_ROOT" "$GOOD_MEMBER"$'renamed = { workspace = true, package = "real-name" }\n')"
run_case "several forbidden keys are listed together" 1 --contains "sets default-features, version next to workspace = true" -- \
  bash "$SCRIPT" --root "$(workspace extra-two "$GOOD_ROOT" "$GOOD_MEMBER"$'tokio = { workspace = true, version = "1", default-features = false }\n')"

# Rule 3: Dependabot's updater finds every requirement of the root's
# [workspace.dependencies] (WorkspaceManifestUpdater at dependabot-core@77f70df).
# Each expected outcome below was confirmed by running that updater on the
# same text. Every failing form is valid TOML that Cargo reads like the
# passing ones.
# rule3_root OLD NEW: print GOOD_ROOT with the text OLD replaced by NEW.
rule3_root() {
  printf '%s' "${GOOD_ROOT//"$1"/"$2"}"
}
# rule3_fails NAME OLD NEW ENTRY [APPENDED]: a fixture with OLD replaced by NEW
# (and APPENDED added at the end of the root manifest) must fail rule 3 for
# ENTRY (`name = "req"`, as the report prints it), with the fix.
rule3_fails() {
  run_case "$1 fails" 1 \
    --contains "Cargo.toml [workspace.dependencies] $4 is written in a form that Dependabot's updater does not find" \
    --contains "Write it under the [workspace.dependencies] header as" -- \
    bash "$SCRIPT" --root "$(workspace "rule3-$tests_run" "$(rule3_root "$2" "$3")"$'\n'"${5-}" "$GOOD_MEMBER")"
}
# rule3_passes NAME OLD NEW: a fixture with OLD replaced by NEW must pass.
rule3_passes() {
  run_case "$1 passes" 0 --contains "Workspace dependencies OK" -- \
    bash "$SCRIPT" --root "$(workspace "rule3-$tests_run" "$(rule3_root "$2" "$3")" "$GOOD_MEMBER")"
}
rule3_fails "a dotted version key (serde.version = \"1\")" 'serde = "1"' 'serde.version = "1"' 'serde = "1"'
run_case "the rule 3 failure names the forms that work" 1 \
  --contains 'serde = "1" or serde = { version = "1", ... }, with no quotes around serde or version and no dotted keys' -- \
  bash "$SCRIPT" --root "$(workspace rule3-message "$(rule3_root 'serde = "1"' 'serde.version = "1"')" "$GOOD_MEMBER")"
rule3_fails "dotted version and features keys" 'serde = "1"' $'serde.version = "1"\nserde.features = ["derive"]' 'serde = "1"'
rule3_fails "a quoted key (\"serde\" = \"1\")" 'serde = "1"' '"serde" = "1"' 'serde = "1"'
rule3_fails "a single-quoted key ('serde' = \"1\")" 'serde = "1"' "'serde' = \"1\"" 'serde = "1"'
rule3_fails "a quoted dotted key (\"serde\".version = \"1\")" 'serde = "1"' '"serde".version = "1"' 'serde = "1"'
rule3_fails "a quoted version key (serde = { \"version\" = \"1\" })" 'serde = "1"' 'serde = { "version" = "1" }' 'serde = "1"'
rule3_fails "a multi-line string requirement" 'serde = "1"' 'serde = """1"""' 'serde = "1"'
rule3_fails "spaces inside the header brackets ([ workspace.dependencies ])" \
  '[workspace.dependencies]' '[ workspace.dependencies ]' 'tokio = "1.45"'
rule3_fails "a quoted header ([workspace.\"dependencies\"])" \
  '[workspace.dependencies]' '[workspace."dependencies"]' 'cc = "1"'
run_case "dependencies.<name> under [workspace] fails" 1 \
  --contains 'Cargo.toml [workspace.dependencies] serde = "1" is written in a form that' -- \
  bash "$SCRIPT" --root "$(workspace ws-dotted $'[workspace]\nresolver = "3"\ndependencies.serde = "1"\n\n[package]\nname = "app"\nversion = "0.1.0"\n\n[dependencies]\nserde = { workspace = true }\n')"
run_case "a [workspace.dependencies] that only a comment names is not the table" 1 \
  --contains 'Cargo.toml [workspace.dependencies] serde = "1" is written in a form that' -- \
  bash "$SCRIPT" --root "$(workspace ws-dotted-comment $'# The requirements: [workspace.dependencies]\n[workspace]\nresolver = "3"\ndependencies.serde = "1"\n\n[package]\nname = "app"\nversion = "0.1.0"\n')"
two_root=$(rule3_root 'serde = "1"' 'serde.version = "1"')
two_root=${two_root//$'\n'cc = \"1\"/$'\n'\"cc\" = \"1\"}
run_case "every unfindable entry is reported, and only those" 1 \
  --contains 'serde = "1" is written' --contains 'cc = "1" is written' --lacks 'cfg-if = "1" is written' -- \
  bash "$SCRIPT" --root "$(workspace rule3-two "$two_root"$'\n' "$GOOD_MEMBER")"
# The updater anchors the key at the start of a line, so neither a
# commented-out copy nor a longer key that ends in the name stands in for the
# real entry.
rule3_fails "a dotted entry beside a commented-out plain copy" 'serde = "1"' \
  $'# serde = "1" (the entry below is dotted)\nserde.version = "1"' 'serde = "1"'
rule3_fails "a dotted entry beside a longer key that ends in its name" 'serde = "1"' \
  $'my-serde = "1"\nserde.version = "1"' 'serde = "1"'
# The table ends at the next line that starts with "[", so a matching line in
# a later table does not count either.
rule3_fails "a dotted entry with a plain copy in a later table" 'serde = "1"' 'serde.version = "1"' 'serde = "1"' \
  $'\n[package.metadata.notes]\nserde = "1"\n'
header_root=${GOOD_ROOT//cc = \"1\"$'\n'/}
header_root=${header_root//$'\n[package]'/$'\n[workspace.dependencies.cc]\nversion = "1"\n\n[package]'}
run_case "the [workspace.dependencies.<name>] header form passes" 0 --contains "Workspace dependencies OK" -- \
  bash "$SCRIPT" --root "$(workspace header-form "$header_root" "$GOOD_MEMBER")"
rule3_passes "a single-quoted requirement (serde = '1')" 'serde = "1"' "serde = '1'"
rule3_passes "an inline table with version after a multi-line array" 'serde = "1"' \
  $'serde = { default-features = false, features = [\n  "derive",\n], version = "1" }'
rule3_passes "an indented key and no spaces around =" 'serde = "1"' '  serde="1"'
rule3_passes "a comment after the header" '[workspace.dependencies]' '[workspace.dependencies] # requirements'
rule3_passes "requirements with operators, wildcards and commas" 'serde = "1"' \
  $'serde = ">= 1.0, < 2"\nurl = "2.*"\nlog = "~0.4"\nitoa = "^1.0"'
rule3_passes "a renamed entry, which Dependabot's parser skips, in any form" 'serde = "1"' \
  $'serde = "1"\nalias.version = "1"\nalias.package = "real"'
rule3_passes "dots in comments and array values" 'serde = "1"' \
  $'# serde.version = "1" is not used.\nserde = { version = "1", features = [\n  "a.b",\n] }'
dir=$(workspace crlf "$GOOD_ROOT" "$GOOD_MEMBER")
awk '{ printf "%s\r\n", $0 }' <<<"$GOOD_ROOT" >"$dir/Cargo.toml"
run_case "a manifest with CRLF line endings passes" 0 --contains "Dependabot's updater finds all 5 requirements there" -- \
  bash "$SCRIPT" --root "$dir"

# Reporting.
run_case "every violation is reported, not only the first" 1 \
  --contains "Cargo.toml [dependencies] axum" --contains "migration/Cargo.toml [dependencies] url" \
  --lacks "Workspace dependencies OK" -- \
  bash "$SCRIPT" --root "$(workspace two-files "$(root_with_dep 'axum = "0.8"')" "$GOOD_MEMBER"$'url = "2"\n')"
run_case "invalid TOML fails and names the file" 1 --contains "migration/Cargo.toml is not valid TOML" -- \
  bash "$SCRIPT" --root "$(workspace bad-toml "$GOOD_ROOT" $'[package\nname = \n')"

# A jq that fails makes the check fail, not pass with nothing checked. The
# wrapper fails on the arguments named in FAIL_ON and runs the real jq
# otherwise.
real_jq=$(command -v jq)
mkdir -p "$WORK/failing-jq"
# shellcheck disable=SC2016 # the wrapper's own script text, expanded when it runs.
printf '#!/usr/bin/env bash\nfor a in "$@"; do [[ $a == *"$FAIL_ON"* ]] && { echo "jq: error: unsupported" >&2; exit 2; }; done\nexec %q "$@"\n' \
  "$real_jq" >"$WORK/failing-jq/jq"
chmod +x "$WORK/failing-jq/jq"
dir=$(workspace failing-jq-ws "$GOOD_ROOT" "$GOOD_MEMBER")
run_case "a jq that cannot run rule 3 (no --rawfile) fails the check" 1 \
  --contains "Cargo.toml [workspace.dependencies] could not be checked against Dependabot's updater: jq failed" \
  --lacks "Workspace dependencies OK" -- \
  env FAIL_ON=--rawfile PATH="$WORK/failing-jq:$PATH" bash "$SCRIPT" --root "$dir"
run_case "a jq failure in rules 1 and 2 fails the check" 1 \
  --contains "migration/Cargo.toml could not be checked: jq failed" --lacks "Workspace dependencies OK" -- \
  env FAIL_ON=extra_keys PATH="$WORK/failing-jq:$PATH" bash "$SCRIPT" --root "$dir"

# Usage errors.
run_case "a root without Cargo.toml is a usage error" 2 --contains "has no Cargo.toml" -- \
  bash "$SCRIPT" --root "$(mkdir -p "$WORK/no-manifest" && printf '%s' "$WORK/no-manifest")"
run_case "a missing root is a usage error" 2 --contains "does not exist" -- bash "$SCRIPT" --root "$WORK/nope"
run_case "--root without a value is a usage error" 2 --contains "--root needs a directory" -- bash "$SCRIPT" --root
run_case "an unknown option is a usage error" 2 --contains "unknown argument" -- bash "$SCRIPT" --bogus

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
