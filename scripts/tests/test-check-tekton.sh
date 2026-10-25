#!/usr/bin/env bash
# Tests for scripts/ci/check-tekton.sh (ADR 0022 section 1).
#
# The real .tekton/push.yaml must pass; copies of it with one invariant
# broken must fail: the nested timeouts, the SHA image tag, the build arg,
# the strategy, the BuildRun fields that Shipwright forbids with an inline
# build spec (the overrides, a trigger and a named Build), master-only
# triggering and the absence of a PR pipeline. One copy restores the
# BuildRun of the first PAC build (spec.timeout beside spec.build.spec),
# which the controller failed on 2026-10-24 with
# BuildRunBuildFieldOverrideForbidden.
# Run from the repository root:
#   bash scripts/tests/test-check-tekton.sh
# Needs mikefarah yq v4 and jq; no network access.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT="$ROOT/scripts/ci/check-tekton.sh"
REAL="$ROOT/.tekton/push.yaml"

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

# variant NAME SED_SCRIPT: copy the real push.yaml into a fresh fixture root,
# apply SED_SCRIPT to it, and print the root.
variant() {
  local dir="$WORK/$1"
  mkdir -p "$dir/.tekton"
  sed -e "$2" "$REAL" >"$dir/.tekton/push.yaml"
  if cmp -s "$REAL" "$dir/.tekton/push.yaml" && [[ $2 != 's/^$//' ]]; then
    echo "fixture '$1': the sed script changed nothing; the test is stale" >&2
    return 1
  fi
  printf '%s' "$dir"
}

run_case "the real .tekton/push.yaml passes" 0 --contains "Tekton OK" -- bash "$SCRIPT" --root "$ROOT"
run_case "an unchanged copy passes" 0 -- bash "$SCRIPT" --root "$(variant same 's/^$//')"

# Rule 4: nested timeouts.
# The innermost layer is spec.build.spec.timeout: another value there that
# still nests passes, so the check reads that field and not spec.timeout.
run_case "a shorter spec.build.spec.timeout passes" 0 --contains "Tekton OK" -- \
  bash "$SCRIPT" --root "$(variant br-shorter 's/,"timeout":"1h30m0s"}}}}/,"timeout":"1h0m0s"}}}}/')"
run_case "BuildRun timeout equal to oc wait fails" 1 --contains "BuildRun spec.build.spec.timeout (100m) must be smaller than oc wait --timeout (100m)" -- \
  bash "$SCRIPT" --root "$(variant br-equal 's/"timeout":"1h30m0s"/"timeout":"100m"/')"
run_case "oc wait longer than the task fails" 1 --contains "oc wait --timeout (2h) must be smaller than task timeout (1h45m0s)" -- \
  bash "$SCRIPT" --root "$(variant wait-long 's/--timeout=100m/--timeout=2h/')"
run_case "task longer than the PipelineRun fails" 1 --contains "task timeout (3h0m0s) must be smaller than PipelineRun timeouts.pipeline (2h0m0s)" -- \
  bash "$SCRIPT" --root "$(variant task-long "s/timeout: '1h45m0s'/timeout: '3h0m0s'/")"
run_case "a missing task timeout fails" 1 --contains "task timeout is missing" -- \
  bash "$SCRIPT" --root "$(variant task-missing "/timeout: '1h45m0s'/d")"
run_case "a missing BuildRun timeout fails" 1 --contains "BuildRun spec.build.spec.timeout is missing" -- \
  bash "$SCRIPT" --root "$(variant br-missing 's/,"timeout":"1h30m0s"}}}}/}}}}/')"
run_case "a missing oc wait timeout fails" 1 --contains "oc wait --timeout is missing" -- \
  bash "$SCRIPT" --root "$(variant wait-missing 's/ --timeout=100m//')"
run_case "a missing pipeline timeout fails" 1 --contains "PipelineRun timeouts.pipeline is missing" -- \
  bash "$SCRIPT" --root "$(variant pipe-missing "s/pipeline: '2h0m0s'/finally: '2h0m0s'/")"

