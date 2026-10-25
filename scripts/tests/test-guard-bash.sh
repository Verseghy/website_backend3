#!/usr/bin/env bash
# test-guard-bash.sh: tests for .claude/hooks/guard-bash.sh, the PreToolUse
# hook of the guardrails (ADR 0024 section 4, layer 2, as amended by ADR 0028).
#
# Why it exists
#   The hook runs before every Bash command an agent runs in this repository.
#   A bug in it either lets a destructive command through or blocks legitimate
#   work, so every rule of ADR 0024 section 4 and ADR 0028 is pinned here:
#   each check 1 command in the cargo loco, app-binary, oc exec, oc rsh and
#   oc debug forms; each check 2 target source (inline variables, -h, -d URIs,
#   host= strings, positional URIs, -u, the environment,
#   IMPORT_SOURCE_DATABASE_URL, plus PGHOSTADDR and PGSERVICE); a command with
#   no target; wrapped forms (sh -c, also with `--` and other options,
#   here-strings, scripts piped into a shell, heredocs, also those that
#   oc exec, oc rsh or oc debug run in the cluster, piped through further
#   stages and line breaks too, in pipelines longer than 16 KiB, and as xargs
#   arguments); the command part of oc exec, oc rsh and oc debug (rsh's
#   --shell and -f included), so that pod, container and namespace names, data
#   paths and search patterns do not count; oc delete of what an oc get of a
#   protected kind produces; the words after an unquoted $(...), `...` or
#   <(...), which still belong to the command around it; the checks ADR 0028
#   adds (namespace and project deletion, argocd app delete, app
#   delete-resource and appset delete, finalizer patches and Delete or Recycle
#   reclaim-policy patches, port-forwards to PostgreSQL, MySQL, MariaDB and
#   Redis ports, the mariadb client), each with the commit, PR and search
#   texts that only mention them; the cargo aliases that check 2 counts
#   (cargo run, r, loco-tool and t); allowed commands; a missing jq; the cost
#   of long commands;
#   and the hook entry of .claude/settings.json, which must equal ADR 0028's
#   layer-1 block and run the hook in exec form with "onFailure": "block".
#
# How it works
#   Each case builds a PreToolUse payload with jq (the shape Claude Code sends,
#   see scripts/tests/fixtures/guard-bash/), pipes it into the hook with a
#   controlled environment and asserts the exit code: 0 allows, 2 blocks. For
#   blocked commands it also asserts that stderr carries the hook's reason.
#
# Usage
#   bash scripts/tests/test-guard-bash.sh     (from the repository root)
#   It needs bash and jq, and no network. It exits non-zero if any case fails.

# The cases are literal command text for the hook to inspect, so $(...),
# backticks and "$VAR" inside single quotes must stay unexpanded. (Placed
# before the first command, the directive covers the whole file.)
# shellcheck disable=SC2016

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
hook="$repo_root/.claude/hooks/guard-bash.sh"
fixtures="$repo_root/scripts/tests/fixtures/guard-bash"

if ! command -v jq >/dev/null 2>&1; then
  echo "test-guard-bash.sh: jq is required to build the test payloads; install jq and retry." >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

passed=0
failed=0

# Records one result and prints one TAP-style line, like the other
# scripts/tests/ files.
#   $1 - "ok" or "fail";
#   $2 - the case description (newlines are shown as \n);
#   $3 - details printed for a failure.
report() {
  local n=$((passed + failed + 1)) desc="${2//$'\n'/\\n}"
  if [[ "$1" == ok ]]; then
    passed=$((passed + 1))
    printf 'ok %d - %s\n' "$n" "${desc:0:160}"
  else
    failed=$((failed + 1))
    printf 'not ok %d - %s\n%s\n' "$n" "${desc:0:160}" "$3" | sed '2,$s/^/    # /'
  fi
}

# Builds the PreToolUse payload for one Bash command. The command goes in on
# stdin, not as an argument, because one argument is limited to 128 KiB.
#   $1 - the command text.
payload_for() {
  printf '%s' "$1" | jq -Rs '. as $c | {
    session_id: "00000000-0000-0000-0000-000000000000",
    transcript_path: "transcript.jsonl",
    cwd: ".",
    permission_mode: "default",
    hook_event_name: "PreToolUse",
    tool_name: "Bash",
    tool_input: {command: $c, description: "test", timeout: 120000, run_in_background: false},
    tool_use_id: "toolu_test"
  }'
}

# env options that remove every variable check 2 reads from the environment,
# so a value in the caller's shell cannot change a verdict.
clean_db_env=(-u DATABASE_URL -u PGHOST -u PGHOSTADDR -u PGSERVICE -u MYSQL_HOST -u IMPORT_SOURCE_DATABASE_URL)

# Runs the hook on a payload file with a clean database environment.
#   $1    - the payload file;
#   $2... - extra VAR=value assignments for the hook's environment.
# Sets: status (the exit code) and the files "$work/out" and "$work/err".
run_hook_file() {
  local file="$1"
  shift
  set +e
  env "${clean_db_env[@]}" "$@" "$hook" <"$file" >"$work/out" 2>"$work/err"
  status=$?
  set -e
}

