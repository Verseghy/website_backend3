#!/usr/bin/env bash
# crate-notices.sh: write the licence notices of the third-party code in the
# backend3 binary, and fail on every gap it can see (ADR 0042; research:
# DECISIONS R23, D115).
#
# Why it exists
#   The public image ships /app/backend3-cli, which links about 417
#   third-party crates and Rust's standard library. Their licences (MIT,
#   Apache-2.0, the BSD licences, MPL-2.0, IJG, ...) ask that the copyright
#   and licence notices, and an Apache-2.0 crate's NOTICE file, travel with
#   the binary. The maintainer chose to generate those notices in the image
#   build and ship them in the image (D115). The Containerfile's rust stage
#   runs this script after the release build, and the runtime stage copies
#   its output to /app/licenses/. The required `cargo-deny check bans licenses
#   sources` job runs the same script on every pull request, so a gap fails
#   before it reaches master.
#
# What it writes into OUT
#   crates.html             cargo-about's rendering of about.hbs: every crate
#                           of the binary's dependency graph for the release
#                           target, with its licence and licence text.
#   notices/<crate>-<version>/<file>
#                           the NOTICE files at the root of those crates.
#   COPYRIGHT-library.html  the notices of the Rust standard library, from the
#                           toolchain that builds the binary.
#   README.txt              an index of the directory, with the IJG statement.
#
# The checks (each failure says what to change)
#   1. cargo-about --fail: every crate's licence expression is read and
#      satisfied by about.toml's `accepted` list.
#   2. Every clarification in about.toml holds. It names a crate that the
#      notices list (a misspelt name would never apply), and for every listed
#      version of that crate each file it names exists and has the SHA-256
#      that about.toml gives. The one exception is a file that holds no
#      licence text in that version (empty, or a flattened link, below): that
#      version ships no licence text there. cargo-about only warns when a
#      clarification fails and falls back to scanning the crate, so without
#      this check a changed file of bundled code, such as libwebp's
#      vendor/COPYING, would drop its copyright notice silently. Every other
#      clarification that cargo-about reports as failed fails here too. The
#      message prints the file's current SHA-256.
#   3. The notices carry every licence file that they should: each file that
#      a clarification names, where it holds a licence text, and, for a crate
#      that gets the standard text of any of its licences, each licence file
#      at its root or in a root LICENSES/ directory. A file counts as carried
#      when one of the crate's licence texts in the notices is exactly its
#      content. cargo-about prints the standard text, which names no
#      copyright holder, when its detector does not recognise a file, so the
#      crate's own notice would be lost; a clarification in about.toml fixes
#      it, and the message prints the file's SHA-256. An empty file, and a
#      one-line file that holds only a relative path ("../LICENSE-MIT", a
#      symbolic link that packaging flattened), hold no licence text and do
#      not count. A crate with no licence text at all fails as well.
#      The check cannot tell which licence a file holds, so a crate that gets
#      the standard text of one licence and ships a file of a licence that
#      its expression does not use fails too; a clarification that names its
#      files settles that case.
#   4. No NOTICE file lies below a crate's root. The script copies the ones
#      at the root (where Apache-2.0 section 4(d) expects them); one deeper
#      down belongs to bundled code and needs a reviewed rule here first.
#   5. Every third-party crate that `cargo tree` links into the binary
#      (normal edges, the release target, no procedural macros) is in the
#      notices, so about.toml's graph filters cannot drop a linked crate.
#      cargo-about also lists the procedural macros, which the binary does not
#      link; listing more costs nothing.
#   6. The toolchain carries COPYRIGHT-library.html.
#
#   Licence files of bundled code below a crate's root are checked only where
#   a clarification names them; a new -sys crate is reviewed for bundled code
#   when it arrives (recipe 5 of the project skill).
#
# Usage
#   scripts/crate-notices.sh --out DIR [--cargo-about PATH]
#
#   --out DIR           the directory to write; it must not exist yet or be
#                       empty, so no file of an earlier run survives.
#   --cargo-about PATH  the cargo-about binary (default: cargo-about on PATH).
#                       The Containerfile pins its version and SHA-256; CI
#                       installs the same version (check-toolchain.sh rule 5).
#
#   It runs `cargo fetch --locked` first, which downloads the crates of every
#   platform, and then cargo-about and `cargo tree` with --frozen, so neither
#   reaches the network and both read exactly Cargo.lock.
#
#   Sourcing the script (`source scripts/crate-notices.sh`) defines its
#   functions and sets its shell options without running it;
#   scripts/tests/test-crate-notices.sh does so to compare
#   read_clarifications with a TOML parser.
#
# Exit codes
#   0  written, every check passed;
#   1  a check failed, or cargo-about failed;
#   2  a usage error, a missing tool or file, or a failing cargo, rustc or jq
#      command.

