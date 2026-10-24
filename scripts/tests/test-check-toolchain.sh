#!/usr/bin/env bash
# Tests for scripts/ci/check-toolchain.sh (ADR 0004 section 2, ADR 0022
# sections 2 and 3).
#
# Builds fixture repositories (rust-toolchain.toml, Containerfile,
# admin/.node-version) in a temporary directory: a consistent one, and one
# per broken rule. Run from the repository root:
#   bash scripts/tests/test-check-toolchain.sh
# Needs mikefarah yq v4; no network access.
# Single-quoted fixtures and sed/jq programs hold a literal `$` on purpose.
# shellcheck disable=SC2016
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT="$ROOT/scripts/ci/check-toolchain.sh"

tests_run=0
failures=0

# run_case NAME EXPECTED_EXIT [--contains TEXT] -- COMMAND...
# Runs COMMAND, compares its exit code and, optionally, looks for TEXT in its
# combined output. Prints one TAP-style line per case.
run_case() {
  local name=$1 expected=$2 needle=""
  shift 2
  if [[ ${1-} == --contains ]]; then
    needle=$2
    shift 2
  fi
  [[ ${1-} == -- ]] && shift
  local output status=0
  output=$("$@" 2>&1) || status=$?
  tests_run=$((tests_run + 1))
  if [[ $status -ne $expected ]]; then
    failures=$((failures + 1))
    printf 'not ok %d - %s (exit %d, expected %d)\n%s\n' "$tests_run" "$name" "$status" "$expected" "$output" | sed '2,$s/^/    # /'
  elif [[ -n $needle && $output != *"$needle"* ]]; then
    failures=$((failures + 1))
    printf 'not ok %d - %s (output lacks "%s")\n%s\n' "$tests_run" "$name" "$needle" "$output" | sed '2,$s/^/    # /'
  else
    printf 'ok %d - %s\n' "$tests_run" "$name"
  fi
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

D1=sha256:1111111111111111111111111111111111111111111111111111111111111111
D2=sha256:2222222222222222222222222222222222222222222222222222222222222222
D3=sha256:3333333333333333333333333333333333333333333333333333333333333333

# A consistent Containerfile in the shape of the real one.
GOOD_CONTAINERFILE="# comment lines are ignored: ARG RUST_VERSION=0.0.0
ARG NODE_VERSION=24
ARG RUST_VERSION=1.99.1
ARG SOURCE_COMMIT=\"\"

FROM registry.access.redhat.com/ubi9/nodejs-\${NODE_VERSION}@$D1 AS admin
ARG NODE_VERSION
RUN npm ci \\
 && npm run build

FROM registry.access.redhat.com/ubi9/ubi@$D2 AS rust
ARG RUST_VERSION
RUN echo \"\$RUST_VERSION\"

FROM registry.access.redhat.com/ubi9/ubi-micro@$D3 AS runtime
COPY --from=rust /src/target/release/backend3-cli /app/backend3-cli
"

# repo NAME CHANNEL NODE_VERSION_FILE CONTAINERFILE: create a fixture
# repository and print its path. An empty CHANNEL or NODE_VERSION_FILE omits
# that file.
repo() {
  local dir="$WORK/$1"
  mkdir -p "$dir/admin"
  if [[ -n $2 ]]; then
    printf '[toolchain]\nchannel = "%s"\ncomponents = ["rustfmt", "clippy"]\nprofile = "minimal"\n' "$2" >"$dir/rust-toolchain.toml"
  fi
  if [[ -n $3 ]]; then
    printf '%s\n' "$3" >"$dir/admin/.node-version"
  fi
  printf '%s' "$4" >"$dir/Containerfile"
  printf '%s' "$dir"
}

# Consistent fixtures.
run_case "a consistent repository passes" 0 --contains "Rust 1.99.1 and Node.js 24 agree" -- \
  bash "$SCRIPT" --root "$(repo good 1.99.1 24 "$GOOD_CONTAINERFILE")"
run_case "1.94.0, the minimum, passes" 0 -- \
  bash "$SCRIPT" --root "$(repo min 1.94.0 24 "${GOOD_CONTAINERFILE//1.99.1/1.94.0}")"
run_case "1.100.0 compares numerically and passes" 0 -- \
  bash "$SCRIPT" --root "$(repo hundred 1.100.0 24 "${GOOD_CONTAINERFILE//1.99.1/1.100.0}")"
run_case "2.0.0 passes" 0 -- \
  bash "$SCRIPT" --root "$(repo two 2.0.0 24 "${GOOD_CONTAINERFILE//1.99.1/2.0.0}")"
run_case "a quoted ARG default passes" 0 -- \
  bash "$SCRIPT" --root "$(repo quoted 1.99.1 24 "${GOOD_CONTAINERFILE//ARG RUST_VERSION=1.99.1/ARG RUST_VERSION=\"1.99.1\"}")"
run_case "a literal nodejs-24 image passes" 0 -- \
  bash "$SCRIPT" --root "$(repo literal 1.99.1 24 "${GOOD_CONTAINERFILE//nodejs-\$\{NODE_VERSION\}/nodejs-24}")"
run_case "lower-case instructions pass" 0 -- \
  bash "$SCRIPT" --root "$(repo lower 1.99.1 24 "$(sed -e 's/^ARG /arg /' -e 's/^FROM /from /' -e 's/ AS / as /' <<<"$GOOD_CONTAINERFILE")")"

# Rule 1: rust-toolchain.toml.
run_case "channel 'stable' fails" 1 --contains "is not an exact release" -- \
  bash "$SCRIPT" --root "$(repo stable stable 24 "$GOOD_CONTAINERFILE")"
run_case "channel '1.94' without a patch fails" 1 --contains "is not an exact release" -- \
  bash "$SCRIPT" --root "$(repo no-patch 1.94 24 "$GOOD_CONTAINERFILE")"
run_case "channel '1.93.1' below the MSRV fails" 1 --contains "below 1.94" -- \
  bash "$SCRIPT" --root "$(repo old 1.93.1 24 "${GOOD_CONTAINERFILE//1.99.1/1.93.1}")"
run_case "a nightly channel fails" 1 --contains "is not an exact release" -- \
  bash "$SCRIPT" --root "$(repo nightly nightly-2026-10-01 24 "$GOOD_CONTAINERFILE")"
run_case "a missing rust-toolchain.toml fails" 1 --contains "rust-toolchain.toml is missing" -- \
  bash "$SCRIPT" --root "$(repo no-toml "" 24 "$GOOD_CONTAINERFILE")"
dir=$(repo no-channel 1.99.1 24 "$GOOD_CONTAINERFILE")
printf '[toolchain]\nprofile = "minimal"\n' >"$dir/rust-toolchain.toml"
run_case "a toolchain file without channel fails" 1 --contains "has no [toolchain] channel" -- bash "$SCRIPT" --root "$dir"
dir=$(repo bad-toml 1.99.1 24 "$GOOD_CONTAINERFILE")
printf '[toolchain\nchannel = \n' >"$dir/rust-toolchain.toml"
run_case "invalid TOML fails" 1 --contains "not valid TOML" -- bash "$SCRIPT" --root "$dir"
dir=$(repo legacy 1.99.1 24 "$GOOD_CONTAINERFILE")
printf '1.99.1\n' >"$dir/rust-toolchain"
run_case "a legacy rust-toolchain file fails" 1 --contains "legacy 'rust-toolchain' file" -- bash "$SCRIPT" --root "$dir"

# Rule 2: RUST_VERSION.
run_case "a RUST_VERSION mismatch fails" 1 --contains "ARG RUST_VERSION is '1.98.0', but rust-toolchain.toml says '1.99.1'" -- \
  bash "$SCRIPT" --root "$(repo rust-mismatch 1.99.1 24 "${GOOD_CONTAINERFILE//ARG RUST_VERSION=1.99.1/ARG RUST_VERSION=1.98.0}")"
run_case "a missing RUST_VERSION default fails" 1 --contains "no 'ARG RUST_VERSION=<version>' default" -- \
  bash "$SCRIPT" --root "$(repo rust-missing 1.99.1 24 "${GOOD_CONTAINERFILE//ARG RUST_VERSION=1.99.1/}")"
run_case "a second, different RUST_VERSION default fails" 1 --contains "ARG RUST_VERSION is '1.95.0'" -- \
  bash "$SCRIPT" --root "$(repo rust-second 1.99.1 24 "${GOOD_CONTAINERFILE//ARG RUST_VERSION$'\n'/ARG RUST_VERSION=1.95.0$'\n'}")"

# Rule 3: NODE_VERSION and admin/.node-version.
run_case "a NODE_VERSION mismatch fails" 1 --contains "ARG NODE_VERSION is '22', but admin/.node-version says '24'" -- \
  bash "$SCRIPT" --root "$(repo node-mismatch 1.99.1 24 "${GOOD_CONTAINERFILE//ARG NODE_VERSION=24/ARG NODE_VERSION=22}")"
run_case "a missing NODE_VERSION default fails" 1 --contains "no 'ARG NODE_VERSION=<major>' default" -- \
  bash "$SCRIPT" --root "$(repo node-missing 1.99.1 24 "${GOOD_CONTAINERFILE//ARG NODE_VERSION=24/}")"
run_case "'v24' in .node-version fails" 1 --contains "must hold the Node.js major only" -- \
  bash "$SCRIPT" --root "$(repo node-v 1.99.1 v24 "$GOOD_CONTAINERFILE")"
run_case "a full version in .node-version fails" 1 --contains "must hold the Node.js major only" -- \
  bash "$SCRIPT" --root "$(repo node-full 1.99.1 24.21.0 "$GOOD_CONTAINERFILE")"
run_case "a missing .node-version fails" 1 --contains "admin/.node-version is missing" -- \
  bash "$SCRIPT" --root "$(repo node-none 1.99.1 "" "$GOOD_CONTAINERFILE")"
run_case "a literal nodejs-22 image with .node-version 24 fails" 1 --contains "uses ubi9/nodejs-22, but admin/.node-version says 24" -- \
  bash "$SCRIPT" --root "$(repo node-image 1.99.1 24 "${GOOD_CONTAINERFILE//nodejs-\$\{NODE_VERSION\}/nodejs-22}")"

# Rule 4: base images.
run_case "a tag instead of a digest fails" 1 --contains "pinned by digest" -- \
  bash "$SCRIPT" --root "$(repo tag 1.99.1 24 "${GOOD_CONTAINERFILE//ubi-micro@$D3/ubi-micro:latest}")"
run_case "a UBI 10 image fails" 1 --contains "must be a UBI 9 image" -- \
  bash "$SCRIPT" --root "$(repo ubi10 1.99.1 24 "${GOOD_CONTAINERFILE//ubi9\/ubi@/ubi10/ubi@}")"
run_case "a non-Red Hat image fails" 1 --contains "must be a UBI 9 image" -- \
  bash "$SCRIPT" --root "$(repo dockerhub 1.99.1 24 "${GOOD_CONTAINERFILE//registry.access.redhat.com\/ubi9\/ubi@$D2/docker.io/library/rust@$D2}")"
run_case "no Node.js stage fails" 1 --contains "no admin stage" -- \
  bash "$SCRIPT" --root "$(repo no-node-stage 1.99.1 24 "${GOOD_CONTAINERFILE//nodejs-\$\{NODE_VERSION\}/ubi}")"
run_case "FROM an earlier stage is allowed" 0 -- \
  bash "$SCRIPT" --root "$(repo stage-ref 1.99.1 24 "$GOOD_CONTAINERFILE
FROM rust AS tests
RUN true")"
run_case "FROM a stage declared later is not a stage reference" 1 --contains "base image 'later'" -- \
  bash "$SCRIPT" --root "$(repo stage-later 1.99.1 24 "FROM later AS first
$GOOD_CONTAINERFILE
FROM registry.access.redhat.com/ubi9/ubi@$D2 AS later")"

# Errors.
run_case "a missing Containerfile fails" 1 --contains "Containerfile is missing" -- \
  bash -c 'rm "$1/Containerfile" && bash "$2" --root "$1"' _ "$(repo no-cf 1.99.1 24 "$GOOD_CONTAINERFILE")" "$SCRIPT"
run_case "a missing root is a usage error" 2 --contains "does not exist" -- bash "$SCRIPT" --root "$WORK/nope"
run_case "an unknown option is a usage error" 2 -- bash "$SCRIPT" --bogus

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
