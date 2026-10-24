#!/usr/bin/env bash
# check-workflow-policy.sh: enforce the GitHub Actions rules of ADR 0022.
#
# Why this exists: GitHub Actions only runs PR checks here. It never builds or
# pushes a production image, holds no secrets and gets no write access; Pipelines-
# as-Code on the hub is the only production delivery path (ADR 0022 section 1).
# Review alone does not keep it that way, so the "Workflow policy" CI job (a
# required context, ADR 0022 section 3) runs this script on every pull request.
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
# The workflows are parsed with mikefarah yq v4 (preinstalled on GitHub's
# Ubuntu runners), which keeps line comments, so rule 6 can read the version
# comment next to each `uses:`. jq then evaluates the rules on a JSON summary.
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
# `line_comment` returns the comment after a scalar without the leading "# ".
# shellcheck disable=SC2016 # the program is yq syntax, not shell.
readonly SUMMARY_YQ='{
  "triggers": [
    (.on | select(tag == "!!str")),
    (.on | select(tag == "!!seq") | .[]),
    (.on | select(tag == "!!map") | keys | .[])
  ],
  "permissions": .permissions,
  "concurrency": .concurrency,
  "jobs_is_map": ((.jobs | tag) == "!!map"),
  "jobs": [
    (.jobs | select(tag == "!!map") | to_entries | .[]) | {
      "id": .key,
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
          "run": .value.run
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

  . as $w
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
  | .[]
'

# check_file FILE: print "FILE: <problem>" for each violation in FILE.
check_file() {
  local file=$1 json docs
  if ! json=$(yq -o json -I 0 "$SUMMARY_YQ" "$file" 2>&1); then
    printf '%s: is not a valid workflow for yq (%s). Fix the YAML syntax.\n' "$file" "${json//$'\n'/ }"
    return
  fi
  docs=$(printf '%s\n' "$json" | jq -s 'length')
  if [[ $docs != 1 ]]; then
    printf '%s: must contain exactly one YAML document (found %s).\n' "$file" "$docs"
    return
  fi
  printf '%s\n' "$json" | jq -r --arg file "$file" "$RULES_JQ | \"\(\$file): \(.)\""
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

  local problems="" file_problems
  for f in "${files[@]}"; do
    file_problems=$(check_file "$f")
    if [[ -n $file_problems ]]; then
      problems+="$file_problems"$'\n'
    fi
  done

  if [[ -z $problems ]]; then
    echo "Workflow policy OK: ${#files[@]} workflow file(s) in $dir comply with ADR 0022."
    return 0
  fi
  {
    echo "Workflow policy violations (ADR 0022 sections 1 and 3):"
    printf '%s' "$problems" | sed 's/^/  - /'
  } >&2
  return 1
}

main "$@"