set -euo pipefail

# The target the Containerfile builds (ADR 0022 section 2). about.toml's
# `targets` says the same; scripts/tests/test-crate-notices.sh checks it.
readonly RELEASE_TARGET=x86_64-unknown-linux-gnu

# The one sentence the IJG licence asks the documentation of a binary to
# state (THIRD_PARTY.md, "Notices that dependencies require").
readonly IJG_STATEMENT="This software is based in part on the work of the Independent JPEG Group."

usage() {
  cat <<'EOF'
Usage: scripts/crate-notices.sh --out DIR [--cargo-about PATH]

Writes the licence notices of the crates in the backend3 binary into DIR
(crates.html, notices/, COPYRIGHT-library.html, README.txt) and fails on a
gap (ADR 0042).

  --out DIR           the output directory; must be missing or empty
  --cargo-about PATH  the cargo-about binary (default: cargo-about on PATH)
  -h, --help          show this help
EOF
}

# Prints an error and exits with code 2.
#   $* - the message: what is wrong and how to fix it.
die() {
  printf 'crate-notices.sh: %s\n' "$*" >&2
  exit 2
}

# Prints a usage error and the usage, and exits with code 2.
#   $* - what was wrong with the arguments.
die_usage() {
  printf 'crate-notices.sh: %s\n\n' "$*" >&2
  usage >&2
  exit 2
}

problems=()

# Records one failed check; main prints them all before it exits with 1.
#   $1 - the message: the crate or file, what is wrong, and the fix.
problem() {
  problems+=("$1")
}

# The clarifications that read_clarifications found in about.toml:
# CLARIFIED maps a crate name to the number of files its clarification
# names, and CLARIFIED_FILES holds one "crate<TAB>path<TAB>sha256" entry per
# file, in the order of about.toml.
declare -A CLARIFIED=()
CLARIFIED_FILES=()

# Succeeds when a file name is a licence file's: LICENSE, LICENCE, COPYING,
# COPYRIGHT or UNLICENSE, in any case, alone or followed by `-`, `.` or `_`
# and a suffix (LICENSE-MIT, LICENSE_APACHE, LICENSE.txt).
#   $1 - the file name, without its directory.
is_licence_name() {
  local re='^(licen[cs]e|copying|copyright|unlicen[cs]e)([-._].*)?$'
  [[ ${1,,} =~ $re ]]
}

# Succeeds when a file name is a NOTICE file's: NOTICE in any case, alone or
# followed by `-`, `.` or `_` and a suffix (NOTICE.txt, NOTICE.md).
#   $1 - the file name, without its directory.
is_notice_name() {
  local re='^notice([-._].*)?$'
  [[ ${1,,} =~ $re ]]
}

# Succeeds when a file holds nothing but a relative path upward, such as
# "../LICENSE-MIT": a symbolic link that `cargo package` stored as text. Such
# a file is no licence text.
#   $1 - the path of the file.
is_flattened_link() {
  local file=$1 size content
  size=$(wc -c <"$file")
  ((size > 0 && size <= 256)) || return 1
  content=$(<"$file")
  [[ $content != *$'\n'* && $content =~ ^\.\./[^[:space:]]+$ ]]
}

