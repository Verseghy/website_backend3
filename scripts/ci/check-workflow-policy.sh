#!/usr/bin/env bash
# check-workflow-policy.sh: enforce the GitHub Actions rules of ADR 0022 and
# ADR 0029.
#
# Why this exists: GitHub Actions only runs PR checks here. It never builds or
# pushes a production image, holds no secrets and gets no write access; Pipelines-
# as-Code on the hub is the only production delivery path (ADR 0022 section 1).
# Agents merge their own PRs through Tide once the required contexts pass, with
# no per-PR reviewer (ADR 0012 section 2), so those checks are the only gate and
# must not be skippable (ADR 0029). Review alone does not keep either true, so
# the "Workflow policy" CI job (a required context, ADR 0022 section 3) runs
# this script on every pull request.
#
# Every workflow file must:
#   1. start with top-level `permissions: contents: read` and nothing else;
#   2. not trigger on `pull_request_target`;
#   3. not grant `packages: write`, `id-token: write` or `write-all`, at the
#      top level or in any job;
#   4. not log in to a container registry, neither with a login action nor
#      with a `docker|podman|buildah|skopeo|oras|nerdctl login` (or helm,
#      crane, regctl equivalent) command in a `run:` script;
#   5. not use secrets: no `secrets` in any `${{ }}` expression and no
#      `secrets:` key on a reusable-workflow job;
#   6. pin every action to a full 40-character commit SHA with a version
#      comment (`uses: owner/repo@<sha> # v1.2.3`); `docker://` references are
#      pinned by `@sha256:` digest; local `./` actions are exempt;
#   7. declare a top-level `concurrency` with `group` and `cancel-in-progress`,
#      so superseded runs are cancelled.
# Rules 1-5 are ADR 0022 section 1; rules 6 and 7 are its section 3.
#
# A workflow that runs at least one required job must also:
#   8. have no `paths` or `paths-ignore` filter on any trigger, and no
#      `branches` or `branches-ignore` filter on any trigger other than `push`.
#      GitHub leaves the checks of a workflow that a filter skipped pending, so
#      the PR could never merge; Prow requires backend3's contexts for every
#      base branch, stacked PRs included. `push` keeps `branches: [master]`,
#      which limits push runs to master (ADR 0022 section 3); a PR gets its
#      required contexts from its `pull_request` run, which this rule keeps
#      unfiltered. `paths` and `paths-ignore` are banned on `push` too, so that
#      one list of filters holds for every trigger.
# And every required job must:
#   9. have no job-level `if:`, except exactly `github.event_name !=
#      'schedule'`, with or without the `${{ }}` delimiters and with any
#      spacing around `!=`. GitHub reports a job that its condition skipped as
#      a successful check, and Tide counts `skipped` as passing, so a condition
#      would let a PR merge untested. The schedule guard is true on every
#      `pull_request` and `push` event; it skips the job only in a scheduled
#      run, which no PR waits for. Step-level `if:` (for example
#      `if: failure()` on a log step) is not checked;
#  10. `needs:` only jobs of the same workflow that are themselves required
#      and have no job-level `if:` (the schedule guard included) and no
#      job-level `continue-on-error` other than `false`. GitHub skips every
#      job that needs a job which failed or was skipped, and a skipped
#      required job passes. A required target fails its own context instead.
#      Because every target is required and so obeys this rule too, the rule
#      also holds for every job reached through a chain of `needs:`. A
#      target's failure blocks the merge only once Prow requires its context,
#      and this script counts a new job as required before Prow lists it (see
#      below), so an existing required job names a new job in `needs:` only
#      after website_k8s's Prow configuration lists it (review checks that);
#  11. have no `continue-on-error`, on the job or on any of its steps, other
#      than the literal `false`. At job level it lets the workflow run pass
#      when the job fails; at step level it lets the job pass when the step
#      fails. An expression such as `${{ matrix.experimental }}` counts as
#      set, because it can be true.
# Rules 8 and 9 are ADR 0029 section 2; rules 10 and 11 are its section 4.
# A step-level `if:` that skips the real work, `pull_request.types` without
# `synchronize` and a `push` trigger limited to tags are left to review
# (ADR 0029 section 4).
#
# Which jobs are required: a job's context is its `name:`, or its job id when
# it has none. Every job is required except those whose context is in
# NOT_REQUIRED_CONTEXTS below, the contexts that ADR 0022 section 3's table
# marks "Required: no". Required is the default, so a new job is covered from
# its first PR by rules 8, 9 and 11; for rule 10 the default is the lenient
# side (rule 10 above, ADR 0029 section 4).
# scripts/tests/test-check-workflow-policy.sh checks that this list and the
# table agree.
#
# The workflows are parsed with mikefarah yq v4 (preinstalled on GitHub's
# Ubuntu runners), which keeps line comments, so rule 6 can read the version
# comment next to each `uses:`. jq then evaluates the rules on a JSON summary.
# GitHub Actions accepts YAML anchors and aliases (GitHub changelog,
# 2025-09-18), so yq resolves them first: an aliased trigger, job or step is
# checked as if it were written out.
#
# Usage: check-workflow-policy.sh [WORKFLOW_DIR]   (default: .github/workflows)
#
# Exit codes: 0 all workflows comply, 1 at least one violation, 2 usage error
# (missing tool, missing directory, no workflow files).
set -euo pipefail

