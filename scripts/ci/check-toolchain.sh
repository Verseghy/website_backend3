#!/usr/bin/env bash
# check-toolchain.sh: keep the Rust and Node.js versions in one place each.
#
# Why this exists: the production image is built only by Pipelines-as-Code
# after a merge (ADR 0022 section 1), so a toolchain mismatch between CI and
# the Containerfile would first show up as a failed build on master. The
# "Toolchain" CI job (a required context, ADR 0022 section 3) runs this script
# on every pull request instead.
#
# Rules:
#   1. rust-toolchain.toml names an exact stable release (MAJOR.MINOR.PATCH,
#      never "stable" or "1.94") at or above 1.94, the MSRV of loco-rs 1.2.0
#      and Sea-ORM 2.0 (ADR 0022 section 3). No legacy `rust-toolchain` file
#      may shadow it.
#   2. Every `ARG RUST_VERSION=<v>` default in the Containerfile equals that
#      channel, and there is at least one (ADR 0022 section 2).
#   3. admin/.node-version holds a bare Node.js major (for example 24), and
#      every `ARG NODE_VERSION=<n>` default in the Containerfile equals it
#      (ADR 0004 section 2).
#   4. Every base image is UBI 9 from registry.access.redhat.com, pinned by
#      digest, and the admin stage's ubi9/nodejs-<major> image matches
#      admin/.node-version (ADR 0022 section 2: the hub's build nodes cannot run
#      a UBI 10 / RHEL 10 userland). `FROM <earlier stage>` is allowed.
# Rule 4 goes beyond the three items in ADR 0022's table for this job; it is
# the static half of the Containerfile rules, which no other check covers
# before the post-merge PAC build.
#
# A toolchain bump changes rust-toolchain.toml and the Containerfile ARG in the
# same PR; a Node.js bump changes admin/.node-version, the ARG and the image
# digest together.
#
# Usage: check-toolchain.sh [--root DIR]   (default: the current directory)
#
# Exit codes: 0 consistent, 1 at least one violation, 2 usage error.
set -euo pipefail

readonly MIN_MAJOR=1
readonly MIN_MINOR=94
readonly UBI9_IMAGE_RE='^registry\.access\.redhat\.com/ubi9/[a-z0-9][a-z0-9._-]*@sha256:[0-9a-f]{64}$'

usage() {
  echo "Usage: check-toolchain.sh [--root DIR]   (default: the current directory)"
}

die_usage() {
  echo "error: $*" >&2
  exit 2
}

# Results shared by the functions below. They are globals rather than
# printed values so that problems recorded inside a function are not lost in
# a command-substitution subshell.
problems=()
RUST_CHANNEL=""
NODE_MAJOR=""
declare -A GLOBAL_ARGS=()

# problem MESSAGE: record one violation.
problem() {
  problems+=("$1")
}