# Rule 3: the BuildRun.
run_case "a :latest image tag fails" 1 --contains "Image Updater only accepts full commit SHA tags" -- \
  bash "$SCRIPT" --root "$(variant latest 's/website_backend3:{{ revision }}/website_backend3:latest/')"
run_case "a missing SOURCE_COMMIT build arg fails" 1 --contains "SOURCE_COMMIT={{ revision }}" -- \
  bash "$SCRIPT" --root "$(variant no-arg 's/,{"name":"build-args","values":\[{"value":"SOURCE_COMMIT={{ revision }}"}\]}//')"
run_case "the cluster buildah strategy fails" 1 --contains "buildah-ubi9" -- \
  bash "$SCRIPT" --root "$(variant cluster-strategy 's/"name":"buildah-ubi9","kind":"BuildStrategy"/"name":"buildah","kind":"ClusterBuildStrategy"/')"
run_case "a Dockerfile name fails" 1 --contains "dockerfile=Containerfile" -- \
  bash "$SCRIPT" --root "$(variant dockerfile 's/"value":"Containerfile"/"value":"Dockerfile"/')"
run_case "a wrong generateName fails" 1 --contains "backend3-pac-" -- \
  bash "$SCRIPT" --root "$(variant gen-name 's/"generateName":"backend3-pac-"/"generateName":"backend2-pac-"/')"
run_case "a pinned branch revision fails" 1 --contains "source.git.revision" -- \
  bash "$SCRIPT" --root "$(variant revision 's/"revision":"{{ revision }}"/"revision":"master"/')"
run_case "stepResources fail" 1 --contains "stepResources must not be set" -- \
  bash "$SCRIPT" --root "$(variant step-resources 's/,"timeout":"1h30m0s"}}}}/,"timeout":"1h30m0s","stepResources":[]}}}}/')"

# Rule 3: no BuildRun-level override next to the inline build spec.
# Regression: the BuildRun of the first PAC build, with the timeout in
# spec.timeout beside spec.build.spec instead of inside it. The fixture is
# made once, before the two cases that read it, so neither depends on the
# other; under set -e a stale sed script stops the run here.
br_override=$(variant br-override 's/"spec":{"build":{"spec":{/"spec":{"timeout":"1h30m0s","build":{"spec":{/; s/,"timeout":"1h30m0s"}}}}/}}}}/')
run_case "spec.timeout beside spec.build.spec (the first PAC build) fails" 1 \
  --contains "spec.timeout must not be set next to the inline spec.build.spec; Shipwright fails such a BuildRun with reason BuildRunBuildFieldOverrideForbidden: \"cannot use 'timeout' override and 'buildSpec' simultaneously\"" -- \
  bash "$SCRIPT" --root "$br_override"
run_case "the first PAC build's BuildRun also lacks the innermost timeout" 1 --contains "BuildRun spec.build.spec.timeout is missing" -- \
  bash "$SCRIPT" --root "$br_override"
run_case "spec.timeout fails even beside a valid spec.build.spec.timeout" 1 --contains "cannot use 'timeout' override and 'buildSpec' simultaneously" -- \
  bash "$SCRIPT" --root "$(variant br-both 's/"spec":{"build":{"spec":{/"spec":{"timeout":"1h30m0s","build":{"spec":{/')"

