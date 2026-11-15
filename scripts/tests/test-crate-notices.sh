#!/usr/bin/env bash
# Tests for scripts/crate-notices.sh (ADR 0042).
#
# Each case copies the script into a scratch repository and runs it against
# stub `cargo`, `rustc` and `cargo-about` programs on PATH, with fixture
# crates in a scratch registry: cargo-about's JSON output, its messages,
# `cargo tree`'s output and the toolchain's sysroot are fixture files. So the
# cases need no network, no real cargo-about and no Rust build, and each one
# sets up exactly the gap it expects the script to find. The fixture JSON
# has the shape of cargo-about 0.9.3's: a licence text from a file carries
# that file's exact content, and crates whose files hold the same text share
# one entry. The last cases read the committed about.toml, about.hbs,
# deny.toml, Containerfile and workflow, and check that they stay in step
# with the script.
#
# Run from the repository root:
#   bash scripts/tests/test-crate-notices.sh
# Needs jq and mikefarah yq v4.
# Single-quoted stub bodies and jq programs hold a literal `$` on purpose.
# shellcheck disable=SC2016
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT="$ROOT/scripts/crate-notices.sh"

tests_run=0
failures=0

# Records one result and prints a TAP-style line.
#   $1 - the case name; $2 - empty when the case passed, else the reason;
#   $3 - the output to show under a failure.
report() {
  tests_run=$((tests_run + 1))
  if [[ -z $2 ]]; then
    printf 'ok %d - %s\n' "$tests_run" "$1"
  else
    failures=$((failures + 1))
    printf 'not ok %d - %s (%s)\n%s\n' "$tests_run" "$1" "$2" "$3" | sed '2,$s/^/    # /'
  fi
}

# run_case NAME EXPECTED_EXIT [--contains TEXT]... [--lacks TEXT]... -- COMMAND...
# Runs COMMAND, compares its exit code and looks for each TEXT in its combined
# output (or checks that it is absent). Leaves the output in $LAST_OUTPUT.
run_case() {
  local name=$1 expected=$2
  shift 2
  local -a wanted=() unwanted=()
  while [[ ${1-} == --contains || ${1-} == --lacks ]]; do
    if [[ $1 == --contains ]]; then wanted+=("$2"); else unwanted+=("$2"); fi
    shift 2
  done
  [[ ${1-} == -- ]] && shift
  local status=0 needle reason=""
  LAST_OUTPUT=$("$@" 2>&1) || status=$?
  if [[ $status -ne $expected ]]; then
    reason="exit $status, expected $expected"
  else
    for needle in "${wanted[@]}"; do
      [[ $LAST_OUTPUT == *"$needle"* ]] || { reason="output lacks \"$needle\""; break; }
    done
    for needle in "${unwanted[@]}"; do
      [[ $LAST_OUTPUT != *"$needle"* ]] || { reason="output has \"$needle\""; break; }
    done
  fi
  report "$name" "$reason" "$LAST_OUTPUT"
}

# check NAME CONDITION...: report whether a test command succeeds.
check() {
  local name=$1
  shift
  if "$@"; then report "$name" "" ""; else report "$name" "condition failed: $1 ..." ""; fi
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The stubs. Each reads its fixture from $FAKE_DIR and logs its arguments to
# $FAKE_DIR/calls.log. cargo-about prints about-stderr in both runs, and
# about-stderr-html as well in the template run.
mkdir -p "$WORK/bin"
cat >"$WORK/bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf 'cargo %s\n' "$*" >>"$FAKE_DIR/calls.log"
case $1 in
  fetch) exit "$(cat "$FAKE_DIR/fetch-exit" 2>/dev/null || echo 0)" ;;
  tree) cat "$FAKE_DIR/tree.txt"; exit "$(cat "$FAKE_DIR/tree-exit" 2>/dev/null || echo 0)" ;;
  *) echo "stub cargo: unexpected subcommand $1" >&2; exit 99 ;;
esac
EOF
cat >"$WORK/bin/rustc" <<'EOF'
#!/usr/bin/env bash
case $* in
  "--print sysroot") printf '%s\n' "$FAKE_DIR/sysroot" ;;
  --version) echo "rustc 1.99.1 (stub)" ;;
  *) echo "stub rustc: unexpected arguments $*" >&2; exit 99 ;;
esac
EOF
cat >"$WORK/bin/cargo-about" <<'EOF'
#!/usr/bin/env bash
if [[ $* == --version ]]; then echo "cargo-about 0.9.3"; exit 0; fi
printf 'cargo-about %s\n' "$*" >>"$FAKE_DIR/calls.log"
[[ -f $FAKE_DIR/about-stderr ]] && cat "$FAKE_DIR/about-stderr" >&2
code=$(cat "$FAKE_DIR/about-exit" 2>/dev/null || echo 0)
((code == 0)) || exit "$code"
out="" json=0 prev=""
for arg in "$@"; do
  [[ $prev == --output-file ]] && out=$arg
  [[ $prev == --format && $arg == json ]] && json=1
  prev=$arg
done
if ((json)); then
  cp "$FAKE_DIR/notices.json" "$out"
else
  [[ -f $FAKE_DIR/about-stderr-html ]] && cat "$FAKE_DIR/about-stderr-html" >&2
  echo "<html>stub</html>" >"$out"
