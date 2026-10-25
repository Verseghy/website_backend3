#!/usr/bin/env bash
# check-tekton.sh: check the invariants of the PAC master build.
#
# Why this exists: .tekton/push.yaml is the only production delivery path
# (ADR 0022 section 1), and a mistake in it shows up only after a merge, when
# PAC runs it on the hub. Its rules are easy to break by hand: the image tag
# must be the commit SHA, the four timeouts must nest, the inline build spec
# takes no BuildRun-level overrides (admission accepts them; the Shipwright
# controller then fails the BuildRun), and there must be no PR build.
# scripts/tests/test-check-tekton.sh runs this script against the real
# file, so the "Test" CI job enforces it on every pull request.
#
# Rules (ADR 0022 section 1):
#   1. .tekton/ holds exactly one pipeline file, push.yaml (no
#      pull-request.yaml: PR builds would queue with master builds behind
#      `concurrency_limit: 1`).
#   2. push.yaml is a tekton.dev/v1 PipelineRun named build-backend3 that runs
#      only on push events to master.
#   3. Its single task's SCRIPT creates a Shipwright BuildRun with
#      generateName backend3-pac-, source revision {{ revision }} of
#      https://github.com/Verseghy/website_backend3, strategy buildah-ubi9 of
#      kind BuildStrategy, dockerfile=Containerfile, the build arg
#      SOURCE_COMMIT={{ revision }}, output
#      ghcr.io/verseghy/website_backend3:{{ revision }}, and no stepResources.
#      Because the build spec is inline (spec.build.spec), the BuildRun names
#      no Build (spec.build.name), sets none of Shipwright's BuildRun-level
#      overrides of a referenced Build (spec.timeout, spec.output,
#      spec.paramValues, spec.env, spec.nodeSelector, spec.tolerations,
#      spec.schedulerName, spec.runtimeClassName) and no
#      spec.build.spec.trigger.
#   4. Each timeout is set and strictly larger than the one inside it:
#      BuildRun spec.build.spec.timeout < oc wait --timeout < task timeout <
#      PipelineRun timeouts.pipeline.
#
# Usage: check-tekton.sh [--root DIR]   (default: the current directory)
#
# Exit codes: 0 compliant, 1 at least one violation, 2 usage error.
set -euo pipefail

readonly REPO_URL='https://github.com/Verseghy/website_backend3'
readonly IMAGE='ghcr.io/verseghy/website_backend3:{{ revision }}'

usage() {
  echo "Usage: check-tekton.sh [--root DIR]   (default: the current directory)"
}

die_usage() {
  echo "error: $*" >&2
  exit 2
}

problems=()
# The BuildRun object embedded in the task SCRIPT, as compact JSON. PAC's
# `{{ revision }}` placeholder only appears inside JSON strings, so the text
# parses as is.
BUILDRUN_JSON=""

# problem MESSAGE: record one violation.
problem() {
  problems+=("$1")
}

# buildrun_get EXPR: print the jq EXPR of BUILDRUN_JSON as text ("" when
# null or absent).
buildrun_get() {
  printf '%s' "$BUILDRUN_JSON" | jq -r "($1) // \"\" | tostring"
}

