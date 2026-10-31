#!/usr/bin/env bash
# test-guard-bash.sh: tests for .claude/hooks/guard-bash.sh, the PreToolUse
# hook of the guardrails (ADR 0024 section 4, layer 2, as amended by ADR
# 0028, ADR 0035 and ADR 0036).
#
# Why it exists
#   The hook runs before every Bash command an agent runs in this repository.
#   A bug in it either lets a destructive command through or blocks legitimate
#   work, so every rule of ADR 0024 section 4, ADR 0028, ADR 0035 and ADR 0036
#   is pinned here:
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
#   (cargo run, r, loco-tool and t); the checks ADR 0035 adds (argocd app
#   confirm-deletion, finalizer and reclaim-policy patches through argocd app
#   patch-resource, Argo CD's deletion-approved annotation named in
#   oc annotate, oc patch or any argocd command, one that only prints it
#   included, as the answer to D109 confirmed, and redis-cli run in the
#   cluster), each with the read-only look-alikes and texts that stay
#   allowed, the port-forward readings that the answer to D106 confirmed,
#   and the readings of ADR 0035 section 2 for every check (options before
#   the oc verb, oc get's value options, and the hook's list of oc's value
#   options compared with the oc 4.22 client when that client is on PATH;
#   escapes, escaped line breaks and whole-object operations in a patch,
#   in JSON and in YAML with its tags, anchors, comments, block scalars
#   and aliases; $'...' and $"..." quoting, also inside other quotes; the
#   echo or printf output of a substitution that the shell runs, also after
#   a runner's own words and double-quoted, next to the commands that the
#   substitution runs; printf's \n and \t before a trigger word;
#   oc delete --raw; port numbers with a leading +; argocd's download
#   names; text that xargs or parallel pass to oc, argocd and the database
#   commands, without echo's options; and argocd as the value of an option
#   of an oc that surely runs and only reads, never behind a runner's
#   options or ssh, nor with a verb that runs a program); the checks ADR
#   0036 adds on D110's answer (redis-benchmark, valkey-cli and keydb-cli
#   wherever redis-cli is blocked; a Redis client as the command of a pod
#   the agent starts, through oc run, its --overrides and its stdin, and
#   through oc create job, cronjob, deployment and deploymentconfig; the
#   finalizer match for argocd app patch; and the deletion-approved
#   annotation and a Redis client in an inline manifest that oc apply,
#   oc create or oc replace reads from a heredoc, a here-string, piped
#   text or a process substitution), each with the look-alikes that stay
#   allowed; the heredoc that xargs or parallel read and the here-string
#   that cat passes on, which ADR 0035 left to ADR 0036, and the other
#   heredoc and stdin readings of ADR 0036 section 3 (every line, after a
#   `<`, beside others, a `<<` that may be quoted, scripts a shell runs,
#   pipelines after a body, what a group pipes on, `< <(...)`); the gaps and
#   the false positive that ADR 0036 records, pinned; resource arguments
#   with an empty type, which must not crash the checks; allowed commands;
#   a missing jq;
#   the cost of long commands; and the hook entry of .claude/settings.json,
#   which must equal ADR 0036's layer-1 block, keep every deny rule of ADR
#   0028's and ADR 0035's, have each deny rule backed by a hook check, and
#   run the hook in exec form with "onFailure": "block".
#
# How it works
#   Each case builds a PreToolUse payload with jq (the shape Claude Code sends,
#   see scripts/tests/fixtures/guard-bash/), pipes it into the hook with a
#   controlled environment and asserts the exit code: 0 allows, 2 blocks. For
#   blocked commands it also asserts that stderr carries the reason of a
#   check, not one of the hook's fail-closed messages.
#
# Usage
#   bash scripts/tests/test-guard-bash.sh     (from the repository root)
#   It needs bash and jq, and no network or cluster access (the oc client,
#   when it is on PATH, only prints its help, with an empty kubeconfig). It
#   exits non-zero if any case fails.

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
  elif [[ "$expected" == 2 ]] && ! grep -q '^Blocked by .claude/hooks/guard-bash.sh (.*, check [12])' "$work/err"; then
    # A block must come from a check, not from the fail-closed paths: their
    # messages start the same way, so a case that the hook only blocks
    # because the policy failed (a jq error) would otherwise pass.
    report fail "reason of a check on stderr for: $label" "  stderr was: $(cat "$work/err")"
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
adr_dir="$repo_root/docs/design/adr"
# Prints the layer-1 JSON block of an ADR: the first json code block after
# the "**Layer 1: deny rules" line.
#   $1 - the ADR file.
layer1_block() {
  awk '/^\*\*Layer 1: deny rules/{f=1} f && /^```json$/{g=1; next} g && /^```$/{exit} g{print}' "$1"
}
adr="$adr_dir/0036-guardrails-for-redis-clients-in-new-pods-argocd-app-patch-an.md"
# The layer-1 JSON block of ADR 0036 section 4, which replaces the block of
# ADR 0035 section 3 (itself the replacement of ADR 0028 section 3's and,
# before it, ADR 0024 section 4's), must be the committed settings. The
# older blocks stay as they were, because an amended ADR keeps its body
# (ADR 0001 section 6).
adr_settings="$(layer1_block "$adr")"
if [[ -n "$adr_settings" ]] && [[ "$(jq -S . <<<"$adr_settings")" == "$(jq -S . "$settings")" ]]; then
  report ok ".claude/settings.json equals the layer-1 block of ADR 0036 section 4" ""
else
  report fail ".claude/settings.json equals the layer-1 block of ADR 0036 section 4" \
    "  diff: $(diff <(jq -S . <<<"$adr_settings" 2>&1) <(jq -S . "$settings" 2>&1) || true)"
fi

# The amendments only add deny rules: every rule of ADR 0028's and ADR
# 0035's blocks is still in the committed settings, so an amendment cannot
# drop one by mistake.
for older in 0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md \
  0035-guardrails-for-in-cluster-redis-argo-cd-patches-and-deletion.md; do
  older_settings="$(layer1_block "$adr_dir/$older")"
  dropped="$(jq -rn --argjson old "${older_settings:-null}" --slurpfile new "$settings" --arg adr "${older:0:4}" \
    'if $old == null then "ADR \($adr) has no layer-1 block" else ($old.permissions.deny - $new[0].permissions.deny) | .[] end' 2>&1)"
  if [[ -z "$dropped" ]]; then
    report ok ".claude/settings.json keeps every deny rule of ADR ${older:0:4}" ""
  else
    report fail ".claude/settings.json keeps every deny rule of ADR ${older:0:4}" "  missing: $dropped"
  fi
done

# Layer 2 covers layer 1: the hook blocks a sample command of every deny
# rule, so the wrapped and reordered forms of that command, which the rule
# itself misses, are guarded too, and a new deny rule without a matching hook
# check fails here. The sample replaces a leading `* ` with a pod deletion,
# and a trailing ` *`, `/*` or `*` with a name; in the patch and annotation
# rules, the first ` *` becomes a PV and a -p option, and every other `*` a
# `=` (`oc patch pv/data -p persistentVolumeReclaimPolicy=Delete`). The same
# derivation is in website_k8s's copy of these tests.
while IFS= read -r rule; do
  sample="$(sed -e 's/^Bash(//' -e 's/)$//' -e 's/^\* /oc delete pod web-0 /' \
    -e 's/ \*$/ data/' -e 's|/\*$|/data|' -e 's/\*$//' \
    -e 's| \*| pv/data -p |' -e 's/\*/=/g' <<<"$rule")"
  payload_for "$sample" >"$work/payload.json"
  run_hook_file "$work/payload.json"
  if [[ "$status" == 2 ]]; then
    report ok "the hook also blocks deny rule $rule ($sample)" ""
  else
    report fail "the hook also blocks deny rule $rule ($sample)" "  exit $status; stderr: $(cat "$work/err")"
  fi
done < <(jq -r '.permissions.deny[]' "$settings")

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

# ------------------------------------------------------------- oc's value options (ADR 0035)

# The hook's oc_value_options lists the oc options that take a separate value,
# from the oc 4.22 client. When that client is on PATH, its help is the
# reference: every option that `oc options`, or the help of a verb whose
# positional words the hook reads (get, delete, exec, rsh, debug,
# port-forward, and since ADR 0036 run, create and the create subcommands
# job, cronjob, deployment and deploymentconfig), shows with a value must be
# listed, and none that it shows as a boolean may be. The exceptions are the
# options that pflag gives a value of their own when they stand alone
# (NoOptDefVal: --cascade, --dry-run and --validate), which take no separate
# value although their help shows one. Help is printed locally, and an empty
# kubeconfig keeps oc from finding a cluster. CI has no oc, and another
# client version may have other options, so the case is skipped then;
# compare the list with that client by hand when it changes.
label="oc_value_options matches the value options of the oc 4.22 client"
if ! command -v oc >/dev/null 2>&1; then
  report ok "$label # SKIP oc is not on PATH" ""
