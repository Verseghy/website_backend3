#!/usr/bin/env bash
# sync-loco-skill.sh: keeps .claude/skills/loco/ a byte-identical copy of the
# Loco agent skill shipped inside the loco-rs crate that Cargo.lock locks
# (ADR 0024 section 3).
#
# Why it exists
#   The skill teaches agents the Loco API of one exact version. Copies from the
#   loco.rs website or from GitHub master describe other versions, and the
#   upstream indexes drift between releases. So the repository vendors the
#   crate's own skills/loco/ tree verbatim, next to a byte copy of the crate's
#   Apache-2.0 LICENSE (ADR 0012 section 4), and CI fails ("Loco skill" job,
#   ADR 0022) whenever the copy and the locked crate differ. Project rules never
#   go into the vendored files; they live in CLAUDE.md and .claude/skills/backend3/.
#
# Usage
#   scripts/sync-loco-skill.sh [--check] [--crate-dir PATH]
#
#   (no option)       make .claude/skills/loco/ equal to <crate>/skills/loco/,
#                     deleting files upstream no longer has, and copy
#                     <crate>/LICENSE to .claude/skills/loco/LICENSE.
#   --check           write nothing; print every difference between the two
#                     trees and the licence file, and exit 1 if there is any.
#   --crate-dir PATH  use PATH as the loco-rs crate directory instead of the
#                     one cargo resolves (for tests against a fixture crate).
#
#   Without --crate-dir the script runs `cargo fetch --locked` and reads the
#   locked loco-rs package's manifest_path from
#   `cargo metadata --format-version 1 --locked` with jq.
#
# Exit codes
#   0  in sync (or synced); 1  --check found differences;
#   2  usage error or a missing tool, file or crate.
#
# The destination is always .claude/skills/loco/ under the repository that
# contains this script (the parent of scripts/), so tests run a copy of the
# script inside a scratch repository.

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/sync-loco-skill.sh [--check] [--crate-dir PATH]

Makes .claude/skills/loco/ a byte-identical copy of skills/loco/ (plus LICENSE)
from the loco-rs crate locked in Cargo.lock (ADR 0024 section 3).

  --check           write nothing; print the differences; exit 1 if any
  --crate-dir PATH  read the crate from PATH instead of asking cargo
  -h, --help        show this help
EOF
}

# Prints an error and exits with code 2.
#   $* - the message: what is wrong and how to fix it.
die() {
  printf 'sync-loco-skill.sh: %s\n' "$*" >&2
  exit 2
}

# Prints a usage error and the usage, and exits with code 2.
#   $* - what was wrong with the arguments.
die_usage() {
  printf 'sync-loco-skill.sh: %s\n\n' "$*" >&2
  usage >&2
  exit 2
}