# Asserts the hook's verdict for one command.
#   $1    - the expected exit code (0 allow, 2 block);
#   $2    - the command text;
#   $3... - extra VAR=value assignments for the hook's environment.
expect() {
  local expected="$1" cmd="$2"
  shift 2
  local label="$cmd"
  (($# > 0)) && label="[$*] $cmd"
  payload_for "$cmd" >"$work/payload.json"
  run_hook_file "$work/payload.json" "$@"
  if [[ "$status" != "$expected" ]]; then
    report fail "exit $expected for: $label" "  got exit $status; stderr: $(cat "$work/err")"
  elif [[ "$expected" == 2 ]] && ! grep -q '^Blocked by .claude/hooks/guard-bash.sh' "$work/err"; then
    report fail "reason on stderr for: $label" "  stderr was: $(cat "$work/err")"
  elif [[ "$expected" == 0 ]] && [[ -s "$work/err" || -s "$work/out" ]]; then
    report fail "silent allow for: $label" "  stdout: $(cat "$work/out"); stderr: $(cat "$work/err")"
  else
    report ok "$label" ""
  fi
}

# Shorthands for expect.
#   $1    - the command text;
#   $2... - extra VAR=value assignments for the hook's environment.
blocks() { expect 2 "$@"; }
allows() { expect 0 "$@"; }

# Asserts that a blocked command's reason contains a text.
#   $1 - the command text;
#   $2 - a fixed string the reason must contain.
reason_mentions() {
  payload_for "$1" >"$work/payload.json"
  run_hook_file "$work/payload.json"
  if [[ "$status" == 2 ]] && grep -qF -- "$2" "$work/err"; then
    report ok "reason for '$1' mentions '$2'" ""
  else
    report fail "reason for '$1' mentions '$2'" "  exit $status; stderr: $(cat "$work/err")"
  fi
}

remote_pg="postgres://loco:loco@db.example.org:5432/backend3"
local_pg="postgres://loco:loco@localhost:5432/backend3_test"
remote_mysql="mysql://root@legacy.example.org:3306/legacy"
local_mysql="mysql://root@127.0.0.1:3306/legacy"

# ------------------------------------------------------------- the file itself

if [[ -x "$hook" ]]; then
  report ok "the hook is executable" ""
else
  report fail "the hook is executable" "  chmod +x .claude/hooks/guard-bash.sh"
fi

# ------------------------------------------------------------- the wiring in .claude/settings.json

settings="$repo_root/.claude/settings.json"
adr="$repo_root/docs/design/adr/0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md"
# The layer-1 JSON block of ADR 0028 section 3, which replaces the block of
# ADR 0024 section 4, must be the committed settings.
adr_settings="$(awk '/^\*\*Layer 1: deny rules/{f=1} f && /^```json$/{g=1; next} g && /^```$/{exit} g{print}' "$adr")"
if [[ -n "$adr_settings" ]] && [[ "$(jq -S . <<<"$adr_settings")" == "$(jq -S . "$settings")" ]]; then
  report ok ".claude/settings.json equals the layer-1 block of ADR 0028 section 3" ""
else
  report fail ".claude/settings.json equals the layer-1 block of ADR 0028 section 3" \
    "  diff: $(diff <(jq -S . <<<"$adr_settings" 2>&1) <(jq -S . "$settings" 2>&1) || true)"
fi

# The hook entry (ADR 0028 section 3): one command handler for Bash, in exec
# form with the script itself as the command and an empty args array (so a
# client that predates args still starts the hook in shell form, rather than
# a bare `bash` reading the JSON input as a script), with "onFailure":
# "block", and not a background hook, on which onFailure has no effect
# (Claude Code docs, "Hooks reference", "Block the action when a hook fails").
handlers="$(jq -c '[.hooks.PreToolUse[]? | select(.matcher == "Bash") | .hooks[]?]' "$settings")"
if jq -e 'length == 1 and (.[0] | .type == "command" and .args == []
          and (.command | type == "string" and endswith("/.claude/hooks/guard-bash.sh"))
          and .onFailure == "block" and (has("async") | not) and (has("asyncRewake") | not))' \
  <<<"$handlers" >/dev/null; then
  report ok "the Bash hook entry is one exec-form command hook with onFailure block" ""
else
  report fail "the Bash hook entry is one exec-form command hook with onFailure block" "  handlers: $handlers"
fi

# Runs the Bash hook entry of settings.json the way Claude Code runs an
# exec-form command hook (Claude Code docs, "Hooks reference", "Exec form and
# shell form"): ${CLAUDE_PROJECT_DIR} is substituted into command and into
# every args element as a plain string, the result is spawned directly with no
# shell, and CLAUDE_PROJECT_DIR is exported to the process.
#   $1 - the project directory; $2 - the payload file.
# Sets: status (the exit code).
run_wired_hook() {
  local dir="$1" payload="$2" word
  local -a argv=()
  while IFS= read -r -d '' word; do argv+=("$word"); done < <(
    jq -j --arg dir "$dir" '.hooks.PreToolUse[] | select(.matcher == "Bash") | .hooks[0]
      | ([.command] + .args)[] | (split("${CLAUDE_PROJECT_DIR}") | join($dir)) + "\u0000"' "$settings")
  set +e
  env "${clean_db_env[@]}" CLAUDE_PROJECT_DIR="$dir" "${argv[@]}" <"$payload" >/dev/null 2>&1
  status=$?
  set -e
}

# The project directory may hold a space: exec form passes the path as one
# argument, which the old shell form needed quotes for.
spaced_project="$work/project dir"
mkdir -p "$spaced_project/.claude/hooks"
cp -p "$hook" "$spaced_project/.claude/hooks/guard-bash.sh"
for project in "$repo_root" "$spaced_project"; do
  where="the repository"
  [[ "$project" == "$repo_root" ]] || where="a project path with a space"
  label="the exec-form hook entry runs the hook from $where"
  run_wired_hook "$project" "$fixtures/pretooluse-bash-kubectl.json"
  wired_block=$status
  run_wired_hook "$project" "$fixtures/pretooluse-bash-allowed.json"
  wired_allow=$status
  if [[ "$wired_block" == 2 && "$wired_allow" == 0 ]]; then
    report ok "$label" ""
  else
    report fail "$label" "  exits: $wired_block (want 2) and $wired_allow (want 0); handlers: $handlers"
  fi
done

# ------------------------------------------------------------- hook input

# Asserts the exit code of the last run_hook_file.
#   $1 - the case name; $2 - the expected exit code.
expect_status() {
  if [[ "$status" == "$2" ]]; then
    report ok "$1" ""
  else
    report fail "$1" "  exit $status, expected $2; stderr: $(cat "$work/err")"
  fi
}

run_hook_file "$fixtures/pretooluse-bash-kubectl.json"
expect_status "full payload fixture with kubectl is blocked" 2
run_hook_file "$fixtures/pretooluse-bash-allowed.json"
expect_status "full payload fixture with the test command is allowed" 0
run_hook_file "$fixtures/pretooluse-write-tool.json"
expect_status "a payload for another tool is allowed" 0
run_hook_file "$fixtures/pretooluse-bash-no-command.json"
expect_status "a Bash payload without a command is allowed" 0
run_hook_file "$fixtures/invalid-json.txt"
if [[ "$status" == 2 ]] && grep -q 'could not read the hook input as JSON' "$work/err"; then
  report ok "invalid JSON input fails closed" ""
else
  report fail "invalid JSON input fails closed" "  exit $status; $(cat "$work/err")"
fi
: >"$work/empty"
run_hook_file "$work/empty"
expect_status "empty input fails closed" 2

# Missing jq: a PATH that holds bash (for the #!/usr/bin/env bash line) and
# nothing else, so `command -v jq` fails inside the hook.
mkdir -p "$work/nojq-bin"
ln -s "$(command -v bash)" "$work/nojq-bin/bash"
set +e
PATH="$work/nojq-bin" "$hook" <"$fixtures/pretooluse-bash-allowed.json" >"$work/out" 2>"$work/err"
status=$?
set -e
if [[ "$status" == 2 ]] && grep -q 'jq is not installed or not on PATH' "$work/err"; then
  report ok "a missing jq fails closed with an actionable reason" ""
else
  report fail "a missing jq fails closed with an actionable reason" "  exit $status; $(cat "$work/err")"
fi

# ------------------------------------------------------------- allowed commands

allows 'git status'
allows 'ls -la'
allows 'npm --prefix admin run build'
allows 'cargo fmt --all --check'
allows 'cargo clippy --locked --workspace --all-targets -- -D warnings'
allows 'podman-compose up -d postgres'
allows 'oc get pods -n web'
allows "echo 'hello world' | wc -c"

# ------------------------------------------------------------- check 1: kubectl

blocks 'kubectl get pods'
blocks '/usr/local/bin/kubectl get pods'
blocks 'KUBECONFIG=kubeconfig kubectl get ns'
blocks 'sudo kubectl get pods'
blocks 'xargs -n1 kubectl delete pod'
blocks 'timeout 30 kubectl get pods'
blocks 'echo start && kubectl apply -f deploy.yaml'
blocks "sh -c 'kubectl delete pod web-0'"
blocks 'bash -lc "kubectl get pods"'
blocks 'echo "$(kubectl version)"'
blocks 'echo `kubectl version`'
blocks "ku''bectl get pods"
blocks 'oc exec web-0 -- kubectl get pods'
blocks "watch 'kubectl get pods'"
blocks 'for p in a b; do kubectl delete pod "$p"; done'
allows 'grep -rn kubectl docs/design'
allows "grep -rn 'kubectl get' docs/design"
allows "git commit -m 'docs(docs): say use oc, never kubectl'"
allows 'oc get pods'
reason_mentions 'kubectl get pods' 'use oc, never kubectl'

# ------------------------------------------------------------- check 1: force delete

blocks 'oc delete pod web-0 --force --grace-period=0'
blocks 'oc delete pod web-0 --grace-period=0 --force'
blocks 'oc delete pod web-0 --grace-period 0 --force'
blocks "sh -c 'oc delete pod web-0 --force --grace-period=0'"
allows 'oc delete pod web-0'
blocks 'oc delete pod web-0 --force'
allows 'oc delete pod web-0 --grace-period=30'
allows 'git push --force-with-lease'
# kubectl turns --force without a grace period into --grace-period=0
# (k8s.io/kubectl pkg/cmd/delete), and pflag reads bool values with
# strconv.ParseBool, so every true spelling counts.
blocks 'oc delete pod web-0 --force --grace-period=-1'
blocks 'oc -n web delete pod web-0 --force'
blocks 'oc delete --force pod/web-0'
for spelling in 1 t T TRUE true True; do
  blocks "oc delete pod web-0 --force=$spelling --grace-period=0"
  blocks "oc delete pod web-0 --force=$spelling"
done
blocks 'oc replace --force -f pod.yaml'
blocks "sh -c -- 'oc delete pod web-0 --force'"
allows 'oc delete pod web-0 --force=false'
allows 'oc delete pod web-0 --force=0'
allows 'git push --force'
allows 'cargo install --force --locked sea-orm-cli@2.0.4'
allows 'podman rm --force web'
allows 'oc adm drain edge --force --grace-period=30'
reason_mentions 'oc delete pod web-0 --force --grace-period=0' 'Force-deleting pods is never allowed'

# ------------------------------------------------------------- check 1: oc delete of PVC, PV, Application

blocks 'oc delete pvc data-postgres-0'
blocks 'oc delete pvc/data-postgres-0'
blocks 'oc delete pvcs --all -n web'
blocks 'oc delete persistentvolumeclaim data'
blocks 'oc delete PersistentVolumeClaims data'
blocks 'oc delete persistentvolumeclaim.v1/data'
blocks 'oc -n web delete pv pv-1'
blocks 'oc delete pv/pv-1'
blocks 'oc delete persistentvolume pv-1'
blocks 'oc delete persistentvolumes pv-1'
blocks 'oc delete application backend3 -n openshift-gitops'
blocks 'oc delete applications backend3'
blocks 'oc delete applications.argoproj.io backend3'
blocks 'oc delete app backend3'
blocks 'oc delete applicationset website'
blocks 'oc delete pods,pvc --all'
blocks 'oc delete --wait=false -n web pvc data'
blocks "bash -c 'oc delete pvc data'"
blocks 'oc exec toolbox -- oc delete pvc data'
allows 'oc get pvc'
allows 'oc describe pvc data'
allows 'oc delete deployment.apps/web'
allows 'oc delete pod pv-builder'
allows 'oc delete pod web -n apps'
allows 'oc delete configmap application-config'
blocks 'oc delete pod/web-0 pvc/data'
blocks 'oc delete pods/web-0,pvc/data'
blocks 'oc --cache-dir .kube-cache delete pvc data'
blocks 'oc --profile cpu delete pvc data'
blocks 'oc --log-flush-frequency 5s delete pvc data'
blocks 'oc --as-user-extra k=v --kuberc kuberc.yaml delete pv pv-1'
allows 'oc delete deployment app -n verseghy'
allows 'oc delete deployment pvc'
allows 'oc delete configmap app pv pvc'
reason_mentions 'oc delete pvc data' 'reclaim policy is Retain'

# ------------------------------------------------------------- check 1: oc delete of what an oc get of PVC, PV, Application produces

# The resources come from the output of an oc get of a protected kind:
# through $(...) (quoted, unquoted or as backticks), a variable, xargs,
# a pipe into -f - or a process substitution.
blocks 'oc delete $(oc get pvc -o name)'
blocks 'oc delete -n web $(oc get pvc -n web -o name)'
blocks 'oc delete "$(oc get persistentvolumeclaims -o name)"'
blocks 'oc delete `oc get pv -o name`'
blocks 'oc delete pod/web-0 $(oc get pvc -o name)'
blocks 'oc get pvc -o name | xargs oc delete'
blocks 'oc get persistentvolume.v1 -o name | xargs -r -n1 oc -n web delete'
blocks 'oc get pvc -o name | grep data | xargs -I{} oc delete {}'
blocks "oc get pvc -o name | xargs -I{} sh -c 'oc delete {}'"
blocks 'oc get pods,pvc -o name | xargs oc delete'
blocks 'oc get pvc/data-0 -o name | parallel oc delete'
blocks 'for r in $(oc get applications -n openshift-gitops -o name); do oc delete -n openshift-gitops "$r"; done'
blocks 'oc get applicationsets.argoproj.io -o name | while read -r r; do oc delete "$r"; done'
blocks 'names=$(oc get app -o name); oc delete $names'
blocks 'oc get pvc -o yaml | oc delete -f -'
blocks 'oc get pvc -o yaml | oc delete --filename=/dev/stdin'
blocks 'oc delete -f <(oc get pv -o yaml)'
# Options after the substitution do not hide it.
blocks 'oc delete -f <(oc get pv -o yaml) --wait=false'
blocks 'oc delete `oc get pv -o name` --wait=false'
blocks 'oc delete $(oc get pvc -o name) --wait=false'
blocks "sh -c 'oc get pvc -o name | xargs oc delete'"
# The producer names another kind, the oc delete fixes its type in the text
# (kubectl refuses to mix `pod` with the `persistentvolumeclaim/...` names),
# or nothing deletes the output.
allows 'oc delete $(oc get pods -o name)'
allows 'oc get deployments -o name | xargs oc delete'
allows 'oc get pods data-pvc -o name | xargs oc delete'
allows 'oc delete pod $(oc get pvc -o name)'
allows 'oc delete pod `oc get pvc -o name` --wait=false'
allows 'oc get pvc -o name | xargs oc describe'
allows 'oc get pvc && oc delete pod web-0'
allows 'oc get pvc; oc delete pod "$POD" -n web'
# xargs and parallel append their input words. kubectl accepts appended
# TYPE/NAME words after a first argument in TYPE/NAME form, and refuses them
# after a type written out on its own; a replace string stands for the type.
blocks 'oc get pvc -o name | xargs oc delete pod/web-0'
blocks 'oc get pvc -o name | parallel oc delete pod/web-0'
blocks 'oc get pvc -o name | xargs -I X oc delete X'
blocks 'oc get pv -o name | xargs --replace=R oc delete R'
allows 'oc get pvc -o name | xargs oc delete pod'
allows 'oc get pvc -n verseghy && oc get pods --no-headers | awk '\''/Evicted/{print $1}'\'' | xargs oc delete pod -n verseghy'
reason_mentions 'oc get pvc -o name | xargs oc delete' 'runs `oc get pvc`'

# ------------------------------------------------------------- check 1: oc delete of namespaces and projects (ADR 0028)

# Deleting a namespace or an OpenShift project deletes everything in it, its
# PVCs included, so both count like a PVC in every spelling and position.
blocks 'oc delete namespace web'
blocks 'oc delete namespaces web'
blocks 'oc delete ns web'
blocks 'oc delete ns/web'
blocks 'oc delete namespace/web'
blocks 'oc delete Namespace web'
blocks 'oc delete namespace.v1/web'
blocks 'oc delete project web'
blocks 'oc delete projects web'
blocks 'oc delete project/web'
blocks 'oc delete projects.project.openshift.io web'
blocks 'oc delete pods,ns --all'
blocks 'oc delete pod/web-0 project/web'
blocks 'oc -n web delete ns verseghy-staging'
blocks "sh -c 'oc delete project web'"
blocks 'oc exec toolbox -- oc delete ns web'
blocks 'oc get ns -o name | grep staging | xargs oc delete'
blocks 'oc delete $(oc get projects -o name)'
blocks 'for n in $(oc get namespaces -o name); do oc delete "$n"; done'
allows 'oc get ns'
allows 'oc get projects'
allows 'oc describe namespace web'
allows 'oc project web'
allows 'oc delete pod web-0 -n web'
allows 'oc delete pod web-0 --namespace=web'
allows 'oc delete configmap ns-config'
allows 'oc delete deployment project'
allows 'oc get ns && oc delete pod "$POD" -n web'
allows "git commit -m 'fix(deploy): block oc delete namespace and oc delete project'"
allows "gh pr create --title 'fix(deploy): widen the guardrails' --body 'oc delete ns web is blocked now'"
allows "grep -rn 'oc delete project' docs"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block namespace deletes\n\noc delete ns web\noc delete project web\nEOF'
reason_mentions 'oc delete ns web' 'namespace or project'
reason_mentions 'oc delete project web' 'deletes everything in it'

# ------------------------------------------------------------- check 1: argocd app delete (ADR 0028)

blocks 'argocd app delete backend3'
blocks 'argocd app delete backend3 --cascade=false'
blocks 'argocd app delete backend3 -y'
blocks 'argocd app delete -l app.kubernetes.io/part-of=website'
blocks 'argocd app delete openshift-gitops/backend3 --app-namespace openshift-gitops'
blocks 'argocd --grpc-web --server argocd.example.org app delete backend3'
blocks 'argocd --core app delete backend3'
blocks 'argocd app --server argocd.example.org delete backend3'
blocks '/usr/local/bin/argocd app delete backend3'
# An option the hook does not know cannot hide the subcommands: before `app`
# its value is only one more positional word, and between `app` and `delete`
# it may take the word after it as its value.
blocks 'argocd --some-new-option value app delete backend3'
blocks 'argocd app --some-new-option v delete backend3'
blocks 'argocd app --some-new-option delete backend3'
blocks 'argocd app --grpc-web --some-new-option v delete backend3'
blocks "sh -c 'argocd app delete backend3'"
blocks 'echo backend3 | xargs argocd app delete'
blocks 'oc exec toolbox -- argocd app delete backend3'
# The hook skips each option of argocd_value_options together with its value.
# An unknown option alone is covered by the rule above, so each listed option
# is tested next to one: if a listed option were dropped from the list, its
# value would be a second positional word before the subcommand, and the
# subcommand would be missed. argocd's global value options (root.go at
# argoproj/argo-cd@17b98d9b6) count for all three deleting commands; each
# subcommand's own value options are tested in its section.
argocd_global_value_options=(--config --server --server-crt --client-crt --client-crt-key --auth-token
  --grpc-web-root-path --logformat --loglevel --header -H --port-forward-namespace --http-retry-max
  --argocd-context --server-name --controller-name --redis-haproxy-name --redis-name
  --repo-server-name --redis-compress --kube-context)
for opt in "${argocd_global_value_options[@]}"; do
  blocks "argocd app --some-new-option v $opt x delete backend3"
  blocks "argocd app --some-new-option v $opt x delete-resource backend3 --kind Pod"
  blocks "argocd appset --some-new-option v $opt x delete website"
done
for opt in --propagation-policy -p --selector -l --app-namespace -N; do
  blocks "argocd app --some-new-option v $opt x delete backend3"
done
# A boolean option takes no value, so it must never be in argocd_value_options:
# listed, it would take the subcommand after it as its value. argocd's global
# boolean options (root.go) before each deleting subcommand pin that.
for opt in --plaintext --insecure --grpc-web --port-forward --core --prompts-enabled; do
  blocks "argocd app $opt delete backend3"
  blocks "argocd app $opt delete-resource backend3 --kind Pod"
  blocks "argocd appset $opt delete website"
done
allows 'argocd app list'
allows 'argocd app get backend3'
allows 'argocd app sync backend3'
allows 'argocd app diff backend3'
allows 'argocd app get delete'
allows 'argocd app --some-new-option=v get delete'
allows 'argocd app --server argocd.example.org get backend3 delete'
allows 'argocd proj list'
allows "git commit -m 'docs(docs): argocd app delete is blocked'"
allows "gh pr comment 204 --body 'never run argocd app delete'"
allows "grep -rn 'argocd app delete' docs"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block argocd app delete\n\nargocd app delete backend3\nEOF'
reason_mentions 'argocd app delete backend3' 'Argo CD Application'

# ------------------------------------------------------------- check 1: argocd app delete-resource and appset delete (D69)

# delete-resource deletes managed resources of any kind, a PVC included, and
# appset delete deletes the ApplicationSet's Applications, so every kind,
# option and position counts, as for app delete.
blocks 'argocd app delete-resource backend3 --kind PersistentVolumeClaim --resource-name data'
blocks 'argocd app delete-resource backend3 --kind Pod --resource-name web-0'
blocks 'argocd app delete-resource backend3 --kind Deployment --all --orphan'
blocks 'argocd app delete-resource openshift-gitops/backend3 --kind=ConfigMap --namespace=web'
blocks 'argocd --grpc-web --server argocd.example.org app delete-resource backend3 --kind Pod'
blocks 'argocd app --server argocd.example.org delete-resource backend3 --kind Pod'
# delete-resource's own value options before it are skipped with their value:
# cobra reads an option that `app` does not define as taking the next word, so
# argocd runs these as delete-resource. Two of them before the subcommand, or
# one next to an unknown option, leave two words that are not the subcommand
# when the hook does not know the option.
blocks 'argocd app --kind Pod delete-resource backend3'
blocks 'argocd app --kind Pod --resource-name data delete-resource backend3'
blocks 'argocd app --project default --namespace web delete-resource backend3 --kind Pod'
blocks 'argocd app --group apps --kind Deployment delete-resource backend3'
for opt in --resource-name --kind --group --namespace --project; do
  blocks "argocd app --some-new-option v $opt x delete-resource backend3"
done
blocks 'argocd --some-new-option value app delete-resource backend3'
blocks 'argocd app --some-new-option v delete-resource backend3 --kind Pod'
blocks 'argocd app --some-new-option delete-resource backend3 --kind Pod'
blocks 'argocd appset delete website'
blocks 'argocd appset delete website website-staging -y'
blocks 'argocd appset delete openshift-gitops/website --yes'
blocks 'argocd --core appset delete website'
blocks 'argocd --server argocd.example.org --grpc-web appset delete website'
blocks 'argocd appset --server argocd.example.org delete website'
blocks 'argocd --some-new-option value appset delete website'
blocks 'argocd appset --some-new-option v delete website'
blocks 'argocd appset --some-new-option delete website'
blocks '/usr/local/bin/argocd appset delete website'
blocks "sh -c 'argocd appset delete website'"
blocks "bash -c 'argocd app delete-resource backend3 --kind PersistentVolumeClaim'"
blocks 'echo website | xargs argocd appset delete'
blocks 'echo backend3 | xargs -I{} argocd app delete-resource {} --kind Pod'
blocks 'oc exec toolbox -- argocd appset delete website'
blocks $'bash <<\'EOF\'\nargocd appset delete website\nEOF'
blocks 'argocd appset delete $(argocd appset list -o name)'
allows 'argocd app get-resource backend3 --kind Pod --resource-name web-0'
allows 'argocd app patch-resource backend3 --kind Deployment --resource-name web --patch "{}"'
allows 'argocd app resources backend3'
allows 'argocd app get delete-resource'
allows 'argocd app --some-new-option=v get delete-resource'
allows 'argocd appset list'
allows 'argocd appset get website'
allows 'argocd appset get delete'
allows 'argocd appset --some-new-option=v get delete'
allows 'argocd appset generate appset.yaml'
allows 'argocd proj delete-resource'
allows "git commit -m 'fix(deploy): block argocd app delete-resource and argocd appset delete'"
allows "gh pr create --title 'fix(deploy): widen the guardrails' --body 'argocd appset delete website is blocked'"
allows "grep -rn 'argocd appset delete' docs"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block argocd appset delete\n\nargocd appset delete website\nargocd app delete-resource backend3 --kind Pod\nEOF'
reason_mentions 'argocd app delete-resource backend3 --kind PersistentVolumeClaim' '`argocd app delete-resource`'
reason_mentions 'argocd app delete-resource backend3 --kind PersistentVolumeClaim' 'a PersistentVolumeClaim included'
reason_mentions 'argocd appset delete website' '`argocd appset delete`'
reason_mentions 'argocd appset delete website' 'all its Applications'
reason_mentions 'argocd appset delete website' 'Stop and ask the maintainer'

# ------------------------------------------------------------- check 1: oc patch of finalizers and reclaim policies (ADR 0028)

# Any change to finalizers counts, in every patch type, because the hook
# cannot tell a removal from an addition in all of them.
blocks "oc patch pvc data -p '{\"metadata\":{\"finalizers\":null}}'"
blocks "oc patch pv pv-1 --type=merge -p '{\"metadata\":{\"finalizers\":[]}}'"
blocks "oc patch application backend3 -n openshift-gitops --type json -p '[{\"op\":\"remove\",\"path\":\"/metadata/finalizers\"}]'"
blocks "oc patch ns web --patch='{\"metadata\":{\"finalizers\":null}}'"
blocks "oc patch namespace web -p '{\"spec\":{\"finalizers\":[]}}'"
blocks "oc patch pvc data -p '{\"metadata\":{\"\$deleteFromPrimitiveList/finalizers\":[\"kubernetes.io/pvc-protection\"]}}'"
blocks "oc patch pvc/data --type json -p='[{\"op\":\"add\",\"path\":\"/metadata/finalizers/-\",\"value\":\"x\"}]'"
blocks "oc patch pvc data -p '{\"metadata\":{\"Finalizers\":null}}'"
blocks "oc -n web patch pvc data -p '{\"metadata\":{\"finalizers\":null}}'"
blocks "oc exec toolbox -- oc patch pvc data -p '{\"metadata\":{\"finalizers\":null}}'"
# A reclaim policy of Delete, or one only the shell knows.
blocks "oc patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Delete\"}}'"
blocks "oc patch pv/pv-1 --type json -p '[{\"op\":\"replace\",\"path\":\"/spec/persistentVolumeReclaimPolicy\",\"value\":\"Delete\"}]'"
blocks "oc patch pv pv-1 -p '{\"spec\":{\"persistentvolumereclaimpolicy\":\"delete\"}}'"
blocks "oc patch pv pv-1 -p 'spec: {persistentVolumeReclaimPolicy: Delete}'"
blocks 'oc patch pv pv-1 -p "{\"spec\":{\"persistentVolumeReclaimPolicy\":\"$POLICY\"}}"'
blocks "bash -c 'oc patch pv pv-1 -p \"{\\\"spec\\\":{\\\"persistentVolumeReclaimPolicy\\\":\\\"Delete\\\"}}\"'"
blocks "oc get pv -o name | xargs -I{} oc patch {} -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Delete\"}}'"
# The deprecated Recycle wipes the volume's data when its claim is released
# (D69), in every patch type, case and wrapper, like Delete.
blocks "oc patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'"
blocks "oc patch pv pv-1 --type=merge -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'"
blocks "oc patch pv/pv-1 --type json -p '[{\"op\":\"replace\",\"path\":\"/spec/persistentVolumeReclaimPolicy\",\"value\":\"Recycle\"}]'"
blocks "oc patch pv pv-1 --patch='{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'"
blocks "oc patch pv pv-1 -p '{\"spec\":{\"persistentvolumereclaimpolicy\":\"recycle\"}}'"
blocks "oc patch pv pv-1 -p 'spec: {persistentVolumeReclaimPolicy: Recycle}'"
blocks "oc -n web patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'"
blocks "sh -c 'oc patch pv pv-1 -p \"{\\\"spec\\\":{\\\"persistentVolumeReclaimPolicy\\\":\\\"Recycle\\\"}}\"'"
blocks "oc exec toolbox -- oc patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'"
blocks "oc get pv -o name | xargs -I{} oc patch {} -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'"
# Retain is what a human asks agents to set; a word elsewhere in the command,
# such as a PV named pv-delete-01 or pv-recycle-01, does not count.
allows "oc patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Retain\"}}'"
allows "oc patch pv pv-delete-01 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Retain\"}}'"
allows "oc patch pv pv-recycle-01 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Retain\"}}'"
allows "oc patch deployment recycler -p '{\"spec\":{\"replicas\":0}}'"
allows "for pv in pv-1 pv-2; do oc patch pv \"\$pv\" -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Retain\"}}'; done"
allows "oc patch deployment web -p '{\"spec\":{\"replicas\":2}}'"
allows "oc patch deployment web --type json -p '[{\"op\":\"remove\",\"path\":\"/spec/template/spec/containers/0/env/1\"}]'"
allows "oc get pvc data -o jsonpath='{.metadata.finalizers}'"
allows "oc get pv -o custom-columns=NAME:.metadata.name,POLICY:.spec.persistentVolumeReclaimPolicy"
allows "git commit -m 'fix(deploy): block an oc patch that removes finalizers'"
allows "gh pr create --body 'oc patch pv -p persistentVolumeReclaimPolicy Delete is blocked'"
allows 'grep -rn persistentVolumeReclaimPolicy docs'
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block finalizer patches\n\noc patch pvc data -p \'{"metadata":{"finalizers":null}}\'\nEOF'
reason_mentions "oc patch pvc data -p '{\"metadata\":{\"finalizers\":null}}'" 'changes finalizers'
reason_mentions "oc patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Delete\"}}'" 'Retain is allowed'
reason_mentions "oc patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'" 'Delete or Recycle'
reason_mentions "oc patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'" 'data wiped'
allows "git commit -m 'fix(deploy): block an oc patch to the Recycle reclaim policy'"
allows "gh pr comment 216 --body 'oc patch pv -p persistentVolumeReclaimPolicy Recycle is blocked'"

# ------------------------------------------------------------- check 1: data-wiping db subcommands

# Every destructive subcommand in every form of ADR 0024 section 4.
db_subcommands=('db reset' 'db down' 'db down 2' 'db truncate' 'db seed --reset' 'db seed -r' 'db seed --from src/fixtures -r')
db_forms=(
  'cargo loco %s'
  'cargo loco -e test %s'
  'backend3-cli %s'
  './target/debug/backend3-cli %s'
  'LOCO_ENV=production /app/backend3-cli %s'
  'oc exec deploy/backend3 -- backend3-cli %s'
  'oc exec -it backend3-0 -c backend3 -- /app/backend3-cli %s'
  'oc rsh backend3-0 /app/backend3-cli %s'
  'oc rsh -n web backend3-0 backend3-cli %s'
  'oc debug deploy/backend3 -- /app/backend3-cli %s'
  "sh -c 'cargo loco %s'"
  'bash -lc "backend3-cli %s"'
  'echo go && cargo loco %s'
  "oc exec deploy/backend3 -- sh -c 'backend3-cli %s'"
  "sh -c -- 'cargo loco %s'"
  "bash <<< 'backend3-cli %s'"
  "echo 'backend3-cli %s' | sh"
  "oc exec -i deploy/backend3 -- sh <<< 'backend3-cli %s'"
  $'oc rsh backend3-0 <<\'EOF\'\nbackend3-cli %s\nEOF'
  # What oc exec feeds the pod counts as a script even when the program in
  # the pod is not a shell, as it does for a heredoc: piped, also through
  # further stages, or as a here-string. (A shell receiver such as `-- sh` or
  # oc rsh was caught before; these forms pin the in-cluster stdin path.)
  "echo 'backend3-cli %s' | oc exec -i deploy/backend3 -- /entrypoint.sh"
  "printf 'backend3-cli %s' | tee run.log | oc exec -i deploy/backend3 -- /entrypoint.sh"
  "oc exec -i deploy/backend3 -- /entrypoint.sh <<< 'backend3-cli %s'"
)
for sub in "${db_subcommands[@]}"; do
  for form in "${db_forms[@]}"; do
    # shellcheck disable=SC2059 # the form is the format string by design
    blocks "$(printf "$form" "$sub")"
  done
done
blocks 'cargo loco db --environment development reset'
blocks 'cargo loco db -e test truncate'
blocks 'echo "$(cargo loco db reset)"'
blocks "eval 'cargo loco db reset'"
blocks "ssh bastion 'backend3-cli db truncate'"
blocks 'timeout 60 cargo loco db reset'
blocks $'bash <<\'EOF\'\ncargo loco db reset\nEOF'
blocks $'cat <<\'EOF\' | sh\ncargo loco db reset\nEOF'
blocks $'cat <<EOF\n$(cargo loco db reset)\nEOF'
allows 'cargo loco db migrate'
allows 'cargo loco db status'
allows 'cargo loco db seed'
allows 'cargo loco db seed --dump'
allows 'cargo loco db entities'
allows 'cargo loco db create'
allows 'backend3-cli db migrate'
allows 'oc exec deploy/backend3 -- backend3-cli db migrate'
allows "git commit -m 'feat(api): block cargo loco db reset in every form'"
allows "grep -rn 'db reset' docs/design"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block db reset\n\nkubectl is banned; oc delete pvc too.\nEOF'
allows $'git commit -m "$(cat <<\'EOF\'\nci(deploy): mention cargo loco db reset\n\nEOF\n)"'
allows $'cat > notes.md <<\'EOF\'\n- `kubectl get pods`\n- cargo loco db reset\nEOF'
reason_mentions 'cargo loco db reset' 'cargo loco db migrate'
reason_mentions 'backend3-cli db truncate' 'db truncate'
reason_mentions 'cargo loco db seed --reset' 'db seed --reset'

# sea-orm-cli migrate fresh, refresh, reset and down.
for sub in fresh refresh reset down; do
  blocks "sea-orm-cli migrate $sub"
  blocks "sea-orm-cli migrate -d ./migration $sub"
  blocks "sea-orm-cli migrate -u $local_pg $sub"
  blocks "oc exec deploy/backend3 -- sea-orm-cli migrate $sub"
  blocks "oc rsh backend3-0 sea-orm-cli migrate $sub"
  blocks "oc debug deploy/backend3 -- sea-orm-cli migrate $sub"
  blocks "sh -c 'sea-orm-cli migrate $sub'"
  blocks "sh -c -- 'sea-orm-cli migrate $sub'"
  blocks "echo 'sea-orm-cli migrate $sub' | bash"
  blocks "echo 'sea-orm-cli migrate $sub' | oc exec -i deploy/backend3 -- /entrypoint.sh"
done
allows "sea-orm-cli migrate up -u $local_pg"
allows 'sea-orm-cli migrate generate add_labels'
reason_mentions 'sea-orm-cli migrate fresh' 'new forward migration'

# ------------------------------------------------------------- check 2: inline variables

blocks "PGHOST=db.example.org psql -c 'select 1'"
blocks "DATABASE_URL=$remote_pg cargo test"
blocks "DATABASE_URL=$remote_pg cargo nextest run"
blocks "DATABASE_URL=$remote_pg cargo loco db migrate"
blocks "PGHOST=db.example.org cargo test"
blocks "MYSQL_HOST=legacy.example.org mysql -e 'select 1'"
blocks "export DATABASE_URL=$remote_pg; cargo loco db migrate"
blocks "env DATABASE_URL=$remote_pg cargo nextest run"
blocks 'DATABASE_URL="$PROD_DATABASE_URL" cargo test'
blocks "DATABASE_URL=$local_pg psql -h db.example.org"
blocks "PGHOST=localhost psql $remote_pg"
allows "PGHOST=localhost psql -c 'select 1'"
allows "PGHOST=/var/run/postgresql psql -c 'select 1'"
allows "DATABASE_URL=$local_pg cargo nextest run --locked --workspace --all-targets"
allows 'DATABASE_URL=postgres://loco:loco@127.0.0.1:5432/backend3_test cargo test --locked --workspace --doc'
allows 'DATABASE_URL=postgres://loco:loco@[::1]:5432/backend3_test cargo test'
allows "MYSQL_HOST=127.0.0.1 mysql -e 'select 1'"
reason_mentions "PGHOST=db.example.org psql" "an inline PGHOST= assignment"

# ------------------------------------------------------------- check 2: -h / --host

blocks 'psql -h db.example.org -U loco backend3'
blocks 'psql -hdb.example.org'
blocks 'psql --host=db.example.org'
blocks 'psql --host db.example.org'
blocks 'psql -h localhost,db.example.org'
blocks 'psql -h "$DB_HOST"'
blocks 'pg_restore -h db.example.org -d backend3 backup.dump'
blocks 'mysql -h legacy.example.org -u root legacy'
blocks 'mysql -hlegacy.example.org legacy'
allows 'psql -h localhost -U loco backend3_development'
allows 'psql -h LOCALHOST'
allows 'psql -h 127.0.0.1'
allows 'psql -h ::1'
allows 'psql -h /var/run/postgresql'
allows 'pg_restore -h localhost -d backend3_development backup.dump'
allows 'mysql -h 127.0.0.1 -u root legacy'
allows 'mysql --host=localhost legacy'
reason_mentions 'psql -h db.example.org' 'from the -h option'

# ------------------------------------------------------------- check 2: -d / --dbname URIs

blocks 'psql -d postgres://loco@db.example.org/backend3'
blocks 'psql --dbname=postgresql://loco:loco@db.example.org:5432/backend3'
blocks 'psql -dpostgres://db.example.org/backend3'
blocks 'pg_restore -d postgres://db.example.org/backend3 backup.dump'
blocks 'psql -d "$DATABASE"'
allows 'psql -d postgresql://loco:loco@localhost:5432/backend3_development'
allows 'pg_restore -d backend3_development backup.dump'
allows 'psql -d backend3_development'

# ------------------------------------------------------------- check 2: host= connection strings

blocks 'psql "host=db.example.org dbname=backend3"'
blocks "psql -d 'dbname=backend3 host=db.example.org user=loco'"
blocks "psql --dbname='hostaddr=192.0.2.10 dbname=backend3'"
blocks "pg_restore -d 'host=db.example.org dbname=backend3' backup.dump"
allows 'psql "host=localhost dbname=backend3_development"'
allows "psql -d 'host=/var/run/postgresql dbname=backend3_development'"
allows "psql \"host='localhost' dbname=backend3_development\""
reason_mentions 'psql "host=db.example.org dbname=backend3"' 'a host= connection key'

# ------------------------------------------------------------- check 2: positional URIs

blocks "psql $remote_pg"
blocks "psql 'postgres://loco@localhost/backend3?host=db.example.org'"
blocks "psql 'postgresql://localhost,db.example.org/backend3'"
blocks "psql 'postgres://loco:p%40ss@db.example.org/backend3'"
blocks "mysql $remote_mysql"
allows "psql $local_pg"
allows "psql 'postgres://loco@[::1]:5432/backend3_test'"
allows "psql 'postgres:///backend3_test?host=/var/run/postgresql'"
allows "psql 'postgresql://%2Fvar%2Frun%2Fpostgresql/backend3_test'"
reason_mentions "psql $remote_pg" 'a connection URI'

# ------------------------------------------------------------- check 2: sea-orm-cli -u / --database-url

blocks "sea-orm-cli migrate up -u $remote_pg"
blocks "sea-orm-cli generate entity --database-url $remote_pg -o src/models/_entities"
blocks "sea-orm-cli migrate status --database-url=$remote_pg"
blocks 'sea-orm-cli migrate up -u "$PROD_URL"'
allows "sea-orm-cli migrate status -u $local_pg"
allows "sea-orm-cli generate entity --database-url=$local_pg -o src/models/_entities"
reason_mentions "sea-orm-cli migrate up -u $remote_pg" 'from the -u option'

# ------------------------------------------------------------- check 2: the environment

blocks 'cargo test' "DATABASE_URL=$remote_pg"
blocks 'cargo nextest run --locked --workspace --all-targets' "DATABASE_URL=$remote_pg"
blocks 'cargo loco db migrate' "DATABASE_URL=$remote_pg"
blocks 'cargo loco start' "DATABASE_URL=$remote_pg"
blocks 'cargo +1.94.0 test' "DATABASE_URL=$remote_pg"
blocks 'cargo --locked t' "DATABASE_URL=$remote_pg"
blocks "psql -c 'select 1'" "DATABASE_URL=$remote_pg"
blocks 'sea-orm-cli migrate status' "DATABASE_URL=$remote_pg"
blocks 'psql' 'PGHOST=db.example.org'
blocks 'cargo test' 'PGHOST=db.example.org'
blocks "mysql -e 'select 1'" 'MYSQL_HOST=legacy.example.org'
allows "psql -c 'select 1'" 'MYSQL_HOST=legacy.example.org'
allows 'cargo nextest run' "DATABASE_URL=$local_pg"
allows "DATABASE_URL=$local_pg cargo test" "DATABASE_URL=$remote_pg"
allows 'psql -h localhost' "DATABASE_URL=$remote_pg"
allows 'git status' "DATABASE_URL=$remote_pg"
allows 'cargo build --release --locked' "DATABASE_URL=$remote_pg"
blocks 'DATABASE_URL=postgres:///backend3_test cargo test' 'PGHOST=db.example.org'
blocks "psql 'postgres:///backend3_test'" 'PGHOST=db.example.org'
allows 'DATABASE_URL=postgres:///backend3_test cargo test' 'PGHOST=/var/run/postgresql'
allows "psql 'postgres:///backend3_test'"
reason_mentions_env() {
  payload_for "$1" >"$work/payload.json"
  run_hook_file "$work/payload.json" "$2"
  if [[ "$status" == 2 ]] && grep -qF -- "$3" "$work/err"; then
    report ok "reason for '$1' with $2 mentions '$3'" ""
  else
    report fail "reason for '$1' with $2 mentions '$3'" "  exit $status; stderr: $(cat "$work/err")"
  fi
}
reason_mentions_env 'cargo test' "DATABASE_URL=$remote_pg" "the environment's DATABASE_URL"

# A local inline PGHOST never hides a remote DATABASE_URL that the command
# itself connects to (Loco reads database.uri from DATABASE_URL, and sqlx
# ignores PGHOST when the URI names a host).
blocks 'PGHOST=localhost cargo loco start' "DATABASE_URL=$remote_pg"
blocks 'PGHOST=localhost cargo test' "DATABASE_URL=$remote_pg"
blocks 'LOCO_ENV=production PGHOST=/run/postgresql cargo loco start' "DATABASE_URL=$remote_pg"
blocks 'PGHOST=localhost sea-orm-cli migrate up' "DATABASE_URL=$remote_pg"
blocks "DATABASE_URL=$local_pg psql" 'PGHOST=db.example.org'
allows "DATABASE_URL=$local_pg cargo test" 'PGHOST=db.example.org'
allows 'psql -h localhost' 'PGHOST=db.example.org'
# PGHOSTADDR, PGSERVICE and libpq key/value spellings.
blocks 'psql "dbname=x host = db.example.org"'
blocks "psql 'dbname=x hostaddr= 192.0.2.10'"
blocks 'PGHOSTADDR=10.0.0.1 psql'
blocks 'PGHOST=localhost PGHOSTADDR=10.0.0.1 psql'
blocks 'psql -h localhost' 'PGHOSTADDR=10.0.0.1'
blocks 'cargo test' 'PGHOSTADDR=10.0.0.1'
blocks 'psql service=prod'
blocks "psql 'postgres:///backend3?service=prod'"
blocks 'PGSERVICE=prod psql'
blocks 'psql' 'PGSERVICE=prod'
allows "psql 'host = localhost dbname=backend3_development'"
# cargo run is what the cargo loco alias expands to.
blocks "DATABASE_URL=$remote_pg cargo run -- db migrate"
blocks "DATABASE_URL=$remote_pg cargo r -- db migrate"
blocks 'cargo run -- start' "DATABASE_URL=$remote_pg"
allows 'cargo run -- db migrate'
allows 'cargo run --release -- start'
# The cargo aliases that run the same programs (D69, ADR 0028 section 4):
# loco-tool is the same alias as loco in .cargo/config.toml, and r and t are
# cargo's built-in aliases of run and test.
blocks "DATABASE_URL=$remote_pg cargo loco-tool db migrate"
blocks 'cargo loco-tool start' "DATABASE_URL=$remote_pg"
blocks "PGHOST=db.example.org cargo loco-tool db migrate"
blocks 'cargo r -- start' "DATABASE_URL=$remote_pg"
blocks "DATABASE_URL=$remote_pg cargo t"
blocks "DATABASE_URL=$remote_pg cargo t --workspace"
blocks 'cargo t' "DATABASE_URL=$remote_pg"
blocks "DATABASE_URL=$remote_pg cargo +stable t"
blocks "IMPORT_SOURCE_DATABASE_URL=$remote_mysql cargo loco-tool task import_mysql"
blocks "IMPORT_SOURCE_DATABASE_URL=$remote_mysql cargo t"
blocks "sh -c 'DATABASE_URL=$remote_pg cargo t'"
blocks 'oc exec deploy/backend3 -- cargo loco-tool db migrate'
blocks 'oc exec deploy/backend3 -- cargo t'
blocks 'cargo loco-tool db reset'
allows "DATABASE_URL=$local_pg cargo loco-tool db migrate"
allows 'cargo loco-tool start'
allows 'cargo r -- start'
allows "DATABASE_URL=$local_pg cargo t"
allows 'cargo t --workspace'
reason_mentions "DATABASE_URL=$remote_pg cargo t" "\`cargo t\` would use the database target"
reason_mentions "DATABASE_URL=$remote_pg cargo loco-tool db migrate" "\`cargo loco-tool\` would use the database target"
# Assignments apply where the shell applies them: later segments and nested
# command strings see them, earlier segments do not, and the latest wins.
blocks "DATABASE_URL=$remote_pg bash -c 'cargo test'"
blocks "export DATABASE_URL=$remote_pg && sh -c 'cargo nextest run'"
blocks "cargo test; export DATABASE_URL=$remote_pg; cargo test"
allows "export DATABASE_URL=$remote_pg; export DATABASE_URL=$local_pg; cargo test"
allows "cargo test; export DATABASE_URL=$remote_pg"

# ------------------------------------------------------------- check 2: IMPORT_SOURCE_DATABASE_URL

blocks "IMPORT_SOURCE_DATABASE_URL=$remote_mysql cargo loco task import_mysql mode:dry-run"
blocks 'IMPORT_SOURCE_DATABASE_URL="$LEGACY_URL" cargo loco task import_mysql'
blocks 'cargo loco task import_mysql mode:dry-run' "IMPORT_SOURCE_DATABASE_URL=$remote_mysql"
blocks 'cargo test' "IMPORT_SOURCE_DATABASE_URL=$remote_mysql"
allows "IMPORT_SOURCE_DATABASE_URL=$local_mysql cargo loco task import_mysql mode:dry-run"
allows "IMPORT_SOURCE_DATABASE_URL=$local_mysql cargo loco task import_mysql" "IMPORT_SOURCE_DATABASE_URL=$remote_mysql"
allows 'cargo loco task import_mysql mode:dry-run' "IMPORT_SOURCE_DATABASE_URL=$local_mysql"
allows 'psql -h localhost' "IMPORT_SOURCE_DATABASE_URL=$remote_mysql"
reason_mentions_env 'cargo loco task import_mysql' "IMPORT_SOURCE_DATABASE_URL=$remote_mysql" 'legacy import source'

# ------------------------------------------------------------- check 2: no target

allows 'cargo test'
allows 'cargo nextest run --locked --workspace --all-targets'
allows 'cargo test --locked --workspace --doc'
allows 'cargo loco task import_mysql mode:dry-run'
allows "psql -c 'select 1'"
allows "mysql -e 'select 1'"
allows 'sea-orm-cli migrate status'

# ------------------------------------------------------------- check 2: run in the cluster

blocks "oc exec postgres-0 -- psql -h localhost -c 'select 1'"
blocks 'oc exec -n web postgres-0 -- psql'
blocks 'oc rsh postgres-0 psql'
blocks 'oc rsh -n web postgres-0 psql -h 127.0.0.1'
blocks 'oc debug node/edge -- chroot /host psql -h 127.0.0.1'
blocks "oc exec -n web deploy/backend3 -- sh -c 'cargo test'"
blocks "oc exec mysql-0 -- mysql -h 127.0.0.1 -e 'select 1'"
blocks 'oc exec deploy/backend3 -- pg_restore -d backend3 backup.dump'
blocks 'oc exec deploy/backend3 -- sea-orm-cli migrate status'
allows 'oc exec postgres-0 -- ls'
allows 'oc logs postgres-0'
allows 'oc exec postgres-0 -- cat /etc/hostname | psql -h localhost'
reason_mentions 'oc rsh postgres-0 psql' 'runs inside the cluster through `oc rsh`'

# A script piped into oc exec, oc rsh or oc debug runs in the cluster, also
# through further pipeline stages; so does a here-string, wherever it stands.
blocks "echo 'psql -c \"select 1\"' | oc exec -i postgres-0 -- sh"
blocks "printf 'mysql -e \"select 1\"\n' | oc rsh mysql-0"
blocks "echo 'psql' | oc debug node/edge"
blocks "printf '%s\n' 'cd /tmp' 'psql -h 127.0.0.1' | oc exec -i -n web postgres-0 -- bash"
blocks "echo 'cargo test' | oc exec -i deploy/backend3 -- sh"
blocks "echo 'pg_restore -d backend3 backup.dump' |& oc exec -i deploy/backend3 -- sh"
blocks "echo 'psql' | oc --namespace web exec -i postgres-0 -- sh"
blocks "echo 'psql' | tr a a | oc exec -i postgres-0 -- sh"
blocks "printf 'mysql\n' | sed 's/x/y/' | cat | oc rsh mysql-0"
blocks "echo 'psql' | xargs oc exec postgres-0 --"
blocks "oc exec -i postgres-0 -- sh <<< 'psql'"
blocks "oc rsh <<< 'psql' postgres-0"
blocks "oc exec -i <<< 'psql' postgres-0 -- cat"
allows "echo 'ls -la' | oc exec -i postgres-0 -- sh"
allows "echo 'select 1' | oc rsh deploy/mysql"
allows "echo 'psql -h localhost' | sh"
allows "echo 'psql' > notes.txt; oc rsh postgres-0 ls"
# bash continues a pipeline on the next line after a |.
blocks $'echo \'psql\' |\noc exec -i postgres-0 -- sh'
blocks $'printf \'mysql\\n\' |\n  tr a a |\n  oc rsh mysql-0'
# Under xargs the piped words are the oc command's arguments, not its stdin
# (xargs gives the command /dev/null): each line takes the place of the
# replace string, or the words are appended. So a pod named like a client is
# a pod, and a client or db subcommand in the command part still counts.
allows 'echo deploy/mysql | xargs -I{} oc exec {} -- df -h'
allows "printf '%s\n' mysql postgres | xargs -I{} oc exec deploy/{} -- df -h"
allows 'echo deploy/mysql deploy/postgres | xargs -n1 -I{} oc exec -n verseghy {} -- du -sh /tmp'
allows 'echo deploy/postgres deploy/mysql | xargs -I{} oc exec {} -- df -h'
blocks 'echo postgres-0 | xargs -I{} oc exec {} -- psql'
blocks 'echo psql | xargs -I % oc rsh postgres-0 %'
blocks "printf 'psql\n' | xargs -I{} oc exec postgres-0 -- sh -c 'cd tmp;{}'"
blocks 'echo reset | xargs oc exec deploy/backend3 -- backend3-cli db'
blocks 'echo truncate | xargs -I{} oc exec deploy/backend3 -- backend3-cli db {}'
# With -a, xargs reads its arguments from the file and the pipe stays stdin.
blocks 'echo psql | xargs -a pods.txt -I{} oc exec -i {} -- sh'

# A pipeline longer than 16 KiB is lexed in pieces (see chunks in the hook);
# the piped text still reaches the stage that reads it.
filler="$(for _ in $(seq 1 1100); do printf '%s ' xxxxxxxxxxxxxxxx; done)"
blocks "echo '$filler psql -h db' | oc exec -i postgres-0 -- sh"
blocks "echo '$filler; backend3-cli db reset' | oc exec -i deploy/backend3 -- sh"
blocks "echo 'psql' | oc exec -i postgres-0 -- sh -c '$filler; cat'"
blocks "echo '$filler psql' |& oc exec -i postgres-0 -- sh"
blocks "echo '$filler psql' | tr a a | oc rsh postgres-0"
blocks "echo '$filler cargo loco db reset' | bash"
allows "echo '$filler' | oc exec -i postgres-0 -- tee /tmp/x"

# Only the command part runs in the cluster: the words after `--` for exec
# and debug, after the pod for rsh. A pod, container or namespace named like
# a client is not one, but a client in the command part always is.
allows 'oc exec -n verseghy deploy/mysql -- df -h'
allows 'oc exec mysql -- df -h'
allows 'oc exec -c mysql deploy/legacy -- ls'
allows 'oc exec deploy/legacy -c mysql -- ls'
allows 'oc exec -n mysql deploy/legacy -- ls'
allows 'oc exec --container=psql postgres-0 -- ls'
allows 'oc rsh deploy/mysql ls -l'
allows 'oc rsh -c psql postgres-0 cat /etc/hostname'
allows 'oc rsh -- mysql ls'
allows 'oc debug deploy/mysql -- df -h'
allows 'oc debug --as-user 1000 deploy/mysql -- df -h'
allows 'oc debug node/psql'
blocks "oc exec -n verseghy deploy/mysql -- mysql -e 'select 1'"
blocks 'oc exec -c mysql mysql-0 -- mysql'
blocks 'oc rsh deploy/mysql mysql'
blocks 'oc rsh deploy/mysql -- psql'
blocks 'oc rsh -- deploy/mysql mysql'
blocks 'oc rsh -c mysql deploy/mysql psql -h 127.0.0.1'
blocks 'oc debug deploy/mysql -- mysql'
blocks 'oc debug --as-user 1000 deploy/mysql -- mysql'
blocks 'oc exec postgres-0 psql'
blocks "oc exec postgres-0 -- sh -c 'cd /tmp;psql'"
blocks "oc exec postgres-0 -- sh -c '\"psql\" -c \"select 1\"'"
blocks "oc exec postgres-0 -- bash -lc 'env PGHOST=localhost psql'"
blocks "parallel 'oc exec {} -- psql' ::: postgres-0"
# rsh's --shell names the program it starts when no command follows the pod,
# and with -f every positional word is the command (oc 4.22, pkg/cli/rsh).
blocks 'oc rsh --shell /usr/bin/psql postgres-0'
blocks 'oc rsh --shell=psql postgres-0'
blocks 'oc rsh -c postgres --shell /usr/bin/psql postgres-0'
blocks 'oc rsh --shell=/usr/bin/mysql mysql-0'
allows 'oc rsh --shell /bin/bash postgres-0'
allows 'oc rsh --shell=/bin/sh deploy/mysql ls'
blocks 'oc rsh -f pod.yaml psql'
blocks 'oc rsh -f pod.yaml psql -h 127.0.0.1'
blocks "oc rsh --filename=pod.yaml mysql -e 'select 1'"
allows 'oc rsh -f pod.yaml ls -l'
# A path under a data or configuration tree is a file, not a program, unless
# it is in a bin directory there; a client run by any other path counts.
allows 'oc exec mysql-0 -c mysql -- ls /var/lib/mysql'
allows 'oc exec -n legacy deploy/mysql -c mysql -- du -sh /var/lib/mysql'
allows 'oc rsh mysql-0 ls /var/lib/mysql /etc/mysql /usr/share/mysql'
blocks "oc exec mysql-0 -- /usr/bin/mysql -e 'select 1'"
blocks 'oc rsh postgres-0 /usr/lib/postgresql/16/bin/psql'
blocks 'oc exec postgres-0 -- /opt/bitnami/postgresql/bin/psql'
blocks 'oc exec postgres-0 -- /var/lib/pgsql/bin/psql'
blocks 'oc exec postgres-0 -- /tmp/psql'
blocks 'oc exec postgres-0 -- ./psql'
# Shell separators cut words only inside a shell script in the command part.
# Elsewhere ( ) | < > are text: a search pattern, a message or an argument.
blocks "oc exec postgres-0 -- sh -c 'cd tmp;psql'"
blocks "oc rsh postgres-0 bash -c 'n=\$(psql -At -c \"select 1\")'"
blocks "echo 'cd tmp;psql' | oc rsh postgres-0"
blocks $'oc rsh postgres-0 <<\'EOF\'\ncd tmp;psql\nEOF'
allows 'oc exec deploy/legacy -- grep -E "(mysql|psql)" /etc/hosts'
allows 'grep -E "oc (exec|rsh) .* psql" -r docs'
allows "rg -n 'oc (exec|rsh|debug).*(psql|mysql)' .claude scripts"
allows 'rg -n "oc exec [^ ]+ -- (psql|mysql)" scripts/tests'
allows "rg 'oc rsh [^ ]+ (psql|mysql)'"
allows "jq -r '.[] | select(.cmd | test(\"oc (exec|rsh) .*(psql|mysql)\"))' cases.json"
allows "git commit -m 'block oc exec|rsh|debug with psql|mysql in the command part'"
allows "gh pr comment 200 --body 'oc rsh <pod> (with psql) is blocked'"
reason_mentions 'oc rsh postgres-0 psql' 'git commit -F <file>'

# A pod looked up with an unquoted $(...) or `...` does not end the command:
# the words after it are still the command part of the oc exec or oc rsh.
blocks 'oc exec $(oc get pod -l app=postgres -o name) -- psql'
blocks 'oc exec `oc get pod -l app=postgres -o name` -- psql'
blocks 'oc exec -n verseghy $(oc get pod -o name | head -1) -- sh -c "psql -c \"select 1\""'
blocks 'oc rsh $(oc get pod -o name) psql'
blocks 'oc debug $(oc get pod -o name) -- mariadb'
blocks 'echo psql | oc exec -i $(oc get pod -o name) -- sh'
blocks 'oc exec $(oc get pod -o name) -- cargo loco db reset'
allows 'oc exec $(oc get pod -l app=web -o name) -- ls /tmp'
allows 'oc exec web-0 -- ls $(echo /tmp) /var'
allows 'oc rsh `oc get pod -l app=web -o name` df -h'

# ------------------------------------------------------------- check 2: oc port-forward to a database (ADR 0028)

# A port-forward makes the cluster database answer on localhost, which check 2
# would count as local. The remote part of a port spec decides: 5432, 3306 or
# 33060, a named port that names a database, or a value that cannot be checked.
blocks 'oc port-forward svc/postgres 5432'
blocks 'oc port-forward svc/postgres 15432:5432'
blocks 'oc port-forward -n verseghy pod/postgres-0 :5432'
blocks 'oc port-forward deploy/mysql 3306:3306'
blocks 'oc port-forward svc/mysql 33060'
blocks 'oc port-forward svc/postgres 05432'
# Each part of the named-port rule: `postgres`, `mariadb` and `sql` anywhere
# in the name (`mysql` contains `sql`), and the whole name `db`.
blocks 'oc port-forward svc/legacy 13306:mysql'
blocks 'oc port-forward svc/backend3-db 15432:postgresql'
blocks 'oc port-forward svc/backend3-db 15432:postgres'
blocks 'oc port-forward svc/backend3-db 15432:tcp-postgres'
blocks 'oc port-forward svc/legacy 13306:mariadb'
blocks 'oc port-forward svc/backend3-db 15432:pgsql'
blocks 'oc port-forward svc/backend3-db 15432:db'
allows 'oc port-forward svc/backend3 8080:http'
allows 'oc port-forward svc/backend3-db 9187:dbmetrics'
# Redis (D69): its port 6379 and Sentinel's 26379, and named ports that
# contain redis or sentinel, in every form the PostgreSQL and MySQL ports take.
blocks 'oc port-forward svc/redis-backend2 6379'
blocks 'oc port-forward svc/redis-backend2 16379:6379'
blocks 'oc port-forward -n verseghy-website deploy/redis-backend2 :6379'
blocks 'oc port-forward pod/cache-0 6379:6379'
blocks 'oc port-forward svc/redis-sentinel 26379'
blocks 'oc port-forward svc/cache 36379:26379'
blocks 'oc port-forward svc/cache 16379:redis'
blocks 'oc port-forward svc/cache 16379:tcp-redis'
blocks 'oc port-forward svc/cache 16379:Redis'
blocks 'oc port-forward svc/cache 36379:tcp-sentinel'
blocks 'oc port-forward svc/web 8080:80 16379:6379'
blocks 'oc --namespace verseghy-website port-forward svc/redis-backend2 6379'
blocks 'oc port-forward --address 0.0.0.0 svc/redis-backend2 6379'
blocks 'oc port-forward svc/redis-backend2 16379:6379 & redis-cli -p 16379'
blocks "sh -c 'oc port-forward svc/redis-backend2 6379'"
blocks 'oc exec toolbox -- oc port-forward svc/redis-backend2 6379'
blocks 'echo 6379 | xargs oc port-forward svc/redis-backend2'
blocks 'oc port-forward $(oc get pod -l app=redis-backend2 -o name) 6379'
blocks $'bash <<\'EOF\'\noc port-forward svc/redis-backend2 16379:6379\nEOF'
# The name of the pod or service does not count, only the port: a Redis
# exporter's metrics port, or a web port, on a service named redis stays
# allowed, and so do ports that only contain 6379 among other digits.
allows 'oc port-forward svc/redis-backend2 9121'
allows 'oc port-forward svc/redis-exporter 9121:9121'
allows 'oc port-forward svc/redis-commander 8081:http'
allows 'oc port-forward svc/web 63790'
allows 'oc port-forward svc/web 8080:16379'
allows "git commit -m 'fix(deploy): block oc port-forward svc/redis-backend2 6379'"
allows "gh pr comment 216 --body 'oc port-forward to 6379 is blocked'"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block port-forwards to Redis\n\noc port-forward svc/redis-backend2 16379:6379\nEOF'
reason_mentions 'oc port-forward svc/redis-backend2 16379:6379' "'16379:6379'"
reason_mentions 'oc port-forward svc/redis-backend2 16379:6379' 'MariaDB or Redis port'
# check 2 has no Redis client, so the reason does not claim it would take a
# forwarded Redis for a local database.
reason_mentions 'oc port-forward svc/redis-backend2 16379:6379' 'does not check Redis clients at all'
blocks 'oc port-forward svc/web 8080:80 15432:5432'
blocks 'oc port-forward --address 0.0.0.0 svc/postgres 5432'
blocks 'oc --namespace verseghy port-forward svc/mysql 3306'
blocks 'oc port-forward svc/postgres "$PORT"'
blocks 'oc port-forward svc/postgres 15432:$REMOTE_PORT'
blocks 'oc port-forward svc/postgres 15432:5432 & psql -h localhost -p 15432'
blocks 'nohup oc port-forward svc/postgres 15432:5432 >/dev/null 2>&1 &'
blocks "sh -c 'oc port-forward svc/postgres 5432'"
blocks 'oc exec toolbox -- oc port-forward svc/postgres 5432'
# Under xargs or parallel, appended ports cannot be seen, and a replace string
# in a port spec cannot be checked.
blocks 'echo 5432 | xargs oc port-forward svc/postgres'
blocks "printf '%s\n' 5432 | xargs -I P oc port-forward svc/postgres P"
blocks 'echo svc/postgres | xargs -I{} oc port-forward {} 5432'
# A target looked up with an unquoted $(...) or `...` keeps the ports after it
# in the same command.
blocks 'oc port-forward $(oc get pod -l app=postgres -o name) 5432'
blocks 'oc port-forward `oc get pod -l app=postgres -o name` 5432'
blocks "oc port-forward pod/\$(oc get pod -l app=postgres -o jsonpath='{.items[0].metadata.name}') 5432"
blocks 'oc port-forward -n web $(oc get pod -l app=postgres -o name) 15432:5432 &'
blocks 'oc port-forward $(oc get svc -o name | grep mysql) 3306 >/dev/null 2>&1 &'
allows 'oc port-forward $(oc get pod -l app=web -o name) 8080:5150'
allows 'oc port-forward `oc get pod -l app=web -o name` 8080'
# An empty port spec is no database port, and it does not crash the check.
allows "oc port-forward svc/backend3 ''"
allows 'oc port-forward svc/backend3 ""'
allows 'oc port-forward svc/backend3 8080:5150'
allows 'oc port-forward -n openshift-gitops svc/openshift-gitops-server 8443:443'
allows 'oc port-forward pod/web-0 5432:8080'
allows 'oc port-forward svc/prometheus 9090'
allows 'oc port-forward svc/postgres-exporter 9187'
# --address takes a value, so the target after it is not read as a port spec.
allows 'oc port-forward --address 0.0.0.0 svc/postgres-exporter 9187'
allows 'echo svc/backend3 | xargs -I{} oc port-forward {} 8080:5150'
allows 'oc port-forward --help'
allows "git commit -m 'fix(deploy): block oc port-forward svc/postgres 5432'"
allows "gh pr comment 204 --body 'oc port-forward to 5432 is blocked'"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block port-forwards to databases\n\noc port-forward svc/postgres 15432:5432\nEOF'
allows "grep -rn 'port-forward' docs"
reason_mentions 'oc port-forward svc/postgres 15432:5432' 'answer on localhost'

# ------------------------------------------------------------- check 2: the mariadb client (ADR 0028)

# mariadb is read like mysql: -h, --host, MYSQL_HOST, and always in the cluster.
blocks 'mariadb -h db.example.org -u root legacy'
blocks 'mariadb --host=legacy.example.org legacy'
blocks 'mariadb -hlegacy.example.org legacy'
blocks 'mariadb -h "$DB_HOST"'
blocks '/usr/bin/mariadb -h legacy.example.org'
blocks "MYSQL_HOST=legacy.example.org mariadb -e 'select 1'"
blocks "mariadb -e 'select 1'" 'MYSQL_HOST=legacy.example.org'
blocks "mariadb $remote_mysql"
blocks "oc exec mariadb-0 -- mariadb -e 'select 1'"
blocks 'oc rsh mariadb-0 mariadb'
blocks 'oc exec mariadb-0 -- /usr/bin/mariadb'
blocks "oc exec deploy/legacy -- sh -c 'mariadb -h 127.0.0.1'"
blocks "echo 'mariadb -e \"select 1\"' | oc exec -i mariadb-0 -- sh"
blocks $'oc rsh mariadb-0 <<\'EOF\'\nmariadb -e "select 1"\nEOF'
allows 'mariadb -h 127.0.0.1 -u root legacy'
allows 'mariadb --host=localhost legacy'
allows "mariadb -e 'select 1'"
allows "MYSQL_HOST=127.0.0.1 mariadb -e 'select 1'"
allows "mariadb -h 127.0.0.1 -e 'select 1'" 'MYSQL_HOST=legacy.example.org'
allows 'oc exec -n verseghy deploy/mariadb -- df -h'
allows 'oc exec mariadb-0 -c mariadb -- ls /var/lib/mysql'
allows "git commit -m 'fix(deploy): check mariadb -h like mysql -h'"
allows "gh pr comment 204 --body 'mariadb -h db.example.org is blocked now'"
allows "gh pr create --title 'fix(deploy): check mariadb' --body 'mariadb --host=legacy.example.org is blocked'"
allows 'grep -rn mariadb docs'
reason_mentions 'mariadb -h db.example.org' 'from the -h option'

# ------------------------------------------------------------- check 2: wrapped forms

blocks 'bash -c "psql -h db.example.org"'
blocks "sh -c 'DATABASE_URL=$remote_pg cargo test'"
blocks 'timeout 30 psql -h db.example.org'
blocks 'echo ok && psql -h db.example.org'
blocks "ssh bastion 'psql -h db.example.org'"
blocks "watch 'psql -h db.example.org -c \"select 1\"'"
blocks $'bash <<\'EOF\'\npsql -h db.example.org\nEOF'
allows "bash -c 'psql -h localhost'"
# The words after an unquoted $(...), `...` or process substitution still
# belong to the command around it.
blocks 'psql -f <(cat q.sql) -h db.example.org'
blocks 'psql $(echo -X) -h db.example.org'
blocks 'psql `echo -X` -h db.example.org'
blocks 'psql -h `cat db-host`'
blocks 'mysql -e "$(cat q.sql)" -h legacy.example.org'
allows 'psql -f <(cat q.sql) -h localhost'
allows 'psql -h localhost -c "$(cat q.sql)" $(echo -X)'
allows 'echo $((1 + 2)) && psql -h localhost'
allows '(cd /tmp && psql -h localhost)'
allows 'diff <(oc get pods) <(oc get pods -n web)'
# A subshell group inside a substitution does not close it early.
blocks 'oc port-forward $( (oc get pod -l app=postgres -o name) | head -1) 5432'
blocks 'psql $( (echo -X) ) -h db.example.org'
# A substitution longer than the 16 KiB chunks a long text is cut into: the
# command around it is still checked when its chunk ends.
long_sub="$(for i in $(seq 1 1200); do printf '  echo line %d of the list\n' "$i"; done)"
blocks "psql -h db.example.org \$(
$long_sub
)"
allows "psql -h localhost \$(
$long_sub
)"