else
  : >"$work/kubeconfig"
  # Red Hat's builds put the release in clientVersion.gitVersion, others in
  # releaseClientVersion.
  oc_release="$(KUBECONFIG="$work/kubeconfig" oc version --client -o json 2>/dev/null \
    | jq -r '(.releaseClientVersion // .clientVersion.gitVersion // "") | ltrimstr("v")' 2>/dev/null || true)"
  if [[ "$oc_release" != 4.22.* ]]; then
    report ok "$label # SKIP oc on PATH is ${oc_release:-of an unknown version}, not 4.22" ""
  else
    # The option lines of oc's help, as "name=default:"; a boolean's default
    # is false or true.
    oc_option_lines="$({
      KUBECONFIG="$work/kubeconfig" oc options
      for verb in get delete exec rsh debug port-forward run create \
        'create job' 'create cronjob' 'create deployment' 'create deploymentconfig'; do
        # The create subcommands are two words, so $verb is split on purpose.
        # shellcheck disable=SC2086
        KUBECONFIG="$work/kubeconfig" oc $verb --help
      done
    } 2>/dev/null | grep -E '^ +(-[A-Za-z], )?--[a-z0-9-]+=' | sed -E 's/^ +//' || true)"
    # Prints the option names of such lines, one per line, sorted (a line
    # "-n, --namespace='':" names -n and --namespace).
    option_names() { sed -E 's/=.*//; s/, /\n/' | LC_ALL=C sort -u; }
    client_values="$(grep -vE '=(false|true):' <<<"$oc_option_lines" | option_names | grep -vxE -- '--cascade|--dry-run|--validate' || true)"
    client_booleans="$(grep -E '=(false|true):' <<<"$oc_option_lines" | option_names || true)"
    hook_values="$(sed -n '/^def oc_value_options:/,/];/p' "$hook" | grep -oE '"-[^"]+"' | tr -d '"' | LC_ALL=C sort -u)"
    missing="$(LC_ALL=C comm -23 <(printf '%s\n' "$client_values") <(printf '%s\n' "$hook_values") | tr '\n' ' ')"
    listed_booleans="$(LC_ALL=C comm -12 <(printf '%s\n' "$client_booleans") <(printf '%s\n' "$hook_values") | tr '\n' ' ')"
    if [[ -n "$client_values" && -z "$missing" && -z "$listed_booleans" ]]; then
      report ok "$label" ""
    else
      report fail "$label" "  value options the hook does not list: ${missing:-none}; booleans it lists: ${listed_booleans:-none} (an option with a value of its own when it stands alone belongs in this case's exceptions instead)"
    fi
  fi
fi

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
# An empty type part names no kind and does not crash the checks (kubectl
# refuses these arguments; ADR 0035). A crash would block them with the
# reason "could not read the hook input as JSON", which says nothing useful.
allows 'oc delete pod/web-0 /x'
allows 'oc delete ,'
allows 'oc delete <(oc get pods -o name)'
allows "oc get pvc; oc delete ''"
allows 'oc get /x -o name | xargs oc describe'
# --raw sends a DELETE to whatever its URI names, so every oc delete --raw
# counts, in any position (ADR 0035); oc get --raw only reads.
blocks 'oc delete --raw /api/v1/namespaces/web'
blocks 'oc delete --raw=/api/v1/namespaces/web/persistentvolumeclaims/data'
blocks 'oc delete --raw /apis/argoproj.io/v1alpha1/namespaces/openshift-gitops/applications/backend3'
blocks 'oc -n web delete --raw /api/v1/namespaces/web/pods/web-0'
blocks 'oc --raw /api/v1/persistentvolumes/pv-1 delete'
blocks 'oc --vmodule x=1 delete --raw /api/v1/namespaces/web'
blocks "sh -c 'oc delete --raw /api/v1/namespaces/web'"
blocks 'echo /api/v1/namespaces/web | xargs oc delete --raw'
allows 'oc get --raw /api/v1/namespaces'
allows 'oc get --raw=/readyz'
allows "git commit -m 'fix(deploy): block oc delete --raw /api/v1/namespaces/web'"
reason_mentions 'oc delete --raw /api/v1/namespaces/web' 'sends a DELETE request to the URI it names'
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
# The value options of oc get (`oc get --help`) are skipped with their value
# (ADR 0035), so the type after them is still read.
blocks 'oc get --sort-by .metadata.name pvc -o name | xargs oc delete'
blocks 'oc get --chunk-size 10 ns -o name | xargs oc delete'
blocks 'oc get --subresource status pvc -o name | xargs oc delete'
blocks 'oc get -L tier pvc -o name | xargs oc delete'
blocks 'oc get --label-columns tier pv -o name | xargs oc delete'
blocks 'oc --vmodule foo=1 get pvc -o name | xargs oc delete'
allows 'oc get --sort-by .metadata.creationTimestamp pods'
allows 'oc get --sort-by .metadata.name pods -o name | xargs oc delete'
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
blocks 'oc delete --raw /api/v1/namespaces/web'
blocks 'oc delete --raw /apis/project.openshift.io/v1/projects/web'
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
# argocd's release downloads keep their file name: argocd-linux-amd64 and the
# other argocd-<os>-<arch> names (ADR 0035).
blocks './argocd-linux-amd64 app delete backend3'
blocks '/usr/local/bin/argocd-darwin-arm64 appset delete website'
blocks 'argocd-windows-amd64.exe app delete-resource backend3 --kind Pod'
# Any name that goes on after argocd with `-`, `_` or `.` counts.
blocks './argocd.exe app delete backend3'
blocks 'argocd_v3 app delete backend3'
allows './argocd-linux-amd64 app list'
# A name that only starts with the letters, with no separator after them,
# is another program, and another word: an Application named argocdapps.
allows 'argocdx app delete backend3'
allows "oc get application argocdapps -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
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

# ------------------------------------------------------------- check 1: argocd app confirm-deletion (D105, ADR 0035)

# confirm-deletion sets the deletion-approved annotation on an Application,
# which releases what a Prune=confirm or Delete=confirm sync option holds
# back, so it is blocked with any options and arguments, in every position,
# spelling and wrapper that the deleting subcommands are.
blocks 'argocd app confirm-deletion backend3'
blocks 'argocd app confirm-deletion openshift-gitops/backend3'
blocks 'argocd app confirm-deletion backend3 -N openshift-gitops'
blocks 'argocd app confirm-deletion -N openshift-gitops backend3'
blocks 'argocd app confirm-deletion --app-namespace=openshift-gitops backend3'
blocks 'argocd app -N openshift-gitops confirm-deletion backend3'
blocks 'argocd app --app-namespace openshift-gitops confirm-deletion backend3'
blocks 'argocd --grpc-web --server argocd.example.org app confirm-deletion backend3'
blocks 'argocd --core app confirm-deletion backend3'
blocks 'argocd app --server argocd.example.org confirm-deletion backend3'
blocks 'argocd --some-new-option value app confirm-deletion backend3'
blocks 'argocd app --some-new-option v confirm-deletion backend3'
blocks 'argocd app --some-new-option confirm-deletion backend3'
blocks '/usr/local/bin/argocd app confirm-deletion backend3'
blocks './argocd-linux-amd64 app confirm-deletion backend3'
# Quotes and backslashes are removed the way the shell removes them.
blocks "argocd app 'confirm-deletion' backend3"
blocks 'argocd app "confirm-deletion" backend3'
blocks 'argocd app confirm"-"deletion backend3'
blocks 'argocd app confirm\-deletion backend3'
blocks "'argocd' app confirm-deletion backend3"
# Environment prefixes, runner words and wrappers.
blocks 'ARGOCD_SERVER=argocd.example.org argocd app confirm-deletion backend3'
blocks 'env ARGOCD_OPTS=--grpc-web argocd app confirm-deletion backend3'
blocks 'sudo argocd app confirm-deletion backend3'
blocks 'timeout 30 argocd app confirm-deletion backend3'
blocks "sh -c 'argocd app confirm-deletion backend3'"
blocks "bash -lc 'argocd app get backend3 && argocd app confirm-deletion backend3'"
blocks "ssh bastion 'argocd app confirm-deletion backend3'"
blocks 'echo backend3 | xargs argocd app confirm-deletion'
blocks 'echo backend3 | xargs -I{} argocd app confirm-deletion {}'
blocks 'oc exec toolbox -- argocd app confirm-deletion backend3'
blocks $'bash <<\'EOF\'\nargocd app confirm-deletion backend3\nEOF'
blocks 'argocd app confirm-deletion $(argocd app list -o name | head -1)'
blocks 'for a in backend3 media; do argocd app confirm-deletion "$a"; done'
# Each value option the hook skips, next to an unknown one (see the
# argocd app delete section), and the global boolean options, which must
# never be skipped with a value.
for opt in "${argocd_global_value_options[@]}" --app-namespace -N; do
  blocks "argocd app --some-new-option v $opt x confirm-deletion backend3"
done
for opt in --plaintext --insecure --grpc-web --port-forward --core --prompts-enabled; do
  blocks "argocd app $opt confirm-deletion backend3"
done
# The subcommands that only read stay allowed, also for an Application named
# confirm-deletion.
allows 'argocd app get backend3'
allows 'argocd app get backend3 --show-operation'
allows 'argocd app wait backend3 --health --timeout 120'
allows 'argocd app history backend3'
allows 'argocd app get confirm-deletion'
allows 'argocd app --some-new-option=v get confirm-deletion'
allows 'argocd app --server argocd.example.org get backend3 confirm-deletion'
allows "git commit -m 'fix(deploy): block argocd app confirm-deletion'"
allows "gh pr comment 220 --body 'argocd app confirm-deletion backend3 is blocked'"
allows "grep -rn 'argocd app confirm-deletion' docs"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block argocd app confirm-deletion\n\nargocd app confirm-deletion backend3\nEOF'
reason_mentions 'argocd app confirm-deletion backend3' '`argocd app confirm-deletion`'
reason_mentions 'argocd app confirm-deletion backend3' 'Prune=confirm or Delete=confirm'
reason_mentions 'argocd app confirm-deletion backend3' 'Stop and ask the maintainer'

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
# A patch that replaces or removes the whole object or its whole metadata
# drops the finalizers without naming them (ADR 0035): a JSON patch on the
# path "" or "/metadata", in JSON or YAML, and the strategic merge directives
# that replace or empty a map, or keep only some of its keys.
blocks "oc patch pvc data --type=json -p '[{\"op\":\"replace\",\"path\":\"/metadata\",\"value\":{\"name\":\"data\",\"namespace\":\"web\"}}]'"
blocks "oc patch pvc data --type json -p '[{\"op\": \"remove\", \"path\": \"/metadata\"}]'"
blocks "oc patch pvc data --type json -p '[{\"op\":\"replace\",\"path\":\"\",\"value\":{}}]'"
blocks "oc patch pvc data --type json -p \"[{op: replace, path: '/metadata', value: {}}]\""
blocks "oc patch pvc data --type json -p '- {op: replace, path: /metadata, value: {name: data}}'"
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: /metadata\n  value: {name: data}\''
blocks "oc patch pvc data -p '{\"metadata\":{\"\$patch\":\"replace\",\"name\":\"data\"}}'"
blocks "oc patch pvc data -p '{\"metadata\":{\"\$patch\":\"delete\"}}'"
blocks "oc patch pvc data -p 'metadata: {\$patch: replace, name: data}'"
blocks "oc patch pvc data -p '{\"metadata\":{\"\$retainKeys\":[\"name\",\"namespace\"]}}'"
# kubectl decodes every patch that does not start with { as YAML (go-yaml),
# which reads a tag or an anchor before a value as the same string, and a
# block scalar or an alias as a string too. The hook reads the value after a
# tag or anchor, and counts a block scalar or an alias after `path:` or
# `$patch:`, whose value it does not read (ADR 0035). A single-quoted empty
# path is the whole object too.
blocks $'oc patch pvc data --type json -p "- op: replace\n  path: \'\'\n  value: {}"'
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: !!str /metadata\n  value: {}\''
blocks "oc patch pvc data --type json -p '[{op: replace, path: !<tag:yaml.org,2002:str> /metadata, value: {}}]'"
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: &p /metadata\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: !!str &p ""\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: |-\n    /metadata\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: >-\n    /metadata\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: add\n  path: /metadata/labels/x\n  value: &p /metadata\n- op: replace\n  path: *p\n  value: {}\''
blocks $'oc patch pvc data -p \'metadata:\n  $patch: !!str replace\''
blocks $'oc patch pvc data -p \'metadata:\n  $patch: &d delete\''
blocks $'oc patch pvc data -p \'metadata:\n  $patch: |-\n    replace\''
# A tagged value split by an escaped line break inside single quotes is seen
# with its spaces removed, where the tag is glued to the value; and a tag
# glued to a value as written is a tag with an empty value, the path "".
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: !!str "/meta\\\n    data"\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: &p "/meta\\\n    data"\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: !!str/metadata\n  value: {}\''
# A tag with no value after it on its line is the empty string for go-yaml,
# the path "" (also after an anchor or before a comment); one whose value
# starts on the next line counts too. An anchor alone is null, which the
# API server refuses as a path.
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: !!str\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: &a !!str # empty\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: !<tag:yaml.org,2002:str>\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: !!str\n    /metadata\n  value: {}\''
blocks "oc patch pvc data --type json -p '[{op: replace, path: !!str , value: {}}]'"
blocks "oc patch pvc data --type json -p '[{op: replace, value: {}, path: !!str }]'"
allows "oc patch deployment web --type json -p '[{op: replace, path: !!str /metadata/labels/app, value: web}]'"
allows $'oc patch deployment web --type json -p \'- op: replace\n  path: /metadata/labels/app # !!str\n  value: web\''
# A comment may also stand there; it runs to the end of its line, and the
# value starts on the next one.
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: # all of it\n    /metadata\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path:\n    # all of it\n    ""\n  value: {}\''
blocks $'oc patch pvc data -p \'metadata:\n  $patch: # all of it\n    replace\''
allows $'oc patch deployment web --type json -p \'- op: replace\n  path: # not /metadata\n    /metadata/labels/app\n  value: web\''
blocks "oc patch application backend3 -n openshift-gitops --type json -p '[{\"op\":\"replace\",\"path\":\"/metadata\",\"value\":{\"name\":\"backend3\"}}]'"
blocks "oc --type json patch pvc data -p '[{\"op\":\"replace\",\"path\":\"/metadata\",\"value\":{}}]'"
blocks "bash -c \"oc patch pvc data --type json -p '[{\\\"op\\\":\\\"remove\\\",\\\"path\\\":\\\"/metadata\\\"}]'\""
allows "oc patch deployment web --type json -p '[{\"op\":\"replace\",\"path\":\"/metadata/labels/app\",\"value\":\"web\"}]'"
allows "oc patch deployment web --type json -p '[{\"op\":\"add\",\"path\":\"/metadata/annotations/note\",\"value\":\"x\"}]'"
allows "oc patch deployment web -p '{\"spec\":{\"\$patch\":\"merge\",\"replicas\":2}}'"
allows "oc patch configmap web -p '{\"data\":{\"path\":\"/metadata-backup\"}}'"
allows $'oc patch deployment web --type json -p \'- op: replace\n  path: !!str /metadata/labels/app\n  value: web\''
# Only a key named path counts: a volume mounted at /metadata is not one.
allows "oc patch deployment web -p '{\"spec\":{\"template\":{\"spec\":{\"containers\":[{\"name\":\"web\",\"volumeMounts\":[{\"name\":\"podinfo\",\"mountPath\":\"/metadata\"}]}]}}}}'"
allows "git commit -m 'fix(deploy): block a JSON patch on the path /metadata'"
reason_mentions "oc patch pvc data --type json -p '[{\"op\":\"replace\",\"path\":\"/metadata\",\"value\":{}}]'" 'whole object or its whole metadata'
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

# ------------------------------------------------------------- check 1: argocd app patch-resource (D105, ADR 0035)

# The oc patch rule, for the same patch sent through Argo CD to a resource
# that an Application manages: finalizers in each of the three --patch-type
# formats, additions included, and a Delete or Recycle reclaim policy, or one
# only the shell knows.
pvc_patch='argocd app patch-resource backend3 --kind PersistentVolumeClaim --resource-name data'
pv_patch='argocd app patch-resource backend3 --kind PersistentVolume --resource-name pv-1'
blocks "$pvc_patch --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "$pvc_patch --patch='{\"metadata\":{\"finalizers\":[]}}'"
blocks "$pvc_patch --patch-type application/merge-patch+json --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "$pvc_patch --patch-type 'application/json-patch+json' --patch '[{\"op\":\"remove\",\"path\":\"/metadata/finalizers\"}]'"
blocks "$pvc_patch --patch-type application/json-patch+json --patch '[{\"op\":\"add\",\"path\":\"/metadata/finalizers/-\",\"value\":\"x\"}]'"
blocks "$pvc_patch --patch-type application/strategic-merge-patch+json --patch '{\"metadata\":{\"\$deleteFromPrimitiveList/finalizers\":[\"kubernetes.io/pvc-protection\"]}}'"
blocks "$pvc_patch --patch '{\"metadata\":{\"Finalizers\":null}}'"
blocks "argocd app patch-resource website --kind Application --resource-name backend3 --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "argocd app patch-resource backend3 --kind PersistentVolumeClaim --all --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "$pv_patch --patch '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Delete\"}}'"
blocks "$pv_patch --patch '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'"
blocks "$pv_patch --patch-type application/json-patch+json --patch '[{\"op\":\"replace\",\"path\":\"/spec/persistentVolumeReclaimPolicy\",\"value\":\"Recycle\"}]'"
blocks "$pv_patch --patch '{\"spec\":{\"persistentvolumereclaimpolicy\":\"delete\"}}'"
blocks "$pv_patch --patch 'spec: {persistentVolumeReclaimPolicy: Recycle}'"
blocks "$pv_patch --patch \"{\\\"spec\\\":{\\\"persistentVolumeReclaimPolicy\\\":\\\"\$POLICY\\\"}}\""
# Every option order: the patch before the Application, before the
# subcommand, and the global options in front.
blocks "argocd app patch-resource --patch '{\"metadata\":{\"finalizers\":null}}' --kind PersistentVolumeClaim backend3"
blocks "argocd app --patch '{\"metadata\":{\"finalizers\":null}}' patch-resource backend3 --kind PersistentVolumeClaim"
blocks "argocd app --kind PersistentVolumeClaim --patch-type application/merge-patch+json patch-resource backend3 --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "argocd --server argocd.example.org --grpc-web app patch-resource backend3 --kind Pod --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "argocd --some-new-option value app patch-resource backend3 --kind Pod --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "argocd app --some-new-option v patch-resource backend3 --kind Pod --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "argocd app --some-new-option patch-resource backend3 --kind Pod --patch '{\"metadata\":{\"finalizers\":null}}'"
for opt in "${argocd_global_value_options[@]}" --resource-name --kind --group --namespace --project --patch --patch-type; do
  blocks "argocd app --some-new-option v $opt x patch-resource backend3 --kind Pod --patch '{\"metadata\":{\"finalizers\":null}}'"
done
for opt in --plaintext --insecure --grpc-web --port-forward --core --prompts-enabled; do
  blocks "argocd app $opt patch-resource backend3 --kind Pod --patch '{\"metadata\":{\"finalizers\":null}}'"
done
# Quotes, environment prefixes and wrappers.
blocks "argocd app 'patch-resource' backend3 --kind Pod --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "argocd app patch\\-resource backend3 --kind Pod --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "ARGOCD_SERVER=argocd.example.org $pvc_patch --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "env ARGOCD_OPTS=--grpc-web $pv_patch --patch '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Delete\"}}'"
blocks "sh -c \"$pvc_patch --patch '{\\\"metadata\\\":{\\\"finalizers\\\":null}}'\""
blocks "oc exec toolbox -- $pvc_patch --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "echo backend3 | xargs -I{} argocd app patch-resource {} --kind PersistentVolumeClaim --patch '{\"metadata\":{\"finalizers\":null}}'"
blocks $'bash <<\'EOF\'\nargocd app patch-resource backend3 --kind PersistentVolumeClaim --patch \'{"metadata":{"finalizers":null}}\'\nEOF'
blocks "./argocd-linux-amd64 app patch-resource backend3 --kind PersistentVolumeClaim --patch '{\"metadata\":{\"finalizers\":null}}'"
# A patch that replaces or removes the whole object or its whole metadata
# drops the finalizers without naming them, through Argo CD as through
# oc patch (ADR 0035).
blocks "$pvc_patch --patch-type application/json-patch+json --patch '[{\"op\":\"replace\",\"path\":\"/metadata\",\"value\":{\"name\":\"data\"}}]'"
blocks "$pvc_patch --patch-type application/json-patch+json --patch '[{\"op\":\"replace\",\"path\":\"\",\"value\":{}}]'"
blocks "$pvc_patch --patch-type application/strategic-merge-patch+json --patch '{\"metadata\":{\"\$patch\":\"replace\",\"name\":\"data\"}}'"
blocks "$pvc_patch --patch-type application/strategic-merge-patch+json --patch '{\"metadata\":{\"\$retainKeys\":[\"name\"]}}'"
allows "$pvc_patch --patch-type application/json-patch+json --patch '[{\"op\":\"add\",\"path\":\"/metadata/labels/tier\",\"value\":\"db\"}]'"
reason_mentions "$pvc_patch --patch-type application/json-patch+json --patch '[{\"op\":\"replace\",\"path\":\"/metadata\",\"value\":{}}]'" 'whole object or its whole metadata'
# JSON's \/ is a /: Argo CD sends the patch on as written, and the API
# server's JSON decoder reads it (ADR 0035).
blocks "$pvc_patch --patch-type application/json-patch+json --patch '[{\"op\":\"replace\",\"path\":\"\\/metadata\",\"value\":{\"name\":\"data\"}}]'"
# A harmless patch, Retain, and the subcommands that only read stay allowed.
allows "argocd app patch-resource backend3 --kind Deployment --resource-name web --patch '{\"spec\":{\"replicas\":2}}'"
allows "$pv_patch --patch '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Retain\"}}'"
allows "argocd app patch-resource backend3 --kind PersistentVolume --resource-name pv-delete-01 --patch '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Retain\"}}'"
allows "argocd app patch-resource backend3 --kind Deployment --resource-name recycler --patch '{\"metadata\":{\"labels\":{\"tier\":\"web\"}}}'"
allows 'argocd app get-resource backend3 --kind PersistentVolumeClaim --resource-name data'
allows 'argocd app resources backend3'
allows 'argocd app manifests backend3'
allows 'argocd app get patch-resource'
allows 'argocd app --some-new-option=v get patch-resource'
allows "git commit -m 'fix(deploy): block argocd app patch-resource of finalizers'"
allows "gh pr comment 220 --body 'argocd app patch-resource with finalizers is blocked'"
allows "grep -rn 'patch-resource' docs"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block finalizer patches through Argo CD\n\nargocd app patch-resource backend3 --kind PersistentVolumeClaim --patch \'{"metadata":{"finalizers":null}}\'\nEOF'
reason_mentions "$pvc_patch --patch '{\"metadata\":{\"finalizers\":null}}'" '`argocd app patch-resource` changes finalizers'
reason_mentions "$pv_patch --patch '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Recycle\"}}'" 'Retain is allowed'

# ------------------------------------------------------------- check 1: argocd app patch of an Application's finalizers (D110, ADR 0036)

# argocd app patch changes the Application itself, whose finalizers decide
# what a later deletion takes along, so the patch rule's finalizer match
# applies: a patch that names finalizers, additions included, in both --type
# formats, escapes included, and one that replaces the whole object or its
# metadata, which ADR 0035 counts as a finalizer change.
app_patch='argocd app patch backend3'
drop_finalizers="--patch '{\"metadata\":{\"finalizers\":null}}'"
blocks "$app_patch --type merge $drop_finalizers"
blocks "$app_patch $drop_finalizers"
blocks "$app_patch --type=merge --patch='{\"metadata\":{\"finalizers\":[]}}'"
blocks "$app_patch --patch '[{\"op\":\"remove\",\"path\":\"/metadata/finalizers\"}]' --type json"
blocks "$app_patch --type json --patch '[{\"op\":\"add\",\"path\":\"/metadata/finalizers/-\",\"value\":\"resources-finalizer.argocd.argoproj.io\"}]'"
blocks "$app_patch --type merge --patch '{\"metadata\":{\"Finalizers\":null}}'"
blocks "$app_patch --type merge --patch '{\"metadata\":{\"fin\\u0061lizers\":null}}'"
blocks "$app_patch --type merge --patch \$'{\"metadata\":{\"fin\\x61lizers\":null}}'"
blocks "$app_patch -N openshift-gitops --type merge $drop_finalizers"
blocks "$app_patch --type json --patch '[{\"op\":\"replace\",\"path\":\"/metadata\",\"value\":{\"name\":\"backend3\"}}]'"
blocks "$app_patch --type json --patch '[{\"op\":\"replace\",\"path\":\"\",\"value\":{}}]'"
blocks "$app_patch --type json --patch '[{\"op\":\"replace\",\"path\":\"\\/metadata\",\"value\":{}}]'"
# Every option order: the patch and its type before the Application and
# before the subcommand, unknown options, argocd's boolean global options,
# and each value option the hook skips, --type among them (ADR 0036), next
# to an unknown option.
blocks "argocd app patch $drop_finalizers --type merge backend3"
blocks "argocd app --type merge $drop_finalizers patch backend3"
blocks "argocd --server argocd.example.org --grpc-web app patch backend3 $drop_finalizers"
blocks "argocd app --some-new-option v patch backend3 $drop_finalizers"
blocks "argocd app --some-new-option patch backend3 $drop_finalizers"
for opt in "${argocd_global_value_options[@]}" --app-namespace -N --patch --type; do
  blocks "argocd app --some-new-option v $opt x patch backend3 $drop_finalizers"
done
for opt in --plaintext --insecure --grpc-web --port-forward --core --prompts-enabled; do
  blocks "argocd app $opt patch backend3 $drop_finalizers"
done
# Quotes, environment prefixes, wrappers, xargs, heredocs and download names.
blocks "argocd app 'patch' backend3 $drop_finalizers"
blocks "argocd app p\\atch backend3 $drop_finalizers"
blocks "ARGOCD_SERVER=argocd.example.org $app_patch --type merge $drop_finalizers"
blocks "sh -c \"$app_patch --type merge --patch '{\\\"metadata\\\":{\\\"finalizers\\\":null}}'\""
blocks "oc exec toolbox -- $app_patch --type merge $drop_finalizers"
blocks "echo backend3 | xargs -I{} argocd app patch {} --type merge $drop_finalizers"
blocks $'bash <<\'EOF\'\nargocd app patch backend3 --type merge --patch \'{"metadata":{"finalizers":null}}\'\nEOF'
blocks "./argocd-linux-amd64 app patch backend3 --type merge $drop_finalizers"
# A patch of anything else, a reclaim-policy word (the reclaim-policy match
# is not part of the answer, and an Application has no reclaim policy), an
# Application named patch, and the commands that only read stay allowed.
allows "$app_patch --type merge --patch '{\"spec\":{\"syncPolicy\":{\"automated\":null}}}'"
allows "$app_patch --patch '[{\"op\":\"replace\",\"path\":\"/spec/source/targetRevision\",\"value\":\"master\"}]' --type json"
allows "$app_patch --type json --patch '[{\"op\":\"add\",\"path\":\"/metadata/labels/tier\",\"value\":\"web\"}]'"
allows "$app_patch --type merge --patch '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Delete\"}}'"
allows 'argocd app get patch'
allows 'argocd app get backend3 -o json | jq .metadata.finalizers'
allows "argocd app get backend3 -o jsonpath='{.metadata.finalizers}'"
allows "git commit -m 'fix(deploy): block argocd app patch of finalizers'"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block finalizer patches of Applications\n\nargocd app patch backend3 --patch \'{"metadata":{"finalizers":null}}\'\nEOF'
reason_mentions "$app_patch --type merge $drop_finalizers" '`argocd app patch` changes finalizers of an Argo CD Application'
reason_mentions "$app_patch --type merge $drop_finalizers" "Argo CD's resources finalizer"

# ------------------------------------------------------------- check 1: Argo CD's deletion-approved annotation (D105, ADR 0035)

# Setting the annotation that argocd app confirm-deletion writes approves the
# same held-back deletions, so oc annotate, oc patch and any argocd command
# that name it are blocked, whatever the resource, the value or the patch
# type, its removal included. A JSON patch path writes the / of the key as ~1.
approval='argocd.argoproj.io/deletion-approved'
blocks "oc annotate application backend3 -n openshift-gitops $approval=2026-10-26T00:00:00Z"
blocks "oc annotate applications.argoproj.io/backend3 $approval=\"\$(date -u +%Y-%m-%dT%H:%M:%SZ)\""
blocks "oc annotate --overwrite application backend3 $approval=now"
blocks "oc annotate application backend3 --overwrite '$approval=now'"
blocks "oc annotate application backend3 \"$approval\"=now"
blocks "oc annotate application backend3 argocd.argoproj.io/deletion\\-approved=now"
blocks "oc annotate application backend3 ARGOCD.ARGOPROJ.IO/DELETION-APPROVED=now"
blocks "oc -n openshift-gitops annotate app backend3 $approval=now"
blocks "oc annotate application --all -n openshift-gitops $approval=now"
blocks "oc annotate application backend3 $approval-"
blocks "oc annotate pvc data $approval=now"
blocks "KUBECONFIG=hub.kubeconfig oc annotate application backend3 $approval=now"
blocks "sh -c 'oc annotate application backend3 $approval=now'"
blocks "oc get application -o name | xargs -I{} oc annotate {} $approval=now"
blocks "oc exec toolbox -- oc annotate application backend3 $approval=now"
blocks $'bash <<\'EOF\'\noc annotate application backend3 argocd.argoproj.io/deletion-approved=now\nEOF'
blocks "oc patch application backend3 -n openshift-gitops --type merge -p '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
blocks "oc patch application backend3 --type json -p '[{\"op\":\"add\",\"path\":\"/metadata/annotations/argocd.argoproj.io~1deletion-approved\",\"value\":\"now\"}]'"
blocks "oc patch app/backend3 --patch='metadata: {annotations: {$approval: now}}'"
blocks "oc -n openshift-gitops patch application backend3 -p '{\"metadata\":{\"annotations\":{\"$approval\":null}}}'"
blocks "argocd app patch backend3 --type merge --patch '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
blocks "argocd app patch backend3 --patch '[{\"op\":\"add\",\"path\":\"/metadata/annotations/argocd.argoproj.io~1deletion-approved\",\"value\":\"now\"}]'"
blocks "argocd app patch-resource website --kind Application --resource-name backend3 --patch '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
blocks "argocd app create backend3 --upsert --repo https://github.com/Verseghy/website_k8s.git --path backend3 --dest-server https://kubernetes.default.svc --dest-namespace web --annotations $approval=now"
blocks "argocd --server argocd.example.org app patch backend3 --type merge --patch '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
blocks "ARGOCD_SERVER=argocd.example.org argocd app patch backend3 --type merge --patch '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
blocks "bash -c \"argocd app patch backend3 --type merge --patch '{\\\"metadata\\\":{\\\"annotations\\\":{\\\"$approval\\\":\\\"now\\\"}}}'\""
blocks "./argocd-linux-amd64 app patch backend3 --type merge --patch '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
# The maintainer confirmed this reading as built (D109): every argocd command
# whose words name the key is blocked, one that only prints it and one in
# which argocd is only an argument included, and the subcommand is not looked
# for, so a writer behind two options that argocd_value_options lacks, which
# cobra hands on to `create`, is caught too. For text, the reason says to pass
# a file; the key is read through a pipe to grep or jq, or with oc get (below).
blocks 'argocd app list --selector deletion-approved=yes'
blocks "argocd app list -o name --selector $approval=yes"
blocks 'grep -rn argocd docs/deletion-approved.md'
blocks 'echo argocd deletion-approved'
# The other argocd checks read an argument-only argocd the same way, as
# they have since ADR 0028, which ADR 0035 section 2 item 3 states.
blocks 'echo argocd app delete x'
blocks 'echo argocd app confirm-deletion x'
blocks "argocd app --repo https://github.com/Verseghy/website_k8s.git --path backend3 create backend3 --upsert --annotations $approval=now"
reason_mentions 'grep -rn argocd docs/deletion-approved.md' '`argocd` command names'
reason_mentions 'grep -rn argocd docs/deletion-approved.md' 'write the text to a file and pass the file'
allows "argocd app get backend3 -o json | jq -r '.metadata.annotations[\"$approval\"]'"
allows 'argocd app list -o name | grep -c deletion-approved'
# Another key, Argo CD's own included, a patch of anything else, and the
# commands that only read the annotation stay allowed.
allows 'oc annotate application backend3 argocd.argoproj.io/refresh=hard'
allows 'oc annotate application backend3 -n openshift-gitops --overwrite argocd.argoproj.io/refresh=normal'
allows 'oc annotate application backend3 argocd.argoproj.io/refresh-'
allows 'oc annotate pvc data argocd.argoproj.io/sync-options=Prune=false'
allows 'oc annotate deployment web example.org/owner=web-team'
allows "oc patch application backend3 --type merge -p '{\"metadata\":{\"annotations\":{\"argocd.argoproj.io/refresh\":\"hard\"}}}'"
allows "argocd app patch backend3 --type merge --patch '{\"spec\":{\"syncPolicy\":{\"automated\":null}}}'"
allows "oc get application backend3 -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
allows 'oc get application backend3 -o yaml | grep deletion-approved'
allows 'oc describe application backend3 | grep -i deletion-approved'
allows 'argocd app get backend3 -o yaml | grep deletion-approved'
allows "git commit -m 'fix(deploy): block the $approval annotation'"
allows "gh pr create --title 'fix(deploy): block deletion approval' --body 'oc annotate application backend3 $approval=now is blocked'"
allows 'grep -rn deletion-approved docs'
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block the deletion-approved annotation\n\noc annotate application backend3 argocd.argoproj.io/deletion-approved=now\nEOF'
reason_mentions "oc annotate application backend3 $approval=now" '`oc annotate`'
reason_mentions "oc annotate application backend3 $approval=now" '`argocd app confirm-deletion` writes'
reason_mentions "oc patch application backend3 --type merge -p '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'" '`oc patch`'
reason_mentions "argocd app patch backend3 --type merge --patch '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'" 'Prune=confirm or Delete=confirm'
# `argocd` as the value of an option of an earlier oc, upstream Argo CD's
# default namespace or a context named so, is not an argocd command, so
# reading the annotation there stays allowed; setting it there does not.
allows "oc get application backend3 -n argocd -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
allows "oc get application backend3 --context argocd -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
allows 'oc -n argocd get applications -o custom-columns=NAME:.metadata.name,APPROVED:.metadata.annotations.argocd\.argoproj\.io/deletion-approved'
allows "timeout 30 oc get application backend3 --namespace argocd -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
# A namespace named like argocd's download names (see check 1: argocd app
# delete) is such a value too.
allows "oc get application backend3 -n argocd-system -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
blocks "oc annotate application backend3 -n argocd $approval=now"
blocks "oc --context argocd annotate application backend3 $approval=now"
blocks "oc patch application backend3 -n argocd --type merge -p '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
blocks "oc exec argocd-server-0 -n argocd -- argocd app patch backend3 --type merge --patch '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
blocks "sudo -n argocd app patch backend3 --type merge --patch '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
reason_mentions "oc annotate application backend3 -n argocd $approval=now" '`oc annotate`'
# That exception holds only where the oc surely runs and only reads (see
# oc_surely_reads in the hook). Behind a runner's own options, or after ssh,
# the word oc may be an option's value or a host, and argocd the program:
# GNU env runs `env -u oc -v argocd ...` (-u unsets a variable named oc, -v
# is --debug), bash runs `exec -a oc -c argocd ...`, and OpenSSH reads
# options again after the host, so `ssh oc -n argocd ...` runs argocd on
# the host oc. Neither reading of the oc verb may be get there either
# (`env -u oc -vu get ...`, `ssh oc -vl get ...`). A verb that runs a
# program runs argocd in a pod (`oc run ... -- sudo -n argocd ...`), and
# every reading of the verb must be get or describe: cobra runs
# `oc --new-option get run ...` as run, for an option the hook does not
# list (`--field-manager` is one it lists since ADR 0036). After
# assignments, runner words without options and `timeout` with a plain
# duration, the oc is the program, so reading the annotation there stays
# allowed. Anything else before it, a runner's options or another number
# included (`timeout -s KILL 30`, `nice 10`), leaves that unsure, which errs
# towards blocking.
approval_patch="--type merge --patch '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
blocks "env -u oc -v argocd app patch backend3 $approval_patch"
blocks "env -u oc -v argocd app create backend3 --upsert --annotations $approval=now"
blocks "env -u oc -v argocd app patch-resource website --kind Application --resource-name backend3 $approval_patch"
blocks 'env -u oc -v argocd app list --selector deletion-approved=yes'
blocks "exec -a oc -c argocd app patch backend3 $approval_patch"
blocks "sudo -u oc -n argocd app patch backend3 $approval_patch"
blocks "sudo -u oc -s argocd app patch backend3 $approval_patch"
blocks "doas -u oc -n argocd app patch backend3 $approval_patch"
blocks "ssh oc -n argocd app patch backend3 $approval_patch"
blocks "ssh -o HostName=hub.example.org oc -n argocd app patch backend3 $approval_patch"
blocks "env -u oc -vu get -v argocd app patch backend3 $approval_patch"
blocks "ssh oc -vl get -n argocd app patch backend3 $approval_patch"
blocks "oc run cli --image quay.io/argoproj/argocd --restart Never --command -- sudo -n argocd app patch backend3 $approval_patch"
blocks "oc --field-manager get run cli --image quay.io/argoproj/argocd --restart Never --command -- sudo -n argocd app patch backend3 $approval_patch"
blocks "oc --new-option get run cli --image quay.io/argoproj/argocd --restart Never --command -- sudo -n argocd app patch backend3 $approval_patch"
blocks "timeout -s KILL 30 oc get application backend3 -n argocd -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
blocks "nice 10 oc get application backend3 -n argocd -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
reason_mentions "env -u oc -v argocd app patch backend3 $approval_patch" '`argocd` command names'
allows "KUBECONFIG=hub.kubeconfig oc get application backend3 -n argocd -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
allows "sudo oc get application backend3 -n argocd -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
allows "timeout 1.5m oc get application backend3 -n argocd -o jsonpath='{.metadata.annotations.argocd\\.argoproj\\.io/deletion-approved}'"
allows "oc describe applications -n argocd -l $approval"

# ------------------------------------------------------------- check 1: the deletion-approved annotation in an inline manifest (D110, ADR 0036)

# A manifest that oc apply, oc create or oc replace reads from the command
# text sets the annotation as oc annotate does, so it is blocked in every
# inline form: a heredoc (quoted, unquoted or <<-), also through cat, a
# here-string, piped echo or printf text, also through more stages and cat,
# and the echo or printf output of a process substitution, in YAML or JSON,
# escapes included, wherever its -f points, after options, prefixes and
# wrappers, and in a --dry-run too, since the hook does not tell a manifest
# that sets the key from one that mentions it.
app_manifest=$'apiVersion: argoproj.io/v1alpha1\nkind: Application\nmetadata:\n  name: backend3\n  namespace: openshift-gitops\n  annotations:\n    argocd.argoproj.io/deletion-approved: "2026-10-26T00:00:00Z"'
blocks "oc apply -f - <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
blocks "oc apply -f - <<EOF"$'\n'"$app_manifest"$'\n'"EOF"
blocks "oc apply -f - <<-'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
blocks "oc create -f - <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
blocks "oc replace -f - <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
blocks "oc apply -f /dev/stdin <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
blocks "oc apply --server-side --dry-run=server -n openshift-gitops -f - <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
blocks "oc -n openshift-gitops apply -f - <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
blocks "KUBECONFIG=hub.kubeconfig timeout 30 oc apply -f - <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
blocks "cat <<'EOF' | oc apply -f -"$'\n'"$app_manifest"$'\n'"EOF"
blocks "cat <<'EOF' | tee app.yaml | oc apply -f -"$'\n'"$app_manifest"$'\n'"EOF"
blocks "oc apply -f - <<< '$app_manifest'"
blocks "oc replace -f - <<< '$app_manifest'"
blocks "echo '$app_manifest' | oc apply -f -"
blocks "echo -n '$app_manifest' | oc create -f -"
blocks "printf '%s\n' '$app_manifest' | oc apply -f -"
blocks "echo '$app_manifest' | tee app.yaml | oc apply -f -"
blocks "cat <<< '$app_manifest' | oc apply -f -"
blocks "oc apply -f <(echo '$app_manifest')"
blocks "oc create -f <(printf '%s\n' '$app_manifest')"
blocks "echo '{\"metadata\":{\"name\":\"backend3\",\"annotations\":{\"$approval\":\"now\"}}}' | oc apply -f -"
blocks "oc apply -f - <<< 'metadata: {annotations: {\"argocd.argoproj.io/deletion\\x2dapproved\": now}}'"
blocks "oc apply -f - <<< 'metadata: {annotations: {\"argocd.argoproj.io/deletion\\u002dapproved\": now}}'"
blocks "sh -c \"oc apply -f - <<< 'metadata: {annotations: {$approval: now}}'\""
blocks "echo '$app_manifest' | oc exec -i toolbox -- oc apply -f -"
blocks $'bash <<\'EOF\'\noc apply -f - <<< \'metadata: {annotations: {argocd.argoproj.io/deletion-approved: now}}\'\nEOF'
# Another annotation, a manifest that names no Redis client nor the key,
# the commands that only read it, a manifest written to a file, and texts
# that only mention the key stay allowed.
allows "oc apply -f - <<'EOF'"$'\n'"${app_manifest/deletion-approved/refresh}"$'\n'"EOF"
allows "echo '${app_manifest/deletion-approved/sync-wave}' | oc apply -f -"
allows "oc get application backend3 -o yaml | grep deletion-approved"
allows "cat > app.yaml <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
allows "echo '$app_manifest' > app.yaml"
allows "echo '$app_manifest' | grep -c deletion-approved"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block the annotation in an inline manifest\n\noc apply -f - with argocd.argoproj.io/deletion-approved is blocked\nEOF'
allows "gh pr comment 220 --body-file - <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"
reason_mentions "oc apply -f - <<< '$app_manifest'" '`oc apply` reads a manifest from the command text'
reason_mentions "oc create -f <(echo '$app_manifest')" '`oc create` reads a manifest'
reason_mentions "cat <<'EOF' | oc replace -f -"$'\n'"$app_manifest"$'\n'"EOF" '`oc replace` reads a manifest'
reason_mentions "echo '$app_manifest' | oc apply -f -" 'a --dry-run one included'

# ------------------------------------------------------------- check 1: patches read as their decoder reads them (ADR 0035)

# kubectl reads a patch as YAML, of which JSON is a part, and both decoders
# turn escapes in quoted strings into characters (oc 4.22 applies all of
# these), so finalizers, a reclaim policy and the deletion-approved key
# spelled with escapes count like the plain words, through oc patch and
# through Argo CD.
blocks "oc patch pvc data -p '{\"metadata\":{\"fin\\u0061lizers\":null}}'"
blocks "oc patch pvc data -p '{\"metadata\":{\"\\u0066\\u0069nalizers\":[]}}'"
blocks "oc patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"\\u0044elete\"}}'"
blocks "oc patch pv pv-1 -p 'spec: {persistentVolumeReclaimPolicy: \"\\x44elete\"}'"
blocks "oc patch pv pv-1 -p 'spec: {persistentVolumeReclaimPolicy: \"\\U00000052ecycle\"}'"
blocks "oc patch pv pv-1 -p '{\"spec\":{\"persistentVolume\\u0052eclaimPolicy\":\"Delete\"}}'"
blocks "oc patch application backend3 --type merge -p '{\"metadata\":{\"annotations\":{\"argocd.argoproj.io/deletion\\u002dapproved\":\"now\"}}}'"
blocks "oc patch application backend3 --type json -p '[{\"op\":\"add\",\"path\":\"/metadata/annotations/argocd.argoproj.io~1deletion\\u002Dapproved\",\"value\":\"now\"}]'"
blocks $'oc patch application backend3 --type merge -p \'metadata:\n  annotations:\n    "argocd.argoproj.io/deletion-\\x61pproved": now\''
blocks $'oc patch application backend3 --type merge -p \'metadata:\n  annotations:\n    ? "argocd.argoproj.io/deletion-\\\n      approved"\n    : now\''
blocks "argocd app patch backend3 --type merge --patch '{\"metadata\":{\"annotations\":{\"argocd.argoproj.io/deletion\\u002dapproved\":\"now\"}}}'"
blocks "$pvc_patch --patch '{\"metadata\":{\"\\u0066inalizers\":null}}'"
blocks "$pv_patch --patch '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"\\u0044elete\"}}'"
blocks "oc patch pvc data -p 'metadata: {\"f\\x69nalizers\": null}'"
blocks "oc patch pvc data -p 'metadata: {\"f\\U00000069nalizers\": null}'"
blocks "oc patch pvc data --type json -p '[{\"op\":\"replace\",\"path\":\"/met\\u0061data\",\"value\":{}}]'"
blocks "$pvc_patch --patch-type application/json-patch+json --patch '[{\"op\":\"replace\",\"path\":\"/met\\u0061data\",\"value\":{}}]'"
# YAML's escaped line break folds a key across lines. The hook removes a `\`
# and LF before anything else, as a shell line continuation; a $'...' part
# can decode one (`\\` and `\n`), and a CR LF line break is not a shell line
# continuation, so the decoder removes those itself (ADR 0035).
blocks $'oc patch pvc data -p \'metadata: {"final\\\n  izers": null}\''
blocks "oc patch pvc data -p \$'metadata: {\"final\\\\\\n  izers\": null}'"
blocks $'oc patch pvc data -p \'metadata: {"final\\\r\n  izers": null}\''
blocks $'oc patch application backend3 --type merge -p \'metadata:\r\n  annotations:\r\n    ? "argocd.argoproj.io/deletion-\\\r\n      approved"\r\n    : now\''
blocks $'oc patch pvc data --type json -p \'- op: replace\r\n  path: /metadata\r\n  value: {name: data}\''
# go-yaml also reads a CR alone, NEL, LS and PS as line breaks.
blocks $'oc patch pvc data -p \'metadata: {"final\\\r  izers": null}\''
blocks $'oc patch pvc data -p \'metadata: {"final\\\xc2\x85  izers": null}\''
blocks $'oc patch pvc data -p \'metadata: {"final\\\xe2\x80\xa8  izers": null}\''
blocks $'oc patch pvc data -p \'metadata: {"final\\\xe2\x80\xa9  izers": null}\''
# Inside single quotes the shell keeps a `\` and LF, which the hook removes
# first as a line continuation, so the next line's indentation stays inside
# a block YAML value that YAML joins (`"/meta\` then `    data"` is
# /metadata). The patch rule also reads each word with its spaces and tabs
# removed and its line breaks kept, which finds the key at the start of its
# line (ADR 0035).
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: "/meta\\\n    data"\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: "\\\n    "\n  value: {}\''
blocks $'oc patch pvc data --type json -p \'- op: replace\n  path: "/meta\\\n\t\tdata"\n  value: {}\''
allows $'oc patch deployment web --type json -p \'- op: replace\n  path: /metadata/labels/app\n  value: web\''
allows $'oc patch deployment web -p \'spec:\n  template:\n    spec:\n      containers:\n      - name: web\n        volumeMounts:\n        - mountPath: "/meta\\\n          data"\n          name: podinfo\''
# An escaped line break that a $'...' part decodes is removed with the next
# line's indentation, as YAML removes it, so a key or a value split by one
# counts as written whole, also before a comment.
blocks "oc patch pvc data --type json -p \$'- op: replace\\n  \"pa\\\\\\n    th\": /metadata\\n  value: {}'"
blocks "oc patch pvc data --type json -p \$'- op: replace\\n  path: \"/meta\\\\\\n    data\"\\n  value: {}'"
blocks "oc patch pvc data --type json -p \$'- op: replace\\n  \"pa\\\\\\n    th\": /metadata # all of it\\n  value: {}'"
# kubectl's YAML decoder refuses JSON's \/ in a JSON patch, but the API
# server reads it in a patch that kubectl or Argo CD sends as written, so the
# hook decodes it everywhere, which errs towards blocking (ADR 0035).
blocks "oc patch pvc data --type json -p '[{\"op\":\"replace\",\"path\":\"\\/metadata\",\"value\":{}}]'"
# A Retain written with an escape, and escapes of other characters, stay
# allowed.
allows "oc patch pv pv-1 -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"\\u0052etain\"}}'"
allows "oc patch deployment web -p '{\"spec\":{\"template\":{\"metadata\":{\"annotations\":{\"note\":\"caf\\u00e9 \\u2713\"}}}}}'"
allows "oc patch configmap web -p '{\"data\":{\"path\":\"C:\\\\temp\"}}'"
allows "oc patch configmap web -p '{\"data\":{\"path\":\"\\/metadata-backup\"}}'"
reason_mentions "oc patch pvc data -p '{\"metadata\":{\"fin\\u0061lizers\":null}}'" 'changes finalizers'

# ------------------------------------------------------------- checks 1 and 2: options before the oc verb (ADR 0035)

# oc finds its verb the way cobra does: an option it does not know at the top
# level takes the next word as its value, and the verb then reads every word
# before it as its own (oc 4.22 runs `oc -p X patch ...`, `oc --type merge
# patch ...` and `oc --overwrite K=V annotate ...`). So such an option cannot
# hide the verb from any check, nor can the global --vmodule, which takes a
# value, and the verb's arguments include the words before it.
blocks "oc -p '{\"metadata\":{\"finalizers\":null}}' patch pvc data"
blocks "oc --patch '{\"metadata\":{\"finalizers\":null}}' patch pvc data"
blocks "oc --type merge patch pvc data -p '{\"metadata\":{\"finalizers\":null}}'"
blocks "oc --type json patch pv pv-1 -p '[{\"op\":\"replace\",\"path\":\"/spec/persistentVolumeReclaimPolicy\",\"value\":\"Delete\"}]'"
blocks "oc -n openshift-gitops -p '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}' patch application backend3 --type merge"
blocks "oc --type merge patch application backend3 -n openshift-gitops -p '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}'"
blocks "oc --field-manager m annotate application backend3 -n openshift-gitops $approval=now"
blocks "oc --resource-version 5 annotate application backend3 $approval=now"
blocks "oc --vmodule x=1 -p '{\"metadata\":{\"finalizers\":null}}' patch pvc data"
blocks 'oc --vmodule x=1 delete pvc data'
blocks 'oc delete --vmodule x=1 pvc data'
blocks 'oc --vmodule=x=1 delete pvc data'
blocks 'oc --some-new-option v delete pvc data'
blocks 'oc --some-new-option v delete $(oc get pvc -o name)'
blocks 'oc --vmodule x=1 delete pod web-0 --force'
blocks 'oc --some-new-option v delete pod web-0 --force'
blocks 'oc --vmodule x=1 exec redis-0 -- redis-cli FLUSHALL'
blocks 'oc --vmodule x=1 --some-new-option v exec redis-0 -- redis-cli FLUSHALL'
blocks 'oc --some-new-option v exec postgres-0 -- psql'
blocks 'oc --some-new-option v rsh redis-0 redis-cli'
blocks "echo 'redis-cli flushall' | oc --some-new-option v exec -i redis-0 -- sh"
blocks 'oc --some-new-option v port-forward svc/postgres 5432'
blocks "sh -c 'oc --type merge patch pvc data -p {\"metadata\":{\"finalizers\":null}}'"
# Two such options before the verb, and positional words before it, which
# the verb reads as its arguments.
blocks "oc --type merge -p '{\"metadata\":{\"finalizers\":null}}' patch pvc data"
blocks "oc --field-manager m --overwrite=true annotate application backend3 $approval=now"
# (Without `=true`, cobra takes annotate for the value of --overwrite, and oc
# refuses the command.)
blocks "oc --overwrite $approval=now annotate application backend3"
blocks 'oc --wait pvc delete data'
blocks 'oc --wait ns delete web'
blocks 'oc --cascade=orphan --wait application delete backend3'
blocks 'oc --wait pvc delete data --force'
blocks 'oc --shell /usr/bin/psql rsh postgres-0'
blocks 'oc --shell=redis-cli rsh redis-0'
blocks 'oc --container redis -i exec redis-0 -- redis-cli ping'
# A boolean option before the verb, and harmless commands after such an
# option, stay allowed.
allows 'oc --insecure-skip-tls-verify get pods -o name'
allows 'oc --vmodule x=1 get pvc'
allows 'oc --vmodule x=1 logs -p web-0'
allows 'oc --match-server-version get application backend3 -o yaml'
allows 'oc --vmodule x=1 port-forward svc/web 8080:80'
allows 'oc --some-new-option v annotate deployment web example.org/owner=web-team'
allows "oc --type merge patch deployment web -p '{\"spec\":{\"replicas\":2}}'"
allows 'oc --some-new-option v exec redis-0 -- ls /data'
# Only the positional word right after such an option may be its value: an
# option with `=value` takes none, and without an option before it the
# first positional word is the verb, so a resource type named like a verb
# does not count.
allows 'oc --some-new-option=v get delete pvc'
allows 'oc get delete pvc'
# oc's own booleans, and a short option longer than two characters, take no
# word in cobra either; a boolean that a newer oc may add is read both ways.
for opt in --disable-compression --insecure-skip-tls-verify --match-server-version --warnings-as-errors; do
  allows "oc $opt get delete pvc"
done
allows 'oc -v6 get delete pvc'
blocks 'oc --some-new-boolean delete pvc data'

# ------------------------------------------------------------- checks 1 and 2: text that xargs or parallel pass on (ADR 0035)

# Piped echo or printf text, or a here-string, that xargs or GNU parallel
# read becomes the arguments of the command they run. That holds for every
# command whose arguments the checks read (oc, argocd and the database
# commands), not only for oc exec, oc rsh and oc debug, so the key words can
# not be fed to a blocked command that way.
blocks 'echo confirm-deletion backend3 | xargs argocd app'
blocks "printf '%s\\n' app confirm-deletion backend3 | xargs argocd"
blocks 'echo delete backend3 | xargs argocd app'
blocks 'echo confirm-deletion backend3 | xargs timeout 30 argocd app'
blocks 'echo confirm-deletion backend3 | parallel argocd app'
blocks 'echo confirm-deletion backend3 | xargs ./argocd-linux-amd64 app'
blocks "xargs argocd app <<< 'confirm-deletion backend3'"
blocks "echo $approval=now | xargs oc annotate application backend3 -n openshift-gitops"
blocks "echo '{\"metadata\":{\"annotations\":{\"$approval\":\"now\"}}}' | xargs -0 oc patch application backend3 --type merge -p"
blocks "echo '{\"metadata\":{\"finalizers\":null}}' | xargs -0 oc patch pvc data -p"
blocks 'echo pvc/data | xargs oc delete'
blocks 'echo pvc data | xargs oc delete'
blocks 'echo backend3 | xargs -I{} oc delete application {}'
blocks 'echo reset | xargs cargo loco db'
blocks 'echo reset | xargs ./target/debug/backend3-cli db'
blocks 'echo migrate fresh | xargs sea-orm-cli'
blocks "echo '-h db.example.org' | xargs psql"
blocks "echo '-h db.example.org backup.dump' | xargs pg_restore"
blocks "echo '-h legacy.example.org' | xargs mysql"
blocks "echo '-h legacy.example.org' | xargs mariadb"
# echo's own options are not printed, as bash's echo reads them, so they
# never reach the fed command, where `-n` would be oc's namespace option and
# take pvc/data as its value (ADR 0035). That holds after an assignment too.
blocks 'echo -n pvc/data | xargs oc delete'
blocks 'echo -ne ns web | xargs oc delete'
blocks 'echo -E -n pvc data | xargs oc delete'
blocks 'X=1 echo -n pvc/data | xargs oc delete'
blocks 'echo -n confirm-deletion backend3 | xargs argocd app'
allows 'echo -n web-0 | xargs oc get pods'
# The first word that is not an option is printed with all after it: here
# x-n is a resource type of its own, as oc reads it.
allows 'echo x-n ns web | xargs oc delete'
# Harmless commands fed that way, a runner that reads its arguments from a
# file (the piped text is then the command's stdin), and other programs,
# for which the text stays data, stay allowed.
allows 'echo backend3 | xargs argocd app get'
allows 'echo backend3 | xargs -I{} argocd app get {}'
allows 'echo pod/web-0 | xargs oc delete'
# With -a the arguments come from apps.txt, which the hook cannot read (like
# `oc delete -f list.txt`), and the piped text is only the command's stdin.
allows 'echo confirm-deletion backend3 | xargs -a apps.txt argocd app'
allows "echo 'oc delete pvc data' | xargs -0 git commit -m"
allows "echo '-h localhost' | xargs psql"

# ------------------------------------------------------------- checks 1 and 2: heredocs that xargs or parallel read, here-strings that cat passes on (ADR 0036)

# ADR 0035 left these spellings of its rules to ADR 0036: a heredoc that
# xargs or GNU parallel read, directly or through cat, gives the command
# they run its arguments, as a here-string does, and a here-string that cat
# passes on through a pipe is read as piped echo text is, by a shell, by
# xargs and by oc exec, oc rsh, oc debug and oc run.
blocks $'xargs oc <<\'EOF\'\ndelete ns web\nEOF'
blocks $'xargs oc delete <<EOF\nns/web\nEOF'
blocks $'parallel oc delete <<\'EOF\'\nns/web\nEOF'
blocks $'cat <<\'EOF\' | xargs oc\ndelete ns web\nEOF'
blocks $'cat <<\'EOF\' | parallel oc delete\npvc/data\nEOF'
blocks $'xargs argocd app <<\'EOF\'\nconfirm-deletion backend3\nEOF'
blocks $'xargs psql -h <<\'EOF\'\ndb.example.org\nEOF'
blocks $'echo start\nxargs oc <<\'EOF\'\ndelete ns web\nEOF'
blocks "cat <<< 'delete ns web' | xargs oc"
blocks "cat <<< 'oc delete ns web' | sh"
blocks "cat <<< 'oc delete ns web' | tr a a | bash"
blocks "cat <<< 'redis-cli flushall' | oc exec -i redis-0 -- sh"
blocks "cat <<< 'psql -c \"select 1\"' | oc exec -i pg-0 -- sh"
blocks "cat -u <<< 'redis-cli flushall' | oc rsh redis-0"
# A heredoc on a later line of the command is read as on the first one
# (ADR 0036): a script that oc exec reads runs in the cluster, and a commit
# message there is data.
blocks $'echo start\noc exec -i pg-0 -- sh <<\'EOF\'\npsql -c x\nEOF'
blocks $'cd /tmp\noc rsh redis-0 <<\'EOF\'\nredis-cli flushall\nEOF'
allows $'git add -A\ngit commit -F - <<\'EOF\'\nfix(deploy): oc delete ns web is blocked\nEOF'
# Other programs, and text that only xargs or cat print, stay allowed.
allows $'xargs echo <<\'EOF\'\ndelete ns web\nEOF'
allows $'xargs -n1 basename <<\'EOF\'\n/tmp/a\nEOF'
allows $'parallel echo <<\'EOF\'\noc delete ns web\nEOF'
allows "cat <<< 'delete ns web' | wc -l"
allows "cat <<< 'redis-cli flushall' > notes.txt"
allows "cat <<< 'oc delete ns web' | grep -c oc"

# ------------------------------------------------------------- checks 1 and 2: where a heredoc stands and what reads it (ADR 0036)

# A `<<` inside quotes is no heredoc, and the lines after it are commands:
# the hook reads a command a second time with only the operators that its
# quoting model finds surely outside quotes (see heredoc_texts), so a
# quoted `<<WORD`, on the first or a later line, hides nothing.
blocks $'true\necho \'<<X\'\noc exec redis-0 -- redis-cli flushall\nX'
blocks $'echo "<<EOF2"\noc delete ns web\nEOF2'
blocks $'echo "a\n<<EOF2"\noc delete ns web\nEOF2'
blocks $'git commit -m "subject\n\nbody mentions <<EOF2 here"\noc exec redis-0 -- redis-cli flushall\nEOF2'
blocks $'# a comment with <<X\noc delete ns web\nX'
blocks $'echo "a\\"<<X"\noc delete ns web\nX'
blocks $'echo \'a"b\'"c <<X"\noc delete ns web\nX'
blocks $'echo `echo <<X`\noc delete ns web\nX'
# The first reading still takes every operator, so a real heredoc after
# quoting the model does not know is read too.
blocks $'echo don\\\'t\noc exec -i redis-0 -- sh <<\'EOF\'\nredis-cli flushall\nEOF'
blocks $'echo `date`\noc exec -i pg-0 -- sh <<\'EOF\'\npsql -c x\nEOF'
# Where the model is sure, a heredoc is data as before: the commit idioms,
# a title with an apostrophe, a $(...) in double quotes and a comment.
allows $'cd "$(git rev-parse --show-toplevel)" && git commit -m "$(cat <<\'EOF\'\nfix(deploy): oc delete ns web is blocked\nEOF\n)"'
allows $'gh pr create --title "fix(deploy): it\'s blocked" --body "$(cat <<\'EOF\'\noc delete ns web is blocked\nEOF\n)"'
allows $'ls # it\'s here\ncat <<\'EOF\' > notes.txt\noc delete ns web\nEOF'
allows $'echo "a" \'b\' ${X} $1 a#b\ncat <<\'EOF\' > notes.txt\noc delete ns web\nEOF'
allows $'git commit -m "subject\n\nbody"\ncat <<\'EOF\' > notes.txt\noc delete ns web\nEOF'
allows $'git commit -m \'subject\n\nbody\'\ncat <<\'EOF\' > notes.txt\noc delete ns web\nEOF'
allows $'git commit -m "$(cat <<\'EOF\'\nmsg\nEOF\n)"\ncat <<\'EOF\' > notes.txt\noc delete ns web\nEOF'
allows $'git commit -m "$(\ncat <<\'EOF\'\nfix(deploy): oc delete ns web is blocked\nEOF\n)"'
# A body with an odd double quote would end the "$( word early if the
# heredoc were read as text, so these pin the model's open "$( too.
allows $'git commit -m "$(cat <<\'EOF\'\nfix(deploy): say "hi\noc delete ns web is blocked\nEOF\n)"'
allows $'git commit -m "$(\ncat <<\'EOF\'\nfix(deploy): say "hi\noc delete ns web is blocked\nEOF\n)"'
allows $'cd $(git rev-parse --show-toplevel)\ncat <<\'EOF\' > notes.txt\noc delete ns web\nEOF'
allows $'cat <<-\'EOF\' > notes.txt\n\toc delete ns web\n\tEOF'
allows 'echo $((1<<2)); oc get pods'
allows $'echo $((1<<2)); cat <<\'EOF\' > notes.txt\noc delete ns web\nEOF'
# A here-string is no heredoc, also when its word looks like a delimiter,
# and the quoting after a pipeline's next line still counts.
blocks $'cat <<<EOF\noc delete ns web\nEOF'
blocks $'cat <<\'EOF\' |\nhello\nEOF\ngrep "x\ny <<Z"\noc delete ns web\nZ'

# Every operator on a line counts, also after a `<` (`<(cat <<'EOF'`), and
# every receiver on the line reads every body on it, so the second heredoc
# of a line and a second heredoc on one descriptor are read too.
blocks $'oc apply -f <(cat <<\'EOF2\'\nmetadata:\n  annotations:\n    argocd.argoproj.io/deletion-approved: x\nEOF2\n)'
blocks $'oc create -f <(cat <<\'EOF\'\nspec:\n  containers:\n  - command: [redis-cli, FLUSHALL]\nEOF\n)'
blocks $'oc exec -i pod -- sh < <(cat <<\'EOF\'\nredis-cli flushall\nEOF\n)'
blocks $'oc exec -i pod -- sh <<\'A\'; oc apply -f - <<\'B\'\nls\nA\nargocd.argoproj.io/deletion-approved: x\nB'
blocks $'oc exec -i pg-0 -- cat <<\'A\'; oc exec -i pg-0 -- sh <<\'B\'\nnotes\nA\npsql -c x\nB'
blocks $'oc apply -f - <<\'EOF\' <<\'EOF2\'\nkind: ConfigMap\nEOF\ncommand: [redis-cli, FLUSHALL]\nEOF2'
allows $'cat <<\'A\' > a.txt; cat <<\'B\' > b.txt\noc delete ns web\nA\nmore\nB'
# A quoted option value with a separator does not hide the verb from the
# receiver tests.
blocks $'oc --selector=\'a|b\' apply -f - <<\'EOF\'\ncommand: [redis-cli, FLUSHALL]\nEOF'
blocks $'oc --selector=\'a|b\' exec -i pod -- sh <<\'EOF\'\nredis-cli flushall\nEOF'
blocks $'oc --selector="a;b" run -i tmp --image=redis -- sh <<\'EOF\'\nredis-cli flushall\nEOF'

# A pipeline that goes on after the bodies, and a group that closes on the
# line after them, take the body as their stdin.
blocks $'cat <<\'EOF\' |\nspec:\n  containers:\n  - command: [redis-cli, FLUSHALL]\nEOF\noc apply -f -'
blocks $'cat <<\'EOF\' |\nredis-cli FLUSHALL\nEOF\noc run -i tmp --image=redis -- sh'
blocks $'cat <<\'EOF\' |\nredis-cli FLUSHALL\nEOF\noc exec -i pod -- sh'
blocks $'cat <<\'EOF\' | tee copy.txt |\nredis-cli flushall\nEOF\noc rsh redis-0'
blocks $'cat <<\'EOF\' |&\ndelete ns web\nEOF\nxargs oc'
blocks $'cat <<\'EOF\' |\noc delete ns web\nEOF\nsh'
blocks $'( cat <<\'EOF2\'\nmetadata:\n  annotations:\n    argocd.argoproj.io/deletion-approved: x\nEOF2\n) | oc apply -f -'
blocks $'{ cat <<\'EOF2\'\nspec: {containers: [{command: [redis-cli]}]}\nEOF2\n} | oc create -f -'
allows $'cat <<\'EOF\' |\nredis-cli FLUSHALL\nEOF\nwc -l'
allows $'cat <<\'EOF\' |\nhello\nEOF\nwc -l\noc get pods'

# What a subshell or brace group prints is piped on like echo text.
blocks "{ echo 'argocd.argoproj.io/deletion-approved: x'; } | oc apply -f -"
blocks "(echo 'argocd.argoproj.io/deletion-approved: x') | oc apply -f -"
blocks "(echo 'command: [redis-cli, FLUSHALL]'; echo x) | oc create -f -"
blocks "(echo 'redis-cli flushall') | oc exec -i redis-0 -- sh"
blocks "{ echo 'redis-cli flushall'; } | oc run -i tmp --image=redis -- sh"
blocks "{ echo 'oc delete ns web'; } | sh"
blocks "( echo 'oc delete ns web' ) 2>/dev/null | bash"
allows "( echo hi; echo there ) | oc exec -i pod -- cat"
allows "{ echo 'kind: ConfigMap'; } | oc apply --dry-run=client -f -"
allows "( echo 'oc delete ns web' ) > out.txt; wc -l out.txt"
allows "{ echo x; } > out.txt; oc get pods"

# A process substitution that `<` redirects to stdin is that stdin.
blocks "oc run -i tmp --image=redis -- sh < <(echo 'redis-cli FLUSHALL')"
blocks "oc exec -i pod -- sh < <(echo 'redis-cli FLUSHALL')"
blocks "oc rsh redis-0 < <(printf 'redis-cli flushall\\n')"
blocks "oc apply -f - < <(echo 'argocd.argoproj.io/deletion-approved: x')"
blocks "xargs oc < <(echo delete ns web)"
allows "oc get pods < <(echo x)"
allows "wc -l < <(echo 'redis-cli flushall')"
allows "oc exec -i pod -- cat < <(echo hello)"

# A heredoc inside a script that a shell runs (a -c string, the arguments
# of eval or ssh, piped echo text) is read as at the top, and data there
# stays data.
blocks $'sh -c \'oc apply -f - <<EOF2\nmetadata:\n  annotations:\n    argocd.argoproj.io/deletion-approved: x\nEOF2\''
blocks $'bash -c "oc apply -f - <<EOF2\nmetadata:\n  annotations:\n    argocd.argoproj.io/deletion-approved: x\nEOF2"'
blocks $'ssh bastion \'oc create -f - <<EOF2\nspec:\n  containers:\n  - command: [redis-cli, FLUSHALL]\nEOF2\''
blocks $'sh -c \'oc run -i tmp --image=redis -- sh <<EOF\nredis-cli flushall\nEOF\''
blocks $'eval \'oc exec -i pg-0 -- sh <<EOF\npsql -c x\nEOF\''
blocks $'echo \'oc exec -i redis-0 -- sh <<EOF\nredis-cli flushall\nEOF\' | sh'
blocks "printf 'oc apply -f - <<EOF\\nargocd.argoproj.io/deletion-approved: x\\nEOF\\n' | oc exec -i deploy/tools -- sh"
blocks "printf 'oc apply -f - <<EOF\\nargocd.argoproj.io/deletion-approved: x\\nEOF\\n' | sh"
allows $'sh -c \'cat > notes.txt <<EOF\noc delete ns web\nEOF\''
allows $'bash -c \'git commit -F - <<EOF\nfix(deploy): oc delete ns web is blocked\nEOF\''

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

# ------------------------------------------------------------- check 2: a Redis client in the cluster (D105, ADR 0035)

# redis-cli run through oc exec, oc rsh or oc debug reaches the Redis in the
# cluster whatever its options and command, ping included. It is found
# wherever check 2 finds psql there: in the command part, by any path, in
# quotes, after env or timeout, in a script, on stdin, under xargs.
blocks 'oc exec deploy/redis-backend2 -- redis-cli FLUSHALL'
blocks 'oc exec deploy/redis-backend2 -- redis-cli ping'
blocks 'oc exec deploy/redis-backend2 -- redis-cli'
blocks 'oc exec -it -n verseghy-website deploy/redis-backend2 -c redis -- redis-cli -n 0 FLUSHDB'
blocks 'oc exec deploy/redis-backend2 -c redis -it -- redis-cli -h 127.0.0.1 -p 6379 flushall async'
blocks 'oc --namespace verseghy-website exec redis-0 -- redis-cli'
blocks 'oc rsh deploy/redis-backend2 redis-cli flushall'
blocks 'oc rsh -c redis deploy/redis-backend2 redis-cli'
blocks 'oc rsh -- deploy/redis-backend2 redis-cli'
blocks 'oc debug deploy/redis-backend2 -- redis-cli -h localhost'
blocks 'oc debug node/edge -- chroot /host redis-cli'
blocks 'oc exec redis-0 redis-cli'
blocks 'oc exec redis-0 -- /usr/local/bin/redis-cli flushall'
blocks 'oc exec redis-0 -- ./redis-cli'
blocks 'oc exec redis-0 -- env REDISCLI_HISTFILE=/dev/null redis-cli flushall'
blocks 'oc exec redis-0 -- timeout 5 redis-cli ping'
blocks "oc exec redis-0 -- 'redis-cli' flushall"
blocks 'oc exec redis-0 -- "redis-cli" flushall'
blocks 'oc exec redis-0 -- redis\-cli flushall'
blocks 'oc exec redis-0 -- r"edis-c"li flushall'
blocks "oc exec redis-0 -- sh -c 'redis-cli flushall'"
blocks "oc exec redis-0 -- sh -c 'cd /data;redis-cli flushall'"
blocks "oc exec redis-0 -- sh -c '\"redis-cli\" flushall'"
blocks 'oc rsh --shell /usr/local/bin/redis-cli redis-0'
blocks 'oc rsh --shell=redis-cli redis-0'
blocks 'oc rsh -f pod.yaml redis-cli flushall'
# Text the oc command reads on stdin runs in the cluster, piped, as a
# here-string or as a heredoc, also through further stages and line breaks,
# and in a pipeline longer than one 16 KiB chunk (redis-cli is a trigger word,
# so the chunk that holds only the echo is not skipped).
blocks 'echo FLUSHALL | oc exec -i deploy/redis-backend2 -- redis-cli'
blocks "echo 'redis-cli flushall' | oc exec -i deploy/redis-backend2 -- sh"
blocks "printf 'redis-cli flushall\n' | oc rsh deploy/redis-backend2"
blocks "echo 'redis-cli flushall' | tr a a | oc exec -i redis-0 -- sh"
blocks $'echo \'redis-cli flushall\' |\noc exec -i redis-0 -- sh'
blocks "oc exec -i redis-0 -- sh <<< 'redis-cli flushall'"
blocks $'oc rsh deploy/redis-backend2 <<\'EOF\'\nredis-cli flushall\nEOF'
blocks $'oc exec -i redis-0 -- sh <<EOF\nredis-cli flushall\nEOF'
blocks "echo '$filler redis-cli flushall' | oc exec -i redis-0 -- sh"
blocks "echo '$filler redis-cli flushall' | tr a a | oc rsh redis-0"
# Environment prefixes, runner words, wrappers and looked-up pods.
blocks 'KUBECONFIG=edge.kubeconfig oc exec redis-0 -- redis-cli ping'
blocks 'timeout 10 oc exec redis-0 -- redis-cli ping'
blocks "sh -c 'oc exec redis-0 -- redis-cli flushall'"
blocks "ssh bastion 'oc exec redis-0 -- redis-cli flushall'"
blocks 'oc exec $(oc get pod -l app=redis-backend2 -o name) -- redis-cli flushall'
blocks 'oc exec `oc get pod -l app=redis-backend2 -o name` -- redis-cli'
blocks 'echo redis-0 | xargs -I{} oc exec {} -- redis-cli flushall'
blocks 'echo flushall | xargs oc exec redis-0 -- redis-cli'
blocks "parallel 'oc exec {} -- redis-cli flushall' ::: redis-0"
blocks 'for p in redis-0 redis-1; do oc exec "$p" -- redis-cli flushall; done'
# A redis-cli on the agent's own host is not checked (D105): the clusters'
# Redis answers there only through a port-forward, which is blocked below.
allows 'redis-cli ping'
allows 'redis-cli -h localhost -p 6379 flushall'
allows 'redis-cli -h redis.example.org ping'
allows 'redis-cli -u redis://127.0.0.1:6379 info'
allows 'podman exec cache redis-cli ping'
allows 'podman run --rm docker.io/library/redis:7 redis-cli --version'
# A pod, container or namespace named like the client is not the command
# part; nor are the server, data paths and the client named only in a
# pattern, a file or a script for the local shell.
allows 'oc exec redis-cli-0 -- ls /data'
allows 'oc exec -c redis-cli deploy/cache -- ls'
allows 'oc exec -n redis-cli deploy/cache -- df -h'
allows 'oc rsh deploy/redis-backend2 ls -l /data'
allows 'oc exec deploy/redis-backend2 -- redis-server --version'
allows 'oc exec deploy/redis-backend2 -- cat /usr/local/etc/redis/redis.conf'
allows 'oc exec redis-0 -- cat /data/redis-cli.log'
allows 'oc exec redis-0 -- ls /data/redis-cli-backup'
allows 'oc exec deploy/legacy -- grep -E "(redis-cli|psql)" /etc/hosts'
allows 'oc logs deploy/redis-backend2'
allows 'oc describe deploy/redis-backend2'
allows "echo 'redis-cli flushall' > notes.txt; oc rsh deploy/redis-backend2 ls"
allows "echo 'redis-cli -h localhost ping' | sh"
allows "git commit -m 'fix(deploy): block redis-cli in oc exec'"
allows "gh pr comment 220 --body 'redis-cli in a pod (through oc exec) is blocked'"
allows "rg -n 'oc (exec|rsh) .*redis-cli' .claude scripts"
# A whole in-cluster command inside a quoted string counts, as it does for
# psql (`parallel 'oc exec {} -- psql'` runs it), so a PR body that quotes
# one is blocked, and the reason says to pass the text as a file.
reason_mentions "gh pr comment 220 --body 'oc exec deploy/redis-backend2 -- redis-cli FLUSHALL'" 'gh pr create --body-file <file>'
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block a Redis client in the cluster\n\noc exec deploy/redis-backend2 -- redis-cli FLUSHALL\nEOF'
reason_mentions 'oc exec deploy/redis-backend2 -- redis-cli FLUSHALL' 'runs inside the cluster through `oc exec`'
reason_mentions 'oc rsh deploy/redis-backend2 redis-cli' 'reaches the Redis there'
reason_mentions 'oc debug deploy/redis-backend2 -- redis-cli' 'A Redis client on this host is not blocked'

# ------------------------------------------------------------- check 2: the other Redis clients (D110, ADR 0036)

# redis-benchmark, valkey-cli and keydb-cli are blocked wherever redis-cli
# is: redis-benchmark ships in the clusters' Redis image and sends any
# command it is given (`redis-benchmark -n 1 FLUSHALL` empties the
# database), and the others are the clients of Redis's protocol-compatible
# forks. Each one is a trigger word, so the chunk of a long pipeline that
# holds only the echo is still read. On the agent's own host none of them
# is checked, and a pod or file named like one is not the command part.
for client in redis-benchmark valkey-cli keydb-cli; do
  blocks "oc exec deploy/redis-backend2 -- $client -n 1 FLUSHALL"
  blocks "oc rsh deploy/redis-backend2 $client flushall"
  blocks "oc debug deploy/redis-backend2 -- /usr/local/bin/$client"
  blocks "oc exec redis-0 -- sh -c 'cd /data;$client flushall'"
  blocks "echo '$client flushall' | oc exec -i redis-0 -- sh"
  blocks "echo '$filler $client flushall' | oc exec -i redis-0 -- sh"
  blocks "oc run tmp --image=redis:6.2-alpine -- $client -h redis-backend2 flushall"
  blocks "oc create job flush --image=redis:6.2-alpine -- $client -h redis-backend2 flushall"
  blocks "oc apply -f - <<< 'command: [$client, -h, redis-backend2, flushall]'"
  allows "$client -h localhost ping"
  allows "oc exec $client-0 -- ls /data"
  allows "oc exec redis-0 -- cat /data/$client.log"
done
reason_mentions 'oc exec deploy/redis-backend2 -- redis-benchmark -n 1 FLUSHALL' '`redis-benchmark` runs inside the cluster through `oc exec`'
allows 'podman run --rm docker.io/library/redis:6.2-alpine redis-benchmark -n 1 -q PING'

# ------------------------------------------------------------- check 2: a Redis client in a pod the agent starts (D110, ADR 0036)

# oc run gives the pod every positional word after the pod's name, before
# `--` and after it, as its arguments or (with --command) its command.
blocks 'oc run tmp -i --image=redis:6.2-alpine -- redis-cli -h redis-backend2 FLUSHALL'
blocks 'oc run tmp --rm -it --restart=Never --image=redis:6.2-alpine -- redis-cli -h redis-backend2 ping'
blocks 'oc run tmp --image=redis:6.2-alpine --command -- /usr/local/bin/redis-cli -h redis-backend2 flushall'
blocks 'oc run tmp --image=redis redis-cli -h redis-backend2 flushall'
# A client before `--` runs too: cobra keeps the words before it, and the
# image's entrypoint runs a first argument that is not an option.
blocks 'oc run tmp --image=redis --command redis-cli -- -h redis-backend2 FLUSHALL'
blocks 'oc run tmp --image=redis:6.2-alpine redis-cli -- -h redis-backend2 FLUSHALL'
blocks 'oc run tmp -i --rm --restart=Never --image=redis redis-benchmark -- -h redis-backend2 -n 1 FLUSHALL'
blocks "oc run tmp --image=redis sh -- -c 'redis-cli flushall'"
blocks 'oc run tmp --image=redis --env A=b -- redis-cli -- flushall'
blocks 'oc create job flush --image=redis redis-cli -- -h redis-backend2 FLUSHALL'
allows 'oc run tmp --image=redis redis-server -- --port 6380'
allows 'oc run tmp --image=redis --command redis-server -- --save ""'
blocks 'oc run tmp --image redis --restart Never redis-cli flushall'
blocks 'oc run --rm tmp --image=redis redis-cli flushall'
blocks 'oc run tmp --new-option v --image=redis redis-cli flushall'
blocks 'oc run tmp --image=redis -- sh -c "redis-cli -h redis-backend2 flushall"'
blocks "oc run tmp --image=redis -- sh -c 'cd /data;redis-cli flushall'"
blocks "oc run tmp --image=redis -- 'redis-cli' flushall"
blocks 'oc run tmp --image=redis -- redis\-cli flushall'
blocks "oc run tmp --image=redis -- \$'redis\\x2dcli' flushall"
blocks 'oc run tmp --image=redis -- env REDISCLI_HISTFILE=/dev/null redis-cli flushall'
blocks 'oc run tmp --image=redis -- timeout 5 redis-cli ping'
blocks 'oc run tmp --env "GREETING=hello there" --image=redis -- redis-cli flushall'
blocks 'oc -n verseghy-website run tmp --image=redis -- redis-cli flushall'
blocks 'oc --image redis run tmp -- redis-cli flushall'
blocks 'KUBECONFIG=edge.kubeconfig oc run tmp --image=redis -- redis-cli flushall'
blocks 'timeout 60 oc run tmp --image=redis -- redis-cli flushall'
blocks "sh -c 'oc run tmp --image=redis -- redis-cli flushall'"
blocks "ssh bastion 'oc run tmp --image=redis -- redis-cli flushall'"
blocks 'echo redis-backend2 | xargs -I{} oc run tmp --image=redis -- redis-cli -h {} flushall'
blocks "echo 'redis-cli flushall' | xargs oc run tmp --image=redis -- sh -c"
blocks 'oc run tmp --image=redis -- oc exec pg-0 -- psql'
blocks "parallel 'oc run {} --image=redis -- redis-cli flushall' ::: tmp"
blocks "parallel 'oc create job {} --image=redis -- redis-cli flushall' ::: flush"
# The --overrides JSON, which can set the container's command or arguments,
# also before the verb and with JSON escapes.
blocks "oc run tmp --image=redis --overrides='{\"spec\":{\"containers\":[{\"name\":\"tmp\",\"image\":\"redis\",\"command\":[\"redis-cli\",\"-h\",\"redis-backend2\",\"FLUSHALL\"]}]}}'"
blocks "oc run tmp --image=redis --overrides '{\"spec\":{\"containers\":[{\"name\":\"tmp\",\"image\":\"redis\",\"args\":[\"sh\",\"-c\",\"redis-cli flushall\"]}]}}'"
blocks "oc run tmp --image=redis --override-type=strategic --overrides='{\"spec\":{\"containers\":[{\"name\":\"tmp\",\"command\":[\"/usr/local/bin/redis-benchmark\",\"-n\",\"1\",\"FLUSHALL\"]}]}}'"
blocks "oc --overrides '{\"spec\":{\"containers\":[{\"name\":\"tmp\",\"command\":[\"redis-cli\"]}]}}' run tmp --image=redis"
blocks "oc run tmp --image=redis --overrides='{\"spec\":{\"containers\":[{\"name\":\"tmp\",\"command\":[\"redis\\u002dcli\"]}]}}'"
# Shell quoting inside a script string of the JSON, which the pod's shell
# removes, and JSON that a heredoc gives --overrides through cat.
blocks "oc run tmp --image=redis --overrides='{\"spec\":{\"containers\":[{\"name\":\"tmp\",\"command\":[\"sh\",\"-c\",\"redis-c\\\"l\\\"i flushall\"]}]}}'"
blocks $'oc run tmp --image=redis --overrides="$(cat <<\'EOF\'\n{"spec":{"containers":[{"name":"tmp","image":"redis","command":["redis-cli","flushall"]}]}}\nEOF\n)"'
# The text oc run passes to the pod on stdin: a heredoc, also through cat,
# a here-string and piped echo or printf text, read for a Redis client
# whatever the pod's program, which errs towards blocking.
blocks $'oc run -i --rm tmp --image=redis --restart=Never -- sh <<\'EOF\'\nredis-cli -h redis-backend2 flushall\nEOF'
blocks $'oc run -i --rm tmp --image=redis --restart=Never -- python3 - <<\'EOF\'\nimport os; os.system("redis-cli flushall")\nEOF'
# The text is also matched by name, so a client in a list or JSON counts.
blocks $'oc run -i tmp --image=python -- python3 <<\'EOF\'\nimport subprocess; subprocess.run(["redis-cli", "FLUSHALL"])\nEOF'
blocks "oc run -i tmp --image=node -- node <<< 'require(\"child_process\").execFileSync(\"redis-benchmark\",[\"-n\",\"1\",\"FLUSHALL\"])'"
allows $'oc run -i tmp --image=python -- python3 <<\'EOF\'\nprint(["redis-server", "--port", "6380"])\nEOF'
blocks $'cat <<\'EOF\' | oc run -i --rm tmp --image=redis --restart=Never -- sh\nredis-cli -h redis-backend2 flushall\nEOF'
blocks $'echo start\noc run -i --rm tmp --image=redis -- sh <<\'EOF\'\nredis-cli flushall\nEOF'
blocks "oc run -i --rm tmp --image=redis -- sh <<< 'redis-cli -h redis-backend2 flushall'"
blocks "echo 'redis-cli -h redis-backend2 flushall' | oc run -i --rm tmp --image=redis -- sh"
blocks "printf 'cd /data\nredis-cli flushall\n' | oc run -i tmp --image=redis -- sh"
blocks "cat <<< 'redis-cli flushall' | oc run -i tmp --image=redis -- sh"
blocks "echo '$filler redis-cli flushall' | oc run -i tmp --image=redis -- sh"
# oc create job, cronjob, deployment and deploymentconfig, and their aliases,
# take the pod's command after `--`; the subcommand is found as oc's verb is,
# behind an option the hook does not know or a boolean of create's own.
blocks 'oc create job flush --image=redis:6.2-alpine -- redis-cli -h redis-backend2 FLUSHALL'
blocks 'oc create cronjob flush --image=redis --schedule="*/5 * * * *" -- redis-cli -h redis-backend2 flushall'
blocks "oc create cronjob flush --image=redis --schedule '*/5 * * * *' -- redis-cli flushall"
blocks "oc create --schedule '*/5 * * * *' cronjob flush --image=redis -- redis-cli flushall"
blocks "oc create --schedule '*/5 * * * *' cronjob flush --image=redis -- sh -c 'redis-cli flushall'"
blocks 'oc create cj flush --image=redis --schedule=@hourly -- redis-cli flushall'
blocks 'oc create deployment flush --image=redis -- redis-cli flushall'
blocks 'oc create deploy flush --image=redis -r 1 -- redis-cli flushall'
blocks 'oc create deploymentconfig flush --image=redis -- redis-cli flushall'
blocks 'oc create dc flush --image=redis -- sh -c "redis-cli flushall"'
blocks 'oc -n verseghy-website create job flush --image=redis -- /usr/local/bin/redis-cli flushall'
blocks 'oc create --save-config job flush --image=redis -- redis-cli flushall'
blocks 'oc create --new-option v job flush --image=redis -- redis-cli flushall'
blocks 'oc create job flush --image=redis redis-cli flushall'
blocks "sh -c 'oc create job flush --image=redis -- redis-cli flushall'"
# A Redis client in an inline manifest that oc apply, oc create or oc
# replace reads, in every form of the deletion-approved case above, as a
# command, as arguments, as a block list, in JSON and with escapes.
job_manifest=$'apiVersion: batch/v1\nkind: Job\nmetadata:\n  name: flush\nspec:\n  template:\n    spec:\n      restartPolicy: Never\n      containers:\n      - name: flush\n        image: redis:6.2-alpine\n        command: ["redis-cli", "-h", "redis-backend2", "FLUSHALL"]'
blocks "oc apply -f - <<'EOF'"$'\n'"$job_manifest"$'\n'"EOF"
blocks "oc create -f - <<EOF"$'\n'"$job_manifest"$'\n'"EOF"
blocks "cat <<'EOF' | oc replace -f -"$'\n'"$job_manifest"$'\n'"EOF"
blocks "oc apply -f - --dry-run=server <<'EOF'"$'\n'"$job_manifest"$'\n'"EOF"
blocks "oc apply -f - <<< '$job_manifest'"
blocks "echo '$job_manifest' | oc apply -f -"
blocks "printf '%s\n' '$job_manifest' | oc create -f -"
blocks "cat <<< '$job_manifest' | oc apply -f -"
blocks "oc apply -f <(echo '$job_manifest')"
blocks $'oc apply -f - <<\'EOF\'\nkind: Pod\nspec:\n  containers:\n  - name: flush\n    image: redis\n    args: [sh, -c, "cd /data; redis-cli flushall"]\nEOF'
blocks $'oc apply -f - <<\'EOF\'\nkind: Pod\nspec:\n  containers:\n  - name: flush\n    image: redis\n    command:\n    - /usr/local/bin/redis-cli\n    - FLUSHALL\nEOF'
blocks "echo '{\"kind\":\"Pod\",\"spec\":{\"containers\":[{\"name\":\"flush\",\"image\":\"redis\",\"command\":[\"redis-cli\",\"FLUSHALL\"]}]}}' | oc apply -f -"
blocks "oc apply -f - <<< 'spec: {containers: [{name: x, image: redis, command: [\"redis\\x2dcli\", FLUSHALL]}]}'"
# A name that a YAML escaped line break folds across lines, and shell
# quoting inside a script string, which the pod's shell removes.
blocks $'oc apply -f - <<\'EOF\'\nspec:\n  containers:\n  - command: ["redis-\\\n     cli", FLUSHALL]\nEOF'
blocks $'oc apply -f - <<\'EOF\'\nspec:\n  containers:\n  - command: [sh]\n    args: ["-c", "redis-c\'l\'i FLUSHALL"]\nEOF'
blocks "echo '{\"spec\":{\"containers\":[{\"args\":[\"-c\",\"redis-c\\\"l\\\"i FLUSHALL\"]}]}}' | oc create -f -"
allows $'oc apply -f - <<\'EOF\'\nspec:\n  containers:\n  - command: [redis-server]\n    args: ["--save", ""]\nEOF'
# A pod or image named like a client, a client only named as a value of an
# option, the server, a database command (which keeps its target check
# there, ADR 0035), what only the command text of another program shows,
# a manifest without a client and one written to a file stay allowed.
allows 'oc run redis-cli --image=redis:6.2-alpine -- redis-server'
allows 'oc run redis-cli --image=redis:6.2-alpine'
allows 'oc run cache --image=redis:6.2-alpine'
allows 'oc run tmp --image=quay.io/example/redis-cli'
# The pod's name after an option that takes a value (see oc_value_options,
# which lists run's since ADR 0036).
allows 'oc run --restart Never redis-cli --image=redis:6.2-alpine'
allows "oc run --overrides '{}' redis-cli --image=redis:6.2-alpine"
# Text that xargs passes to oc run or oc apply is their arguments, not what
# the pod or oc reads on stdin.
allows 'echo redis-cli | xargs oc run --image=redis:6.2-alpine'
allows 'echo deletion-approved.yaml | xargs oc apply -f'
allows 'oc run tmp --image redis-cli -- sleep 3600'
allows 'oc run tmp --image=quay.io/example/redis-cli -- sleep 3600'
allows 'oc run tmp --image=redis -l app=redis-cli -- redis-server'
allows 'oc run tmp --image=redis --env=CLIENT=redis-cli -- redis-server'
allows 'oc run tmp --image=redis --restart=Never -- cat /data/redis-cli.log'
allows "oc run tmp --image=redis --overrides='{\"metadata\":{\"labels\":{\"app\":\"cache\"}}}' -- redis-server"
allows 'oc run psql-client --image=postgres:18 -- psql -h localhost -c "select 1"'
blocks 'oc run psql-client --image=postgres:18 -- psql -h postgres -c "select 1"'
allows $'oc run -i tmp --image=busybox -- tee /tmp/notes <<\'EOF\'\npsql -h postgres and oc delete ns web are only text here\nEOF'
allows 'oc create deployment cache --image=redis:6.2-alpine -- redis-server --save ""'
allows 'oc create deployment redis-cli --image=redis'
allows 'oc create job --from=cronjob/redis-cli backup-now'
allows 'oc create configmap redis-cli-config --from-literal=client=redis-cli'
allows 'oc create service clusterip redis --tcp=6379:6379'
allows "oc apply -f - <<'EOF'"$'\n'"${job_manifest/redis-cli/redis-server}"$'\n'"EOF"
allows $'oc apply -f - <<\'EOF\'\nkind: Service\nmetadata:\n  name: redis-cli-0\nEOF'
allows $'oc apply -f - <<\'EOF\'\nkind: ConfigMap\nmetadata:\n  name: my-redis-cli\nEOF'
allows "cat > job.yaml <<'EOF'"$'\n'"$job_manifest"$'\n'"EOF"
allows "echo 'redis-cli ping' > notes.txt; oc run tmp --image=busybox -- sleep 1"
allows "git commit -m 'fix(deploy): block redis-cli in oc run and oc create job'"
allows $'git commit -F - <<\'EOF\'\nfix(deploy): block a Redis client in a new pod\n\noc run tmp --image=redis -- redis-cli FLUSHALL\nEOF'
reason_mentions 'oc run tmp --image=redis -- redis-cli flushall' 'the pod that `oc run` starts'
reason_mentions 'oc create job flush --image=redis -- redis-cli flushall' 'the pod that `oc create job` starts'
reason_mentions 'oc create cj flush --image=redis --schedule=@hourly -- redis-cli flushall' '`oc create cj`'
reason_mentions "oc run tmp --image=redis --overrides='{\"spec\":{\"containers\":[{\"name\":\"tmp\",\"command\":[\"redis-cli\"]}]}}'" 'in its --overrides'
reason_mentions "oc apply -f - <<< '$job_manifest'" 'reads a manifest from the command text that names `redis-cli`'

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
# For a Service, kubectl reads the remote port with Go's strconv.Atoi, which
# takes a leading +, and forwards the plain number (ADR 0035).
blocks 'oc port-forward -n verseghy svc/postgres 15432:+5432'
blocks 'oc port-forward svc/mysql 13306:+3306'
blocks 'oc port-forward -n verseghy svc/redis-backend2 :+6379'
blocks 'oc port-forward svc/postgres +05432'
allows 'oc port-forward -n verseghy svc/postgres 19187:+9187'
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
# check 2 does not check a redis-cli on this host (only in the cluster), so
# the reason does not claim it would take a forwarded Redis for a local
# database.
reason_mentions 'oc port-forward svc/redis-backend2 16379:6379' 'does not check a Redis client on this host at all'
# The answer to D106 confirmed both readings of D69 as built: the check knows
# the engines the clusters run (PostgreSQL, MySQL, Redis) by their ports, and
# never reads the pod or service name. So a metrics port on a database
# Service, and the default port of an engine the clusters do not run, stay
# allowed until an amending ADR adds them.
allows 'oc port-forward svc/postgres 9187'
allows 'oc port-forward -n verseghy svc/postgres 19187:9187'
allows 'oc port-forward svc/mongodb 27017'
allows 'oc port-forward svc/mysql 9104:9104'
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
# The mariadb:// scheme of a connection URI counts like mysql://, for both
# clients.
blocks 'mariadb mariadb://root@legacy.example.org:3306/legacy'
blocks 'mysql mariadb://root@legacy.example.org:3306/legacy'
allows 'mariadb mariadb://root@127.0.0.1:3306/legacy'
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

# ------------------------------------------------------------- every check: $'...' and $"..." quoting (ADR 0035)

# bash decodes the escapes of an ANSI-C quoted $'...' part (\xHH, \NNN, \uHHHH,
# \UHHHHHHHH, \n and the rest) and reads a $"..." part as a double-quoted
# string, so neither hides a program, a verb, a kind, a key or a target, also
# in a heredoc's receiving line and in a -c string run in a pod. A `\'` does
# not end a $'...' part. Text in them that only mentions a command stays
# allowed.
blocks "oc delete \$'ns' web"
blocks 'oc delete $"ns" web'
blocks "oc delete \$'\\x6e\\x73' web"
blocks "oc delete \$'\\156\\163' web"
blocks "oc delete \$'\\u006e\\U00000073' web"
# A NUL ends a $'...' part in bash: `$'n\0x's` is ns.
blocks "oc delete \$'n\\0x's web"
blocks "o\$'c' delete ns web"
blocks "oc \$'delete' ns web"
blocks "oc delete \$'pvc' data"
blocks "oc delete \$'p'v\$\"c\" data"
blocks "kube\$'ctl' delete pvc data"
blocks "\$'\\x6b'ubectl get pods"
blocks "argocd app \$'delete' backend3"
blocks "argocd app \$'confirm-deletion' backend3"
blocks "oc annotate application backend3 \$'argocd.argoproj.io/deletion-\\x61pproved=now'"
blocks "oc patch pvc data -p \$'{\"metadata\":{\"fin\\x61lizers\":null}}'"
blocks "cargo loco db \$'reset'"
blocks "oc rsh postgres-0 \$'psql'"
blocks "oc exec redis-0 -- \$'redis-cli' flushall"
blocks "oc exec postgres-0 -- sh -c \"\\\$'psql'\""
blocks "oc exec redis-0 -- sh -c \"\\\$'redis-cli' ping\""
blocks "o\$'c' exec -i postgres-0 -- sh <<'EOF'"$'\npsql\nEOF'
blocks "o\$'c' exec -i redis-0 -- sh <<'EOF'"$'\nredis-cli flushall\nEOF'
blocks "psql -h \$'db.example.org'"
blocks "PGHOST=\$'db.example.org' psql"
blocks "DATABASE_URL=\$'postgres://loco:loco@db.example.org:5432/backend3' cargo loco start"
blocks "oc port-forward svc/postgres \$'15432:5432'"
# The short forms of the numeric escapes, \cX and a NUL from each of them
# are decoded as bash decodes them (the reason names the decoded type).
reason_mentions "oc delete \$'n\\0x's web" "names 'ns'"
reason_mentions "oc delete \$'n\\x0junk's web" "names 'ns'"
reason_mentions "oc delete \$'\\u6e\\u73' web" "names 'ns'"
reason_mentions "oc delete \$'\\U6e\\U73' web" "names 'ns'"
reason_mentions "oc delete \$'ns\\c@junk' web" "names 'ns'"
# A $' inside double or single quotes is text there, as in bash, so a NUL
# after it cannot hide the rest of the line from the heredoc receiver test
# or a script from the in-cluster rule (ADR 0035).
blocks $'echo "$\'\\0"; oc exec -i postgres-0 -- sh <<\'EOF\' ; echo "\'"\npsql -c \'select 1\'\nEOF'
blocks $'echo \'x$\'; echo \\0; oc exec -i redis-0 -- sh <<\'EOF\' ; echo \'y\'\nredis-cli flushall\nEOF'
blocks "oc exec postgres-0 -- sh -c \"echo \\\"\\\$'\\\\0\\\" ; psql ; echo \\\"'\\\"\""
blocks "oc exec redis-0 -- sh -c \"echo \\\"\\\$'\\\\0\\\" ; redis-cli flushall ; echo \\\"'\\\"\""
blocks 'o$"c" exec -i postgres-0 -- sh <<'"'EOF'"$'\npsql\nEOF'
blocks "ba\$'sh' <<'EOF'"$'\noc delete ns web\nEOF'
# Read part by part, a $' inside double quotes is text, and a $'...' part
# after it is still decoded: here neither the quote-stripped reading nor
# one that took `$'\0"; o$'` for a part would find oc.
blocks $'echo "$\'\\0"; o$\'c\' exec -i postgres-0 -- sh <<\'EOF\'\npsql\nEOF'
blocks "oc exec postgres-0 -- sh -c '\$\"psql\" -c \"select 1\"'"
# The part-by-part reading decodes the escapes of a $'...' part there.
blocks "oc \$'\\x65xec' -i postgres-0 -- sh <<'EOF'"$'\npsql\nEOF'
blocks "oc exec postgres-0 -- sh -c \"\$'\\x70sql' -c 'select 1'\""
blocks "oc exec redis-0 -- sh -c \"\$'\\x72edis-cli' flushall\""
# ADR 0028's reading, with every quote and backslash removed, still counts
# next to the part-by-part one when a $'...' part is there. It errs towards
# blocking: inside double quotes bash keeps these backslashes, so it would
# run `ba\sh` and `ps\ql`, which do not exist (ADR 0035).
blocks "X=\$'1' b\"a\\s\"h <<'EOF'"$'\noc delete ns web\nEOF'
blocks $'oc exec postgres-0 -- sh -c "\\"ps\\\\ql\\" -c \\$\'select 1\'"'
allows "echo \$'\\'' 'oc delete ns web'"
allows "echo \$'a\\' ; oc delete ns web'"
allows "echo x\$'a\\' ; oc delete ns web'"
allows "echo \$'it\\'s fine'"
allows "printf \$'%s\\t%s\\n' a b"
allows "echo \$'' && oc get pods"
allows "echo \$'\\x' \$'\\c' \$'\\777' \$'\\UFFFFFFFF'"
allows "oc get pods -o name | grep \$'\\tweb'"
allows "psql -h \$'localhost'"
allows "git commit -m \$'fix(deploy): block oc delete ns\\n\\noc delete ns web'"
allows 'echo $"oc delete ns web is blocked"'
# bash reads `$'n\'s'` as n's, which names no protected type.
allows "oc delete \$'n\\'s' web"

# ------------------------------------------------------------- every check: substitutions whose output the shell runs (ADR 0035)

# A shell, `source` or `.` that gets a process substitution reads its output
# as a script, and the shell runs the output of a $(...) or `...` that stands
# where the command name goes (also in a -c or eval string, which is read
# again as commands). When that output is echo or printf text, it is split as
# commands, like echo text piped into a shell, and in place of the command
# name its words take the substitution's place.
blocks "bash <(echo 'oc delete ns web')"
blocks "sh <(printf 'oc delete pvc data\\n')"
blocks "source <(echo 'oc delete ns web')"
blocks ". <(printf 'oc delete ns web\\n')"
blocks "bash < <(echo 'kubectl get pods')"
blocks "bash <(echo 'echo psql | oc exec -i postgres-0 -- sh')"
blocks "bash <(echo 'echo redis-cli flushall | oc exec -i redis-0 -- sh')"
blocks "bash <(echo 'argocd app confirm-deletion backend3')"
blocks "bash -c \"\$(echo 'oc delete ns web')\""
blocks "eval \"\$(echo 'oc delete ns web')\""
blocks "eval \"\$(printf '%s\\n' 'psql -h db.example.org')\""
blocks "\$(echo 'oc delete ns web')"
blocks '$(echo oc) delete ns web'
blocks '`echo oc` delete pvc data'
blocks 'sudo $(echo oc) delete ns web'
blocks '$(echo oc) exec postgres-0 -- psql'
blocks '$(echo oc) exec redis-0 -- redis-cli flushall'
blocks '$(echo argocd) app confirm-deletion backend3'
blocks "\$(echo oc) annotate application backend3 $approval=now"
# The command name's place is found as for kubectl: after a runner word, its
# own options and values may come first.
blocks 'timeout 30 $(echo oc) delete ns web'
blocks 'sudo -u root $(echo oc) delete pvc data'
blocks 'X=1 $(echo oc) delete ns web'
blocks 'X=1 "$(echo oc)" delete ns web'
# The output there may name a shell, which then reads a here-string or
# piped text as a script.
blocks "\$(echo bash) <<< 'oc delete ns web'"
blocks "echo 'oc delete ns web' | \$(echo bash)"
blocks 'nice -n 10 `echo oc` delete ns web'
# There the substitution may also be an argument of the runner or of the
# command, and the hook does not apply printf formats, so the substitution
# stays in front of its output: a check that counts an expansion as not
# checkable still sees one (bash forwards 5432 here).
blocks 'sudo oc port-forward svc/postgres $(printf 54%s 32)'
blocks 'sudo oc port-forward svc/postgres "$(printf 54%s 32)"'
# A double-quoted $(...) or `...` there is the command name, as one word.
blocks '"$(echo oc)" delete ns web'
blocks '"`echo oc`" delete pvc data'
blocks 'sudo "$(printf oc)" delete ns web'
blocks '"$(echo kubectl)" get pods'
blocks '"$(echo oc)" exec redis-0 -- redis-cli flushall'
# Its output is read like any word there, also when it holds no trigger word:
# a runner, a shell or eval.
blocks '"$(echo sudo)" kubectl get pods'
blocks "\"\$(echo bash)\" <<< 'oc delete ns web'"
blocks "echo 'oc delete ns web' | \"\$(echo bash)\""
blocks "\"\$(echo eval)\" 'oc delete ns web'"
# bash removes the line breaks at the end of a substitution's output, and
# echo's options are not printed.
blocks "\"\$(printf 'oc\\n')\" delete ns web"
blocks "\"\`printf 'oc\\n\\n'\`\" delete pvc data"
blocks '"$(echo -n oc)" delete ns web'
blocks '"$(echo -e oc)" delete ns web'
blocks '"$(echo -E oc)" delete ns web'
# -nx is not an echo option, so bash prints it, and the command is "-nx oc".
allows '"$(echo -nx oc)" delete ns web'
# A -c string, a here-string and the text that eval, ssh or watch run again
# are read as written, so the commands that a substitution there runs still
# count next to its output, also after a runner word (ADR 0035).
blocks 'eval "$(echo x; oc delete ns web)"'
blocks 'eval "$(echo x)$(oc delete ns web)"'
blocks 'eval "`printf x; oc delete pvc data`"'
blocks 'ssh host "$(echo x; oc delete ns web)"'
blocks 'watch "$(echo x; oc delete ns web)"'
blocks 'sudo bash -c "$(echo -n oc delete ns web)"'
# printf's \n starts a new line, and an assignment may come before the echo.
blocks "bash <(printf 'echo hi\\noc delete ns web\\n')"
blocks "bash <(X=1 echo 'oc delete ns web')"
# A printf's \n or \t is a blank before a trigger word for the fast path too,
# in a pipe as in a substitution (ADR 0035): `ls\noc` prints oc as a word.
blocks "printf 'ls\\noc delete ns web\\n' | sh"
blocks "printf 'cd /tmp;\\toc delete ns web\\n' | bash"
allows "bash <(echo 'oc get pods')"
# Only the substitution's own echo or printf counts, not one before it.
allows "echo 'oc delete ns web is blocked'; bash <(echo 'oc get pods')"
allows 'diff <(echo a) <(echo b)'
allows "cat <(echo 'oc delete ns web')"
allows "x=\$(echo 'oc delete ns web')"
allows 'if $(echo true); then oc get pods; fi'
allows "\$(echo echo) 'oc delete ns web is blocked'"
allows "gh pr comment 220 --body 'bash <(echo oc delete ns web) is blocked now'"
# A quoted substitution that is an argument, or whose output is more than
# one word, names no program.
allows 'echo "$(echo oc)" delete ns web'
allows "\"\$(echo 'oc delete')\" ns web"
allows 'x="$(echo oc)" && echo "$x"'
allows '"$(echo oc)"x delete ns web'

# ------------------------------------------------------------- the gaps that ADR 0036 records

# What D110's answer keeps out of the checks, pinned as allowed, so that
# closing a gap is a deliberate change with an answer or an amending ADR of
# its own (ADR 0001 section 6) rather than a side effect: a raw socket,
# manifest files, editors, a Redis client patched into an existing pod
# spec, variables, brace lists, globs, and inline manifests that change
# finalizers or a reclaim policy. ADR 0036's "Negative and risks" lists
# them; the standing rules of CLAUDE.md hold there.
allows "printf 'FLUSHALL\\r\\n' | oc exec -i deploy/redis-backend2 -- nc -w 1 127.0.0.1 6379"
allows 'oc exec deploy/redis-backend2 -- sh -c "printf FLUSHALL | nc 127.0.0.1 6379"'
allows 'oc apply -f job.yaml'
allows "cat > app.yaml <<'EOF'"$'\n'"$app_manifest"$'\n'"EOF"$'\n'"oc apply -f app.yaml"
allows 'oc apply -k overlays/cleanup'
allows 'oc edit application backend3'
allows 'argocd app edit backend3'
allows "oc patch deploy/redis-backend2 --type json -p '[{\"op\":\"replace\",\"path\":\"/spec/template/spec/containers/0/command\",\"value\":[\"redis-cli\",\"FLUSHALL\"]}]'"
allows 'C=redis-cli; oc exec deploy/redis-backend2 -- $C flushall'
allows 'oc run tmp --image=redis -- "$CLIENT" flushall'
allows 'oc exec deploy/redis-backend2 -- {redis-cli,flushall}'
allows 'oc exec deploy/redis-backend2 -- /usr/local/bin/redis-cl? flushall'
allows $'oc apply -f - <<\'EOF\'\nkind: PersistentVolumeClaim\nmetadata:\n  name: data\n  finalizers: []\nEOF'
allows $'oc apply -f - <<\'EOF\'\nkind: PersistentVolume\nmetadata:\n  name: pv-1\nspec:\n  persistentVolumeReclaimPolicy: Delete\nEOF'
allows "oc get pvc data -o json | jq 'del(.metadata.finalizers)' | oc replace -f -"
# Paths outside the verbs the answer names, which need an answer of their
# own (ADR 0036, "Negative and risks"): oc adm must-gather, which starts a
# pod with the image and command it is given; argocd app create reading an
# Application from stdin or a process substitution; and oc set probe and
# oc set deployment-hook, which put a command into an existing pod spec.
allows 'oc adm must-gather --image=redis -- redis-cli -h redis-backend2 FLUSHALL'
allows $'argocd app create --upsert -f - <<\'EOF\'\nmetadata:\n  name: backend3\n  annotations:\n    argocd.argoproj.io/deletion-approved: x\nEOF'
allows "argocd app create --upsert -f <(echo 'metadata: {annotations: {argocd.argoproj.io/deletion-approved: x}}')"
allows 'oc set probe deploy/redis-backend2 --liveness -- redis-cli ping'
allows 'oc set deployment-hook dc/redis-backend2 --pre -c redis -- redis-cli flushall'
# Spellings of the stdin and heredoc readings that the hook does not read
# (ADR 0036, "Negative and risks"): a cat here-string inside a group, and
# a heredoc in a group that does not close on the line after the body.
allows "{ cat <<< 'redis-cli flushall'; } | oc exec -i deploy/redis-backend2 -- sh"
allows $'( cat <<\'EOF\'\nargocd.argoproj.io/deletion-approved: x\nEOF\necho done\n) | oc apply -f -'

# ------------------------------------------------------------- false positives that ADR 0036 records

# The receiving line of a heredoc is read with its quotes removed, so a
# title that names oc run, oc apply, oc create or oc replace makes the body
# after it count as the pod's stdin or a manifest; the reason says how to
# pass such a body instead, which stays allowed.
blocks $'gh pr create --title "fix(deploy): block redis clients in oc run (ADR 0036)" --body "$(cat <<\'EOF\'\nBlocks redis-cli in new pods.\nEOF\n)"'
blocks $'gh issue create --title "oc apply of inline manifests" --body-file - <<\'EOF\'\nnames redis-cli\nEOF'
reason_mentions $'gh issue create --title "oc apply of inline manifests" --body-file - <<\'EOF\'\nnames redis-cli\nEOF' 'gh pr create --body-file <file>'
reason_mentions $'gh pr create --title "fix(deploy): block redis clients in oc run (ADR 0036)" --body "$(cat <<\'EOF\'\nBlocks redis-cli in new pods.\nEOF\n)"' 'gh pr create --body-file <file>'
blocks $'gh issue create --title "oc run fails" --body-file - <<\'EOF\'\n["redis-cli","x"]\nEOF'
reason_mentions $'gh issue create --title "oc run fails" --body-file - <<\'EOF\'\n["redis-cli","x"]\nEOF' 'the text that this `oc run` passes to the pod it starts names `redis-cli`'
reason_mentions $'gh issue create --title "oc run fails" --body-file - <<\'EOF\'\n["redis-cli","x"]\nEOF' 'gh pr create --body-file <file>'
reason_mentions $'gh issue create --title "oc apply of approvals" --body-file - <<\'EOF\'\nargocd.argoproj.io/deletion-approved\nEOF' 'gh pr create --body-file <file>'
allows 'gh pr create --title "fix(deploy): block redis clients in oc run (ADR 0036)" --body-file pr-body.md'

# A synthetic segment counts only with its marker (see synthetic_marker):
# an ordinary segment of the same shape is not decoded.
allows 'oc get pods; echo manifest apply cmVkaXMtY2xp'
allows 'oc get pods; printf pod-stdin run cmVkaXMtY2xp'
allows 'oc get pods; echo manifest apply YXJnb2NkLmFyZ29wcm9qLmlvL2RlbGV0aW9uLWFwcHJvdmVk'

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

# A heredoc counts on every line since ADR 0036, so a command with many of
# them is read heredoc by heredoc; the work must still grow linearly.
heredoc_lines="$(for i in $(seq 1 800); do printf "cat > f%d.txt <<'EOF'\nline %d mentions cargo and the db\nEOF\n" "$i" "$i"; done)"
allows_in_time "800 heredocs, one after another" "$heredoc_lines"

# An inline manifest is encoded into one synthetic word (ADR 0036), and a
# long one must cost no more than the text around it.
manifest_body="$(for i in $(seq 1 900); do printf '  key%d: value %d mentions cargo and the db\n' "$i" "$i"; done)"
allows_in_time "a 32 KB inline manifest in a heredoc into oc apply" "oc apply -f - <<'EOF'"$'\n'"data:"$'\n'"$manifest_body"$'\n'"EOF"
allows_in_time "a 32 KB inline manifest piped into oc apply" "printf '%s' 'data:"$'\n'"$manifest_body' | oc apply -f -"

# Every `<<` is looked at with the quoting model, and one whose delimiter
# never comes makes the reader search the rest of the text, so many of
# them must stay cheap too: unclosed operators (arithmetic shifts), and
# quoted ones, which make the hook read the command twice.
unclosed_lines="$(for i in $(seq 1 1000); do printf 'echo $((1<<%d)) mentions oc get pods\n' "$i"; done)"
allows_in_time "1000 lines with an unclosed <<" "$unclosed_lines"
quoted_lines="$(for i in $(seq 1 400); do printf "echo 'a <<X%d' && oc get pods \"it's\"\nX%d\n" "$i" "$i"; done)"
allows_in_time "400 quoted << operators, each with its delimiter" "$quoted_lines"

# ------------------------------------------------------------- summary

printf '# %d passed, %d failed\n' "$passed" "$failed"
if ((failed > 0)); then
  exit 1
fi