check_only=false
crate_dir=""
crate_dir_given=false
while (($# > 0)); do
  case "$1" in
    --check)
      check_only=true
      shift
      ;;
    --crate-dir)
      (($# >= 2)) || die_usage "--crate-dir needs a path, for example --crate-dir ~/.cargo/registry/src/<index>/loco-rs-1.2.0"
      crate_dir="$2"
      crate_dir_given=true
      shift 2
      ;;
    --crate-dir=*)
      crate_dir="${1#--crate-dir=}"
      crate_dir_given=true
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      die_usage "unknown argument '$1'"
      ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$repo_root/.claude/skills/loco"
dest_label=".claude/skills/loco"

# Resolves the directory of the loco-rs crate that Cargo.lock locks, and the
# label used in messages. Sets: crate_dir, crate_label.
resolve_locked_crate() {
  local manifest="$repo_root/Cargo.toml"
  [[ -f "$manifest" ]] || die "no Cargo.toml at the repository root ($repo_root); run this script from a checkout of website_backend3, or pass --crate-dir."
  [[ -f "$repo_root/Cargo.lock" ]] || die "no Cargo.lock at the repository root; it must be committed (ADR 0024 section 3). Run 'cargo generate-lockfile' and commit it."
  command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH; install the toolchain from rust-toolchain.toml (rustup installs it on first use)."
  command -v jq >/dev/null 2>&1 || die "jq is not on PATH; install jq and retry."

  # Downloads the locked crates (a no-op when they are cached). --locked makes
  # a stale Cargo.lock an error instead of a silent update.
  cargo fetch --locked --manifest-path "$manifest" >&2 ||
    die "'cargo fetch --locked' failed. If Cargo.lock is out of date, update it in a dedicated PR (ADR 0024 section 3: cargo update -p loco-rs --precise X.Y.Z)."

  local metadata
  metadata="$(cargo metadata --format-version 1 --locked --manifest-path "$manifest")" ||
    die "'cargo metadata --format-version 1 --locked' failed; see the cargo error above."

  local found
  found="$(jq -r '
    [.packages[] | select(.name == "loco-rs")] as $p
    | if ($p | length) == 0 then "error\tCargo.lock locks no loco-rs package; Cargo.toml must depend on loco-rs = \"=X.Y.Z\""
      elif ($p | length) > 1 then "error\tCargo.lock locks more than one loco-rs package (\($p | map(.version) | join(", "))); keep exactly one"
      elif (($p[0].source // "") | test("^(registry|sparse)\\+") | not) then "error\tloco-rs \($p[0].version) does not come from a registry (source: \($p[0].source // "a path")); the skill is vendored only from the crates.io release"
      else "ok\t\($p[0].version)\t\($p[0].manifest_path)" end' <<<"$metadata")" ||
    die "could not read the cargo metadata output with jq."

  local status version manifest_path
  IFS=$'\t' read -r status version manifest_path <<<"$found"
  [[ "$status" == ok ]] || die "${found#error$'\t'}"
  crate_dir="$(dirname "$manifest_path")"
  crate_label="loco-rs $version"
}

if [[ "$crate_dir_given" == true ]]; then
  [[ -n "$crate_dir" ]] || die_usage "--crate-dir needs a non-empty path."
  [[ -d "$crate_dir" ]] || die "--crate-dir '$crate_dir' is not a directory."
  crate_dir="$(cd "$crate_dir" && pwd)"
  crate_label="the crate at $crate_dir"
else
  resolve_locked_crate
fi

src="$crate_dir/skills/loco"
src_license="$crate_dir/LICENSE"
[[ -f "$src/SKILL.md" ]] || die "$crate_label has no skills/loco/SKILL.md ($src). Is this a loco-rs crate of version 1.2.0 or later?"
[[ -f "$src_license" ]] || die "$crate_label has no LICENSE file ($src_license); the vendored skill must carry the crate's licence (ADR 0012 section 4)."
[[ ! -e "$src/LICENSE" ]] || die "upstream now ships skills/loco/LICENSE, which collides with the licence copy this script writes; ADR 0024 section 3 needs a decision before the next sync."

# Lists the files of a tree as relative paths, NUL-separated and sorted.
# Symbolic links and other non-regular entries are reported as "!path", so
# the caller can refuse them.
#   $1 - the directory to list.
list_tree() {
  (
    cd "$1"
    find . \( -type f -o -type l -o -type p -o -type s -o -type b -o -type c \) -print0 |
      while IFS= read -r -d '' path; do
        path="${path#./}"
        if [[ -f "$path" && ! -L "$path" ]]; then
          printf '%s\0' "$path"
        else
          printf '!%s\0' "$path"
        fi
      done | LC_ALL=C sort -z
  )
}

src_files=()
while IFS= read -r -d '' path; do
  [[ "$path" != '!'* ]] || die "upstream skills/loco/${path#!} is not a regular file; the script copies regular files only."
  src_files+=("$path")
done < <(list_tree "$src")
((${#src_files[@]} > 0)) || die "$src is empty."

dest_files=()
if [[ -d "$dest" ]]; then
  while IFS= read -r -d '' path; do
    dest_files+=("$path")
  done < <(list_tree "$dest")
fi

differences=0

# Records and prints one difference found by --check.
#   $1 - the kind: missing, differs or extra;
#   $2 - the path relative to the skill directory;
#   $3 - an explanation.
difference() {
  differences=$((differences + 1))
  printf '%-8s %s/%s: %s\n' "$1" "$dest_label" "$2" "$3"
}

# Prints a short unified diff of a changed text file, for the CI log.
#   $1 - the vendored file; $2 - the upstream file.
show_diff() {
  command -v diff >/dev/null 2>&1 || return 0
  diff -u --label "upstream" --label "vendored" "$2" "$1" | head -n 60 | sed 's/^/    /' || true
}

if [[ "$check_only" == true ]]; then
  for path in "${src_files[@]}"; do
    if [[ ! -e "$dest/$path" && ! -L "$dest/$path" ]]; then
      difference missing "$path" "upstream has it, the vendored copy does not"
    elif [[ ! -f "$dest/$path" || -L "$dest/$path" ]]; then
      difference differs "$path" "not a regular file in the vendored copy"
    elif ! cmp -s "$src/$path" "$dest/$path"; then
      difference differs "$path" "content differs from upstream"
      show_diff "$dest/$path" "$src/$path"
    fi
  done
  for path in "${dest_files[@]}"; do
    rel="${path#!}"
    [[ "$rel" != LICENSE ]] || continue
    if [[ ! -e "$src/$rel" ]]; then
      difference extra "$rel" "upstream no longer has it (or never had it)"
    fi
  done
  if [[ ! -e "$dest/LICENSE" ]]; then
    difference missing LICENSE "the copy of the crate's LICENSE is missing"
  elif [[ ! -f "$dest/LICENSE" || -L "$dest/LICENSE" ]] || ! cmp -s "$src_license" "$dest/LICENSE"; then
    difference differs LICENSE "not a byte copy of the crate's LICENSE"
  fi

  if ((differences > 0)); then
    printf '\n%d difference(s) between %s and %s/skills/loco.\n' "$differences" "$dest_label" "$crate_label" >&2
    printf 'Run scripts/sync-loco-skill.sh to re-sync. Never edit the vendored files by hand: project rules go into CLAUDE.md or .claude/skills/backend3/ (ADR 0024 section 3).\n' >&2
    exit 1
  fi
  printf 'Loco skill: %s matches %s (%d files plus LICENSE).\n' "$dest_label" "$crate_label" "${#src_files[@]}"
  exit 0
fi

# Sync: remove what upstream no longer has, then copy every upstream file and
# the licence. Copying only changed files keeps unchanged files' timestamps.
removed=0
copied=0
mkdir -p "$dest"
for path in "${dest_files[@]}"; do
  rel="${path#!}"
  [[ "$rel" != LICENSE ]] || continue
  if [[ "$path" == '!'* || ! -e "$src/$rel" || -d "$src/$rel" ]]; then
    rm -f -- "$dest/$rel"
    removed=$((removed + 1))
  fi
done
for path in "${src_files[@]}"; do
  if [[ -d "$dest/$path" && ! -L "$dest/$path" ]]; then
    rm -rf -- "${dest:?}/$path"
  fi
  if [[ ! -f "$dest/$path" ]] || ! cmp -s "$src/$path" "$dest/$path"; then
    mkdir -p "$(dirname "$dest/$path")"
    cp -- "$src/$path" "$dest/$path"
    copied=$((copied + 1))
  fi
done
if [[ ! -f "$dest/LICENSE" ]] || ! cmp -s "$src_license" "$dest/LICENSE"; then
  rm -rf -- "${dest:?}/LICENSE"
  cp -- "$src_license" "$dest/LICENSE"
  copied=$((copied + 1))
fi
# Directories left empty by the removals.
find "$dest" -mindepth 1 -type d -empty -delete

printf 'Loco skill: synced %s from %s (%d files plus LICENSE; %d written, %d removed).\n' \
  "$dest_label" "$crate_label" "${#src_files[@]}" "$copied" "$removed"