fi
exit 0
EOF
chmod +x "$WORK/bin/cargo" "$WORK/bin/rustc" "$WORK/bin/cargo-about"

# fixture NAME: create a case directory with a scratch repository (the
# script and the files it requires), an empty registry and a sysroot that
# carries COPYRIGHT-library.html; print its path.
fixture() {
  local dir="$WORK/$1"
  mkdir -p "$dir/repo/scripts" "$dir/registry" "$dir/sysroot/share/doc/rust"
  cp "$SCRIPT" "$dir/repo/scripts/crate-notices.sh"
  : >"$dir/repo/Cargo.toml"
  : >"$dir/repo/Cargo.lock"
  : >"$dir/repo/about.toml"
  : >"$dir/repo/about.hbs"
  echo "<html>std notices</html>" >"$dir/sysroot/share/doc/rust/COPYRIGHT-library.html"
  : >"$dir/tree.txt"
  printf '%s' "$dir"
}

# add_crate DIR NAME VERSION [FILE CONTENT]...: create a fixture crate in
# DIR's registry with the given files (paths relative to the crate).
add_crate() {
  local dir=$1 name=$2 version=$3
  shift 3
  local crate="$dir/registry/$name-$version"
  mkdir -p "$crate"
  echo "[package]" >"$crate/Cargo.toml"
  while (($# >= 2)); do
    mkdir -p "$(dirname "$crate/$1")"
    printf '%s' "$2" >"$crate/$1"
    shift 2
  done
}

# notices_json DIR SPEC...: write DIR/notices.json in the shape of
# `cargo-about generate --format json`. Each SPEC is NAME:VERSION:TEXTS,
# where TEXTS is a comma-separated list, empty for a crate without any text:
# `std` is a standard licence text (no file behind it), anything else is the
# path of a file of the crate whose content becomes one of its texts. As
# cargo-about does, crates with the same text share one entry, which keeps
# the first crate's source_path.
notices_json() {
  local dir=$1 spec name version texts item crate
  shift
  local -a items=()
  {
    for spec in "$@"; do
      IFS=: read -r name version texts <<<"$spec"
      crate="$dir/registry/$name-$version"
      jq -nc --arg name "$name" --arg version "$version" --arg dir "$crate" \
        '{crate: {name: $name, version: $version, dir: $dir}}'
      items=()
      [[ -z $texts ]] || IFS=, read -r -a items <<<"$texts"
      for item in "${items[@]}"; do
        if [[ $item == std ]]; then
          jq -nc --arg key "$name $version" '{of: $key, source_path: null, text: "The standard MIT text."}'
        else
          jq -nc --arg key "$name $version" --arg path "$crate/$item" --rawfile text "$crate/$item" \
            '{of: $key, source_path: $path, text: $text}'
        fi
      done
    done
  } | jq -s '
    [.[] | select(has("crate")) | .crate
     | . + {id: "\(.name) \(.version) (registry+https://github.com/rust-lang/crates.io-index)"}] as $crates
    | (reduce $crates[] as $c ({}; .["\($c.name) \($c.version)"] = $c)) as $by
    | {
        overview: [],
        crates: [$crates[] | {package: {id, name, version, manifest_path: "\(.dir)/Cargo.toml"}, license: "MIT"}],
        licenses: ([.[] | select(has("of"))] | group_by([.source_path == null, .text])
          | map({id: "MIT", source_path: .[0].source_path, text: .[0].text,
                 used_by: [.[] | $by[.of] | {crate: {id, name, version}}]}))
      }' >"$dir/notices.json"
}

# about_toml DIR TEXT: write the fixture repository's about.toml.
about_toml() {
  printf '%s' "$2" >"$1/repo/about.toml"
}

# clarification CRATE PATH SUM [LICENSE]: print an about.toml clarification
# of one file in the form the committed about.toml uses.
clarification() {
  printf '[%s.clarify]\nlicense = "%s"\n[[%s.clarify.files]]\npath = "%s"\nchecksum = "%s"\n' \
    "$1" "${4:-MIT}" "$1" "$2" "$3"
}

# sha FILE-CONTENT: print the SHA-256 of a string.
sha() {
  printf '%s' "$1" | sha256sum | cut -d' ' -f1
}

# run DIR [ARGS...]: run the fixture's copy of the script with the stubs,
# writing into DIR/out unless ARGS name another output.
run() {
  local dir=$1
  shift
  local -a args=("$@")
  ((${#args[@]} > 0)) || args=(--out "$dir/out")
  FAKE_DIR=$dir PATH="$WORK/bin:$PATH" bash "$dir/repo/scripts/crate-notices.sh" "${args[@]}"
}

# run_private DIR [ARGS...]: `run` under umask 077, so every file the script
# creates is private unless the script itself makes it world-readable.
run_private() {
  (
    umask 077
    run "$@"
  )
}

MIT_TEXT=$'Copyright (c) 2026 Example\n\nPermission is hereby granted, free of charge, ...\n'
BSD_TEXT=$'Copyright (c) 2010, Example Corp\n\nRedistribution and use in source and binary forms, ...\n'
ZERO_SUM=0000000000000000000000000000000000000000000000000000000000000000

# --- The happy path ---------------------------------------------------------

d=$(fixture good)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
add_crate "$d" bravo 2.0.0 README.md "no licence file here"
add_crate "$d" charlie 0.9.0 LICENSE "../LICENSE"
add_crate "$d" delta 0.58.2 LICENSE-APACHE "Apache text" NOTICE "Apache Delta, copyright the authors" \
  NOTICE.txt "second notice" src/notice/mod.rs "a directory named notice is no NOTICE file"
add_crate "$d" echo 1.2.3+build.4 LICENSE-MIT "../LICENSE-MIT"
notices_json "$d" alpha:1.0.0:LICENSE bravo:2.0.0:std charlie:0.9.0:std delta:0.58.2:LICENSE-APACHE echo:1.2.3+build.4:std
printf '%s\n' "backend3 v0.1.0 ($d/repo)" "alpha v1.0.0" "bravo v2.0.0" "alpha v1.0.0 (*)" \
  "migration v0.1.0 ($d/repo/migration)" "delta v0.58.2" "echo v1.2.3+build.4" >"$d/tree.txt"
run_case "a complete graph passes" 0 \
  --contains "5 crates, 2 NOTICE file(s)" --lacks "problem" -- run_private "$d"
check "crates.html is cargo-about's template output" grep -q "<html>stub</html>" "$d/out/crates.html"
check "the root NOTICE files are copied under notices/<crate>-<version>/" \
  cmp -s "$d/registry/delta-0.58.2/NOTICE" "$d/out/notices/delta-0.58.2/NOTICE"
check "NOTICE.txt is copied too" test -f "$d/out/notices/delta-0.58.2/NOTICE.txt"
check "COPYRIGHT-library.html comes from the sysroot" grep -q "std notices" "$d/out/COPYRIGHT-library.html"
check "README.txt states the IJG sentence" \
  grep -qx "    This software is based in part on the work of the Independent JPEG Group." "$d/out/README.txt"
check "README.txt lists each NOTICE file" grep -qx "      notices/delta-0.58.2/NOTICE.txt" "$d/out/README.txt"
check "README.txt counts the crates and names the tools" \
  grep -q "The 5 third-party crates" "$d/out/README.txt"
check "README.txt names cargo-about's version" grep -q "Written by cargo-about 0.9.3" "$d/out/README.txt"
check "README.txt names the toolchain" grep -q "rustc 1.99.1 (stub)" "$d/out/README.txt"
check "every written file is world-readable, also under umask 077" \
  test -z "$(find "$d/out" \( -type f ! -perm -a+r \) -o \( -type d ! -perm -a+rx \))"
check "cargo fetch runs with --locked" grep -q "^cargo fetch --locked " "$d/calls.log"
check "cargo-about runs frozen, with --fail, the release target and about.toml" \
  grep -q -- "generate --manifest-path $d/repo/Cargo.toml --config $d/repo/about.toml --frozen --fail --target x86_64-unknown-linux-gnu" "$d/calls.log"
check "cargo-about runs twice: JSON for the checks, the template for the file" \
  test "$(grep -c '^cargo-about ' "$d/calls.log")" -eq 2
check "the template run uses about.hbs" grep -q -- "--output-file $d/out/crates.html $d/repo/about.hbs" "$d/calls.log"
check "cargo tree reads the linked crates of the release target, without colour" \
  grep -q -- "tree --frozen --color never --manifest-path $d/repo/Cargo.toml -e normal,no-proc-macro --target x86_64-unknown-linux-gnu --prefix none --format {p}" "$d/calls.log"

d=$(fixture no-notices)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE
run_case "a graph without NOTICE files passes" 0 --contains "1 crates, 0 NOTICE file(s)" -- run "$d"
check "README.txt says that no crate ships a NOTICE file" \
  grep -q "None of these crates ships a NOTICE file, so there is no notices/." "$d/out/README.txt"
check "no notices/ directory is written without NOTICE files" test ! -e "$d/out/notices"

# --- Check 2: every clarification holds --------------------------------------

d=$(fixture clarify-holds)
add_crate "$d" webp-sys 0.14.4 vendor/COPYING "$BSD_TEXT" src/lib.rs "code"
notices_json "$d" webp-sys:0.14.4:std,vendor/COPYING
about_toml "$d" "# A comment, and a value with one after it.
$(clarification webp-sys vendor/COPYING "$(sha "$BSD_TEXT")" "MIT AND BSD-3-Clause" | sed 's/^path = .*/& # the bundled libwebp/')
"
run_case "a clarification of bundled code that holds passes" 0 --lacks "problem" -- run "$d"

d=$(fixture clarify-stale-below-root)
add_crate "$d" webp-sys 0.14.4 vendor/COPYING "$BSD_TEXT"
notices_json "$d" webp-sys:0.14.4:std
about_toml "$d" "$(clarification webp-sys vendor/COPYING "$ZERO_SUM" "MIT AND BSD-3-Clause")"
printf '2026-11-01T00:00:00.000000000Z \033[33m[WARN]\033[0m failed to validate all files specified in clarification for crate webp-sys 0.14.4: checksum mismatch, expected '\''%s'\''\n' "$ZERO_SUM" >"$d/about-stderr"
run_case "a changed file of bundled code fails, with its new SHA-256" 1 \
  --contains "webp-sys 0.14.4: vendor/COPYING has changed" \
  --contains "renew its checksum in about.toml to $(sha "$BSD_TEXT") (about.toml has $ZERO_SUM)" \
  --lacks "cargo-about could not apply" --lacks "the notices do not carry vendor/COPYING" -- run "$d"
check "the stale checksum is the only problem" grep -q "1 problem(s)" <<<"$LAST_OUTPUT"

d=$(fixture clarify-stale-recognised)
add_crate "$d" sync_wrapper 1.0.2 LICENSE "$MIT_TEXT"
notices_json "$d" sync_wrapper:1.0.2:LICENSE
about_toml "$d" "$(clarification sync_wrapper LICENSE "$ZERO_SUM")"
run_case "a changed root file fails even when cargo-about's scan reads it" 1 \
  --contains "sync_wrapper 1.0.2: LICENSE has changed" -- run "$d"

d=$(fixture clarify-flattened-version)
add_crate "$d" tz 0.10.4 LICENSE "$MIT_TEXT"
add_crate "$d" tz 0.9.0 LICENSE "../LICENSE"
add_crate "$d" tz 0.8.0 LICENSE ""
notices_json "$d" tz:0.10.4:LICENSE tz:0.9.0:std tz:0.8.0:std
about_toml "$d" "$(clarification tz LICENSE "$(sha "$MIT_TEXT")")"
printf '[WARN] failed to validate all files specified in clarification for crate tz 0.9.0: checksum mismatch\n[WARN] failed to validate all files specified in clarification for crate tz 0.8.0: clarification file is empty\n' >"$d/about-stderr"
run_case "a version whose clarified file holds no text (a flattened link, an empty file) passes" 0 \
  --contains "for crate tz 0.9.0" --lacks "problem" -- run "$d"

d=$(fixture clarify-missing-file)
add_crate "$d" tz 0.10.4 LICENSE "$MIT_TEXT"
add_crate "$d" tz 0.11.0 COPYING "$MIT_TEXT"
notices_json "$d" tz:0.10.4:LICENSE tz:0.11.0:COPYING
about_toml "$d" "$(clarification tz LICENSE "$(sha "$MIT_TEXT")")"
run_case "a version without the clarified file fails" 1 \
  --contains "tz 0.11.0 has no file LICENSE, which about.toml's clarification of tz names" -- run "$d"

d=$(fixture clarify-misspelt)
add_crate "$d" webp-sys 0.14.4 vendor/COPYING "$BSD_TEXT"
notices_json "$d" webp-sys:0.14.4:std
about_toml "$d" "$(clarification webp_sys vendor/COPYING "$(sha "$BSD_TEXT")")"
run_case "a clarification of a crate that the notices do not list fails" 1 \
  --contains "about.toml clarifies webp_sys, which the notices do not list" -- run "$d"

d=$(fixture clarify-rejected-version)
add_crate "$d" deflate-sys 1.27.0 LICENSE "../LICENSE" libdeflate/COPYING "$MIT_TEXT"
notices_json "$d" deflate-sys:1.27.0:std
about_toml "$d" "[deflate-sys.clarify]
license = \"Apache-2.0 AND MIT\"
[[deflate-sys.clarify.files]]
path = \"LICENSE\"
license = \"Apache-2.0\"
checksum = \"$ZERO_SUM\"
[[deflate-sys.clarify.files]]
path = \"libdeflate/COPYING\"
license = \"MIT\"
checksum = \"$(sha "$MIT_TEXT")\"
"
echo "[WARN] failed to validate all files specified in clarification for crate deflate-sys 1.27.0: checksum mismatch" >"$d/about-stderr"
run_case "a version that rejects the clarification fails when another named file holds text" 1 \
  --contains "deflate-sys 1.27.0: the notices do not carry libdeflate/COPYING" \
  --lacks "cargo-about could not apply" -- run "$d"

d=$(fixture clarify-unexplained)
add_crate "$d" webp-sys 0.14.4 vendor/COPYING "$BSD_TEXT"
notices_json "$d" webp-sys:0.14.4:std,vendor/COPYING
about_toml "$d" "$(clarification webp-sys vendor/COPYING "$(sha "$BSD_TEXT")")"
echo "[WARN] failed to validate all files specified in clarification for crate webp-sys 0.14.4: stream did not contain valid UTF-8" >"$d/about-stderr"
run_case "any other clarification that cargo-about rejects fails" 1 \
  --contains "cargo-about could not apply about.toml's clarification of webp-sys 0.14.4 (stream did not contain valid UTF-8)" -- run "$d"

# about.toml forms that the reader does not understand fail instead of
# going unchecked.
about_form_case() {
  local name=$1 text=$2 message=$3 d
  d=$(fixture "form-$name")
  add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
  notices_json "$d" alpha:1.0.0:LICENSE
  about_toml "$d" "$text"
  run_case "about.toml: $name fails" 1 --contains "$message" -- run "$d"
}
sum=$(sha "$MIT_TEXT")
about_form_case "a partial-file key" "$(clarification alpha LICENSE "$sum")
start = \"Copyright\"" "[[alpha.clarify.files]] sets 'start'"
about_form_case "an inline files list" "[alpha.clarify]
license = \"MIT\"
files = [{ path = \"LICENSE\", checksum = \"$sum\" }]" "[alpha.clarify] sets only \`license\` here"
about_form_case "a clarification without files" "[alpha.clarify]
license = \"MIT\"" "the clarification of alpha names no file"
about_form_case "a quoted crate name" "[\"alpha\".clarify]
license = \"MIT\"" "is a clarification table that scripts/crate-notices.sh cannot read"
about_form_case "a dotted key" "alpha.clarify.license = \"MIT\"" "is a clarification that scripts/crate-notices.sh cannot read"
about_form_case "a files table without a checksum" "[alpha.clarify]
license = \"MIT\"
[[alpha.clarify.files]]
path = \"LICENSE\"" "the [[alpha.clarify.files]] table needs both \`path\` and \`checksum\`"
about_form_case "a path outside the crate" "$(clarification alpha ../beta/LICENSE "$sum")" "names '../beta/LICENSE'; name a file inside the crate"
about_form_case "a checksum that is no SHA-256" "$(clarification alpha LICENSE ABC)" "is 'ABC', which is not a SHA-256"
about_form_case "a files table before its clarify table" "[[alpha.clarify.files]]
path = \"LICENSE\"
checksum = \"$sum\"
[alpha.clarify]
license = \"MIT\"" "comes before its [alpha.clarify] table"
BACKSLASH=$'\x5c'
about_form_case "an escaped string" "$(clarification alpha "LICEN${BACKSLASH}x53E" "$sum")" "cannot read 'path = \"LICEN${BACKSLASH}x53E\"'"

# --- Check 3: the licence files that the notices must carry -----------------

for file in LICENSE LICENCE LICENSE-MIT LICENSE_MIT license.txt COPYING COPYRIGHT UNLICENSE LICENSES/MIT.txt; do
  d=$(fixture "standard-$(tr '/.' '--' <<<"$file")")
  add_crate "$d" sync_wrapper 1.0.2 "$file" "$MIT_TEXT"
  notices_json "$d" sync_wrapper:1.0.2:std
  run_case "the standard text fails when the crate ships $file" 1 \
    --contains "sync_wrapper 1.0.2 gets the standard text of a licence" \
    --contains "although it ships $file, which the notices do not carry" \
    --contains "[sync_wrapper.clarify] entry of about.toml with its SHA-256, $(sha "$MIT_TEXT")" -- run "$d"
done

d=$(fixture two-line-link)
add_crate "$d" foxtrot 1.0.0 LICENSE $'../LICENSE\nmore text\n'
notices_json "$d" foxtrot:1.0.0:std
run_case "a file that only starts with ../ is a licence text" 1 --contains "although it ships LICENSE" -- run "$d"

d=$(fixture link-with-space)
add_crate "$d" golf 1.0.0 LICENSE "../see the licence"
notices_json "$d" golf:1.0.0:std
run_case "a relative path with blanks is not a flattened link" 1 --contains "although it ships LICENSE" -- run "$d"

d=$(fixture empty-licence)
add_crate "$d" hotel 1.0.0 LICENSE ""
notices_json "$d" hotel:1.0.0:std
run_case "an empty licence file holds no licence text and does not count" 0 -- run "$d"

d=$(fixture licence-elsewhere)
add_crate "$d" india 1.0.0 src/LICENSE "$MIT_TEXT" LICENSE.md.bak/keep "a directory, not a file"
notices_json "$d" india:1.0.0:std
run_case "licence files below the root, and directories, do not count" 0 -- run "$d"

d=$(fixture licence-like-names)
add_crate "$d" juliet 1.0.0 LICENSEE.md "not a licence file name" COPYINGS "nor this"
notices_json "$d" juliet:1.0.0:std
run_case "names that only start like a licence file do not count" 0 -- run "$d"

d=$(fixture mixed)
add_crate "$d" mozjpeg-sys 2.2.3 LICENSE "$BSD_TEXT"
notices_json "$d" mozjpeg-sys:2.2.3:LICENSE,std
run_case "a crate whose standard text sits beside its one carried licence file passes" 0 -- run "$d"

d=$(fixture and-expression)
add_crate "$d" kilo 1.0.0 LICENSE-MIT "$MIT_TEXT" LICENSE-BSD "$BSD_TEXT"
notices_json "$d" kilo:1.0.0:LICENSE-MIT,std
run_case "a crate with its own text for one licence fails when another licence file is not carried" 1 \
  --contains "kilo 1.0.0 gets the standard text of a licence" --contains "although it ships LICENSE-BSD" \
  --lacks "ships LICENSE-MIT" -- run "$d"

d=$(fixture shared-text)
add_crate "$d" lima 1.0.0 LICENSE "$MIT_TEXT"
add_crate "$d" mike 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" lima:1.0.0:LICENSE mike:1.0.0:LICENSE,std
check "the fixture shares one text between two crates, as cargo-about does" \
  test "$(jq '[.licenses[] | select(.source_path != null) | .used_by | length] | max' "$d/notices.json")" -eq 2
run_case "a licence file counts as carried when another crate's entry holds the same text" 0 -- run "$d"

d=$(fixture other-file)
add_crate "$d" november 1.0.0 LICENSE "$MIT_TEXT" src/lib.rs "// SPDX header and code"
notices_json "$d" november:1.0.0:src/lib.rs,std
run_case "a text from another file does not carry the licence file" 1 \
  --contains "november 1.0.0 gets the standard text of a licence" --contains "although it ships LICENSE" -- run "$d"

d=$(fixture no-text)
add_crate "$d" oscar 1.0.0
notices_json "$d" oscar:1.0.0:
run_case "a crate without any licence text fails" 1 --contains "oscar 1.0.0: the notices hold no licence text" -- run "$d"

# --- Check 4: NOTICE files below a crate's root ------------------------------

d=$(fixture deep-notice)
add_crate "$d" papa 1.0.0 LICENSE "$MIT_TEXT" vendor/libfoo/NOTICE "bundled notice"
notices_json "$d" papa:1.0.0:LICENSE
run_case "a NOTICE file below the root fails and is named" 1 \
  --contains "papa 1.0.0 has a NOTICE file below its root, vendor/libfoo/NOTICE" -- run "$d"

d=$(fixture deep-notice-md)
add_crate "$d" quebec 1.0.0 LICENSE "$MIT_TEXT" third_party/notice.md "bundled notice"
notices_json "$d" quebec:1.0.0:LICENSE
run_case "a lower-case notice.md below the root fails too" 1 --contains "third_party/notice.md" -- run "$d"

d=$(fixture notice-like)
add_crate "$d" romeo 1.0.0 LICENSE "$MIT_TEXT" src/notices.rs "code" NOTICEBOARD "not a NOTICE file"
notices_json "$d" romeo:1.0.0:LICENSE
run_case "names that only start like NOTICE are neither copied nor reported" 0 \
  --contains "1 crates, 0 NOTICE file(s)" -- run "$d"

# --- Check 5: linked crates that the notices leave out -----------------------

d=$(fixture missing-linked)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE
printf '%s\n' "alpha v1.0.0" "windows-sys v0.61.2" "alpha v2.0.0" >"$d/tree.txt"
run_case "a linked crate missing from the notices fails" 1 \
  --contains "windows-sys v0.61.2 is linked into the binary" \
  --contains "alpha v2.0.0 is linked into the binary" --lacks "alpha v1.0.0 is linked" -- run "$d"

# cargo tree's " (*)" for a package printed before, exactly as the CI log of
# the cargo-deny job showed it under CARGO_TERM_COLOR=always: yellow and dim.
# The stub prints tree.txt whatever --color says, so these cases hold even if
# a cargo ignored --color never.
STAR=$' \033[33m\033[2m(*)\033[39m\033[22m'

d=$(fixture tree-coloured)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
add_crate "$d" bravo 2.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE bravo:2.0.0:LICENSE
printf '%s\n' "backend3 v0.1.0 ($d/repo)" "alpha v1.0.0" "bravo v2.0.0 (proc-macro)" "alpha v1.0.0$STAR" \
  "bravo v2.0.0 (proc-macro)$STAR" "migration v0.1.0 ($d/repo/migration)$STAR" >"$d/tree.txt"
run_case "coloured (*) markers and (proc-macro) suffixes leave the crates' names as they are" 0 \
  --lacks "is linked into the binary" --lacks "problem" -- run "$d"

d=$(fixture tree-coloured-missing)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE
printf '%s\n' "alpha v1.0.0$STAR" "windows-sys v0.61.2" "windows-sys v0.61.2$STAR" "zeta v3.0.0$STAR" >"$d/tree.txt"
run_case "a missing crate that cargo tree marks with a coloured (*) is named once, without colour" 1 \
  --contains "windows-sys v0.61.2 is linked into the binary" --contains "zeta v3.0.0 is linked into the binary" \
  --contains "2 problem(s)" --lacks $'\033' -- run "$d"

d=$(fixture tree-other-source)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE
printf '%s\n' "alpha v1.0.0" "gamma v1.0.0 (registry \`example\`)" \
  "delta v0.1.0 (https://example.org/delta.git?rev=1234abcd#1234abcd)$STAR" >"$d/tree.txt"
run_case "a crate from another source than crates.io is third-party code and is checked too" 1 \
  --contains "gamma v1.0.0 is linked into the binary" --contains "delta v0.1.0 is linked into the binary" -- run "$d"

d=$(fixture tree-unreadable)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE
printf '%s\n' "alpha v1.0.0" "[build-dependencies]" "beta v1.0.0 (feature)x" >"$d/tree.txt"
run_case "a cargo tree line of another form fails instead of being misread" 1 \
  --contains "cannot read the line '[build-dependencies]' of cargo tree's output" \
  --contains "cannot read the line 'beta v1.0.0 (feature)x' of cargo tree's output" \
  --contains "teach tree_crates in scripts/crate-notices.sh" --lacks "is linked into the binary" -- run "$d"

# tree_crates on its own: each pair is a line of cargo tree's output and the
# line that tree_crates prints for it ("-" for none), with every form of a
# `{p}` line that cargo 1.99 prints, coloured and not.
tree_pairs=(
  "aho-corasick v1.1.5$STAR" $'crate\taho-corasick v1.1.5'
  "aho-corasick v1.1.5" $'crate\taho-corasick v1.1.5'
  "hybrid-array v0.4.15 (*)" $'crate\thybrid-array v0.4.15'
  "serde_derive v1.0.228 (proc-macro)" $'crate\tserde_derive v1.0.228'
  "async-trait v0.1.92 (proc-macro)$STAR" $'crate\tasync-trait v0.1.92'
  "icu v2.0.0-beta.1+build.7" $'crate\ticu v2.0.0-beta.1+build.7'
  "backend3 v0.1.0 (/src)" $'local\tbackend3 v0.1.0'
  "migration v0.1.0 (/src/migration)$STAR" $'local\tmigration v0.1.0'
  "gamma v1.0.0 (registry \`example\`)" $'crate\tgamma v1.0.0'
  "epsilon v1.0.0 (dir /vendor/epsilon-1.0.0)" $'crate\tepsilon v1.0.0'
  "delta v0.1.0 (https://example.org/delta.git#1234abcd)" $'crate\tdelta v0.1.0'
  "" "-"
  "[dev-dependencies]" $'unreadable\t[dev-dependencies]'
  "alpha v1.0.0 (*) (*)" $'unreadable\talpha v1.0.0 (*) (*)'
  "alpha 1.0.0" $'unreadable\talpha 1.0.0'
)
tree_input="" tree_expected=""
for ((i = 0; i < ${#tree_pairs[@]}; i += 2)); do
  tree_input+="${tree_pairs[i]}"$'\n'
  [[ ${tree_pairs[i + 1]} == - ]] || tree_expected+="${tree_pairs[i + 1]}"$'\n'
done
tree_actual=$(bash -c 'source "$1" && tree_crates' _ "$SCRIPT" <<<"$tree_input" 2>&1) || true
check "tree_crates reads each form of cargo tree's {p} lines" test "$tree_actual" = "${tree_expected%$'\n'}"

# --- Check 6 and cargo-about's messages and failures -------------------------

d=$(fixture no-copyright)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE
rm "$d/sysroot/share/doc/rust/COPYRIGHT-library.html"
run_case "a toolchain without COPYRIGHT-library.html fails" 1 \
  --contains "has no share/doc/rust/COPYRIGHT-library.html" -- run "$d"

d=$(fixture about-fails)
notices_json "$d"
echo "error: failed to satisfy license requirements" >"$d/about-stderr"
echo 1 >"$d/about-exit"
run_case "a cargo-about failure fails with its message and the fix" 1 \
  --contains "failed to satisfy license requirements" --contains "cargo-about failed (see its messages above)" \
  --contains "add the licence to \`accepted\` only if deny.toml allows it" -- run "$d"

d=$(fixture about-warns)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE
printf '2026-11-01T02:31:44.222303427Z \033[33m[WARN]\033[0m a warning about something else\n' >"$d/about-stderr"
run_case "cargo-about's warnings are shown" 0 --contains "  [WARN] a warning about something else" -- run "$d"
check "the warnings are shown once, without colour sequences or time stamps" \
  test "$(grep -c "cargo-about said:" <<<"$LAST_OUTPUT")" -eq 1 -a "$(grep -c $'\033' <<<"$LAST_OUTPUT")" -eq 0 \
  -a "$(grep -c "2026-11-01T" <<<"$LAST_OUTPUT")" -eq 0 -a "$(grep -c "cargo-about also said" <<<"$LAST_OUTPUT")" -eq 0
printf '2026-11-01T02:31:45.000000000Z [WARN] a warning of the template run\n' >"$d/about-stderr-html"
rm -rf "$d/out"
run_case "the template run's own warnings are shown too" 0 \
  --contains $'cargo-about also said, rendering about.hbs:\n  [WARN] a warning of the template run' -- run "$d"
check "the template run's warnings leave out what the JSON run said" \
  test "$(grep -c "a warning about something else" <<<"$LAST_OUTPUT")" -eq 1

d=$(fixture empty-graph)
notices_json "$d"
run_case "an empty crate list fails" 1 --contains "cargo-about listed no crate at all" -- run "$d"

# --- Usage and preconditions -------------------------------------------------

d=$(fixture usage)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE
run_case "--help prints the usage" 0 --contains "Usage: scripts/crate-notices.sh --out DIR" -- run "$d" --help
run_case "--out is required" 2 --contains "--out is required" -- run "$d" --cargo-about cargo-about
run_case "--out needs a value" 2 --contains "--out needs a directory" -- run "$d" --out
run_case "an unknown argument fails" 2 --contains "unknown argument '--bogus'" -- run "$d" --bogus
run_case "a missing cargo-about fails" 2 --contains "ARG CARGO_ABOUT_VERSION" -- \
  run "$d" --out "$d/out-x" --cargo-about "$d/no-such-cargo-about"
mkdir -p "$d/full"
echo stale >"$d/full/crates.html"
run_case "a non-empty output directory fails" 2 --contains "is not empty; remove it first" -- run "$d" --out "$d/full"
check "the stale file is left alone" grep -q stale "$d/full/crates.html"
echo file >"$d/a-file"
run_case "an output path that is a file fails" 2 --contains "exists and is not a directory" -- run "$d" --out "$d/a-file"
mkdir -p "$d/empty"
run_case "an empty output directory is fine" 0 -- run "$d" --out "$d/empty"
run_case "--cargo-about takes a path" 0 -- run "$d" --out "$d/out-path" --cargo-about "$WORK/bin/cargo-about"

d=$(fixture no-about-toml)
rm "$d/repo/about.toml"
run_case "a missing about.toml fails" 2 --contains "about.toml is missing at the repository root" -- run "$d"

d=$(fixture fetch-fails)
echo 101 >"$d/fetch-exit"
run_case "a failing cargo fetch exits 2" 2 --contains "'cargo fetch --locked' failed" -- run "$d"

d=$(fixture tree-fails)
add_crate "$d" alpha 1.0.0 LICENSE "$MIT_TEXT"
notices_json "$d" alpha:1.0.0:LICENSE
echo 101 >"$d/tree-exit"
run_case "a failing cargo tree exits 2" 2 --contains "'cargo tree' failed" -- run "$d"

d=$(fixture bad-json)
echo "not json" >"$d/notices.json"
run_case "unreadable cargo-about JSON exits 2" 2 --contains "could not read cargo-about's JSON output with jq" -- run "$d"

run_case "sourcing the script defines its functions without running it" 0 --lacks "Usage" -- \
  bash -c 'source "$1" && declare -F read_clarifications >/dev/null' _ "$SCRIPT"

# --- The committed files stay in step with the script ------------------------

about_targets=$(yq -p toml -o json '.targets' "$ROOT/about.toml" | jq -c .)
script_target=$(sed -n 's/^readonly RELEASE_TARGET=//p' "$SCRIPT")
check "about.toml's targets is the script's release target" test "$about_targets" = "[\"$script_target\"]"
check "the release target is the Containerfile's default target" test "$script_target" = x86_64-unknown-linux-gnu

about_accepted=$(yq -p toml -o json '.accepted' "$ROOT/about.toml" | jq -c 'sort')
deny_allowed=$(yq -p toml -o json '.licenses.allow' "$ROOT/deny.toml" | jq -c 'sort')
check "about.toml accepts exactly deny.toml's allow-list" test "$about_accepted" = "$deny_allowed"
about_exceptions=$(yq -p toml -o json '.' "$ROOT/about.toml" |
  jq -c '[to_entries[] | select(.value | type == "object" and has("accepted")) | {crate: .key, allow: .value.accepted}] | sort')
deny_exceptions=$(yq -p toml -o json '.licenses.exceptions' "$ROOT/deny.toml" | jq -c '[.[] | {crate, allow}] | sort')
check "about.toml's per-crate licences are deny.toml's exceptions" test "$about_exceptions" = "$deny_exceptions"
check "every clarification checksum is a SHA-256" test -z "$(yq -p toml -o json '.' "$ROOT/about.toml" |
  jq -r '.[] | objects | .clarify? // empty | .files[] | .checksum | select(test("^[0-9a-f]{64}$") | not)')"
toml_clarified=$(yq -p toml -o json '.' "$ROOT/about.toml" |
  jq -r 'to_entries[] | select(.value | type == "object" and has("clarify")) | .key as $k | .value.clarify.files[] | [$k, .path, .checksum] | @tsv' |
  LC_ALL=C sort)
script_clarified=$(bash -c 'source "$1" && read_clarifications "$2" && ((${#problems[@]} == 0)) && printf "%s\n" "${CLARIFIED_FILES[@]}"' _ "$SCRIPT" "$ROOT/about.toml" |
  LC_ALL=C sort)
check "the script reads every clarification of about.toml as a TOML parser does" \
  test -n "$toml_clarified" -a "$toml_clarified" = "$script_clarified"
check "about.toml's clarifications use only the keys the script checks" test -z "$(yq -p toml -o json '.' "$ROOT/about.toml" |
  jq -r '.[] | objects | .clarify? // empty | ((keys - ["license", "files"])[]), (.files[] | keys - ["path", "license", "checksum"] | .[])')"

statement=$(sed -n 's/^readonly IJG_STATEMENT="\(.*\)"$/\1/p' "$SCRIPT")
check "about.hbs states the script's IJG sentence" grep -qF "<p>$statement</p>" "$ROOT/about.hbs"
check "THIRD_PARTY.md states the same IJG sentence" grep -qF "${statement,}" "$ROOT/THIRD_PARTY.md"
check "about.hbs never uses Handlebars' unescaped triple braces" bash -c '! grep -q "{{{" "$1"' _ "$ROOT/about.hbs"

containerfile=$(sed -n '/^FROM .* AS rust$/,$p' "$ROOT/Containerfile")
check "the rust stage runs the script after the release build" \
  bash -c 'grep -q "cargo build --release --locked" <<<"$1" && grep -q "bash scripts/crate-notices.sh --out /src/target/licenses" <<<"$1"' _ "$containerfile"
check "the runtime stage ships the output in /app/licenses/" \
  grep -q "^COPY --from=rust /src/target/licenses/ /app/licenses/$" "$ROOT/Containerfile"
check "the required cargo-deny job runs the script" test "$(yq '.jobs["bans-licenses-sources"].steps[].run // "" | select(test("scripts/crate-notices.sh --out"))' "$ROOT/.github/workflows/cargo-deny.yml" | wc -l)" -eq 1

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