# ------------------------------------------------------------- a long command stays cheap

long_body="$(for i in $(seq 1 3000); do printf 'line %d: plain text that mentions cargo and psql\n' "$i"; done)"
# EPOCHREALTIME (bash 5) gives the time for the log line; it is informative
# only, so older shells just skip it.
start="${EPOCHREALTIME:-}"
allows "cat > notes.txt <<'EOF'"$'\n'"$long_body"$'\n'"EOF"
if [[ -n "$start" ]]; then
  end="${EPOCHREALTIME}"
  elapsed_ms=$(((${end/[.,]/} - ${start/[.,]/}) / 1000))
  printf '# a %d-byte heredoc command took %d ms\n' "${#long_body}" "$elapsed_ms"
fi


# Many segments with inline assignments: each segment checks only the
# assignments in scope for it, so the work grows linearly. (Before, every
# segment walked every assignment of the whole command: 1600 such lines took
# about 100 s, longer than a hook may run.) The bound is generous for slow CI
# runners and still far below the quadratic cost.
assign_body="$(for i in $(seq 1 1600); do printf 'echo step %d && DATABASE_URL=%s cargo test case_%d;\n' "$i" "$local_pg" "$i"; done)"
start="${EPOCHREALTIME:-}"
allows "$assign_body"
if [[ -n "$start" ]]; then
  end="${EPOCHREALTIME}"
  elapsed_ms=$(((${end/[.,]/} - ${start/[.,]/}) / 1000))
  printf '# a %d-byte command with 1600 inline assignments took %d ms\n' "${#assign_body}" "$elapsed_ms"
  if ((elapsed_ms < 30000)); then
    report ok "1600 segments with inline assignments are checked in under 30 s" ""
  else
    report fail "1600 segments with inline assignments are checked in under 30 s" "  took ${elapsed_ms} ms"
  fi