usage() {
  echo "Usage: check-workflow-policy.sh [WORKFLOW_DIR]   (default: .github/workflows)"
}

die_usage() {
  echo "error: $*" >&2
  exit 2
}

# The job contexts that ADR 0022 section 3's table marks as not required.
# "cargo-deny check advisories" is left out of branch protection because a new
# RustSec advisory can fail it with no change in the repository. A context
# joins this list only with an ADR that amends that table's Required column to
# "no" (ADR 0001 section 6). The same PR points the test at the amended column
# (ADR 0029 section 2); the test fails while the list and the column disagree.
readonly NOT_REQUIRED_CONTEXTS=("cargo-deny check advisories")

# require_tools: fail with an actionable message unless jq and mikefarah yq v4
# are installed. The Python "yq" wrapper has a different syntax and would
# produce confusing errors.
require_tools() {
  command -v jq >/dev/null || die_usage "jq is required but not installed."
  command -v yq >/dev/null || die_usage "yq (mikefarah, v4) is required but not installed: https://github.com/mikefarah/yq"
  local version
  version=$(yq --version 2>&1 || true)
  if [[ $version != *mikefarah* || $version != *" v4."* ]]; then
    die_usage "this script needs mikefarah yq v4 (https://github.com/mikefarah/yq); found: $version"
  fi
}

# The yq program that turns one workflow into the JSON summary jq checks.
# `explode(.)` first replaces every alias with a copy of its anchored node and
# applies merge keys, so that, for example, `pull_request: *push_settings`
# shows its `branches` filter to rule 8 (a `select(tag == "!!map")` does not
# see through an alias). The copies keep their line comments, so rule 6 still
# reads them.
# `line_comment` returns the comment after a scalar without the leading "# ".
# `trigger_filters` lists the keys under each trigger written as a mapping
# (`pull_request: {paths: [...]}`); a trigger written as a string or a list
# item has no filters. The job's `if:` is read as `.value["if"]` and stored as
# `condition`, because `if` is a keyword in both yq and jq. `needs` is kept as
# written (a string, a list or something invalid) for rule 10 to normalise.
# `continue-on-error` is stored as `continue_on_error`; an absent key is null,
# which rules 10 and 11 tell apart from an explicit `false`.
# shellcheck disable=SC2016 # the program is yq syntax, not shell.
readonly SUMMARY_YQ='explode(.) | {
  "triggers": [
    (.on | select(tag == "!!str")),
    (.on | select(tag == "!!seq") | .[]),
    (.on | select(tag == "!!map") | keys | .[])
  ],
  "trigger_filters": [
    (.on | select(tag == "!!map") | to_entries | .[] | select(.value | tag == "!!map")) | {
      "event": .key,
      "filters": (.value | keys)
    }
  ],
  "permissions": .permissions,
  "concurrency": .concurrency,
  "jobs_is_map": ((.jobs | tag) == "!!map"),
  "jobs": [
    (.jobs | select(tag == "!!map") | to_entries | .[]) | {
      "id": .key,
      "name": .value.name,
      "condition": .value["if"],
      "needs": .value.needs,
      "continue_on_error": .value["continue-on-error"],
      "permissions": .value.permissions,
      "has_secrets": (.value | has("secrets")),
      "uses": .value.uses,
      "uses_comment": (.value.uses | line_comment),
      "steps": [
        ((.value.steps // []) | to_entries | .[]) | {
          "index": .key,
          "name": .value.name,
          "uses": .value.uses,
          "uses_comment": (.value.uses | line_comment),
          "run": .value.run,
          "continue_on_error": .value["continue-on-error"]
        }
      ]
    }
  ],
  "expressions": [.. | select(tag == "!!str") | select(test("[$][{][{]"))]
}'