# Succeeds when a file holds a licence text: it is not empty and not a
# flattened link (is_flattened_link).
#   $1 - the path of the file, which must exist.
holds_text() {
  [[ -s $1 ]] && ! is_flattened_link "$1"
}

# Prints the SHA-256 of a file's bytes, in lower-case hex: the checksum that
# about.toml's clarifications give for a whole file.
#   $1 - the path of the file.
file_sha256() {
  local sum
  sum=$(sha256sum <"$1")
  printf '%s\n' "${sum%% *}"
}

# Prints, one per line, the licence files of a crate that hold a licence
# text: those at its root, and those in a root LICENSES/ directory (the REUSE
# layout), leaving out empty files and flattened links, which hold none.
#   $1 - the crate's directory.
crate_licence_files() {
  local dir=$1 path name
  for path in "$dir"/* "$dir"/LICENSES/*; do
    [[ -f $path ]] || continue
    name=${path##*/}
    if [[ $path == "$dir"/LICENSES/* ]] || is_licence_name "$name"; then
      if holds_text "$path"; then
        printf '%s\n' "$path"
      fi
    fi
  done
}

# Reads the clarifications of about.toml into CLARIFIED and CLARIFIED_FILES,
# for check 2. about.toml writes each one in the form that cargo-about
# documents: a [<crate>.clarify] table that sets `license`, and one
# [[<crate>.clarify.files]] table per file that sets `path`, `checksum` and
# optionally `license`, each value in double quotes. That is all this reader
# understands, so anything else that mentions clarify (an inline `files`
# list, a `git` entry, a `start` or `end` key that checksums only part of a
# file, a dotted key) is a problem rather than a clarification that goes
# unchecked. scripts/tests/test-crate-notices.sh compares the result for the
# committed about.toml with a TOML parser's.
#   $1 - the path of about.toml.
read_clarifications() {
  local file=$1 raw line section="" crate="" path="" sum="" start=0 n=0
  local re_files='^\[\[[[:space:]]*([A-Za-z0-9_-]+)\.clarify\.files[[:space:]]*\]\]$'
  local re_table='^\[[[:space:]]*([A-Za-z0-9_-]+)\.clarify[[:space:]]*\]$'
  local re_value='^([A-Za-z0-9_-]+)[[:space:]]*=[[:space:]]*"([^"\\]*)"$'
  # A comment starts at a # outside double quotes.
  local re_comment='^(([^#"]|"[^"]*")*)#.*$'
  local where="about.toml line"

  # Ends the [[<crate>.clarify.files]] table being read, if any, and records
  # its file.
  end_files_table() {
    [[ $section == files ]] || return 0
    if [[ -z $path || -z $sum ]]; then
      problem "$where $start: the [[$crate.clarify.files]] table needs both \`path\` and \`checksum\`."
    elif [[ $path == /* || /$path/ == */../* ]]; then
      problem "$where $start: the clarification of $crate names '$path'; name a file inside the crate, relative to its root."
    elif ! [[ $sum =~ ^[0-9a-f]{64}$ ]]; then
      problem "$where $start: the checksum of $crate's '$path' is '$sum', which is not a SHA-256 (64 lower-case hex digits)."
    else
      CLARIFIED_FILES+=("$crate"$'\t'"$path"$'\t'"$sum")
      CLARIFIED[$crate]=$((${CLARIFIED[$crate]-0} + 1))
    fi
  }

  while IFS= read -r raw || [[ -n $raw ]]; do
    n=$((n + 1))
    line=$raw
    [[ $line =~ $re_comment ]] && line=${BASH_REMATCH[1]}
    line=${line#"${line%%[![:space:]]*}"}
    line=${line%"${line##*[![:space:]]}"}
    [[ -n $line ]] || continue
    if [[ $line == \[* ]]; then
      end_files_table
      path="" sum="" start=$n
      if [[ $line =~ $re_files ]]; then
        section=files crate=${BASH_REMATCH[1]}
        [[ -n ${CLARIFIED[$crate]+set} ]] ||
          problem "$where $n: [[$crate.clarify.files]] comes before its [$crate.clarify] table, which sets the crate's licence; write that table first."
      elif [[ $line =~ $re_table ]]; then
        section=clarify crate=${BASH_REMATCH[1]}
        CLARIFIED[$crate]=${CLARIFIED[$crate]-0}
      else
        section=other
        [[ $line != *clarify* ]] ||
          problem "$where $n: '$line' is a clarification table that scripts/crate-notices.sh cannot read; write [<crate>.clarify] and [[<crate>.clarify.files]] tables."
      fi
      continue
    fi
    case $section in
      files)
        if [[ $line =~ $re_value ]]; then
          case ${BASH_REMATCH[1]} in
            path) path=${BASH_REMATCH[2]} ;;
            checksum) sum=${BASH_REMATCH[2]} ;;
            license) ;;
            *) problem "$where $n: [[$crate.clarify.files]] sets '${BASH_REMATCH[1]}'; scripts/crate-notices.sh compares whole files and reads only path, license and checksum, so clarify the whole file." ;;
          esac
        else
          problem "$where $n: cannot read '$line' in [[$crate.clarify.files]]; write key = \"value\"."
        fi
        ;;
      clarify)
        if ! [[ $line =~ $re_value && ${BASH_REMATCH[1]} == license ]]; then
          problem "$where $n: [$crate.clarify] sets only \`license\` here; name each file in a [[$crate.clarify.files]] table (git entries need the network, which the build does not use)."
        fi
        ;;
      *)
        [[ $line != *clarify* ]] ||
          problem "$where $n: '$line' is a clarification that scripts/crate-notices.sh cannot read; write [<crate>.clarify] and [[<crate>.clarify.files]] tables."
        ;;
    esac
  done <"$file"
  end_files_table
  for crate in "${!CLARIFIED[@]}"; do
    ((CLARIFIED[$crate] > 0)) ||
      problem "about.toml: the clarification of $crate names no file; add a [[$crate.clarify.files]] table with the file that holds its licence text."
  done
}

# Prints its input without ANSI colour sequences: cargo-about's logger colours
# its level names even with --color never, which only governs the
# diagnostics.
strip_colour() {
  sed $'s/\x1b\[[0-9;]*m//g'
}

# Prints cargo-about's log without colour sequences and without the time
# stamp that starts each line, so that the logs of the two runs compare
# line by line.
normalise_log() {
  strip_colour | sed -E 's/^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z[[:space:]]+//'
}

# Runs cargo-about generate with the repository's configuration and the
# release target. Exits the script with 1 when cargo-about fails, after its
# messages.
#   $1 - the log file for cargo-about's messages;
#   $* - the remaining arguments: the output options (--format json, or the
#        template) and the output file.
run_cargo_about() {
  local log=$1
  shift
  if ! "$cargo_about" --color never generate \
    --manifest-path "$repo_root/Cargo.toml" --config "$repo_root/about.toml" \
    --frozen --fail --target "$RELEASE_TARGET" "$@" 2>"$log"; then
    normalise_log <"$log" >&2
    printf 'crate-notices.sh: cargo-about failed (see its messages above). If it names a licence that about.toml does not accept, add the licence to `accepted` only if deny.toml allows it for that crate (ADR 0022 section 4, ADR 0027).\n' >&2
    exit 1
  fi
}

# Prints cargo-about's messages: those of the JSON run, then those of the
# template run that the JSON run did not print (the two runs read the same
# crates, so they usually say the same).
#   $1 - the JSON run's log; $2 - the template run's log.
show_cargo_about_messages() {
  local json_log=$1 html_log=$2 extra
  if [[ -s $json_log ]]; then
    printf 'cargo-about said:\n' >&2
    normalise_log <"$json_log" | sed 's/^/  /' >&2
  fi
  extra=$(grep -vxF -f <(normalise_log <"$json_log") < <(normalise_log <"$html_log") || true)
  if [[ -n $extra ]]; then
    printf 'cargo-about also said, rendering about.hbs:\n' >&2
    sed 's/^/  /' <<<"$extra" >&2
  fi
}

# Prints "crate<TAB>version<TAB>reason" for every clarification that
# cargo-about reports as failed in its logs (reason may be empty).
#   $* - the log files.
failed_clarifications() {
  local re='failed to validate all files specified in clarification for crate ([^[:space:]]+) ([^[:space:]:]+)(: (.*))?$'
  local line
  cat "$@" | normalise_log | while IFS= read -r line; do
    if [[ $line =~ $re ]]; then
      printf '%s\t%s\t%s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "${BASH_REMATCH[4]}"
    fi
  done | LC_ALL=C sort -u
}

# Reads cargo-about's JSON output and prints one tab-separated line per
# listed crate: name, version, directory, the number of licence texts the
# notices give it, and how many of those are standard texts (no file of the
# crate behind them).
#   $1 - the JSON file of `cargo-about generate --format json`.
crate_rows() {
  jq -r '
    (.licenses | map({standard: (.source_path == null), ids: [.used_by[].crate.id]})) as $texts
    | .crates[]
    | .package as $p
    | [$texts[] | select(any(.ids[]; . == $p.id))] as $mine
    | [$p.name, $p.version, ($p.manifest_path | sub("/Cargo.toml$"; "")),
       ($mine | length), ([$mine[] | select(.standard)] | length)]
    | @tsv' "$1"
}

# Prints one tab-separated line per licence text in cargo-about's JSON output
# that comes from a file: the text in base64, then each crate that uses it as
# "name vVERSION". cargo-about prints a text once for all crates whose files
# hold the same text, so check 3 compares contents, not paths.
#   $1 - the JSON file of `cargo-about generate --format json`.
own_texts() {
  jq -r '.licenses[] | select(.source_path != null)
    | [(.text | @base64), (.used_by[] | "\(.crate.name) v\(.crate.version)")] | @tsv' "$1"
}

# Prints "name vVERSION" for every third-party crate that `cargo tree` links
# into the binary for the release target, sorted and unique: normal edges
# only and no procedural macros, without the workspace's own crates (whose
# lines carry their path in parentheses).
linked_crates() {
  cargo tree --frozen --manifest-path "$repo_root/Cargo.toml" \
    -e normal,no-proc-macro --target "$RELEASE_TARGET" --prefix none --format '{p}' |
    sed -e 's/ (\*)$//' -e '/ (/d' | LC_ALL=C sort -u
}

# Writes README.txt, the index of the output directory.
#   $1 - the output directory; $2 - the number of crates listed;
#   $3 - the cargo-about version line; $4 - the rustc version line;
#   $5... - the NOTICE files copied, relative to the output directory.
write_readme() {
  local out=$1 count=$2 about_version=$3 rustc_version=$4
  shift 4
  {
    printf '%s\n' \
      "Licence notices of the third-party code in /app/backend3-cli" \
      "=============================================================" \
      "" \
      "/app/backend3-cli, the program of this image, is website_backend3," \
      "licensed AGPL-3.0-only. Its source is" \
      "https://github.com/Verseghy/website_backend3, at the commit that the image's" \
      "org.opencontainers.image.revision label names. It is built with Rust and" \
      "links third-party crates and the Rust standard library. This directory" \
      "holds their notices (ADR 0042 of that repository):" \
      "" \
      "crates.html" \
      "    The $count third-party crates of the binary's dependency graph for" \
      "    $RELEASE_TARGET, without build scripts' and tests'" \
      "    dependencies, with their versions, licences and licence texts. The" \
      "    procedural macros that run while the binary compiles are listed too." \
      "    Written by $about_version from Cargo.lock when the image was built." \
      "    The source of each crate is published on crates.io at the version" \
      "    listed: https://crates.io/crates/<crate>/<version>." \
      ""
    if (($# > 0)); then
      printf '%s\n' \
        "notices/" \
        "    The NOTICE files that these crates ship, which the Apache License 2.0" \
        "    (section 4(d)) asks to pass on:"
      printf '      %s\n' "$@"
    else
      printf '%s\n' "NOTICE files" "    None of these crates ships a NOTICE file, so there is no notices/."
    fi
    printf '%s\n' \
      "" \
      "COPYRIGHT-library.html" \
      "    The copyright notices of the Rust standard library and its" \
      "    dependencies, from the toolchain that built the binary ($rustc_version)." \
      "" \
      "The Independent JPEG Group" \
      "    $IJG_STATEMENT" \
      "" \
      "The npm packages bundled into the admin UI, which this image serves at" \
      "/admin, are listed with their licence texts in /app/admin/dist/licenses.txt."
  } >"$out/README.txt"
}

main() {
  local out="" cargo_about=cargo-about
  while (($# > 0)); do
    case $1 in
      --out)
        (($# >= 2)) || die_usage "--out needs a directory, for example --out target/licenses"
        out=$2
        shift 2
        ;;
      --cargo-about)
        (($# >= 2)) || die_usage "--cargo-about needs the path of the cargo-about binary"
        cargo_about=$2
        shift 2
        ;;
      -h | --help)
        usage
        return 0
        ;;
      *) die_usage "unknown argument '$1'" ;;
    esac
  done
  [[ -n $out ]] || die_usage "--out is required."

  repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
  local tool
  for tool in cargo rustc jq sha256sum base64; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is not on PATH; install it and retry (cargo and rustc come from rust-toolchain.toml)."
  done
  command -v "$cargo_about" >/dev/null 2>&1 ||
    die "cargo-about ('$cargo_about') is not on PATH; install the version that the Containerfile pins (ARG CARGO_ABOUT_VERSION), or pass --cargo-about PATH."
  local file
  for file in Cargo.toml Cargo.lock about.toml about.hbs; do
    [[ -f $repo_root/$file ]] || die "$file is missing at the repository root ($repo_root)."
  done
  if [[ -e $out && ! -d $out ]]; then
    die "--out '$out' exists and is not a directory."
  fi
  if [[ -d $out && -n $(ls -A "$out") ]]; then
    die "--out '$out' is not empty; remove it first, so that no file of an earlier run is shipped."
  fi
  mkdir -p "$out"
  out=$(cd "$out" && pwd)

  local work
  work=$(mktemp -d)
  # shellcheck disable=SC2064 # expand $work now: it is local to main.
  trap "rm -rf '$work'" EXIT

  # cargo-about and cargo tree run with --frozen, so every crate of every
  # platform must be on disk first: cargo metadata reads all of them even
  # when about.toml's target leaves them out.
  cargo fetch --locked --manifest-path "$repo_root/Cargo.toml" >&2 ||
    die "'cargo fetch --locked' failed; if Cargo.lock is out of date, update it and commit it."

  # Two runs: the JSON feeds the checks below, the template writes the file
  # that ships. Both read the same Cargo.lock, configuration and target.
  run_cargo_about "$work/about-json.log" --format json --output-file "$work/notices.json"
  run_cargo_about "$work/about-html.log" --output-file "$out/crates.html" "$repo_root/about.hbs"
  show_cargo_about_messages "$work/about-json.log" "$work/about-html.log"

  local rows
  rows=$(crate_rows "$work/notices.json") || die "could not read cargo-about's JSON output with jq."

  # The listed crates, by row: rows_key ("name vVERSION"), rows_name,
  # rows_version, rows_dir, rows_texts (licence texts) and rows_standard
  # (standard texts among them); versions_of maps a name to its row numbers.
  local name version dir texts standard count=0 i
  local -a rows_key=() rows_name=() rows_version=() rows_dir=() rows_texts=() rows_standard=()
  local -A versions_of=()
  while IFS=$'\t' read -r name version dir texts standard; do
    [[ -n $name ]] || continue
    rows_key+=("$name v$version")
    rows_name+=("$name")
    rows_version+=("$version")
    rows_dir+=("$dir")
    rows_texts+=("$texts")
    rows_standard+=("$standard")
    versions_of[$name]+=" $count"
    count=$((count + 1))
  done <<<"$rows"
  ((count > 0)) || problem "cargo-about listed no crate at all; check about.toml and the manifest."
  local -A listed=()
  for ((i = 0; i < count; i++)); do listed[${rows_key[i]}]=1; done

  # Check 2: every clarification holds for every listed version of its
  # crate. explained marks the crate versions whose failed clarification is
  # accounted for (a file that holds no text, or a problem already
  # reported); reported marks "key|path" pairs that check 3 need not repeat.
  read_clarifications "$repo_root/about.toml"
  local entry rel sum actual path key reason
  local -A explained=() reported=() unlisted=()
  for entry in "${CLARIFIED_FILES[@]}"; do
    IFS=$'\t' read -r name rel sum <<<"$entry"
    if [[ -z ${versions_of[$name]-} ]]; then
      [[ -n ${unlisted[$name]-} ]] ||
        problem "about.toml clarifies $name, which the notices do not list, so the clarification never applies. Correct the crate's name, or remove the clarification if the crate left the graph."
      unlisted[$name]=1
      continue
    fi
    for i in ${versions_of[$name]}; do
      key=${rows_key[i]} path="${rows_dir[i]}/$rel"
      if [[ ! -f $path ]]; then
        problem "${rows_name[i]} ${rows_version[i]} has no file $rel, which about.toml's clarification of $name names, so cargo-about ignores the clarification for this version. Find the file that now holds its licence text, read it, and update the clarification."
        explained[$key]=1
      elif ! holds_text "$path"; then
        # This version ships no licence text there (the chrono-tz 0.9.0
        # case): cargo-about rejects the clarification for it and gives the
        # standard text, as for a crate without a licence file. Check 3
        # still fails if another file the clarification names holds text.
        explained[$key]=1
      else
        actual=$(file_sha256 "$path")
        if [[ $actual != "$sum" ]]; then
          problem "${rows_name[i]} ${rows_version[i]}: $rel has changed, so cargo-about ignores about.toml's clarification of $name for this version and its notice can be lost. Read the file and, if it still holds the licence text, renew its checksum in about.toml to $actual (about.toml has $sum)."
          explained[$key]=1 reported["$key|$rel"]=1
        fi
      fi
    done
  done
  local failed
  failed=$(failed_clarifications "$work/about-json.log" "$work/about-html.log")
  while IFS=$'\t' read -r name version reason; do
    [[ -n $name && -z ${explained["$name v$version"]-} ]] || continue
    problem "cargo-about could not apply about.toml's clarification of $name $version${reason:+ ($reason)}. Fix the clarification so that it applies to every listed version of the crate."
  done <<<"$failed"

  # The crates' own licence texts, by content: own_sums maps a crate's key to
  # the SHA-256 of each text that the notices carry for it from a file.
  local b64 users user
  local -a users_list=()
  local -A own_sums=()
  own_texts "$work/notices.json" >"$work/own-texts.tsv" || die "could not read cargo-about's JSON output with jq."
  while IFS=$'\t' read -r b64 users; do
    [[ -n $b64 ]] || continue
    sum=$(base64 -d <<<"$b64" | sha256sum)
    sum=${sum%% *}
    IFS=$'\t' read -r -a users_list <<<"$users"
    for user in "${users_list[@]}"; do own_sums[$user]+=" $sum"; done
  done <"$work/own-texts.tsv"

  # Checks 3 and 4, and the NOTICE files to copy.
  local -a notices=()
  local -A required=()
  for ((i = 0; i < count; i++)); do
    name=${rows_name[i]} version=${rows_version[i]} dir=${rows_dir[i]} key=${rows_key[i]}
    if ((rows_texts[i] == 0)); then
      problem "$name $version: the notices hold no licence text for it. Clarify its licence in about.toml with the file that holds the text."
    fi
    # The files that the notices must carry: `clarified` wins over `root`
    # for a file that is both, because its message is the more precise.
    required=()
    if ((rows_standard[i] > 0)); then
      while IFS= read -r path; do
        [[ -n $path ]] && required[${path#"$dir"/}]=root
      done < <(crate_licence_files "$dir")
    fi
    if [[ -n ${CLARIFIED[$name]-} ]]; then
      for entry in "${CLARIFIED_FILES[@]}"; do
        IFS=$'\t' read -r _ rel _ <<<"$entry"
        [[ $entry == "$name"$'\t'* && -f $dir/$rel ]] && holds_text "$dir/$rel" && required[$rel]=clarified
      done
    fi
    # In name order, so the messages come out the same in every run.
    while IFS= read -r rel; do
      [[ -n $rel && -z ${reported["$key|$rel"]-} ]] || continue
      actual=$(file_sha256 "$dir/$rel")
      [[ ${own_sums[$key]-} != *" $actual"* ]] || continue
      if [[ ${required[$rel]} == clarified ]]; then
        problem "$name $version: the notices do not carry $rel, which about.toml's clarification of $name names: cargo-about did not apply the clarification to this version (see its messages above), and the file's notice is lost. Make the clarification fit every listed version, for example by naming only files that hold a licence text in each."
      else
        problem "$name $version gets the standard text of a licence, which names no copyright holder, although it ships $rel, which the notices do not carry: cargo-about's detector does not recognise the file. Read the file and name it in the [$name.clarify] entry of about.toml with its SHA-256, $actual."
      fi
    done < <(printf '%s\n' "${!required[@]}" | LC_ALL=C sort)
    for path in "$dir"/*; do
      [[ -f $path ]] && is_notice_name "${path##*/}" || continue
      rel="notices/$name-$version/${path##*/}"
      mkdir -p "$out/notices/$name-$version"
      cp "$path" "$out/$rel"
      notices+=("$rel")
    done
    # find only narrows the candidates down (thousands of files per crate);
    # is_notice_name decides, as it does at the root.
    while IFS= read -r -d '' path; do
      is_notice_name "${path##*/}" || continue
      problem "$name $version has a NOTICE file below its root, ${path#"$dir"/}, which the notices do not carry. Check whether the binary holds the code it covers, and if so teach scripts/crate-notices.sh to copy it (ADR 0042)."
    done < <(find "$dir" -mindepth 2 -type f -iname 'notice*' -print0 | LC_ALL=C sort -z)
  done

  # Check 5.
  local linked
  linked=$(linked_crates) || die "'cargo tree' failed; see the cargo error above."
  while IFS= read -r name; do
    [[ -n $name && -z ${listed[$name]-} ]] || continue
    problem "$name is linked into the binary (cargo tree -e normal,no-proc-macro --target $RELEASE_TARGET) but the notices leave it out; check about.toml's targets and ignore-* settings."
  done <<<"$linked"

  # Check 6.
  local sysroot copyright
  sysroot=$(rustc --print sysroot) || die "'rustc --print sysroot' failed."
  copyright="$sysroot/share/doc/rust/COPYRIGHT-library.html"
  if [[ -f $copyright ]]; then
    cp "$copyright" "$out/COPYRIGHT-library.html"
  else
    problem "the Rust toolchain at $sysroot has no share/doc/rust/COPYRIGHT-library.html, the standard library's notices; install the toolchain with rustup, whose rustc component carries the file."
  fi

  local about_version rustc_version
  about_version=$("$cargo_about" --version)
  rustc_version=$(rustc --version)
  write_readme "$out" "$count" "$about_version" "$rustc_version" "${notices[@]}"
  # The runtime image runs as an arbitrary UID (ADR 0023 section 8), and the
  # files keep the build's umask otherwise: cp and cargo-about create them
  # with it.
  chmod -R a+rX "$out"

  if ((${#problems[@]} > 0)); then
    {
      echo "Crate licence notices: ${#problems[@]} problem(s) (ADR 0042):"
      local p
      for p in "${problems[@]}"; do
        printf '  - %s\n' "$p"
      done
    } >&2
    return 1
  fi
  echo "Crate licence notices written to $out: $count crates, ${#notices[@]} NOTICE file(s), the standard library's notices and README.txt."
}

# Run only when executed, so that the tests can source the functions.
if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  main "$@"
fi
