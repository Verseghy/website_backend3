#!/usr/bin/env bash
# test-sync-loco-skill.sh: tests for scripts/sync-loco-skill.sh (ADR 0024
# section 3).
#
# Why it exists
#   The "Loco skill" CI job relies on `sync-loco-skill.sh --check` failing on
#   every kind of drift, and bump PRs rely on the sync making the vendored copy
#   exactly equal to the locked crate's. These tests pin both directions for
#   the cases ADR 0024 names (an equal tree, an edited file, an extra file, a
#   missing file, a changed LICENSE) plus deletions, nested and spaced paths,
#   argument errors, and the cargo path (cargo fetch --locked, then
#   cargo metadata read with jq) against a stub cargo.
#
# How it works
#   Each case builds a scratch repository: a copy of the script under
#   scripts/, and a copy of scripts/tests/fixtures/loco-skill/crate/ as the
#   crate. The script writes to .claude/skills/loco/ of the repository that
#   holds it, so the real vendored skill is never touched.
#
# Usage
#   bash scripts/tests/test-sync-loco-skill.sh     (from the repository root)
#   It needs bash, jq and coreutils, and no network or cargo. It exits
#   non-zero if any case fails.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$repo_root/scripts/sync-loco-skill.sh"
fixture_crate="$repo_root/scripts/tests/fixtures/loco-skill/crate"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

tests_run=0
failures=0

# Prints one TAP-style result line.
#   $1 - "ok" or "fail"; $2 - the case name; $3 - details for a failure.
report() {
  tests_run=$((tests_run + 1))
  if [[ "$1" == ok ]]; then
    printf 'ok %d - %s\n' "$tests_run" "$2"
  else
    failures=$((failures + 1))
    printf 'not ok %d - %s\n%s\n' "$tests_run" "$2" "$3" | sed '2,$s/^/    # /'
  fi
}

# Creates a fresh scratch repository and crate. Sets: repo, crate, skill.
#   $1 - a name for the scratch directory.
fresh() {
  repo="$work/$1/repo"
  crate="$work/$1/crate"
  skill="$repo/.claude/skills/loco"
  mkdir -p "$repo/scripts"
  cp "$script" "$repo/scripts/sync-loco-skill.sh"
  cp -R "$fixture_crate" "$crate"
}

# Runs the scratch copy of the script. Sets: status, output (stdout+stderr).
#   $@ - the script's arguments.
run_sync() {
  status=0
  output="$(bash "$repo/scripts/sync-loco-skill.sh" "$@" 2>&1)" || status=$?
}

# Asserts the last run's exit code and, optionally, a text in its output.
#   $1 - the case name; $2 - the expected exit code; $3 - optional text.
expect_run() {
  if [[ "$status" != "$2" ]]; then
    report fail "$1" "exit $status, expected $2"$'\n'"$output"
  elif [[ -n "${3-}" && "$output" != *"$3"* ]]; then
    report fail "$1" "output lacks '$3'"$'\n'"$output"
  else
    report ok "$1" ""
  fi
}

# Asserts that a command succeeds.
#   $1 - the case name; $2 - details printed for a failure; $3... - the command.
expect_true() {
  local name="$1" detail="$2"
  shift 2
  if "$@"; then report ok "$name" ""; else report fail "$name" "$detail"; fi
}

# Whether a text contains a fixed string.
#   $1 - the text; $2 - the string.
contains() { [[ "$1" == *"$2"* ]]; }

# Asserts that the vendored copy equals the crate: the same files with the
# same bytes, plus a byte copy of the crate's LICENSE.
#   $1 - the case name.
expect_equal_tree() {
  local problems=""
  if ! diff -r -x LICENSE "$crate/skills/loco" "$skill" >"$work/diff.txt" 2>&1; then
    problems="trees differ: $(cat "$work/diff.txt")"
  elif ! cmp -s "$crate/LICENSE" "$skill/LICENSE"; then
    problems="LICENSE is not a byte copy of the crate's"
  fi
  if [[ -z "$problems" ]]; then report ok "$1" ""; else report fail "$1" "$problems"; fi
}