# read_rust_channel ROOT: set RUST_CHANNEL from ROOT/rust-toolchain.toml and
# apply rule 1. Leaves RUST_CHANNEL empty when the file is missing or broken.
read_rust_channel() {
  local root=$1 file="$1/rust-toolchain.toml" channel
  if [[ -e $root/rust-toolchain ]]; then
    problem "a legacy 'rust-toolchain' file exists; rustup would prefer it over rust-toolchain.toml. Delete it."
  fi
  if [[ ! -f $file ]]; then
    problem "rust-toolchain.toml is missing. Add it with [toolchain] channel = \"<exact stable release>\"."
    return
  fi
  if ! channel=$(yq -p toml -o yaml '.toolchain.channel // ""' "$file" 2>&1); then
    problem "rust-toolchain.toml is not valid TOML: ${channel//$'\n'/ }"
    return
  fi
  if [[ -z $channel ]]; then
    problem "rust-toolchain.toml has no [toolchain] channel. Set channel = \"<exact stable release>\"."
    return
  fi
  RUST_CHANNEL=$channel
  if ! [[ $channel =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
    problem "rust-toolchain.toml channel '$channel' is not an exact release. Use MAJOR.MINOR.PATCH, for example the current stable from https://static.rust-lang.org/dist/channel-rust-stable.toml."
    return
  fi
  local major=$((10#${BASH_REMATCH[1]})) minor=$((10#${BASH_REMATCH[2]}))
  if ((major < MIN_MAJOR || (major == MIN_MAJOR && minor < MIN_MINOR))); then
    problem "rust-toolchain.toml channel '$channel' is below $MIN_MAJOR.$MIN_MINOR, the MSRV of loco-rs 1.2.0 and Sea-ORM 2.0."
  fi
}

# read_node_major ROOT: set NODE_MAJOR from ROOT/admin/.node-version.
# Leaves NODE_MAJOR empty when the file is missing or malformed.
read_node_major() {
  local file="$1/admin/.node-version" value
  if [[ ! -f $file ]]; then
    problem "admin/.node-version is missing. Write the Node.js LTS major into it, for example 24."
    return
  fi
  value=$(tr -d '[:space:]' <"$file")
  if ! [[ $value =~ ^[0-9]+$ ]]; then
    problem "admin/.node-version holds '$value'; it must hold the Node.js major only, for example 24."
    return
  fi
  NODE_MAJOR=$value
}

# containerfile_instructions FILE: print the instructions of a Containerfile,
# one per line, with comment lines dropped and backslash continuations joined.
containerfile_instructions() {
  awk '
    /^[[:space:]]*#/ && cont == "" { next }
    {
      line = $0
      if (line ~ /\\[[:space:]]*$/) { sub(/\\[[:space:]]*$/, " ", line); cont = cont line; next }
      print cont line
      cont = ""
    }
    END { if (cont != "") print cont }
  ' "$1"
}

# substitute TEXT: print TEXT with ${NAME} and $NAME expanded from the global
# ARG defaults (the ARGs before the first FROM, the only ones FROM may use).
substitute() {
  local text=$1 name
  for name in "${!GLOBAL_ARGS[@]}"; do
    text=${text//\$\{$name\}/${GLOBAL_ARGS[$name]}}
    text=${text//\$$name/${GLOBAL_ARGS[$name]}}
  done
  printf '%s' "$text"
}

# unquote VALUE: print VALUE without trailing blanks and surrounding quotes.
unquote() {
  local value=$1
  value=${value%"${value##*[![:space:]]}"}
  if [[ $value == \"*\" || $value == \'*\' ]]; then
    value=${value:1:${#value}-2}
  fi
  printf '%s' "$value"
}

# check_base_image IMAGE EXPANDED: apply rule 4 to one non-stage FROM image.
check_base_image() {
  local image=$1 expanded=$2
  if ! [[ $expanded =~ $UBI9_IMAGE_RE ]]; then
    problem "Containerfile: base image '$image' must be a UBI 9 image from registry.access.redhat.com/ubi9/ pinned by digest (@sha256:...). Get the digest with: skopeo inspect --no-tags docker://registry.access.redhat.com/ubi9/<name>:latest | jq -r .Digest"
  fi
  if [[ $expanded =~ /ubi9/nodejs-([0-9]+)@ ]]; then
    NODE_STAGE_FOUND=1
    if [[ -n $NODE_MAJOR && ${BASH_REMATCH[1]} != "$NODE_MAJOR" ]]; then
      problem "Containerfile: the admin stage uses ubi9/nodejs-${BASH_REMATCH[1]}, but admin/.node-version says $NODE_MAJOR. Change both together, with the new image digest."
    fi
  fi
}

# check_containerfile ROOT: apply rules 2-4 to ROOT/Containerfile.
check_containerfile() {
  local file="$1/Containerfile"
  if [[ ! -f $file ]]; then
    problem "Containerfile is missing at the repository root."
    return
  fi

  local -a stages=() rust_defaults=() node_defaults=() words=()
  local seen_from=0 line keyword rest name value image alias expanded stage is_stage i
  NODE_STAGE_FOUND=0
  while IFS= read -r line; do
    keyword="" rest=""
    read -r keyword rest <<<"$line" || true
    case ${keyword^^} in
      ARG)
        # ARG NAME or ARG NAME=VALUE (quotes optional).
        [[ $rest =~ ^([A-Za-z_][A-Za-z0-9_]*)(=(.*))?$ ]] || continue
        name=${BASH_REMATCH[1]}
        [[ -n ${BASH_REMATCH[2]} ]] || continue
        value=$(unquote "${BASH_REMATCH[3]}")
        ((seen_from)) || GLOBAL_ARGS[$name]=$value
        if [[ $name == RUST_VERSION ]]; then rust_defaults+=("$value"); fi
        if [[ $name == NODE_VERSION ]]; then node_defaults+=("$value"); fi
        ;;
      FROM)
        seen_from=1
        read -r -a words <<<"$rest"
        i=0
        while [[ ${words[i]-} == --* ]]; do i=$((i + 1)); done
        image=${words[i]-}
        alias=""
        if [[ ${words[i + 1]-} =~ ^[Aa][Ss]$ ]]; then alias=${words[i + 2]-}; fi
        if [[ -z $image ]]; then
          problem "Containerfile: a FROM line names no image."
        else
          expanded=$(substitute "$image")
          is_stage=0
          for stage in "${stages[@]}"; do
            [[ ${expanded,,} == "$stage" ]] && is_stage=1
          done
          ((is_stage)) || check_base_image "$image" "$expanded"
        fi
        if [[ -n $alias ]]; then stages+=("${alias,,}"); fi
        ;;
    esac
  done < <(containerfile_instructions "$file")

  if ((${#rust_defaults[@]} == 0)); then
    problem "Containerfile has no 'ARG RUST_VERSION=<version>' default. Add one equal to the rust-toolchain.toml channel${RUST_CHANNEL:+ ($RUST_CHANNEL)}."
  fi
  for value in "${rust_defaults[@]}"; do
    if [[ -n $RUST_CHANNEL && $value != "$RUST_CHANNEL" ]]; then
      problem "Containerfile ARG RUST_VERSION is '$value', but rust-toolchain.toml says '$RUST_CHANNEL'. Change both in the same PR."
    fi
  done
  if ((${#node_defaults[@]} == 0)); then
    problem "Containerfile has no 'ARG NODE_VERSION=<major>' default. Add one equal to admin/.node-version${NODE_MAJOR:+ ($NODE_MAJOR)}."
  fi
  for value in "${node_defaults[@]}"; do
    if [[ -n $NODE_MAJOR && $value != "$NODE_MAJOR" ]]; then
      problem "Containerfile ARG NODE_VERSION is '$value', but admin/.node-version says '$NODE_MAJOR'. Change both in the same PR."
    fi
  done
  if ((!NODE_STAGE_FOUND)); then
    problem "Containerfile has no admin stage based on registry.access.redhat.com/ubi9/nodejs-<major>@sha256:... (ADR 0004 section 2)."
  fi
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
  command -v yq >/dev/null || die_usage "yq (mikefarah, v4) is required but not installed: https://github.com/mikefarah/yq"
  local version
  version=$(yq --version 2>&1 || true)
  [[ $version == *mikefarah* ]] || die_usage "this script needs mikefarah yq v4; found: $version"

  read_rust_channel "$root"
  read_node_major "$root"
  check_containerfile "$root"

  if ((${#problems[@]} == 0)); then
    echo "Toolchain OK: Rust $RUST_CHANNEL and Node.js $NODE_MAJOR agree across rust-toolchain.toml, admin/.node-version and the Containerfile."
    return 0
  fi
  {
    echo "Toolchain check failed (ADR 0004 section 2, ADR 0022 sections 2 and 3):"
    local p
    for p in "${problems[@]}"; do
      echo "  - $p"
    done
  } >&2
  return 1
}

main "$@"