# duration_seconds DURATION: print a Go duration such as 1h30m0s, 100m or
# 90s in seconds; print nothing for anything else.
duration_seconds() {
  local d=$1
  [[ -n $d && $d =~ ^(([0-9]+)h)?(([0-9]+)m)?(([0-9]+)s)?$ ]] || return 0
  printf '%d' $((10#${BASH_REMATCH[2]:-0} * 3600 + 10#${BASH_REMATCH[4]:-0} * 60 + 10#${BASH_REMATCH[6]:-0}))
}

# yq_get FILE EXPR: print the scalar EXPR selects in FILE ("" when absent).
yq_get() {
  yq -r "($2) // \"\"" "$1"
}

# shipwright_rejects TEST WHAT REASON MESSAGE HINT: record a violation when
# the jq boolean TEST is true for BUILDRUN_JSON.
#   TEST     jq expression that mirrors Shipwright's own condition (a pointer
#            field is set, or a list or map is non-empty), so this check
#            rejects exactly what the controller rejects.
#   WHAT     the clause that names the offending field and where it is.
#   REASON   Shipwright's condition reason, such as
#            BuildRunBuildFieldOverrideForbidden.
#   MESSAGE  Shipwright's condition message, verbatim, so the CI output can
#            be matched against a failed BuildRun's status.
#   HINT     what to write instead.
shipwright_rejects() {
  local test=$1 what=$2 reason=$3 message=$4 hint=$5
  if [[ $(buildrun_get "$test") == true ]]; then
    problem "BuildRun: $what; Shipwright fails such a BuildRun with reason $reason: \"$message\". $hint"
  fi
}

# forbid_override TEST FIELD MESSAGE HINT: shipwright_rejects for a
# BuildRun-level FIELD that overrides a referenced Build, which Shipwright
# rejects with reason BuildRunBuildFieldOverrideForbidden. TEST, MESSAGE and
# HINT are as for shipwright_rejects; FIELD is the field path to name.
forbid_override() {
  local test=$1 field=$2 message=$3 hint=$4
  shipwright_rejects "$test" "$field must not be set next to the inline spec.build.spec" \
    BuildRunBuildFieldOverrideForbidden "$message" "$hint"
}

# check_inline_overrides: apply the last sentence of rule 3.
#
# Why: a BuildRun's spec.timeout, spec.output, spec.paramValues and the other
# fields below override the values of a Build it references by name. With an
# inline spec.build.spec there is nothing to override, and the Shipwright
# controller fails the BuildRun at once, before any build pod exists. Naming a
# Build in spec.build.name beside the inline spec fails the same way, as an
# ambiguous build. The CRD schema and the Shipwright conversion webhook accept
# all of these (oc create --dry-run=server passes), so only this check catches
# the mistake before the merge. The first PAC build of backend3 failed this
# way on 2026-10-24, on Builds for Red Hat OpenShift 1.8.2, with spec.timeout.
#
# The conditions, reasons and messages are those that BuildRunFields in
# pkg/validate/validate.go returns for a BuildRun with spec.build.spec set, in
# the Shipwright source that the 1.8.2 controller image is built from
# (redhat-openshift-builds/shipwright-io-build@99ba109). Two of its results
# are not repeated here: stepResources, because rule 3 already rejects it at
# every level of the BuildRun, and BuildRunNoRefOrSpec (neither name nor
# spec), because without an inline spec the source and strategy checks of
# rule 3 already fail.
check_inline_overrides() {
  [[ $(buildrun_get '.spec.build.spec != null') == true ]] || return 0
  local inside="Set it inside spec.build.spec instead."
  shipwright_rejects '.spec.build.name != null' \
    "spec.build.name must not be set next to the inline spec.build.spec" \
    BuildRunAmbiguousBuild "fields 'buildRef' and 'buildSpec' are mutually exclusive" \
    "Remove it; this BuildRun builds from its inline spec, not from a Build object."
  forbid_override '.spec.timeout != null' spec.timeout \
    "cannot use 'timeout' override and 'buildSpec' simultaneously" \
    "Set the build timeout in spec.build.spec.timeout instead (ADR 0022 section 1)."
  forbid_override '.spec.output != null' spec.output \
    "cannot use 'output' override and 'buildSpec' simultaneously" "$inside"
  forbid_override '(.spec.paramValues // [] | length) > 0' spec.paramValues \
    "cannot use 'paramValues' override and 'buildSpec' simultaneously" "$inside"
  forbid_override '(.spec.env // [] | length) > 0' spec.env \
    "cannot use 'env' override and 'buildSpec' simultaneously" "$inside"
  # The trigger sits inside the inline spec, not beside it, so it gets its own
  # wording; the reason is the same override reason.
  shipwright_rejects '.spec.build.spec.trigger != null' \
    "spec.build.spec.trigger must not be set in a BuildRun's inline build spec" \
    BuildRunBuildFieldOverrideForbidden \
    "cannot use 'triggers' override in the 'BuildRun', only allowed in the 'Build'" \
    "Remove it; PAC already triggers this build."
  forbid_override '(.spec.nodeSelector // {} | length) > 0' spec.nodeSelector \
    "cannot use 'nodeSelector' override and 'buildSpec' simultaneously" "$inside"
  forbid_override '(.spec.tolerations // [] | length) > 0' spec.tolerations \
    "cannot use 'tolerations' override and 'buildSpec' simultaneously" "$inside"
  forbid_override '.spec.schedulerName != null' spec.schedulerName \
    "cannot use 'schedulerName' override and 'buildSpec' simultaneously" "$inside"
  forbid_override '.spec.runtimeClassName != null' spec.runtimeClassName \
    "cannot use 'runtimeClassName' override and 'buildSpec' simultaneously" "$inside"
}

# check_push_yaml FILE: apply rules 2-4 to the PipelineRun in FILE.
check_push_yaml() {
  local file=$1
  if ! yq 'true' "$file" >/dev/null 2>&1; then
    problem ".tekton/push.yaml is not valid YAML."
    return
  fi

  [[ $(yq_get "$file" .apiVersion) == tekton.dev/v1 ]] || problem "push.yaml: apiVersion must be tekton.dev/v1."
  [[ $(yq_get "$file" .kind) == PipelineRun ]] || problem "push.yaml: kind must be PipelineRun."
  [[ $(yq_get "$file" .metadata.name) == build-backend3 ]] || problem "push.yaml: metadata.name must be build-backend3."
  [[ $(yq_get "$file" '.metadata.annotations["pipelinesascode.tekton.dev/on-event"]') == '[push]' ]] ||
    problem "push.yaml: annotation pipelinesascode.tekton.dev/on-event must be '[push]' (master builds only)."
  [[ $(yq_get "$file" '.metadata.annotations["pipelinesascode.tekton.dev/on-target-branch"]') == '[master]' ]] ||
    problem "push.yaml: annotation pipelinesascode.tekton.dev/on-target-branch must be '[master]'."

  local tasks
  tasks=$(yq '.spec.pipelineSpec.tasks | length' "$file")
  if [[ $tasks != 1 ]]; then
    problem "push.yaml: spec.pipelineSpec.tasks must hold exactly one task (found $tasks)."
    return
  fi

  local script json
  script=$(yq_get "$file" '.spec.pipelineSpec.tasks[0].params[] | select(.name == "SCRIPT") | .value')
  if [[ -z $script ]]; then
    problem "push.yaml: the task has no SCRIPT parameter."
    return
  fi
  json=$(printf '%s\n' "$script" | sed -n "s/^BR=\$(echo '\(.*\)' | oc create -o name -f -)\$/\1/p")
  if [[ -z $json ]]; then
    problem "push.yaml: SCRIPT does not start with BR=\$(echo '<BuildRun JSON>' | oc create -o name -f -)."
    return
  fi
  if ! BUILDRUN_JSON=$(printf '%s' "$json" | jq -c . 2>/dev/null); then
    problem "push.yaml: the BuildRun in SCRIPT is not valid JSON."
    return
  fi

  [[ $(buildrun_get .kind) == BuildRun ]] || problem "BuildRun: kind must be BuildRun."
  [[ $(buildrun_get .metadata.generateName) == backend3-pac- ]] || problem "BuildRun: metadata.generateName must be backend3-pac-."
  [[ $(buildrun_get .spec.build.spec.source.git.url) == "$REPO_URL" ]] || problem "BuildRun: source.git.url must be $REPO_URL."
  [[ $(buildrun_get .spec.build.spec.source.git.revision) == '{{ revision }}' ]] || problem "BuildRun: source.git.revision must be {{ revision }}, the pushed commit."
  [[ $(buildrun_get .spec.build.spec.strategy.name) == buildah-ubi9 && $(buildrun_get .spec.build.spec.strategy.kind) == BuildStrategy ]] ||
    problem "BuildRun: strategy must be {name: buildah-ubi9, kind: BuildStrategy} (the UBI 9 copy; the cluster strategy needs x86-64-v3)."
  [[ $(buildrun_get '.spec.build.spec.paramValues[]? | select(.name == "dockerfile") | .value') == Containerfile ]] ||
    problem "BuildRun: paramValues must set dockerfile=Containerfile."
  [[ $(buildrun_get 'any(.spec.build.spec.paramValues[]? | select(.name == "build-args") | .values[]?.value; . == "SOURCE_COMMIT={{ revision }}")') == true ]] ||
    problem "BuildRun: paramValues build-args must include SOURCE_COMMIT={{ revision }} (the admin footer's source link)."
  [[ $(buildrun_get .spec.build.spec.output.image) == "$IMAGE" ]] ||
    problem "BuildRun: output.image must be $IMAGE; Image Updater only accepts full commit SHA tags."
  [[ $(buildrun_get '[.. | objects | select(has("stepResources"))] | length') == 0 ]] ||
    problem "BuildRun: stepResources must not be set; capacity comes from the buildah-ubi9 strategy (ADR 0022 section 1)."
  check_inline_overrides

  # Rule 4: nested timeouts. The innermost layer is the inline build spec's
  # timeout, which the Shipwright controller copies onto the build TaskRun;
  # the BuildRun's own spec.timeout is forbidden next to it (rule 3).
  local build_t wait_t task_t pipe_t
  build_t=$(buildrun_get .spec.build.spec.timeout)
  wait_t=$(printf '%s\n' "$script" | sed -n 's/.*oc wait .*--timeout=\([0-9hms]*\).*/\1/p' | head -n 1)
  task_t=$(yq_get "$file" '.spec.pipelineSpec.tasks[0].timeout')
  pipe_t=$(yq_get "$file" '.spec.timeouts.pipeline')
  local -a names=("BuildRun spec.build.spec.timeout" "oc wait --timeout" "task timeout" "PipelineRun timeouts.pipeline")
  local -a values=("$build_t" "$wait_t" "$task_t" "$pipe_t")
  local -a secs=()
  local i ok=1
  for i in 0 1 2 3; do
    secs[i]=$(duration_seconds "${values[i]}")
    if [[ -z ${secs[i]} ]]; then
      problem "timeouts: ${names[i]} is missing or not a duration such as 1h30m0s (found '${values[i]}')."
      ok=0
    fi
  done
  if ((ok)); then
    for i in 0 1 2; do
      if ((secs[i] >= secs[i + 1])); then
        problem "timeouts: ${names[i]} (${values[i]}) must be smaller than ${names[i + 1]} (${values[i + 1]}); each layer is larger than the one inside it."
      fi
    done
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
  command -v yq >/dev/null || die_usage "yq (mikefarah, v4) is required but not installed."
  command -v jq >/dev/null || die_usage "jq is required but not installed."
  [[ -d $root/.tekton ]] || die_usage "'$root/.tekton' does not exist."

  local -a pipeline_files=()
  local f
  while IFS= read -r -d '' f; do
    pipeline_files+=("${f##*/}")
  done < <(find "$root/.tekton" -maxdepth 1 -type f \( -name '*.yaml' -o -name '*.yml' \) -print0 | sort -z)
  if [[ ${pipeline_files[*]-} != push.yaml ]]; then
    problem ".tekton/ must hold only push.yaml (found: ${pipeline_files[*]-none}). PR images are not built on the hub (ADR 0022 section 1)."
  fi
  if [[ -f $root/.tekton/push.yaml ]]; then
    check_push_yaml "$root/.tekton/push.yaml"
  fi

  if ((${#problems[@]} == 0)); then
    echo "Tekton OK: .tekton/push.yaml builds master only, tags the commit SHA and nests its timeouts."
    return 0
  fi
  {
    echo "Tekton check failed (ADR 0022 section 1):"
    local p
    for p in "${problems[@]}"; do
      echo "  - $p"
    done
  } >&2
  return 1
}

main "$@"