# A fingerprint of every path, its type and every file's content under a
# directory, used to prove that --check writes nothing.
#   $1 - the directory.
fingerprint() {
  (
    cd "$1"
    find . -type d | LC_ALL=C sort | sed 's/^/dir /'
    find . ! -type d | LC_ALL=C sort | while IFS= read -r path; do
      if [[ -L "$path" ]]; then
        printf 'link %s\n' "$path"
      else
        printf 'file %s %s\n' "$path" "$(cksum <"$path")"
      fi
    done
  )
}

# ------------------------------------------------------------- argument handling

fresh args
run_sync --help
expect_run "--help prints the usage" 0 "Usage: scripts/sync-loco-skill.sh"
run_sync --bogus
expect_run "an unknown argument is a usage error" 2 "unknown argument '--bogus'"
run_sync --crate-dir
expect_run "--crate-dir without a path is a usage error" 2 "--crate-dir needs a path"
run_sync --crate-dir ''
expect_run "an empty --crate-dir is a usage error" 2 "--crate-dir needs a non-empty path"
run_sync --crate-dir "$work/does-not-exist"
expect_run "a missing crate directory is reported" 2 "is not a directory"
mkdir -p "$work/not-a-crate"
run_sync --crate-dir "$work/not-a-crate"
expect_run "a crate without skills/loco/SKILL.md is reported" 2 "has no skills/loco/SKILL.md"
rm "$crate/LICENSE"
run_sync --crate-dir "$crate"
expect_run "a crate without LICENSE is reported" 2 "has no LICENSE file"
fresh upstream-license
printf 'x\n' >"$crate/skills/loco/LICENSE"
run_sync --crate-dir "$crate"
expect_run "an upstream skills/loco/LICENSE is refused" 2 "collides with the licence copy"
fresh no-manifest
run_sync --check
expect_run "without --crate-dir, a missing Cargo.toml is reported" 2 "no Cargo.toml at the repository root"

# ------------------------------------------------------------- first sync and an equal tree

fresh equal
run_sync --check --crate-dir "$crate"
expect_run "--check on an empty destination fails" 1 "missing  .claude/skills/loco/SKILL.md"
expect_true "--check created nothing" "$skill exists" test ! -e "$skill"
run_sync --crate-dir "$crate"
expect_run "the first sync succeeds" 0 "4 files plus LICENSE"
expect_equal_tree "the first sync makes a byte-identical copy with LICENSE"
expect_true "a path with a space is copied" "missing" test -f "$skill/recipes/with space.md"
run_sync --check --crate-dir "$crate"
expect_run "--check on an equal tree passes" 0 "matches the crate at"
run_sync --crate-dir="$crate"
expect_run "a second sync writes nothing" 0 "0 written, 0 removed"

# ------------------------------------------------------------- an edited file

fresh edited
run_sync --crate-dir "$crate" >/dev/null
printf 'local override\n' >>"$skill/doctrine.md"
before="$(fingerprint "$skill")"
run_sync --check --crate-dir "$crate"
expect_run "--check reports an edited file" 1 "differs  .claude/skills/loco/doctrine.md: content differs from upstream"
expect_true "--check shows the diff of an edited file" "$output" contains "$output" "+local override"
expect_true "--check wrote nothing" "the destination changed" test "$(fingerprint "$skill")" = "$before"
run_sync --crate-dir "$crate"
expect_run "the sync restores an edited file" 0 "1 written, 0 removed"
expect_equal_tree "the tree is equal again after restoring the edit"

# ------------------------------------------------------------- an extra file

fresh extra
run_sync --crate-dir "$crate" >/dev/null
printf 'extra\n' >"$skill/recipes/extra.md"
mkdir -p "$skill/notes"
printf 'extra\n' >"$skill/notes/override.md"
run_sync --check --crate-dir "$crate"
expect_run "--check reports an extra file" 1 "extra    .claude/skills/loco/recipes/extra.md"
expect_run "--check reports an extra file in a new directory" 1 "extra    .claude/skills/loco/notes/override.md"
run_sync --crate-dir "$crate"
expect_run "the sync deletes extra files" 0 "0 written, 2 removed"
expect_true "the sync removes directories it emptied" "notes/ still exists" test ! -e "$skill/notes"
expect_equal_tree "the tree is equal again after deleting extras"