# The jq program that prints one line per violation of the summary.
# shellcheck disable=SC2016 # the program is jq syntax, not shell.
readonly RULES_JQ='
  def perm_problems($where):
    if type == "string" then
      (if . == "write-all" then ["\($where): `permissions: write-all` grants every scope, including packages and id-token. Grant only what the job needs."] else [] end)
    elif type == "object" then
      [to_entries[] | select((.key == "packages" or .key == "id-token") and .value == "write")
       | "\($where): grants `\(.key): write`. Workflows here never publish images or mint cloud tokens (ADR 0022 section 1); remove it."]
    else [] end;

  def pin_problems($where; $uses; $comment):
    if ($uses | type) != "string" then []
    elif ($uses | startswith("./")) then []
    elif ($uses | startswith("docker://")) then
      (if ($uses | test("@sha256:[0-9a-f]{64}$")) then [] else
        ["\($where): `\($uses)` is not pinned by digest. Use docker://<image>@sha256:<digest>."] end)
    elif ($uses | test("^[^@\\s]+@[0-9a-f]{40}$") | not) then
      ["\($where): `\($uses)` is not pinned to a full commit SHA. Resolve the tag with `gh api repos/<owner>/<repo>/commits/<tag> --jq .sha` and write `uses: <owner>/<repo>@<sha> # <tag>`."]
    elif ($comment | test("^v?[0-9]+(\\.[0-9]+)*") | not) then
      ["\($where): `\($uses)` has no version comment. Append the release it points to, for example `# v1.2.3`, so reviewers and Dependabot can read it."]
    else [] end;

  def login_problems($where; $uses; $run):
    (if ($uses | type) == "string" and ($uses | ascii_downcase | split("@")[0] | test("login"))
     then ["\($where): `\($uses)` logs in to a registry. Workflows never log in to a container registry (ADR 0022 section 1)."]
     else [] end)
    + (if ($run | type) == "string" and ($run | test("\\b(docker|podman|buildah|skopeo|oras|nerdctl)\\s+login\\b|\\bhelm\\s+registry\\s+login\\b|\\bcrane\\s+auth\\s+login\\b|\\bregctl\\s+registry\\s+login\\b"; "i"))
     then ["\($where): the run script logs in to a container registry. Workflows never log in to a registry (ADR 0022 section 1)."]
     else [] end);

  # A job context is the job name, or the job id when it has none. A job is
  # required unless its context is in $not_required (rules 8 to 11).
  def context: (.name // .id) | tostring;
  def required: (context | IN($not_required[])) | not;

  # The one job-level condition a required job may carry (rule 9):
  # github.event_name != (quote)schedule(quote), bare or inside the expression
  # delimiters, with any spacing. \u0027 is the single quote, which cannot
  # appear literally in this single-quoted shell string.
  def schedule_guard:
    type == "string"
    and test("^\\s*(github\\.event_name\\s*!=\\s*\u0027schedule\u0027|\\$\\{\\{\\s*github\\.event_name\\s*!=\\s*\u0027schedule\u0027\\s*\\}\\})\\s*$");

  # A job-level condition on one line: a block scalar (`if: |`) spans several,
  # and the report prints one line per violation. Each line break, with the
  # spaces around it, becomes one space.
  def one_line: tostring | gsub("\\s*\n\\s*"; " ") | sub("\\s+$"; "");

  # Backquoted names, at most three of them, for one readable message line.
  def code_list:
    map("`\(.)`")
    | if length > 3 then (.[:3] | join(", ")) + " and \(length - 3) more" else join(", ") end;

  # Rules 10 and 11: `continue-on-error` counts as set unless it is absent
  # (null) or the literal `false`. A string, such as the expression
  # `${{ matrix.experimental }}`, counts as set because it can be true; so
  # does a quoted `'false'`, because only the YAML boolean is certain to be
  # false. The comparisons are explicit because jq treats `false` like null
  # in `//`.
  def coe_set: . != null and . != false;

  # The value of a `continue-on-error` as the report shows it: an expression
  # as written (on one line), any other string in double quotes, so that a
  # quoted `'false'` does not read as the allowed boolean, and a boolean bare.
  def coe_text:
    if type == "string" and test("[$][{][{]") then one_line else tojson end;

  # Rule 10: the job ids a job needs. `needs:` is one job id or a list of
  # them; null means none. Any other type yields no ids here and is reported
  # by rule 10 itself.
  def needs_list:
    if type == "string" then [.]
    elif type == "array" then map(tostring)
    else [] end;

  # A job and its context, as the messages of rules 9 to 11 name it.
  def job_ref: "job \(.id | tostring) (`\(context)`)";

  . as $w
  # Rule 10 looks up each needs: target by its job id.
  | ($w.jobs | map({key: (.id | tostring), value: .}) | from_entries) as $by_id
  | (if $w.permissions == {"contents": "read"} then [] else
      ["top level: `permissions` must be exactly `contents: read` (found: \($w.permissions | tojson)). Add it at the top of the file; grant extra scopes per job only when a job needs them."] end)
  + (if ($w.triggers | index("pull_request_target")) != null then
      ["top level: triggers on `pull_request_target`, which runs fork code with the base repository'"'"'s token and secrets. Use `pull_request` (ADR 0022 section 1)."] else [] end)
  + ($w.permissions | perm_problems("top level"))
  + (if ($w.concurrency | type) == "object" and ($w.concurrency | has("group")) and ($w.concurrency | has("cancel-in-progress")) then [] else
      ["top level: needs `concurrency:` with `group:` and `cancel-in-progress:` so superseded runs are cancelled (ADR 0022 section 3)."] end)
  + (if $w.jobs_is_map then [] else ["top level: `jobs` is missing or not a mapping."] end)
  + [$w.expressions[] | scan("\\$\\{\\{[^}]*\\}\\}") | select(test("\\bsecrets\\b"))
     | "expression `\(.)` uses a secret. PR checks here need no secrets (ADR 0022 section 3); remove it."]
  + [$w.jobs[] as $job
     | ("job " + ($job.id | tostring)) as $jw
     | ($job.permissions | perm_problems($jw))
       + (if $job.has_secrets then ["\($jw): passes `secrets:` to a reusable workflow. PR checks here need no secrets; remove it."] else [] end)
       + pin_problems($jw; $job.uses; $job.uses_comment)
       + [$job.steps[] as $s
          | ("\($jw), step \($s.index + 1)" + (if $s.name then " (\($s.name))" else "" end)) as $sw
          | pin_problems($sw; $s.uses; $s.uses_comment) + login_problems($sw; $s.uses; $s.run)
          | .[]]
     | .[]]
  + ([$w.jobs[] | select(required) | context] as $req
     | if ($req | length) == 0 then [] else
         [$w.trigger_filters[] as $t
          | $t.filters[]
          | select(IN("paths", "paths-ignore") or ($t.event != "push" and IN("branches", "branches-ignore")))
          | if $t.event == "push" then
              "trigger `push`: has a `\(.)` filter, and this workflow runs required jobs (\($req | code_list)). ADR 0029 bans `paths` and `paths-ignore` on every trigger of such a workflow, `push` included; remove the filter. `push` may keep `branches: [master]`."
            else
              "trigger `\($t.event)`: has a `\(.)` filter, and this workflow runs required jobs (\($req | code_list)). GitHub leaves the checks of a workflow that a filter skipped pending, so the pull request could never merge (ADR 0029). Remove the filter: required jobs run on every pull request, whatever it changes and whatever its base branch."
            end]
       end)
  + [$w.jobs[] | select(required) | select(.condition != null) | select(.condition | schedule_guard | not)
     | "\(job_ref): has the job-level condition `if: \(.condition | one_line)`. GitHub reports a job that its condition skipped as successful, and Tide merges on that, so a required job must always run (ADR 0029). Remove the condition. The only one allowed is `if: github.event_name != \u0027schedule\u0027`, which skips nothing on a pull request or a push; a job that must not run on every pull request cannot be a required context (ADR 0022 section 3)."]
  # Rule 10: every needs: target of a required job is a job of this workflow
  # that is itself required, with no job-level if: and no continue-on-error.
  # One line per reason, so that each says what to change.
  + [$w.jobs[] | select(required) as $job
     | ($job | job_ref) as $jw
     | ($job.needs | type) as $nt
     | if $nt != "null" and $nt != "string" and $nt != "array" then
         "\($jw): has a `needs:` that is neither a job id nor a list of job ids (found: \($job.needs | tojson)). Write `needs: <job id>` or `needs: [<job id>, ...]`."
       else
         ($job.needs | needs_list | .[]) as $t
         | if ($by_id | has($t) | not) then
             "\($jw): needs `\($t)`, which is not a job id of this workflow. `needs:` names job ids, not job names, and only jobs of the same file; fix the id."
           else
             ($by_id[$t] | job_ref) as $dw
             | $by_id[$t] as $dep
             | (if ($dep | required | not) then
                  "\($jw): needs \($dw), which is not a required context. GitHub skips a job whose `needs:` failed or was skipped and reports the skipped required job as successful, so a failure of \($dw) would let the pull request merge (ADR 0029 section 4). Drop it from `needs:`."
                else empty end),
               (if $dep.condition != null then
                  "\($jw): needs \($dw), which has the job-level condition `if: \($dep.condition | one_line)`. When the condition skips it, GitHub skips \($jw) as well and reports it as successful (ADR 0029 section 4). Remove the condition or drop the job from `needs:`; a job that a required job needs has no job-level `if:`, the schedule guard included."
                else empty end),
               (if ($dep.continue_on_error | coe_set) then
                  "\($jw): needs \($dw), which has `continue-on-error: \($dep.continue_on_error | coe_text)`. A job that a required job needs must fail the run when it fails (ADR 0029 section 4). Remove `continue-on-error` or drop the job from `needs:`."
                else empty end)
           end
       end]
  # Rule 11: no continue-on-error, other than false, on a required job or on
  # any of its steps.
  + [$w.jobs[] | select(required) as $job
     | ($job | job_ref) as $jw
     | (if ($job.continue_on_error | coe_set) then
          "\($jw): has `continue-on-error: \($job.continue_on_error | coe_text)`, which lets the workflow run pass when the job fails; a required job must fail the gate when it fails (ADR 0029 section 4). Remove it; a job that may fail cannot be a required context (ADR 0022 section 3)."
        else empty end),
       ($job.steps[] | select(.continue_on_error | coe_set)
        | "\($jw), step \(.index + 1)\(if .name then " (\(.name))" else "" end): has `continue-on-error: \(.continue_on_error | coe_text)`, which lets the job pass when this step fails, so the required context would pass with the step failed (ADR 0029 section 4). Remove it.")]
  | .[]
'

# check_file FILE NOT_REQUIRED_JSON: print "FILE: <problem>" for each
# violation in FILE.
#   FILE               the workflow file to check.
#   NOT_REQUIRED_JSON  NOT_REQUIRED_CONTEXTS as a JSON array of strings; the
#                      jobs whose context it lists are exempt from rules 8
#                      to 11.
check_file() {
  local file=$1 not_required_json=$2 json docs err
  # stderr stays out of the JSON: yq prints warnings there even on success
  # (for example about merge keys, which explode(.) applies), and they would
  # break the jq step. Only on failure is yq run again for its error message.
  if ! json=$(yq -o json -I 0 "$SUMMARY_YQ" "$file" 2>/dev/null); then
    err=$(yq -o json -I 0 "$SUMMARY_YQ" "$file" 2>&1 >/dev/null || true)
    printf '%s: is not a valid workflow for yq (%s). Fix the YAML syntax.\n' "$file" "${err//$'\n'/ }"
    return
  fi
  docs=$(printf '%s\n' "$json" | jq -s 'length')
  if [[ $docs != 1 ]]; then
    printf '%s: must contain exactly one YAML document (found %s).\n' "$file" "$docs"
    return
  fi
  printf '%s\n' "$json" | jq -r --arg file "$file" --argjson not_required "$not_required_json" \
    "$RULES_JQ | \"\(\$file): \(.)\""
}

main() {
  if [[ ${1-} == -h || ${1-} == --help ]]; then
    usage
    return 0
  fi
  (($# <= 1)) || die_usage "expected at most one argument, the workflow directory."
  local dir=${1:-.github/workflows}
  [[ -d $dir ]] || die_usage "workflow directory '$dir' does not exist."
  require_tools

  local -a files=()
  local f
  # GitHub reads workflows only from the top level of .github/workflows.
  while IFS= read -r -d '' f; do
    files+=("$f")
  done < <(find "$dir" -maxdepth 1 -type f \( -name '*.yml' -o -name '*.yaml' \) -print0 | sort -z)
  ((${#files[@]} > 0)) || die_usage "no *.yml or *.yaml workflow files in '$dir'."

  # The list as a JSON array of strings for jq (`--args` and `$ARGS` need jq 1.6).
  local not_required_json
  # shellcheck disable=SC2016 # $ARGS is jq syntax, not shell.
  not_required_json=$(jq -cn '$ARGS.positional' --args "${NOT_REQUIRED_CONTEXTS[@]}")

  local problems="" file_problems
  for f in "${files[@]}"; do
    file_problems=$(check_file "$f" "$not_required_json")
    if [[ -n $file_problems ]]; then
      problems+="$file_problems"$'\n'
    fi
  done

  if [[ -z $problems ]]; then
    echo "Workflow policy OK: ${#files[@]} workflow file(s) in $dir comply with ADR 0022 and ADR 0029."
    return 0
  fi
  {
    echo "Workflow policy violations (ADR 0022 sections 1 and 3, ADR 0029):"
    printf '%s' "$problems" | sed 's/^/  - /'
  } >&2
  return 1
}

main "$@"