# The other overrides Shipwright rejects next to spec.build.spec, one copy
# each. Each entry is "fixture|JSON to insert|Shipwright's message"; the JSON
# lands at the start of the BuildRun spec, beside spec.build.
overrides=(
  "ovr-output|\"output\":{\"image\":\"ghcr.io/verseghy/website_backend3:latest\"},|cannot use 'output' override and 'buildSpec' simultaneously"
  "ovr-params|\"paramValues\":[{\"name\":\"dockerfile\",\"value\":\"Containerfile\"}],|cannot use 'paramValues' override and 'buildSpec' simultaneously"
  "ovr-env|\"env\":[{\"name\":\"A\",\"value\":\"b\"}],|cannot use 'env' override and 'buildSpec' simultaneously"
  "ovr-node-selector|\"nodeSelector\":{\"kubernetes.io/arch\":\"amd64\"},|cannot use 'nodeSelector' override and 'buildSpec' simultaneously"
  "ovr-tolerations|\"tolerations\":[{\"key\":\"a\",\"operator\":\"Exists\"}],|cannot use 'tolerations' override and 'buildSpec' simultaneously"
  "ovr-scheduler|\"schedulerName\":\"default-scheduler\",|cannot use 'schedulerName' override and 'buildSpec' simultaneously"
  "ovr-runtime-class|\"runtimeClassName\":\"runc\",|cannot use 'runtimeClassName' override and 'buildSpec' simultaneously"
)
for entry in "${overrides[@]}"; do
  IFS='|' read -r fixture insert message <<<"$entry"
  edit="s|\"spec\":{\"build\":{\"spec\":{|\"spec\":{$insert\"build\":{\"spec\":{|"
  run_case "${fixture#ovr-} beside spec.build.spec fails" 1 --contains "$message" -- \
    bash "$SCRIPT" --root "$(variant "$fixture" "$edit")"
done
# A trigger is rejected from inside the inline build spec, so its fixture puts
# it there and the message says "in", not "next to".
run_case "trigger inside spec.build.spec fails" 1 \
  --contains "spec.build.spec.trigger must not be set in a BuildRun's inline build spec; Shipwright fails such a BuildRun with reason BuildRunBuildFieldOverrideForbidden: \"cannot use 'triggers' override in the 'BuildRun', only allowed in the 'Build'\"" -- \
  bash "$SCRIPT" --root "$(variant ovr-trigger 's/"spec":{"build":{"spec":{/&"trigger":{"when":[]},/')"
# A Build named in spec.build.name beside the inline spec is ambiguous, a
# different reason from the overrides; admission accepts it too.
run_case "spec.build.name beside spec.build.spec fails" 1 \
  --contains "spec.build.name must not be set next to the inline spec.build.spec; Shipwright fails such a BuildRun with reason BuildRunAmbiguousBuild: \"fields 'buildRef' and 'buildSpec' are mutually exclusive\"" -- \
  bash "$SCRIPT" --root "$(variant build-name 's/"spec":{"build":{"spec":{/"spec":{"build":{"name":"backend3","spec":{/')"
# Shipwright rejects a list or map override only when it is non-empty, and the
# check mirrors that, so its message never claims a rejection that does not
# happen.
run_case "an empty spec.env list passes, as Shipwright accepts it" 0 --contains "Tekton OK" -- \
  bash "$SCRIPT" --root "$(variant ovr-env-empty 's/"spec":{"build":{"spec":{/"spec":{"env":[],"build":{"spec":{/')"
run_case "broken BuildRun JSON fails" 1 --contains "not valid JSON" -- \
  bash "$SCRIPT" --root "$(variant bad-json 's/"kind":"BuildRun",/"kind":"BuildRun"/')"

# Rule 2: PipelineRun triggers.
run_case "a main target branch fails" 1 --contains "on-target-branch must be '[master]'" -- \
  bash "$SCRIPT" --root "$(variant main "s/on-target-branch: '\[master\]'/on-target-branch: '[main]'/")"
run_case "a pull_request event fails" 1 --contains "on-event must be '[push]'" -- \
  bash "$SCRIPT" --root "$(variant pr-event "s/on-event: '\[push\]'/on-event: '[pull_request]'/")"
run_case "a renamed PipelineRun fails" 1 --contains "build-backend3" -- \
  bash "$SCRIPT" --root "$(variant name 's/name: build-backend3/name: build-backend2/')"

# Rule 1: only push.yaml.
dir=$(variant with-pr 's/^$//')
cp "$dir/.tekton/push.yaml" "$dir/.tekton/pull-request.yaml"
run_case "a .tekton/pull-request.yaml fails" 1 --contains "must hold only push.yaml" -- bash "$SCRIPT" --root "$dir"
mkdir -p "$WORK/no-push/.tekton"
run_case "a .tekton/ without push.yaml fails" 1 --contains "found: none" -- bash "$SCRIPT" --root "$WORK/no-push"
run_case "a root without .tekton/ is a usage error" 2 --contains "does not exist" -- bash "$SCRIPT" --root "$WORK"

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