# A file that upstream dropped is an extra file in the vendored copy.
fresh upstream-dropped
run_sync --crate-dir "$crate" >/dev/null
rm "$crate/skills/loco/recipes/endpoint.md"
run_sync --check --crate-dir "$crate"
expect_run "--check reports a file upstream no longer has" 1 "extra    .claude/skills/loco/recipes/endpoint.md"
run_sync --crate-dir "$crate"
expect_run "the sync deletes a file upstream dropped" 0 "1 removed"
expect_equal_tree "the tree follows upstream's deletion"

# A symbolic link in the vendored copy is not a vendored file.
fresh symlink
run_sync --crate-dir "$crate" >/dev/null
ln -s doctrine.md "$skill/alias.md"
run_sync --check --crate-dir "$crate"
expect_run "--check reports a symbolic link as extra" 1 "extra    .claude/skills/loco/alias.md"
run_sync --crate-dir "$crate"
expect_run "the sync removes a symbolic link" 0 "1 removed"
expect_equal_tree "the tree is equal after removing the link"

# ------------------------------------------------------------- a missing file

fresh missing
run_sync --crate-dir "$crate" >/dev/null
rm "$skill/recipes/endpoint.md"
run_sync --check --crate-dir "$crate"
expect_run "--check reports a missing file" 1 "missing  .claude/skills/loco/recipes/endpoint.md"
run_sync --crate-dir "$crate"
expect_run "the sync restores a missing file" 0 "1 written, 0 removed"
expect_equal_tree "the tree is equal again after restoring the file"

# A file that upstream added is missing from the vendored copy.
fresh upstream-added
run_sync --crate-dir "$crate" >/dev/null
printf '# New recipe\n' >"$crate/skills/loco/recipes/new.md"
run_sync --check --crate-dir "$crate"
expect_run "--check reports a file upstream added" 1 "missing  .claude/skills/loco/recipes/new.md"
run_sync --crate-dir "$crate"
expect_run "the sync adds a file upstream added" 0 "1 written"
expect_equal_tree "the tree follows upstream's addition"

# A directory where upstream has a file.
fresh dir-for-file
run_sync --crate-dir "$crate" >/dev/null
rm "$skill/doctrine.md"
mkdir -p "$skill/doctrine.md"
run_sync --check --crate-dir "$crate"
expect_run "--check reports a directory where upstream has a file" 1 "differs  .claude/skills/loco/doctrine.md"
run_sync --crate-dir "$crate"
expect_run "the sync replaces the directory with the file" 0
expect_equal_tree "the tree is equal after replacing the directory"

# ------------------------------------------------------------- a changed LICENSE

fresh license-changed
run_sync --crate-dir "$crate" >/dev/null
printf 'changed\n' >>"$skill/LICENSE"
run_sync --check --crate-dir "$crate"
expect_run "--check reports a changed LICENSE" 1 "differs  .claude/skills/loco/LICENSE: not a byte copy of the crate's LICENSE"
run_sync --crate-dir "$crate"
expect_run "the sync restores the LICENSE" 0 "1 written"
expect_equal_tree "the LICENSE is a byte copy again"

fresh license-missing
run_sync --crate-dir "$crate" >/dev/null
rm "$skill/LICENSE"
run_sync --check --crate-dir "$crate"
expect_run "--check reports a missing LICENSE" 1 "missing  .claude/skills/loco/LICENSE"
run_sync --crate-dir "$crate"
expect_run "the sync restores a missing LICENSE" 0
expect_equal_tree "the LICENSE exists again"

fresh license-upstream-changed
run_sync --crate-dir "$crate" >/dev/null
printf 'new upstream terms\n' >>"$crate/LICENSE"
run_sync --check --crate-dir "$crate"
expect_run "--check reports a LICENSE that changed upstream" 1 "differs  .claude/skills/loco/LICENSE"

