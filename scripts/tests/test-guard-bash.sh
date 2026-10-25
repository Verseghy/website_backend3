#!/usr/bin/env bash
# test-guard-bash.sh: tests for .claude/hooks/guard-bash.sh, the PreToolUse
# hook of the guardrails (ADR 0024 section 4, layer 2).
#
# Why it exists
#   The hook runs before every Bash command an agent runs in this repository.
#   A bug in it either lets a destructive command through or blocks legitimate
#   work, so every rule of ADR 0024 section 4 is pinned here: each check 1
#   command in the cargo loco, app-binary, oc exec, oc rsh and oc debug forms;
#   each check 2 target source (inline variables, -h, -d URIs, host= strings,
#   positional URIs, -u, the environment, IMPORT_SOURCE_DATABASE_URL, plus
#   PGHOSTADDR and PGSERVICE); a command with no target; wrapped forms (sh -c,
#   also with `--` and other options, here-strings, scripts piped into a
#   shell, heredocs, also those that oc exec, oc rsh or oc debug run in the
#   cluster, piped through further stages and line breaks too, in pipelines
#   longer than 16 KiB, and as xargs arguments); the command part of oc exec,
#   oc rsh and oc debug (rsh's --shell and -f included), so that pod,
#   container and namespace names, data paths and search patterns do not
#   count; oc delete of what an oc get of a protected kind produces; allowed
#   commands; a missing jq; and the cost of long commands.
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
adr="$repo_root/docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md"
# The layer-1 JSON block of ADR 0024 section 4 must be the committed settings.
adr_settings="$(awk '/^\*\*Layer 1: deny rules/{f=1} f && /^```json$/{g=1; next} g && /^```$/{exit} g{print}' "$adr")"
if [[ -n "$adr_settings" ]] && [[ "$(jq -S . <<<"$adr_settings")" == "$(jq -S . "$settings")" ]]; then
  report ok ".claude/settings.json equals the layer-1 block of ADR 0024 section 4" ""
else
  report fail ".claude/settings.json equals the layer-1 block of ADR 0024 section 4" \
    "  diff: $(diff <(jq -S . <<<"$adr_settings" 2>&1) <(jq -S . "$settings" 2>&1) || true)"
fi
# Claude Code runs a shell-form hook command with `sh -c` and exports
# CLAUDE_PROJECT_DIR (Claude Code docs, "Hooks reference"); do the same.
hook_command="$(jq -r '.hooks.PreToolUse[] | select(.matcher == "Bash") | .hooks[0].command' "$settings")"
set +e
env "${clean_db_env[@]}" CLAUDE_PROJECT_DIR="$repo_root" sh -c "$hook_command" <"$fixtures/pretooluse-bash-kubectl.json" >/dev/null 2>&1
wired_block=$?
env "${clean_db_env[@]}" CLAUDE_PROJECT_DIR="$repo_root" sh -c "$hook_command" <"$fixtures/pretooluse-bash-allowed.json" >/dev/null 2>&1
wired_allow=$?
set -e
if [[ "$wired_block" == 2 && "$wired_allow" == 0 ]]; then
  report ok "the hook command in settings.json runs the hook under sh -c" ""
else
  report fail "the hook command in settings.json runs the hook under sh -c" \
    "  command: $hook_command; exits: $wired_block (want 2) and $wired_allow (want 0)"
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
blocks "sh -c 'oc get pvc -o name | xargs oc delete'"
# The producer names another kind, the oc delete fixes its type in the text
# (kubectl refuses to mix `pod` with the `persistentvolumeclaim/...` names),
# or nothing deletes the output.
allows 'oc delete $(oc get pods -o name)'
allows 'oc get deployments -o name | xargs oc delete'
allows 'oc get pods data-pvc -o name | xargs oc delete'
allows 'oc delete pod $(oc get pvc -o name)'
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

# ------------------------------------------------------------- check 2: wrapped forms

blocks 'bash -c "psql -h db.example.org"'
blocks "sh -c 'DATABASE_URL=$remote_pg cargo test'"
blocks 'timeout 30 psql -h db.example.org'
blocks 'echo ok && psql -h db.example.org'
blocks "ssh bastion 'psql -h db.example.org'"
blocks "watch 'psql -h db.example.org -c \"select 1\"'"
blocks $'bash <<\'EOF\'\npsql -h db.example.org\nEOF'
allows "bash -c 'psql -h localhost'"

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