fi

# Asserts that a long command is allowed and checked within 30 s, like the
# case above, and logs its time. Without EPOCHREALTIME it only asserts the
# verdict.
#   $1 - what the command is, for the report;
#   $2 - the command text.
allows_in_time() {
  local start end elapsed_ms
  start="${EPOCHREALTIME:-}"
  allows "$2"
  if [[ -n "$start" ]]; then
    end="${EPOCHREALTIME}"
    elapsed_ms=$(((${end/[.,]/} - ${start/[.,]/}) / 1000))
    printf '# %s (%d bytes) took %d ms\n' "$1" "${#2}" "$elapsed_ms"
    if ((elapsed_ms < 30000)); then
      report ok "$1 is checked in under 30 s" ""
    else
      report fail "$1 is checked in under 30 s" "  took ${elapsed_ms} ms"
    fi
  fi
}

# Text that oc exec reads on stdin is split as commands, so one long line of
# it is one long segment, as are long echo arguments. Each word costs the same
# however long its segment is. (When every word looked back over its segment,
# or every trigger word read the rest of it, a 32 KB line took about a minute
# or more, longer than a hook may run.)
stdin_line="$(for i in $(seq 1 920); do printf 'word %d mentions cargo and the db ' "$i"; done)"
allows_in_time "a 32 KB here-string into oc exec" "oc exec -i deploy/backend3 -- tee /tmp/notes.txt <<< '$stdin_line'"
allows_in_time "32 KB of piped printf text into oc exec" "printf '%s' '$stdin_line' | oc exec -i deploy/backend3 -- tee /tmp/notes.txt"
allows_in_time "32 KB of echo arguments" "echo $stdin_line | oc get pods -n web"

# ------------------------------------------------------------- summary

printf '# %d passed, %d failed\n' "$passed" "$failed"
if ((failed > 0)); then
  exit 1
fi