# ------------------------------------------------------------- the cargo path (stub cargo)

# A stub cargo that logs its arguments and answers `metadata` according to
# STUB_MODE: ok, none, two, path, fail-fetch, fail-metadata.
mkdir -p "$work/bin"
cat >"$work/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$STUB_LOG"
case "$1" in
  fetch)
    [[ "$STUB_MODE" != fail-fetch ]] || { echo "error: the lock file needs to be updated" >&2; exit 101; }
    ;;
  metadata)
    [[ "$STUB_MODE" != fail-metadata ]] || { echo "error: failed to parse manifest" >&2; exit 101; }
    loco='{"name": "loco-rs", "version": "1.2.0", "source": "registry+https://github.com/rust-lang/crates.io-index", "manifest_path": $mp}'
    case "$STUB_MODE" in
      ok) jq -n --arg mp "$STUB_CRATE/Cargo.toml" "{packages: [{name: \"serde\", version: \"1.0.0\", source: \"registry+x\", manifest_path: \"/x/Cargo.toml\"}, $loco]}" ;;
      none) jq -n '{packages: [{name: "serde", version: "1.0.0"}]}' ;;
      two) jq -n --arg mp "$STUB_CRATE/Cargo.toml" "{packages: [$loco, ($loco | .version = \"1.3.0\")]}" ;;
      path) jq -n --arg mp "$STUB_CRATE/Cargo.toml" '{packages: [{name: "loco-rs", version: "1.2.0", source: null, manifest_path: $mp}]}' ;;
    esac
    ;;
esac
STUB
chmod +x "$work/bin/cargo"

# Runs the scratch script with the stub cargo first on PATH.
#   $1 - the stub mode; $2... - the script's arguments.
run_with_stub() {
  local mode="$1"
  shift
  : >"$work/cargo.log"
  status=0
  output="$(PATH="$work/bin:$PATH" STUB_MODE="$mode" STUB_LOG="$work/cargo.log" STUB_CRATE="$crate" \
    bash "$repo/scripts/sync-loco-skill.sh" "$@" 2>&1)" || status=$?
}

fresh cargo
: >"$repo/Cargo.toml"
run_with_stub ok --check
expect_run "without Cargo.lock the script refuses" 2 "no Cargo.lock at the repository root"
: >"$repo/Cargo.lock"
run_with_stub ok
expect_run "the sync reads the locked crate from cargo metadata" 0 "from loco-rs 1.2.0"
expect_equal_tree "the sync through cargo metadata copies the crate's skill"
if grep -qx -- "fetch --locked --manifest-path $repo/Cargo.toml" "$work/cargo.log" &&
  grep -qx -- "metadata --format-version 1 --locked --manifest-path $repo/Cargo.toml" "$work/cargo.log"; then
  report ok "it runs cargo fetch --locked, then cargo metadata --format-version 1 --locked" ""
else
  report fail "it runs cargo fetch --locked, then cargo metadata --format-version 1 --locked" "$(cat "$work/cargo.log")"
fi
run_with_stub ok --check
expect_run "--check through cargo metadata passes on an equal tree" 0 "matches loco-rs 1.2.0"
run_with_stub none --check
expect_run "no locked loco-rs package is reported" 2 "locks no loco-rs package"
run_with_stub two --check
expect_run "two locked loco-rs packages are reported" 2 "more than one loco-rs package (1.2.0, 1.3.0)"
run_with_stub path --check
expect_run "a loco-rs that is not from a registry is refused" 2 "does not come from a registry"
run_with_stub fail-fetch --check
expect_run "a failing cargo fetch --locked is reported" 2 "'cargo fetch --locked' failed"
run_with_stub fail-metadata --check
expect_run "a failing cargo metadata is reported" 2 "'cargo metadata --format-version 1 --locked' failed"

# ------------------------------------------------------------- summary

printf '# %d run, %d failed\n' "$tests_run" "$failures"
((failures == 0))
