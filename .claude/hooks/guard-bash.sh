#!/usr/bin/env bash
# guard-bash.sh: the PreToolUse hook that screens every Bash command an agent
# runs in this repository (layer 2 of the guardrails, ADR 0024 section 4 as
# amended by ADR 0028, ADR 0035 and ADR 0036).
#
# Why it exists
#   Layer 1, the deny rules in .claude/settings.json, matches command text as
#   written. It misses the same command behind a wrapper (`sh -c '...'`), with
#   another flag order, or as the app binary instead of `cargo loco`. This hook
#   reads the whole command, splits it the way a shell would (ANSI-C $'...'
#   quoting decoded), finds oc's verb the way oc's command parser does, also
#   behind options it does not know (`oc --wait ns delete web`), and blocks:
#
#   check 1, dangerous commands anywhere in the command:
#     - kubectl (Verseghy uses oc, never kubectl);
#     - force deletion: a true --force on oc delete or oc replace, whatever
#       the grace period, and a true --force with --grace-period=0 anywhere;
#     - oc delete naming a PVC, PV, Argo CD Application, namespace or project
#       in any spelling (ApplicationSets too, because deleting one deletes its
#       Applications; a namespace or project takes everything in it along),
#       also when the command line runs an `oc get` of one of them and the
#       oc delete takes its resources from another command's output:
#       `oc delete $(oc get pvc -o name)`, `oc get pvc -o name | xargs oc
#       delete`, `oc get pvc -o yaml | oc delete -f -`; and every
#       `oc delete --raw`, which deletes whatever its URI names (ADR 0035);
#     - `argocd app delete`, `argocd app delete-resource` and `argocd appset
#       delete`, with any options (ADR 0028; the last two since D69), and
#       `argocd app confirm-deletion` (since D105, ADR 0035), which releases
#       the deletions that a Prune=confirm or Delete=confirm sync option
#       holds back; argocd counts under its release download names too
#       (`argocd-linux-amd64`);
#     - an `oc patch`, or (since D105) an `argocd app patch-resource`, whose
#       patch names finalizers, or names a PV's persistentVolumeReclaimPolicy
#       with Delete, Recycle (since D69) or a value only the shell knows
#       (ADR 0028), read as the JSON or YAML decoder reads it, escapes
#       included, or that replaces or removes a whole object or its whole
#       metadata, and with it the finalizers (ADR 0035);
#     - (since D110, ADR 0036) an `argocd app patch` with the same finalizer
#       match, since an Application's finalizers decide what deleting it
#       takes along (the reclaim-policy match does not apply: an Application
#       has none);
#     - (since D105) an `oc annotate`, an `oc patch` or any argocd command
#       that names Argo CD's deletion-approved annotation: it is what `argocd
#       app confirm-deletion` writes, so setting it another way releases the
#       same deletions; and (since D110, ADR 0036) an inline manifest that
#       names it, which oc apply, oc create or oc replace reads from the
#       command text: a heredoc, a here-string, piped echo or printf text or
#       the echo or printf output of a process substitution;
#     - Loco's data-wiping subcommands `db reset`, `db down`, `db truncate`
#       and `db seed --reset` / `-r` of any binary: cargo loco, the app binary
#       backend3-cli, or inside oc exec / oc rsh / oc debug;
#     - sea-orm-cli `migrate fresh|refresh|reset|down`.
#   check 2, the database target of database-touching commands (cargo loco,
#     cargo run, cargo test, cargo nextest and the aliases cargo r, cargo
#     loco-tool and cargo t, psql, pg_restore, mysql, mariadb, sea-orm-cli):
#     it blocks when the target host is not local (localhost, 127.0.0.1,
#     ::1 or a Unix socket) or cannot be checked, and for cargo
#     commands it checks the importer source IMPORT_SOURCE_DATABASE_URL the
#     same way. Any listed command, and a Redis client (redis-cli since
#     D105, ADR 0035; redis-benchmark, valkey-cli and keydb-cli since D110,
#     ADR 0036), run through oc exec, oc rsh or oc debug (also as a
#     heredoc, a here-string or an echo or printf script piped into them,
#     through any further pipeline stages) runs in the cluster and is always
#     blocked. (Since D110, ADR 0036) a Redis client is also blocked as the
#     command of a pod that the agent starts: in the command part of `oc
#     run` (`oc run tmp --image=redis -- redis-cli -h <service> FLUSHALL`),
#     its --overrides JSON and the text it passes to the pod on stdin, in
#     the command part of `oc create job|cronjob|deployment|deploymentconfig`,
#     and in an inline manifest that oc apply, oc create or oc replace reads;
#     a database command there keeps its target check, as before. A Redis
#     client on the agent's own host is not checked: the clusters' Redis
#     answers there only through a port-forward, which this check blocks.
#     Only the command part of oc exec, oc rsh, oc debug, oc run or oc
#     create counts: for exec and debug the words after `--`, or else the
#     words after the target; for run and create the positional words after
#     the pod's name, before `--` and after it (`oc run tmp --image=redis
#     --command redis-cli -- -h <service> FLUSHALL` runs redis-cli); for
#     rsh the words after the pod, the program its --shell names, and with
#     -f every word. So a pod, container, namespace or image named like a
#     client (`oc exec deploy/mysql -- df -h`, `oc run redis-cli
#     --image=redis -- redis-server`) is not blocked by its name, nor is a
#     data path such as /var/lib/mysql. A pod looked up with an unquoted
#     $(...) or `...` does not end the command part (`oc exec $(oc get pod
#     -o name) -- psql` is blocked). It also blocks an
#     `oc port-forward` to a PostgreSQL, MySQL, MariaDB or (since D69) Redis
#     port (ADR 0028), also written with a leading `+` (ADR 0035), because
#     the forwarded database then answers on localhost, which this check
#     would otherwise take for a local one. No Redis client is a listed
#     command, so a forwarded Redis would answer on localhost with no check
#     at all.
#
#   Layer 3, the load_config guard and the CLI pre-check in the Rust code, is
#   the one that cannot be bypassed from a shell. This hook is a second line of
#   defence against agent mistakes, not a security boundary: commands that come
#   from files or scripts are not visible to it (`oc exec -i pod -- sh <
#   script.sh`, `oc delete -f list.txt`, `oc patch --patch-file p.json`), and
#   neither is text that only a shell run computes, because the hook reads the
#   command and does not run it: a variable's value (`P=psql; oc exec pod --
#   $P`), printf formats and echo -e escapes, brace expansion
#   (`deletion-appro{v,}ed`), globs, or decoded output (`base64 -d`). (The
#   output of an echo or printf that a shell runs is read, see lex, and so
#   are the escapes of $'...' quoting, see ansi_c_text; a substitution that
#   fills in only part of the command name, `o$(echo c) delete ...`, or a
#   double-quoted one with double quotes inside, is not, and each echo or
#   printf output is read as a line of its own, also where bash prints it
#   on one line, `printf oc; echo ' delete ...'`.) Some routes are not
#   checked at all, here or in website_k8s's hook: an `oc replace`, `oc
#   apply` or `oc edit` that drops finalizers, replaces an object or sets a
#   reclaim policy, an inline manifest that does so included, and `oc
#   replace --raw`; `argocd app sync --prune` and `argocd app set
#   --auto-prune`; an ssh tunnel to a database (`ssh -L 15432:<db>:5432
#   <host>`); file commands in a pod (`oc exec <pod> -- rm -rf
#   /var/lib/postgresql/data`); commands that an interpreter runs
#   (`python3 -c '...'`); and output redirected into a process substitution
#   (`echo '...' > >(bash)`). The answer to D110 keeps these as gaps (ADR
#   0036): a manifest in a file, in another command's output or in an
#   editor (`oc apply -f job.yaml`, `oc edit`, `argocd app edit`); a Redis
#   client patched into an existing pod spec (`oc patch deploy ...` that
#   sets a container's command, `oc set probe ... -- redis-cli`); and a raw
#   socket or any other program that speaks the Redis protocol (`nc`).
#   ADR 0036 also records paths outside the verbs the answer names (`oc
#   adm must-gather --image=... -- <command>`, `argocd app create -f -`)
#   and spellings these readings miss (a cat here-string inside a group,
#   a heredoc in a group that does not close on the line after its body).
#   ADR 0035's and ADR 0036's "Negative and risks" list these paths.
#
# Contract with Claude Code (verified against the "Hooks reference" of the
# Claude Code docs, 2026-10-25)
#   - stdin is one JSON object; the Bash command is at .tool_input.command and
#     the tool name at .tool_name;
#   - exit 0 lets the command run;
#   - exit 2 blocks it, and stderr is shown to the agent as the reason, so every
#     message says what was refused and what to do instead;
#   - any other exit code is a failure. .claude/settings.json sets
#     "onFailure": "block" on this hook (Claude Code v2.1.295 or later), so a
#     failure, a hook that cannot start or one that reaches its timeout blocks
#     the command too. Without it a failure would be NON-blocking, and an older
#     client ignores the field, so every failure path below still exits 2
#     itself (fail closed): a missing jq, unreadable input, or an unexpected
#     error in this script.
#   .claude/settings.json runs it in exec form: "command" is
#   "${CLAUDE_PROJECT_DIR}/.claude/hooks/guard-bash.sh" with "args": [], so no
#   shell is involved and Claude Code substitutes the placeholder as a plain
#   string. It still exports CLAUDE_PROJECT_DIR to the hook process.
#
# Design notes
#   - It runs before every Bash call, so it must be fast: it starts one jq
#     process, and jq returns "allow" at once for any command that contains
#     neither a trigger word (kubectl, oc, db, cargo, psql, ...) nor one of the
#     variables check 2 reads. Typical commands take a few tens of
#     milliseconds; long ones, and long text that is split again as commands,
#     are cut into chunks first (see `chunks`), the lexer keeps what it needs
#     about a segment's earlier words in flags, and each segment sees only the
#     assignments in scope for it, so the work grows linearly with the length
#     of the command.
#   - The policy is a jq program embedded below, so the hook depends only on
#     bash and jq. jq's regular expressions do the tokenising in C. jq reads
#     it from a process substitution, because it is longer than one
#     command-line argument may be on Linux (128 KiB).
#   - oc's verb and argocd's subcommands are found the way cobra, which both
#     use, finds them (see oc_verb_indices and argocd_subcommands), so an
#     option written before them, a verb-local one included (`oc -p
#     '<patch>' patch ...`), cannot hide them. oc's verb is read in two
#     ways, the hook's and cobra's, and each verb's words with and without
#     the words before it (see oc_verb_words; `oc --wait pvc delete data`
#     deletes the claim), and every reading counts (ADR 0035).
#   - Shell parsing is approximated, deliberately erring towards blocking. A
#     word is parsed again as a command only where a shell would run it (the
#     command string of -c, also after `--` or other options; the arguments of
#     eval, ssh and watch; a here-string read by a shell; echo or printf
#     output piped into a shell or into oc exec, oc rsh or oc debug, or in a
#     substitution whose output the shell runs; and $(...) inside double
#     quotes), and a heredoc body counts as a script only when a shell reads
#     it (the receiver is matched with quotes and backslashes removed, and a
#     variable as the receiver counts as a shell), so commit messages and
#     files that merely mention a command are not blocked. A heredoc is
#     found on every line, after any `<`, and inside the scripts above
#     (ADR 0036); a `<<` that may stand inside quotes makes the hook read
#     the command twice, once with it as a heredoc and once as text (see
#     heredoc_texts), so a quoted `<<WORD` cannot hide the commands after
#     it. The cases it handles are pinned by scripts/tests/test-guard-bash.sh,
#     which must pass before this file changes (it runs in the CI "Test"
#     job).
#   - ADR 0035 section 2 reads command text the way the programs read it,
#     for every check: options before the oc verb, oc get's value options,
#     escapes and whole-object operations in a patch, $'...' and $"..."
#     quoting, substitutions whose output the shell runs, `oc delete --raw`,
#     port numbers with a `+`, argocd's download names, text that xargs or
#     parallel pass on, and argocd as the value of an oc option. Where a
#     reading blocks more than needed (every `oc delete --raw`, every
#     `$patch: replace|delete` or `$retainKeys`, wherever it stands), ADR
#     0035's "Negative and risks" says so.
#   - Where ADR 0024 leaves room, it is stricter. The maintainer accepted
#     these points as built, and ADR 0028 section 4 records them; a new point
#     like them needs the maintainer's acceptance or an amending ADR first
#     (ADR 0001 section 6):
#       - check 2 checks every explicit target source that is present (inline
#         variables, options and connection strings), not only the first,
#         because psql prefers -h over PGHOST; an environment variable is
#         checked unless the command does not use it (see db_targets), so a
#         local inline PGHOST cannot hide a remote DATABASE_URL;
#       - it also reads PGHOSTADDR (libpq and sqlx use it) and treats a
#         PGSERVICE or service= entry as not checkable;
#       - a true --force on oc delete or oc replace is blocked even without
#         --grace-period=0, because kubectl then sets the grace period to 0;
#       - an ApplicationSet counts like an Application, because deleting one
#         deletes its Applications;
#       - heredocs read by a shell, here-strings, and echo or printf output
#         piped into a shell (also through further pipeline stages and across
#         a line break after |) are checked like `sh -c`, because they run
#         the same commands; a heredoc that only data reads, such as a commit
#         message, is skipped;
#       - an oc delete whose resources come from another command's output
#         ($(...), a variable, xargs, `-f -` or a process substitution) counts
#         as naming a protected kind when the same command line runs an
#         `oc get` of one, because the hook cannot read that output;
#       - text that oc exec, oc rsh or oc debug reads on stdin (a heredoc, a
#         here-string or piped echo or printf output) counts as a script that
#         runs in the cluster, even when the command in the pod is not a
#         shell. Under xargs the text becomes the oc command's arguments
#         instead, as xargs passes it;
#       - check 2 counts cargo run, cargo r, cargo loco-tool and cargo t like
#         the listed cargo loco and cargo test, because they start the same
#         programs (see db_commands).
#     The last three points are the maintainer's answer to D69 of
#     2026-10-25, which ADR 0028 section 4 records too.
#   - It reads DATABASE_URL, PGHOST, PGHOSTADDR, PGSERVICE, MYSQL_HOST and
#     IMPORT_SOURCE_DATABASE_URL from its own environment (the one Claude Code
#     passes to hooks) for the "environment" target source of check 2. That is
#     a deliberate exception to the explicit-argument rule (ADR 0008): the hook
#     is a process-boundary check whose whole point is to see the environment
#     a command inherits.
#
# Usage (for tests and manual checks)
#   printf '%s' '{"tool_name":"Bash","tool_input":{"command":"kubectl get pods"}}' \
#     | .claude/hooks/guard-bash.sh; echo "exit $?"

set -euo pipefail

# Prints the reason to stderr and blocks the command (exit 2).
#   $* - the message shown to the agent.
block() {
  printf '%s\n' "$*" >&2
  exit 2
}

# Any unexpected failure blocks the command instead of silently letting it run.
trap 'block "Blocked by .claude/hooks/guard-bash.sh: the hook failed unexpectedly (line $LINENO), so the command was blocked to be safe. Run \"bash scripts/tests/test-guard-bash.sh\" from the repository root to find the fault."' ERR

if ! command -v jq >/dev/null 2>&1; then
  block "Blocked by .claude/hooks/guard-bash.sh: jq is not installed or not on PATH, and this guardrail hook needs it to read the command, so every Bash command is blocked until it is available (fail closed, ADR 0024 section 4). Install jq (for example 'sudo dnf install jq', 'sudo apt-get install jq' or 'brew install jq') and retry."
fi

# The policy, as a jq program. It reads the hook input and prints either
# "allow" or "block" followed by a newline and the reason.
IFS= read -r -d '' GUARD_JQ <<'JQ' || true
# ------------------------------------------------------------------ messages

# Formats a block reason.
#   $check - the check number of ADR 0024 section 4 (1 or 2);
#   $text  - what was refused and what to do instead.
def msg($check; $text):
  "Blocked by .claude/hooks/guard-bash.sh (ADR 0024 section 4 as amended by ADR 0028, ADR 0035 and ADR 0036, check \($check)): \($text)\n"
  + "The guardrails are described in docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md, section 4, "
  + "docs/design/adr/0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md, "
  + "docs/design/adr/0035-guardrails-for-in-cluster-redis-argo-cd-patches-and-deletion.md "
  + "and docs/design/adr/0036-guardrails-for-redis-clients-in-new-pods-argocd-app-patch-an.md.";

# Advice for a block that may be a false positive caused by text that only
# mentions a command, for example in a commit message.
def text_only_hint:
  " If the words only appear as text (a commit message, a PR body, a search), "
  + "write the text to a file and pass the file (git commit -F <file>, gh pr create --body-file <file>), "
  + "or use the Grep tool for searches.";

# Advice for a block of an inline manifest or of a pod's stdin text (ADR
# 0036) that may be a false positive: the receiving line of a heredoc is
# read with its quotes removed, so a commit, PR or issue title that names
# `oc apply` or `oc run` makes the body that follows count as a manifest or
# as the pod's stdin. It names only the text-file options of git and gh,
# not a manifest file: a manifest in a file is a gap that ADR 0036 records,
# not a way around this check.
def manifest_text_hint:
  " If the text is only a commit message, PR or issue body on a command line that also names oc run, oc apply, oc create or oc replace "
  + "(in a title, say), pass the body from a file (git commit -F <file>, gh pr create --body-file <file>).";

# ------------------------------------------------------------------ helpers

def basename: sub("^.*/"; "");

# Whether a program name, without its directory, is argocd: `argocd`, or a
# name of argocd's release downloads, which keep their file name when they
# are installed (`argocd-linux-amd64`, `argocd-darwin-arm64`,
# `argocd-windows-amd64.exe`; ADR 0035). Any name that starts with `argocd`
# and goes on with `-`, `_` or `.` counts, which errs towards blocking.
def is_argocd_name: test("^argocd(?:[-_.]|$)");

# The Redis clients that check 2 knows, by program name: redis-cli (D105,
# ADR 0035), the client that the clusters' Redis image ships, and (D110, ADR
# 0036) redis-benchmark, which ships in the same image and sends any command
# it is given (`redis-benchmark -n 1 FLUSHALL` empties the database), and
# valkey-cli and keydb-cli, the clients of Redis's two protocol-compatible
# forks. Each one is blocked where it runs in the cluster, and none is
# checked on the agent's own host (see redis_clients). Other programs that
# speak the protocol, a raw socket (`nc`) among them, are a recorded gap.
def redis_client_names: ["redis-cli", "redis-benchmark", "valkey-cli", "keydb-cli"];

def is_assignment: test("^[A-Za-z_][A-Za-z0-9_]*=");

# The value of a string of hexadecimal (base 16) or octal (base 8) digits.
#   $base - 16 or 8.
def digits_value($base):
  ascii_downcase | explode | reduce .[] as $c (0; . * $base + (if $c >= 97 then $c - 87 else $c - 48 end));

# The character of a code point, or U+FFFD for a number that is not a
# Unicode scalar value (a surrogate or a value beyond U+10FFFF), which jq's
# implode would refuse. The escapes decoded with it (see ansi_c_text and
# decoded_text) only need to be right for the ASCII letters the checks match.
def codepoint_text:
  if (. >= 0 and . < 55296) or (. > 57343 and . <= 1114111) then [.] | implode else "\ufffd" end;

# The positional words of an argument list: options are skipped, and so is the
# word after an option that takes a separate value.
#   $valopts - the options that take a separate value, for example ["-n"].
def positionals($valopts):
  reduce .[] as $w ({skip: false, out: []};
    if .skip then .skip = false
    elif ($w | startswith("-")) and $w != "-" then .skip = ($w | IN($valopts[]))
    else .out += [$w] end)
  | .out;

# The indices of the positional words of an argument list (as positionals
# reads them), as a stream. It is lazy, so first(...) or limit(n; ...) stops
# reading at the word it needs: the checks look for a verb or a subcommand at
# every occurrence of a trigger word, and reading the whole rest of a long
# segment each time would make their cost quadratic.
#   $valopts - the options that take a separate value, for example ["-n"].
def positional_indices($valopts):
  foreach .[] as $w ({i: -1, skip: false, hit: false};
    .i += 1
    | if .skip then .skip = false | .hit = false
      elif ($w | startswith("-")) and $w != "-" then .skip = ($w | IN($valopts[])) | .hit = false
      else .hit = true end;
    select(.hit) | .i);

# The index of the first positional word of an argument list, or null when
# there is none.
#   $valopts - the options that take a separate value, for example ["-n"].
def first_positional_index($valopts): first(positional_indices($valopts)) // null;

# The first positional word of an argument list, or null when there is none.
#   $valopts - the options that take a separate value, for example ["-n"].
def first_positional($valopts):
  . as $a | ($a | first_positional_index($valopts)) as $k | if $k == null then null else $a[$k] end;

# oc options that take a separate value, so their value is not read as a
# resource, a verb or a pod (for example the namespace in `-n apps`): the
# global options of `oc options` (oc 4.22) and the value options of the verbs
# whose positional words the checks read: get (`--sort-by`, `--chunk-size`,
# `-L`, `--label-columns` and `--subresource` among them, ADR 0035), delete,
# exec, rsh, debug and port-forward, and (ADR 0036) run, create and the
# create subcommands whose command part check_in_cluster reads (job,
# cronjob, deployment and deploymentconfig: `--overrides`, `--restart`,
# `--env`, `--schedule`, `--from`, `-r` and the others). (check_oc_patch and
# check_deletion_approval read every word of an oc patch or oc annotate, and
# an oc apply or oc replace counts only by its verb.) An option that pflag
# gives a value of its own when it stands alone (NoOptDefVal: the --cascade
# of delete and run, the --dry-run of delete, debug, run and create, and
# create's --validate) takes no separate value and is not listed.
# A boolean option must never be listed: the word after it would be skipped,
# and a pod taken for its value would let the next word, the command, pass
# for the pod in check_in_cluster (`oc run --rm tmp redis-cli`, were --rm
# listed, would read redis-cli as the pod's name). That is why patch's -p is
# not listed: it is the boolean --previous of `oc logs -p`.
# scripts/tests/test-guard-bash.sh compares the list with the oc 4.22 client
# when that client is on PATH.
def oc_value_options:
  ["-n", "--namespace", "-l", "--selector", "-f", "--filename", "-o", "--output",
   "--field-selector", "--context", "--cluster", "--kubeconfig", "--kuberc", "--server", "-s",
   "--token", "--user", "--as", "--as-group", "--as-uid", "--as-user-extra", "-k", "--kustomize",
   "--timeout", "--grace-period", "--request-timeout", "--template", "--raw",
   "--certificate-authority", "--client-certificate", "--client-key", "--cache-dir",
   "--tls-server-name", "--loglevel", "-v", "--v", "--vmodule", "--log-flush-frequency",
   "--profile", "--profile-output", "-c", "--container", "--shell",
   "--pod-running-timeout", "--image", "--image-stream", "--node-name", "--to-namespace",
   "--as-user", "--address", "--chunk-size", "-L", "--label-columns", "--sort-by",
   "--subresource",
   "--annotations", "--env", "--field-manager", "--from", "--image-pull-policy", "--labels",
   "--override-type", "--overrides", "--port", "-r", "--replicas", "--restart", "--schedule"];

# The options that oc itself defines as booleans (`oc options`, oc 4.22), and
# its help flag. Before the verb, cobra reads every other option without `=`
# as one that takes the next word as its value (see cobra_verb_index).
def oc_root_boolean_options:
  ["--disable-compression", "--insecure-skip-tls-verify", "--match-server-version",
   "--warnings-as-errors", "-h", "--help"];

# The index of oc's verb in the words after `oc` as cobra finds it, or null.
# cobra's Command.Find (its stripFlags, in the cobra that oc 4.22 vendors)
# reads the words before the verb with oc's own options only: an option
# `--name`, or a two-character `-x`, without `=` takes the next word as its
# value unless oc itself defines it as a boolean, also when only the verb
# defines it, and also when the verb reads it as a boolean. So oc 4.22 runs
# `oc -p '<patch>' patch pvc data`, `oc --type merge patch ...` and
# `oc --field-manager m annotate ...` as patch and annotate, and
# `oc --vmodule x=1 delete ...` as delete. `--` ends the search.
def cobra_verb_index:
  first(foreach .[] as $w ({i: -1, skip: false, stop: false, hit: false};
      .i += 1
      | .hit = false
      | if .stop then .
        elif .skip then .skip = false
        elif $w == "--" then .stop = true
        elif ($w | startswith("-")) then
          .skip = (($w | contains("=") | not)
                   and (($w | startswith("--")) or ($w | length) == 2)
                   and ($w | IN(oc_root_boolean_options[]) | not))
        else .hit = true end;
      select(.hit) | .i)) // null;

# The indices of the words after `oc` that oc may run as its verb, as a
# stream: the first positional word as positionals reads it with
# oc_value_options, and the verb cobra finds (see cobra_verb_index). Each one
# covers what the other misses: cobra's reading finds the verb behind an
# option the hook does not list (`oc -p '<patch>' patch ...`, which
# oc_value_options cannot list because `-p` is the boolean --previous of
# `oc logs -p`), and the hook's list finds it behind a boolean that a newer
# oc may define on oc itself. The checks look at both, which errs towards
# blocking.
def oc_verb_indices:
  ([first_positional_index(oc_value_options), cobra_verb_index] | map(values) | unique)[];

# The words that oc's verb parses, for the verb at index $k of the words after
# `oc`, as a stream of one or two lists. cobra hands the verb every word but
# the verb itself (Command.Find, argsMinusFirstX), options before the verb
# included, and the verb's own options are interspersed (pflag), so the first
# list is all of them: in `oc --wait pvc delete data` the verb reads
# `pvc data` (oc 4.22 applies `oc --overwrite <key>=<value> annotate ...`
# the same way), and in `oc --shell /usr/bin/psql rsh pod` its --shell. When
# the words before the verb hold a word that positionals reads as positional,
# an option there that the hook does not list may take it as its value
# instead, so the words after the verb alone are the second list.
#   $k - the index of the verb (see oc_verb_indices).
def oc_verb_words($k):
  .[:$k] as $pre
  | ($pre + .[$k + 1:]),
    (if ($pre | positionals(oc_value_options) | length) > 0 then .[$k + 1:] else empty end);

# The verbs of every `oc` in a list of words, as a stream (see
# oc_verb_indices).
def oc_verbs:
  . as $s
  | range(0; $s | length) as $i
  | select(($s[$i] | basename) == "oc")
  | $s[$i + 1:] as $w
  | $w | oc_verb_indices | $w[.];

# The first `oc` in a list of words that may run one of $verbs, as
# {verb, i} with the verb and the index i of its `oc`, or null. Options may
# stand between oc and the verb (see oc_verb_indices).
#   $verbs - the oc verbs to look for, for example ["exec", "rsh", "debug"].
def oc_with_verb($verbs):
  . as $s
  | first(range(0; $s | length) as $i
      | select(($s[$i] | basename) == "oc")
      | $s[$i + 1:] as $w
      | $w | oc_verb_indices | $w[.]
      | select(IN($verbs[]))
      | {verb: ., i: $i}) // null;

# The first `oc exec`, `oc rsh` or `oc debug` in a list of words (see
# oc_with_verb): what it reads on stdin runs in the cluster as a script.
def in_cluster_oc: oc_with_verb(["exec", "rsh", "debug"]);

# The first `oc run` in a list of words (ADR 0036): what it reads on stdin
# goes to the pod it starts (`oc run -i tmp --image=redis -- sh`), so check 2
# looks for a Redis client in it (see check_in_cluster).
def pod_stdin_oc: oc_with_verb(["run"]);

# The first `oc apply`, `oc create` or `oc replace` in a list of words (ADR
# 0036): what it reads on stdin is an inline manifest (`oc apply -f - <<'EOF'`),
# which check 1 reads for the deletion-approved annotation and check 2 for a
# Redis client. Every text it gets on stdin counts, whatever its -f says, as
# ADR 0028 reads every text that oc exec gets; that errs towards blocking.
def manifest_oc: oc_with_verb(["apply", "create", "replace"]);

# The words of a text, cut at blanks with plain string splits. Their cost
# grows linearly with the text; a regex split (splits) rescans the string for
# every match (see `chunks`), which is too slow for a long script.
def blank_words: split("\n")[] | split("\r")[] | split("\t")[] | split(" ")[] | select(length > 0);

# Whether a word is an oc option that names its target in a file instead of
# a positional word: -f, --filename, -k, --kustomize, also joined to its value.
def is_filename_option:
  IN("-f", "--filename", "-k", "--kustomize") or test("^-[fk].|^--(?:filename|kustomize)=");

# The values of the --shell options in a list of words (`--shell X` or
# `--shell=X`): the program oc rsh starts when no command follows the target.
def shell_option_values:
  . as $a
  | range(0; $a | length) as $i
  | if $a[$i] == "--shell" then ($a[$i + 1] // empty)
    elif ($a[$i] | startswith("--shell=")) then $a[$i][8:]
    else empty end;

# The command part of `oc exec`, `oc rsh` or `oc debug`: the words it runs
# inside the cluster. The input is the words the verb parses (see
# oc_verb_words): the words after it, and any options before it. Only these
# words can start a program there, so a pod, container or namespace named like a
# database client (`oc exec -n verseghy deploy/mysql -- df -h`) is not one.
# The rules follow the oc 4.22 sources (pkg/cli/rsh, pkg/cli/debug and the
# vendored k8s.io/kubectl pkg/cmd/exec):
#   - exec and debug read their own options up to `--`, and the command is
#     every word after it. Words between the target and `--` never run: exec
#     drops them, and debug reads them as more resources or as KEY=VALUE
#     environment changes. Without `--`, which oc 4.22 refuses for exec
#     ("exec [POD] [COMMAND] is not supported anymore") but older clients ran,
#     every word after the target counts.
#   - rsh stops reading its own options at the target (SetInterspersed(false)),
#     so the command is every word after the target, options included; with
#     `--` before the target, the target is the first word after `--`. Its
#     --shell names the program it starts when no command follows, so that
#     value counts too. With -f the target comes from the file, and every
#     positional word is the command (rsh.go: `o.Command = args[0:]`).
#   - (ADR 0036) run, whose first positional word is the name of the pod it
#     starts, reads its options interspersed, and gives every positional
#     word after the name to the container as its arguments, or with
#     --command as its command (k8s.io/kubectl pkg/cmd/run: `params["args"]
#     = args[1:]`). cobra drops the `--` from those words and keeps the
#     ones before it, and run checks where the `--` stood only to refuse
#     one right after `run` ("NAME is required for run"), so the command
#     part is every positional word after the name before `--` and every
#     word after it: `oc run tmp --image=redis --command redis-cli -- -h
#     <service> FLUSHALL` runs redis-cli, and so does `oc run tmp
#     --image=redis redis-cli -- ...`, because the image's entrypoint runs
#     a first argument that is not an option as the command. An image
#     named like a client (`--image redis-cli`) is not one, and neither is
#     the pod's own name (`oc run redis-cli --image=redis -- redis-server`).
#     The create subcommands that start pods are read the same way (see
#     in_cluster_parts); they refuse a second word before `--` ("exactly
#     one NAME is required"), so there the words before it only err
#     towards blocking.
#   $verb - exec, rsh, debug or run.
def in_cluster_command($verb):
  . as $a
  | ($a | index("--")) as $dash
  | ($a[:($dash // ($a | length))] | first_positional_index(oc_value_options)) as $t
  | if $verb == "rsh" then
      $a[:($t // $dash // ($a | length))] as $opts
      | [$opts | shell_option_values]
        + (if any($opts[]; is_filename_option) then
             (if $t != null then $a[$t:] elif $dash != null then $a[$dash + 1:] else [] end)
           elif $t != null then $a[$t + 1:]
           elif $dash != null then $a[$dash + 2:]
           else [] end)
    elif $verb == "run" then
      (if $t == null then [] else $a[$t + 1:($dash // ($a | length))] | positionals(oc_value_options) end)
      + (if $dash == null then [] else $a[$dash + 1:] end)
    elif $dash != null then $a[$dash + 1:]
    elif $t == null then []
    else $a[$t + 1:] end;

# The `oc create` subcommands that make objects whose pods run a command
# given after `--` (D110, ADR 0036; `oc create --help`, oc 4.22), with their
# aliases: `job NAME --image=image -- [COMMAND] [args...]`, and cronjob (cj),
# deployment (deploy) and deploymentconfig (dc) the same way. Each takes
# exactly one NAME before `--` ("exactly one NAME is required"), and the
# words after it are the command of the pod's container.
def pod_create_types: ["job", "cronjob", "cj", "deployment", "deploy", "deploymentconfig", "dc"];

# The command parts of one reading of an oc verb that runs a command in the
# cluster, as a stream of {verb, part} (see in_cluster_command). For `oc
# create`, the subcommand is found as oc's verb is found, in both readings
# (see oc_verb_indices), so an option before it that the hook does not list
# cannot hide it (`oc create --schedule '*/5 * * * *' cronjob x -- ...`), and
# each reading that names one of pod_create_types gives the words after it,
# which create reads as run reads its own: the NAME, then the command. Any
# other subcommand (`oc create configmap ...`) runs no command and gives no
# part. The verb in the result names the subcommand too (`create job`), for
# the block reason.
#   $verb - exec, rsh, debug, run or create.
def in_cluster_parts($verb):
  if $verb == "create" then
    . as $a
    | $a | oc_verb_indices as $ty
    | select($a[$ty] | IN(pod_create_types[]))
    | {verb: "create \($a[$ty])", part: ($a[$ty + 1:] | in_cluster_command("run"))}
  else {verb: $verb, part: in_cluster_command($verb)} end;

# The synthetic segment for text that oc exec, oc rsh or oc debug reads on
# stdin, which runs in the cluster: `oc <verb> stdin -- sh -c '<text>'`. The
# word "stdin" stands in for the target, so that the command part (see
# in_cluster_command) holds `sh -c '<text>'` for every verb, rsh included.
#   $verb - exec, rsh or debug.
def in_cluster_script($verb): ["oc", $verb, "stdin", "--", "sh", "-c", .];

# The first word of a synthetic segment that carries a text the shell does
# not run as commands (ADR 0036): an inline manifest that oc apply, oc create
# or oc replace reads (kind "manifest"), or the text that oc run passes to
# the pod it starts (kind "pod-stdin"). Its text is base64-encoded, so that
# no other check reads it as words: a manifest is data, and a ConfigMap that
# quotes `oc exec pod -- psql` must not count as that command. The NUL keeps
# it from colliding with a word the lexer gets from shell text (a NUL ends a
# $'...' part, and bash itself cannot pass one in a word); a forged one only
# makes the checks read a text as such a text, which errs towards blocking.
# It holds the word oc, so that the fast path (see may_matter) lexes a chunk
# that strip_heredocs filled with such a line.
def synthetic_marker: "\u0000oc-guard-synthetic";

# The synthetic segment for a text (see synthetic_marker), as a list of
# words: [marker, kind, verb, base64 of the text].
#   $kind - "manifest" or "pod-stdin";
#   $verb - the oc verb that reads the text (apply, create, replace or run).
def synthetic_segment($kind; $verb): [synthetic_marker, $kind, $verb, @base64];

# The kind, verb and decoded text of a synthetic segment (see
# synthetic_segment), or empty for any other segment. A forged segment whose
# text is not base64 decodes to "", which names nothing.
#   $kind - "manifest" or "pod-stdin".
def synthetic_text($kind):
  select(length == 4 and .[0] == synthetic_marker and .[1] == $kind)
  | {verb: .[2], text: (.[3] | try @base64d catch "")};

# How xargs or GNU parallel among the words before a command passes its input
# on to that command: null when neither runs it, or when xargs reads its
# arguments from a file (-a, --arg-file), because the command then gets the
# piped input on stdin (GNU xargs leaves stdin alone with -a, and otherwise
# gives the command /dev/null); {replace: R} when the command's words hold
# the replace string R (xargs -I R, -i or --replace, and parallel's default
# {}), so that each input line takes R's place; or {replace: null} when the
# input words are appended to the command.
#   $before  - the words before the command;
#   $command - the command and its arguments.
def runner_feed($before; $command):
  (first(range(($before | length) - 1; -1; -1) as $j
     | select($before[$j] | basename | IN("xargs", "parallel")) | $j) // null) as $x
  | if $x == null then null
    elif ($before[$x] | basename) == "xargs"
         and any($before[$x + 1:][]; IN("-a", "--arg-file") or test("^-a.|^--arg-file=")) then null
    else
      $before[$x + 1:] as $o
      | (first(range(0; $o | length) as $k | $o[$k] as $w
          | if $w == "-I" then ($o[$k + 1] // empty)
            elif ($w | startswith("--replace=")) then $w[10:]
            elif $w == "-i" or $w == "--replace" then "{}"
            elif ($w | test("^-[Ii].")) then $w[2:]
            else empty end)
         // (if ($before[$x] | basename) == "parallel" then "{}" else null end)) as $r
      | {replace: (if $r != null and $r != "" and any($command[]; contains($r)) then $r else null end)}
    end;

# The programs whose arguments the checks read: oc and argocd for check 1,
# and the database commands of check 2 (the app binary backend3-cli takes
# Loco's db subcommands). When xargs or GNU parallel runs one of them, the
# text they read on stdin becomes its arguments (see runner_fed_index).
# argocd is not listed: runner_fed_index finds it by is_argocd_name, under
# its own name and its download names.
def fed_programs: ["oc", "cargo", "backend3-cli", "sea-orm-cli", "psql", "pg_restore", "mysql", "mariadb"];

# The index of the command that xargs or GNU parallel runs in a segment, when
# it is one of fed_programs (argocd also under its download names, see
# is_argocd_name), or null: the first such word after the last xargs or
# parallel word, so wrappers such as `timeout 5` may stand between. oc exec,
# oc rsh and oc debug are found by in_cluster_oc first.
def runner_fed_index:
  . as $s
  | (first(range(($s | length) - 1; -1; -1) as $j
       | select($s[$j] | basename | IN("xargs", "parallel")) | $j) // null) as $x
  | if $x == null then null
    else first(range($x + 1; $s | length) as $k
           | select($s[$k] | basename | IN(fed_programs[]) or is_argocd_name) | $k) // null end;

# The segments that xargs or GNU parallel run with the text they read on
# stdin, which is the input: with each input line in place of the replace
# string (`xargs -I{} oc annotate {} ...`), or with the input words appended
# (`echo pvc/data | xargs oc delete`), as runner_feed says. A segment with
# the replace string filled in is the command as the runner runs it, so the
# runner and its options are left out: kept, they would make the checks that
# ask runner_feed (generated_delete, check_port_forward) take the filled-in
# command for one that the runner still appends words to. An appended
# segment keeps them, so those checks still see that the words came from
# the runner.
#   $cur  - the words of the segment;
#   $i    - the index of the command they run;
#   $feed - what runner_feed returned for it, never null.
def fed_segments($cur; $i; $feed):
  if $feed.replace != null then
    (first(range($i - 1; -1; -1) as $j | select($cur[$j] | basename | IN("xargs", "parallel")) | $j) // $i) as $x
    | [split("\n")[] | select(test("\\S")) | sub("^[ \\t]+"; "") as $line
       | $cur[:$x] + [$cur[$i:][] | split($feed.replace) | join($line)]]
  else [$cur + [blank_words]] end;

# Words after which kubectl is being run rather than mentioned.
def runner_words:
  ["sudo", "doas", "env", "exec", "command", "builtin", "nohup", "nice", "time",
   "timeout", "xargs", "watch", "stdbuf", "setsid", "strace", "ionice", "flock",
   "chroot", "ssh", "eval", "noglob", "rsh", "debug", "-exec", "-execdir", "-ok",
   "-okdir", "then", "do", "else", "elif", "if", "while", "until", "!", "{"];

# The first positional word after `cargo`: the cargo subcommand, or null.
# `+toolchain` and global options are skipped. It stops at the subcommand (see
# positional_indices for why).
def cargo_subcommand:
  first(foreach .[] as $w ({skip: false, sub: null};
      if .skip then .skip = false
      elif ($w | startswith("+")) then .
      elif ($w | startswith("-")) then .skip = ($w | IN("--color", "--config", "-Z", "-C"))
      else .sub = $w end;
      select(.sub != null) | .sub)) // null;

# ------------------------------------------------------------------ lexing
#
# The command is split into segments (simple commands) of words. Separators
# (; & && || | |& and newlines) end a segment, and so do the parentheses of a
# subshell group. Redirections are dropped. An unquoted $(...), `...` or
# process substitution <(...) is lexed as segments of its own, and the
# segment around it goes on after it with a placeholder word in its place
# (see lex_chunk). When the shell runs the output of such a substitution as
# commands, and it is echo or printf output, that text is split as commands
# too (see substitution_use, ADR 0035).
# Quotes and backslashes are removed from each word the way the shell joins
# them; a $'...' part has its escapes decoded (see ansi_c_text), and a $"..."
# part counts as double-quoted (ADR 0035). A quoted string that the shell
# would run as a command (after `-c`, `--command`, `eval`, `ssh` or `watch`,
# and every $(...) or `...` inside double quotes) is split again into
# segments of its own, recursively.

# One token per match. Capture groups, in order: [comment, redirection,
# separator, word]. Quotes and escapes stay inside the word for word_text. A
# $'...' part comes first among a word's parts, so that a `\'` inside it does
# not end it.
def token_re:
  "(#[^\\n]*)"
  + "|((?:[0-9]+|&)?[<>]+&?[0-9-]*)"
  + "|(&&|\\|\\||;;|\\|&|[;&|()`\\n])"
  + "|((?:\\$'(?:\\\\[\\s\\S]|[^'\\\\])*'|'[^']*'|\"(?:\\\\[\\s\\S]|[^\"\\\\])*\"|\\\\[\\s\\S]|[^\\s'\"\\\\;&|()<>`#])"
  + "(?:\\$'(?:\\\\[\\s\\S]|[^'\\\\])*'|'[^']*'|\"(?:\\\\[\\s\\S]|[^\"\\\\])*\"|\\\\[\\s\\S]|[^\\s'\"\\\\;&|()<>`])*)";

# The parts of one word. Capture groups: [single-quoted content,
# double-quoted content (also of a $"..." part, bash's locale quoting, which
# is a double-quoted string that bash may translate), escaped character,
# plain text, the content of a $'...' part (ANSI-C quoting, see ansi_c_text)].
# Plain text stops at a `$` right before a quote, so that the `$` of $'...'
# and $"..." belongs to the quoted part, as the shell reads it, and is not
# kept as text (`$'ns'` is ns, not $ns).
def part_re:
  "'([^']*)'|\\$?\"((?:\\\\[\\s\\S]|[^\"\\\\])*)\"|\\\\([\\s\\S])|((?:[^'\"\\\\$]|\\$(?!['\"]))+)"
  + "|\\$'((?:\\\\[\\s\\S]|[^'\\\\])*)'";

# Removes the backslashes that escape $ ` " \ inside double quotes.
def dq_unescape: gsub("\\\\(?<c>[$`\"\\\\])"; .c);

# The single-character escapes of bash's ANSI-C quoting ($'...'), by the
# character after the backslash (bash manual, "ANSI-C Quoting").
def ansi_c_simple_escapes:
  {"a": "\u0007", "b": "\b", "e": "\u001b", "E": "\u001b", "f": "\f", "n": "\n", "r": "\r",
   "t": "\t", "v": "\u000b", "'": "'", "\"": "\"", "?": "?"};

# The text inside a $'...' word part, as bash decodes it (bash manual,
# "ANSI-C Quoting"; ADR 0035): the escapes of ansi_c_simple_escapes, \NNN
# (one to three octal digits), \xHH (one or two hexadecimal digits), \uHHHH
# and \UHHHHHHHH (a Unicode code point) and \cX (a control character). An
# unknown escape keeps its backslash, and a NUL ends the text, as in bash. A
# byte above 0x7F becomes the code point of that number, which is enough for
# the ASCII words the checks match. So `$'\x6e\x73'` reads as ns. The text
# is cut at backslashes with a plain string split, so the cost grows
# linearly with it.
def ansi_c_text:
  def unescape_head:
    .[0:1] as $c
    | if $c | test("[0-7]") then
        capture("^(?<d>[0-7]{1,3})(?<r>[\\s\\S]*)$") | (.d | digits_value(8) % 256 | codepoint_text) + .r
      elif $c == "x" and test("^x[0-9A-Fa-f]") then
        capture("^x(?<d>[0-9A-Fa-f]{1,2})(?<r>[\\s\\S]*)$") | (.d | digits_value(16) | codepoint_text) + .r
      elif $c == "u" and test("^u[0-9A-Fa-f]") then
        capture("^u(?<d>[0-9A-Fa-f]{1,4})(?<r>[\\s\\S]*)$") | (.d | digits_value(16) | codepoint_text) + .r
      elif $c == "U" and test("^U[0-9A-Fa-f]") then
        capture("^U(?<d>[0-9A-Fa-f]{1,8})(?<r>[\\s\\S]*)$") | (.d | digits_value(16) | codepoint_text) + .r
      elif $c == "c" and length > 1 then
        (.[1:2] | explode[0] | (if . == 63 then 127 else . % 32 end) | codepoint_text) + .[2:]
      elif ansi_c_simple_escapes | has($c) then ansi_c_simple_escapes[$c] + .[1:]
      else "\\" + . end;
  # An empty text splits into no pieces at all.
  if . == "" then ""
  else
    split("\\") as $p
    # .lit: the backslash before this piece was the second one of a `\\`.
    | reduce $p[1:][] as $q ({out: $p[0], lit: false};
        if .lit then .out += $q | .lit = false
        elif $q == "" then .out += "\\" | .lit = true
        else .out += ($q | unescape_head) end)
    | (.out | split("\u0000") | .[0]) // ""
  end;

# Programs whose quoted argument is itself a shell command line.
def reparse_words: ["eval", "ssh", "watch"];

# Programs that are shells (or feed their input to one). A heredoc read by one
# of them is a script and is checked; any other heredoc is data and skipped.
# It is matched against the line with quotes and backslashes removed, because
# the shell runs `s\h` and `s''h` as sh.
def shell_receiver_re:
  "(?:^|[\\s;&|(`])(?:[^\\s;&|()`]*/)?(?:sh|bash|zsh|dash|ksh|ash|fish|busybox|ssh|eval|source|exec|rsh|debug|su|sudo|doas|xargs)(?=[\\s;&|)`]|$)";

# A variable used as the receiving command (`| "$SHELL"`, `| $0`, `| ${X}`):
# it usually holds a shell, and what it holds cannot be checked, so the
# heredoc counts as a script.
def var_receiver_re:
  "(?:^|[|;&(])\\s*\\$(?:\\{[A-Za-z_][A-Za-z0-9_]*\\}|[A-Za-z_][A-Za-z0-9_]*|[0-9])(?=[\\s;&|)]|$)"
  + "|(?:^|\\s)\\$\\{?(?:SHELL|BASH)\\}?(?=[\\s;&|)]|$)";

# `oc exec`, `oc rsh` or `oc debug` on the receiving line: the heredoc body
# runs inside the cluster. Global options may stand between oc and the verb.
def in_cluster_receiver_re:
  "(?:^|[\\s;&|(])(?:[^\\s;&|()]*/)?oc\\s(?:[^;&|()\\n]*\\s)?(?<verb>exec|rsh|debug)(?=[\\s;&|)]|$)";

# `oc run` on the receiving line (ADR 0036): the heredoc body goes to the pod
# it starts, so check 2 reads it for a Redis client (see pod_stdin_oc).
def pod_stdin_receiver_re:
  "(?:^|[\\s;&|(])(?:[^\\s;&|()]*/)?oc\\s(?:[^;&|()\\n]*\\s)?run(?=[\\s;&|)]|$)";

# `oc apply`, `oc create` or `oc replace` on the receiving line (ADR 0036):
# the heredoc body is an inline manifest (see manifest_oc), also when `cat`
# passes it on (`cat <<'EOF' | oc apply -f -`). The verb is the first one
# after the oc, for the block reason.
def manifest_receiver_re:
  "(?:^|[\\s;&|(])(?:[^\\s;&|()]*/)?oc\\s(?:[^;&|()\\n]*?\\s)?(?<verb>apply|create|replace)(?=[\\s;&|)]|$)";

# xargs or GNU parallel on the receiving line (ADR 0036, the fix that ADR
# 0035's "Follow-up work" names): they read the heredoc body as the
# arguments of the command they run, directly or through cat (`xargs oc
# <<'EOF'`, `cat <<'EOF' | xargs oc`), as they read a here-string.
def runner_receiver_re:
  "(?:^|[\\s;&|(`])(?:[^\\s;&|()`]*/)?(?:xargs|parallel)(?=[\\s;&|)`]|$)";

# The words a shell would see in a text, joined, which is close enough for
# the receiver tests above and for check_in_cluster, as a stream of one or
# two readings. The callers count a match in either (ADR 0035):
#   - the text with every quote and backslash removed, so `s\h`, `s''h` and
#     `"psql"` read as sh and psql. It errs towards blocking: it also removes
#     a backslash that the shell keeps inside quotes;
#   - when the text holds a `$'` or a `$"`, also the text read part by part
#     as the lexer reads a word (see part_re): a $'...' part decoded (see
#     ansi_c_text) and the `$` of a $"..." part dropped, so `o$'c' exec`
#     names oc. A `$'` inside double or single quotes is text there, as in
#     bash, so it never starts a part that a NUL would cut short:
#     `echo "$'\0"; psql; echo "'"` still holds psql, in this reading and in
#     the first. (Only text that holds `$'` or `$"` pays for the scan.)
def unquoted_texts:
  (split("\"") | join("") | split("'") | join("") | split("\\") | join("")),
  (if test("\\$['\"]") then
     [scan(part_re) | .[0] // (.[1] | values | dq_unescape) // .[2] // .[3] // (.[4] | values | ansi_c_text)]
     | join("")
   else empty end);

# Quotes a text as one single-quoted shell word.
def sq_quote: "'" + gsub("'"; "'\\''") + "'";

# A text with every ; & | ( ) inside a quoted string replaced by `_`, for
# the receiver tests (ADR 0036): the receiver patterns stop at a separator
# between oc and its verb, so a quoted option value that holds one (`oc
# --selector='a|b' apply -f -`) would otherwise hide the verb. Only the
# receiving line of a heredoc is read this way, so the gsub's cost per match
# stays small.
def quoted_separators_blanked:
  gsub("(?<s>'[^']*'|\"(?:\\\\[\\s\\S]|[^\"\\\\])*\")"; .s | gsub("[;&|()]"; "_"));

# ------------------------------------------------------------------ heredocs
#
# A heredoc is found on its receiving line, the line that holds the `<<`
# operator; its body is the lines after that line, up to a line that holds
# only the delimiter (blanks around it allowed, which ends a body early only
# where a `<<-` would, and errs towards reading more text as commands). A
# `<<` inside quotes is no operator: `echo '<<X'` prints <<X, and the lines
# after it are commands. The hook reads every command twice where that is
# not sure (see heredoc_texts), and these patterns tell where it is sure: a
# small model of bash's quoting, line by line. It knows single and double
# quotes, a $(...) with plain content and simple strings, `$` before
# anything else, and a comment that starts a word; a line with anything it
# does not model (a backslash, a backtick, $'...', a nested or unbalanced
# $(...)) makes the quoting "unknown" from there on. The patterns use
# possessive loops, and each item starts at a different character, so each
# line is read once.

# The content of a $(...) that the model reads: plain text and simple
# strings, with no parentheses, `$`, backslash, backtick or comment.
def heredoc_sub_content_re: "(?:[^'\"\\\\`$()#\\n]|'[^'\\n]*'|\"[^\"\\\\`$\\n]*\")";

# A $(...) with such content, or an arithmetic $((...)) with plain content
# (`$((1<<2))`, whose `<<` is a shift), which leaves the quoting where it
# was.
def heredoc_sub_re:
  "\\$\\((?:" + heredoc_sub_content_re + "*|\\([^'\"\\\\`$()#\\n]*\\))\\)";

# One character, `$` or $(...) inside a double-quoted string.
def heredoc_dq_content_re: "(?:[^\"\\\\`$\\n]|\\$(?![('\"])|" + heredoc_sub_re + ")";

# One item outside quotes: a plain character, a `$` that starts no quote or
# $( (`$1`, `${x}`), a whole single- or double-quoted string, a `#` inside a
# word (`a#b`, `$#`), or a modelled $(...).
def heredoc_out_item_re:
  "(?:[^'\"\\\\`$#\\n]|\\$(?![('\"])|'[^'\\n]*'|\"" + heredoc_dq_content_re + "*\"|(?<=[^\\s;&|()<>])#|" + heredoc_sub_re + ")";

# An open double-quoted string with an open $( in it, at the end of a text:
# `git commit -m "$(cat ` before `<<'EOF'`. Inside the $( the quoting starts
# afresh, so a `<<` there is an operator.
def heredoc_dqsub_tail_re: "\"" + heredoc_dq_content_re + "*\\$\\(" + heredoc_sub_content_re + "*";

# How a line that the model reads ends, after its items: with nothing, a
# comment, an open single-quoted string, an open "$( or an open
# double-quoted string. The names of the captures say which.
def heredoc_line_end_re:
  "(?:|(?:^|(?<=[\\s;&|()<>]))#[^\\n]*|(?<sq>')[^'\\n]*|(?<ds>" + heredoc_dqsub_tail_re + ")|(?<dq>\")"
  + heredoc_dq_content_re + "*)$";

# The start of a line, by the quoting it starts in, up to the point where
# it is outside quotes again: at once ("out"), after the `'` that closes a
# string ("sq") or the `"` that closes one ("dq"), or after the `)"` that
# closes an open "$( ("dqsub", whose content is read as heredoc_sub_content_re
# reads it).
#   $st - "out", "sq", "dq" or "dqsub".
def heredoc_state_prefix_re($st):
  if $st == "out" then "^"
  elif $st == "sq" then "^[^'\\n]*'"
  elif $st == "dq" then "^" + heredoc_dq_content_re + "*\""
  else "^" + heredoc_sub_content_re + "*\\)\"" end;

# The quoting at the end of a line, from the quoting at its start: "out",
# "sq", "dq", "dqsub" or "unknown" (see the notes above). A line with none
# of ' " \ ` $ # leaves "out" as it is, without a regex. "unknown" stays.
#   $st - the quoting at the start of the line.
def heredoc_next_state($st):
  . as $l
  | if $st == "unknown" then "unknown"
    elif $st == "out" and (test("['\"\\\\`$#]") | not) then "out"
    else
      (first(capture(heredoc_state_prefix_re($st) + "(?:" + heredoc_out_item_re + ")*+" + heredoc_line_end_re))
       | if .sq != null then "sq" elif .ds != null then "dqsub" elif .dq != null then "dq" else "out" end)
      // (if $st == "sq" and test("^[^'\\n]*$") then "sq"
          elif $st == "dq" and test("^" + heredoc_dq_content_re + "*$") then "dq"
          elif $st == "dqsub" and test("^" + heredoc_sub_content_re + "*$") then "dqsub"
          else "unknown" end)
    end;

# Whether the text before a `<<` on a line, which starts in quoting $st,
# leaves the `<<` surely outside quotes: the model reads it to its end
# outside quotes, or inside an open "$( (a `<<` in the content of an open
# "$( started on an earlier line counts too).
#   $st - the quoting at the start of the line.
def heredoc_operator_is_sure($st):
  if $st == "unknown" then false
  else test(heredoc_state_prefix_re($st) + "(?:" + heredoc_out_item_re + ")*+(?:" + heredoc_dqsub_tail_re + ")?$")
       or ($st == "dqsub" and test("^" + heredoc_sub_content_re + "*$"))
  end;

# The heredoc operators on a line, as a stream of {offset, length, q, d}: the
# `<<` or `<<-`, the quote q around the delimiter ('', ' or ") and the
# delimiter d. A `<<<` (a here-string) is none: its first `<<` is followed
# by a `<`, which no delimiter starts with, and the look-behind keeps its
# last two from counting. Neither is a `<<` whose delimiter is not a plain
# word (`<<\EOF`, `<<"$X"`): its body is then read as commands, which errs
# towards blocking.
def heredoc_operators:
  match("(?<!<)<<-?[ \\t]*(?<q>['\"]?)(?<d>[A-Za-z0-9_.-]+)\\k<q>"; "g")
  | {offset, length, q: .captures[0].string, d: .captures[1].string};

# Whether a line ends a heredoc with delimiter $d.
#   $d - the delimiter.
def is_heredoc_delimiter($d):
  contains($d) and (sub("^[ \\t]+"; "") | sub("[ \\t]+$"; "")) == $d;

# What a receiving line becomes once its heredocs are read, from the line
# itself, its heredocs $taken in order, each with its body, and the line
# $next after the last body. A body is kept when a shell reads it, and only
# its $(...) and `...` are kept when the delimiter is unquoted (the shell
# expands those even in data), so a commit message or a file written with
# `cat <<'EOF'` cannot trip check 1. A body that oc exec, oc rsh or oc debug
# reads is kept as the -c script of a synthetic `oc <verb> stdin -- sh -c
# '<body>'` line (see in_cluster_script), so that check 2 sees the database
# commands in it next to the oc that runs them in the cluster. lex does the
# same for here-strings and piped echo or printf text. ADR 0036 adds three
# receivers, each read in addition to the above:
#   - oc run: the body is the text the pod gets, kept as a synthetic
#     "pod-stdin" segment for check 2's Redis client rule (see
#     synthetic_marker), and not as commands, since the pod's program need
#     not be a shell (a shell receiver keeps it as a script, as before);
#   - oc apply, oc create or oc replace: the body is an inline manifest, kept
#     as a synthetic "manifest" segment;
#   - xargs or GNU parallel: the body is also put on the receiving line as a
#     here-string, in place of the heredoc, which lex passes on as their
#     input (and cat passes on as its output, see cat_here_strings), so that
#     `xargs oc <<'EOF'` with `delete ns web` in the body reads as `xargs oc
#     delete ns web`.
# The receivers are found on the line with its operators taken out, in each
# reading of unquoted_texts, also with the separators inside quoted strings
# blanked (see quoted_separators_blanked). Every receiver on the line reads
# every body on it, which errs towards blocking: bash gives a command the
# last of its heredocs (`oc apply -f - <<'A' <<'B'` reads B) and each
# command its own. When the pipeline goes on after the bodies, because the
# line ends in `|` or `|&` (`cat <<'EOF' |`, the body, `EOF`, then `oc apply
# -f -`), or when $next closes a group (`( cat <<'EOF'`, the body, `EOF`,
# then `) | oc apply -f -`), $next is part of the receiving text too, since
# its command reads what the receiving line prints. The result is
# {line, readings, joined}: the receiving line without its operators (with
# here-strings in their place for xargs or parallel), the reading lines,
# each after a line break, and whether $next is part of the receiving text.
#   $taken - the heredocs, as heredoc_operators gives them, with .body;
#   $next  - the line after the last body, or null at the end of the text.
def heredoc_replacement($taken; $next):
  . as $l
  | (reduce ($taken | reverse[]) as $op ($l; .[:$op.offset] + " " + .[$op.offset + $op.length:])) as $line
  | (($next != null) and (($line | test("\\|&?[ \\t]*$")) or ($next | test("^[ \\t]*[)}]")))) as $joined
  | ($line + (if $joined then " " + $next else "" end)) as $recv
  # Each reading of the line counts (see unquoted_texts).
  | [$recv | unquoted_texts, (select(test("['\"]")) | quoted_separators_blanked | unquoted_texts)] as $plain
  | (first($plain[] | select(test(in_cluster_receiver_re)) | capture(in_cluster_receiver_re).verb) // null) as $verb
  | any($plain[]; test(pod_stdin_receiver_re)) as $pod
  | (first($plain[] | select(test(manifest_receiver_re)) | capture(manifest_receiver_re).verb) // null) as $manifest
  | any($plain[]; test(runner_receiver_re)) as $runner
  | any($plain[]; test(shell_receiver_re) or test(var_receiver_re)) as $shell
  | {joined: $joined,
     line: (if $runner then
              reduce ($taken | reverse[]) as $op ($l; .[:$op.offset] + " <<< " + ($op.body | sq_quote) + " " + .[$op.offset + $op.length:])
            else $line end),
     readings: ([$taken[] | .body as $body
        | (if $verb != null then "oc \($verb) stdin -- sh -c \($body | sq_quote)"
           elif $shell then $body
           elif .q == "" then [$body | scan("\\$\\([\\s\\S]*?\\)|`[^`]*`")] | join("\n")
           else "" end),
          (if $pod then $body | synthetic_segment("pod-stdin"; "run") | join(" ") else empty end),
          (if $manifest != null then $body | synthetic_segment("manifest"; $manifest) | join(" ") else empty end)
        | "\n" + .] | join(""))};

# Reads the heredocs of a text (see heredoc_replacement), line by line.
# Every operator on a line counts, in order (ADR 0036), each taking the
# lines up to its delimiter; one whose delimiter never comes is not a
# heredoc, and the search goes on with the next. A heredoc counts on every
# line of the command, also after a `<` (`oc apply -f <(cat <<'EOF'`, ADR
# 0036 fixes this): before, a pattern anchored with jq's `^`, which matches
# only at the start of the whole text, found one only on the first line,
# and only before any `<` there, so a script that oc exec read on a later
# line (`echo hi`, then `oc exec -i pg-0 -- sh <<'EOF'` and `psql`) ran on
# the host for the checks.
#   $aware - false to take every operator; true to take only the operators
#            that the quoting model says stand surely outside quotes (see
#            heredoc_operator_is_sure), and to keep the others as command
#            text. heredoc_texts reads both ways.
def strip_heredocs($aware):
  split("\n") as $lines
  | ($lines | length) as $n
  | reduce range(0; $n) as $i ({out: [], st: "out", skip: -1};
      if $i <= .skip then .
      else
        $lines[$i] as $l
        | .st as $st
        | [if $l | contains("<<") then $l | heredoc_operators else empty end
           | select(($aware | not) or ($l[:.offset] | heredoc_operator_is_sure($st)))] as $ops
        # Each operator takes the lines from the end of the previous body.
        | (reduce $ops[] as $op ({c: ($i + 1), taken: []};
             .c as $c
             | (first(range($c; $n) as $j | select($lines[$j] | is_heredoc_delimiter($op.d)) | $j) // null) as $j
             | if $j == null then . else .taken += [$op + {body: ($lines[$c:$j] | join("\n"))}] | .c = $j + 1 end)) as $h
        | (if $aware then .st = ($l | heredoc_next_state($st)) else . end)
        | if ($h.taken | length) == 0 then .out += [$l]
          else
            $lines[$h.c] as $next
            | ($l | heredoc_replacement($h.taken; $next)) as $r
            # When the next line goes on with the pipeline, it comes right
            # after the receiving line, and the readings after both, so that
            # a reading does not stand between them as a stage of its own
            # (`cat <<< '...' |&`, then `xargs oc`). A next line with an
            # operator of its own is read as a receiving line instead.
            | if $r.joined and ($next | contains("<<") | not) then
                .out += [$r.line + "\n" + $next + $r.readings] | .skip = $h.c
                | (if $aware then .st as $s2 | .st = ($next | heredoc_next_state($s2)) else . end)
              else .out += [$r.line + $r.readings] | .skip = $h.c - 1 end
          end
      end)
  | .out | join("\n");

# The texts the checks lex for a command (ADR 0036): the command with every
# heredoc operator read as one (see strip_heredocs), and, when that differs,
# also with only those that surely stand outside quotes read as one. So a
# `<<` that may be quoted (`echo '<<X'`, a commit message that mentions
# `<<EOF`, a heredoc after a line with a backslash) neither hides the
# commands after it nor keeps a real heredoc's body from its receiver; both
# readings are checked, which errs towards blocking. A text without `<<` is
# itself.
def heredoc_texts:
  if test("<<") then strip_heredocs(false) as $a | $a, (strip_heredocs(true) | select(. != $a))
  else . end;

# Cuts a string at every $sep that is outside quotes, as a stream of pieces.
# A piece ends only where the counts of ' and " so far are both even. That
# parity test ignores escaped quotes, so it can keep more text together than
# needed, which costs time but never correctness.
#   $sep - a command separator: "\n", ";", "&&", "||" or "|".
def split_outside_quotes($sep):
  reduce (split($sep)[]) as $p ({out: [], cur: [], sq: 0, dq: 0};
    .cur += [$p]
    | .sq = ((.sq + ($p | split("'") | length) - 1) % 2)
    | .dq = ((.dq + ($p | split("\"") | length) - 1) % 2)
    | if .sq == 0 and .dq == 0 then .out += [.cur | join($sep)] | .cur = [] else . end)
  | (.out + (if (.cur | length) > 0 then [.cur | join($sep)] else [] end))[];

# Cuts a pipeline at every | outside quotes, as a stream of pieces, and keeps
# the | (or |&) at the end of every piece but the last. lex sees from that
# trailing separator that the next piece reads this one's output, and passes
# the piped echo or printf text on to it.
def split_pipeline:
  [split_outside_quotes("|")] as $p
  | range(0; $p | length) as $k
  # `a |& b` was cut into "a " and "& b": the & belongs to the separator.
  | ($p[$k] | if $k > 0 and startswith("&") then .[1:] else . end)
    + (if $k == ($p | length) - 1 then ""
       elif ($p[$k + 1] | startswith("&")) then "|&"
       else "|" end);

# jq's scan, match and gsub recompute every match's offset from the start of
# the string, so their cost grows with (matches x length). A long text is
# therefore cut into chunks before it is lexed: at newlines first, and a piece
# that is still long at ; then && then || then |. Every one of those is a
# segment separator, so cutting there outside quotes changes no segment; at
# | the separator stays on the piece (see split_pipeline), so that the text an
# echo or printf pipes into a later stage still reaches it. Text up to 16 KiB
# is not cut.
def chunks:
  def refine($seps):
    if length <= 16384 or ($seps | length) == 0 then .
    elif $seps[0] == "|" then split_pipeline
    else split_outside_quotes($seps[0]) | refine($seps[1:]) end;
  refine(["\n", ";", "&&", "||", "|"]);

# Words (quotes already removed) that name a shell, or a program that runs
# its standard input or a here-string as a script (oc rsh and oc debug start
# a shell when they get no command), or a variable that usually holds a shell.
def shell_words: ["sh", "bash", "zsh", "dash", "ksh", "ash", "fish", "busybox", "rsh", "debug", "ssh", "su"];
def is_shell_word: (sub("^.*/"; "") | IN(shell_words[])) or test("^\\$\\{?(?:SHELL|BASH|0)\\}?$");

# Shell options that take a separate value, between -c and its command string
# (`bash -c -o pipefail '...'`).
def shell_value_options: ["-o", "+o", "-O", "+O", "--rcfile", "--init-file"];

# A command can only be blocked if it contains one of these words, so every
# other command is allowed without being parsed. The Redis clients (D105,
# D110; see redis_client_names) are blocked only next to an oc, but they are
# trigger words too: a long pipeline is cut into chunks, and a chunk without
# a trigger word is skipped, so the echo that pipes `redis-cli FLUSHALL` into
# an oc exec in a later chunk would be lost (see lex).
def trigger_re:
  "kubectl|grace-period|\\boc\\b|\\bdb\\b|psql|pg_restore|mysql|mariadb|sea-orm-cli|cargo|argocd|"
  + (redis_client_names | join("|"));

# The variables check 2 reads.
def watched_var_re: "DATABASE_URL|PGHOST|PGSERVICE|MYSQL_HOST";

# Whether a piece of shell text can matter to any check: it holds a trigger
# word, or an assignment that check 2 reads. Quotes and backslashes are
# dropped first, so k'ube'ctl still counts. A `\n` or `\t` counts as a
# blank before that (ADR 0035): lex turns them into a line break and a tab
# in the text a printf prints (see printed_text), so in `printf
# 'ls\noc delete ns web' | sh` the word oc starts after the `\n`, which
# dropping only the backslash would join to it (`lsnoc`). split/join is
# linear, unlike a gsub with one match per quote. Text with a $'...' or
# $"..." part always counts (ADR 0035), because its escapes (`$'\x6f\x63'`
# is oc) or the `$` before its quote (`o$"c"`) hide the word from this
# test. lex skips the chunks for which this is false, because no check can
# block a segment without such a word.
def may_matter:
  test("\\$['\"]")
  or (split("\"") | join("") | split("'") | join("")
      | split("\\n") | join(" ") | split("\\t") | join(" ") | split("\\") | join("")
      | test(trigger_re + "|" + watched_var_re));

# The words an echo prints, from the words after it: its leading options
# (`-n`, `-e`, `-E` and their combinations, such as `-ne`) are dropped, as
# bash's echo and coreutils' read them (ADR 0035), so `echo -n pvc/data |
# xargs oc delete` passes pvc/data on, not an oc option `-n`. The first other
# word and every word after it are printed, `--` and a lone `-` included.
def echo_operands:
  (first(range(0; length) as $i | select(.[$i] | test("^-[neE]+$") | not) | $i) // length) as $k
  | .[$k:];

# The index of the command word of a segment, or null: the first word that
# is not an assignment, nor the `{` that opens a brace group (ADR 0036), so
# that `{ echo ...; } | oc apply -f -` prints what its echo prints.
#   $seg - the words of the segment.
def command_word_index($seg):
  first(range(0; $seg | length) | select($seg[.] | is_assignment or . == "{" | not)) // null;

# The words that the command of a segment prints when it is an echo or a
# printf (see echo_operands; printf's format is not applied), or null.
#   $seg - the words of the segment, assignments before the command included.
def printed_words($seg):
  command_word_index($seg) as $j
  | if $j == null then null
    elif ($seg[$j] | basename) == "echo" then $seg[$j + 1:] | echo_operands
    elif ($seg[$j] | basename) == "printf" then $seg[$j + 1:]
    else null end;

# The text that the echo and printf commands among a list of segments print,
# read as lex reads piped text: echo joins its arguments with spaces, printf's
# \n and \t become a line break and a tab, and each command prints a line of
# its own. null when none of the segments is an echo or printf.
def printed_text:
  [.[] | printed_words(.) | values
   | [.[] | split("\\n") | join("\n") | split("\\t") | join("\t")] | join(" ")]
  | if length == 0 then null else join("\n") end;

# The here-strings that a `cat` segment prints (ADR 0036, the fix that ADR
# 0035's "Follow-up work" names), as one piped word, or null: cat with no
# file operand copies its stdin, the here-string, to the next stage of the
# pipeline (`cat <<< 'delete ns web' | xargs oc`, `cat <<< 'redis-cli
# flushall' | oc exec -i redis-0 -- sh`), where lex reads it as it reads
# piped echo text. A cat that also names files counts too, which errs
# towards blocking. Several here-strings are joined line by line, as lex
# joins them for a segment's stdin.
#   $seg  - the words of the segment, assignments before the command included;
#   $hstr - its here-strings.
def cat_here_strings($seg; $hstr):
  if ($hstr | length) == 0 then null
  else
    command_word_index($seg) as $j
    | if $j != null and ($seg[$j] | basename) == "cat" then [$hstr | join("\n")] else null end
  end;

# Whether a word stands where the command name goes, from the words of the
# segment before it, as check_kubectl reads that place (ADR 0035): every one
# is an assignment, or one of them is a runner word (see runner_words),
# whose own options and values may come between (`timeout 30`, `sudo -u
# root`). After a runner word every later word counts, which errs towards
# blocking.
def is_command_place: all(.[]; is_assignment) or any(.[]; IN(runner_words[]));

# Whether a word is the command name with none of a runner's own words before
# it: every word before it is an assignment or a runner word (`sudo`, `env
# X=1`). Only there does the output of a substitution in the command name's
# place take the place of the substitution outright (see close_sub and
# lex_chunk); after a runner's options or values, the word may be one of
# them, so its own text stays in front of the output (ADR 0035).
def is_bare_command_place: all(.[]; is_assignment or IN(runner_words[]));

# How the shell uses the output of an unquoted substitution that has just
# closed (ADR 0035), from the words of the segment around it, up to its
# placeholder (the `$` or "`" word of a $(...) or `...`, or the /dev/fd/63 of
# a process substitution, see lex_chunk):
#   - "script" when a shell, `source` or `.` gets the process substitution:
#     it reads the output as a script (`bash <(echo '...')`);
#   - "command" when the $(...) or `...` is the whole word where the command
#     name goes: the shell runs the output as the command and its first
#     arguments (`$(echo oc) delete ...`). That place is found as
#     check_kubectl finds it: every word before is an assignment, or one of
#     them is a runner word, whose own options and values may stand between
#     (`timeout 30 $(echo oc) delete ...`, `sudo -u root $(echo oc) ...`),
#     which errs towards blocking (close_sub then keeps the placeholder in
#     front of the output, see is_bare_command_place). In a -c or eval
#     string, which lex reads again as commands, that is the form `bash -c
#     "$(echo '...')"` and `eval "$(echo '...')"` take. A double-quoted one
#     in that place is read by lex (see quoted_command_text);
#   - null otherwise: the output is an argument, a value or text.
#   $sh - whether a word before the placeholder is a shell (lex_chunk's .sh).
def substitution_use($sh):
  (.[-1] // "") as $p
  | .[:-1] as $before
  | if $p == "/dev/fd/63" then
      (if $sh or any($before[]; IN("source", ".")) then "script" else null end)
    elif ($p == "$" or $p == "`") and ($before | is_command_place) then "command"
    else null end;

# Splits shell text into an array of segments; each segment is an array of
# words. Nested command strings are appended as further segments, after the
# segments of the text they came from. A word is split again as a command
# line when a shell would run it:
#   - the command string of -c (or --command), also after more options such
#     as `--` or `-o pipefail` (`sh -c -- '...'`), and for a shell also in a
#     combined option (`sh -ce '...'`);
#   - every argument after eval, ssh or watch;
#   - the word after `<<<` when the segment runs a shell (`bash <<< '...'`);
#   - the arguments of an echo or printf whose output is piped into a shell
#     (`echo '...' | sh`), with printf's \n and \t turned into whitespace,
#     also through further stages (`echo '...' | tr a b | sh`), and across a
#     line break after the | (bash continues a pipeline on the next line);
#   - what oc exec, oc rsh or oc debug reads on stdin, the piped echo or
#     printf text and its here-strings, which runs inside the cluster: it is
#     split as commands for check 1, and it also becomes the synthetic
#     segment `oc <verb> stdin -- sh -c '<text>'` (see in_cluster_script),
#     as a heredoc does (see strip_heredocs), so that check_in_cluster sees
#     the database commands in it next to the oc that runs them there. Under
#     xargs (or GNU parallel) the text is not the command's stdin but its
#     arguments (see runner_feed), so the commands it runs become segments of
#     their own instead, with each input line in place of the replace string
#     (`xargs -I{} oc exec {} -- df -h`), or with the input words appended;
#   - (ADR 0035) the same text under xargs or GNU parallel when they run
#     another command whose arguments the checks read (see fed_programs):
#     `echo confirm-deletion x | xargs argocd app` becomes the segment
#     `xargs argocd app confirm-deletion x` (see fed_segments). Read by any
#     other program, the text stays data;
#   - (ADR 0036) a here-string of a `cat` in a pipeline, which reaches the
#     next stage as piped echo text does (see cat_here_strings);
#   - (ADR 0036) the same text read by an oc run, which it passes to the
#     pod it starts, or by an oc apply, oc create or oc replace, which reads
#     it as an inline manifest, and the echo or printf output of a process
#     substitution that one of the latter reads as its file: each is kept
#     as a synthetic segment (see synthetic_marker), not split as commands;
#   - (ADR 0035) the echo or printf output of an unquoted substitution that
#     the shell runs (see substitution_use): a process substitution that a
#     shell, `source` or `.` reads, and a $(...) or `...` in place of the
#     command name, whose output words also take its place in the segment.
#     A word there that is one double-quoted $(...) or `...` takes that
#     output as one word, since the quotes keep the shell from splitting it
#     (`"$(echo oc)" delete ...`, see quoted_command_text);
#   - and, in any other word, every $(...) or `...` inside double quotes.
# A long text is cut into chunks first (see `chunks`), and chunks that cannot
# matter (see may_matter) are not lexed, unless $all asks for every chunk.
#   $depth - the nesting level; nesting stops at 6 to bound the work;
#   $all   - true to lex every chunk, also one without a trigger word, for a
#            text whose printed words are read (see quoted_command_text):
#            `echo bash` there names a shell, and `echo sudo` a runner.
def lex($depth; $all):
  # The echo or printf output of a $(...) or `...` written as a word's only,
  # double-quoted part, for a word in the command name's place (ADR 0035):
  # the shell runs that output as the command name, as one word, so
  # `"$(echo oc)" delete ns web` runs `oc delete ns web`, while
  # `"$(echo 'oc delete')"` names a program called "oc delete". The line
  # breaks at the end of the output are removed, as bash removes them from
  # every command substitution, so `"$(printf 'oc\n')"` names oc too. Empty
  # when the text inside prints nothing that the hook reads (see
  # printed_text).
  #   $content - the double-quoted part's text, quotes removed.
  def quoted_command_text($content):
    $content | dq_unescape | capture("^(?:\\$\\((?<a>[\\s\\S]*)\\)|`(?<b>[^`]*)`)$") | (.a // .b)
    | lex($depth + 1; true) | printed_text | values | sub("\\n+$"; "");
  # The segments of a text that a shell runs as a script: a -c string, the
  # arguments of eval, ssh or watch, a here-string or piped text that a
  # shell reads, what oc exec, oc rsh or oc debug read on stdin, the output
  # of a substitution that the shell runs, and a $(...) or `...` inside
  # double quotes. Its heredocs are read first (ADR 0036, see
  # heredoc_texts), as for the whole command, so that `sh -c 'oc apply -f -
  # <<EOF ... EOF'` reads the manifest, and a script for oc exec inside `ssh
  # host '...'` runs in the cluster.
  def script_segments: gsub("\\\\\\n"; "") | heredoc_texts | lex($depth + 1; false)[];
  # Ends the current segment at separator $sep and resets the per-segment
  # state. When the segment reads text on stdin, from an echo or printf
  # earlier in its pipeline or from its here-strings, that text is handled as
  # described above.
  def finish_segment($sep):
    ($sep == "|" or $sep == "|&") as $piped
    | if (.cur | length) > 0 then
        # Only a segment that reads such text needs to find its oc.
        (if .pipe != null or (.hstr | length) > 0 then .cur | in_cluster_oc else null end) as $oc
        # Or, under xargs or parallel, another command the checks read
        # (ADR 0035): `echo confirm-deletion x | xargs argocd app` runs
        # `argocd app confirm-deletion x`. A here-string is a redirection,
        # not an argument, so its words are left out of that command (the
        # lexer keeps them in .cur, where other checks read them as text).
        | (if (.hstr | length) > 0 then .cur - .hstr else .cur end) as $args
        | (if $oc == null and (.pipe != null or (.hstr | length) > 0) then $args | runner_fed_index else null end) as $fed
        # printf's \n and \t, with plain string splits (a gsub is quadratic).
        | [(.pipe // [])[] | split("\\n") | join("\n") | split("\\t") | join("\t")] as $piped_in
        # echo joins its arguments with spaces; each here-string is a line.
        | ([($piped_in | join(" ")), .hstr[]] | map(select(test("\\S"))) | join("\n")) as $stdin
        | .cur as $cur
        | (if $depth >= 6 or $stdin == "" then .
           elif $oc != null then
             runner_feed($cur[:$oc.i]; $cur[$oc.i:]) as $feed
             | if $feed == null then
                 .nested += [$stdin | in_cluster_script($oc.verb)] + [$stdin | script_segments]
               else .nested += ($stdin | fed_segments($cur; $oc.i; $feed)) end
           elif .pipe != null and .sh then
             .nested += [$piped_in[] | select(test("\\S")) | script_segments]
           else . end)
        # Not exclusive with the shell case: `echo x | xargs sh -c '...'`
        # finds no fed program, and a fed program after a shell word gets
        # its arguments either way.
        | (if $depth >= 6 or $stdin == "" or $fed == null then .
           else runner_feed($args[:$fed]; $args[$fed:]) as $feed
             | if $feed == null then . else .nested += ($stdin | fed_segments($args; $fed; $feed)) end
           end)
        # (ADR 0036) The same text read by an oc run goes to the pod it
        # starts, and read by an oc apply, oc create or oc replace it is an
        # inline manifest: each becomes a synthetic segment (see
        # synthetic_marker), unless xargs or parallel run that oc, which
        # then gets the text as its arguments (see $fed above). Neither is
        # lexed as commands here; a shell in the segment still reads it as
        # a script, as above. An oc exec in the same segment takes it as
        # well (`echo ... | oc exec -i pod -- oc apply -f -`).
        | (if $depth >= 6 or $stdin == "" then .
           else
             ($cur | pod_stdin_oc) as $pod
             | ($cur | manifest_oc) as $manifest
             | (if $pod != null and runner_feed($cur[:$pod.i]; $cur[$pod.i:]) == null
                then .nested += [$stdin | synthetic_segment("pod-stdin"; "run")] else . end)
             | (if $manifest != null and runner_feed($cur[:$manifest.i]; $cur[$manifest.i:]) == null
                then .nested += [$stdin | synthetic_segment("manifest"; $manifest.verb)] else . end)
           end)
        # The words an echo or printf pipes on (see printed_words), without
        # an echo's options and the assignments before it, or the
        # here-strings of a cat (see cat_here_strings).
        | .pipe = (if $piped then (printed_words(.cur) // cat_here_strings(.cur; .hstr) // .pipe) else null end)
        | .segs += [.cur] | .cur = []
      # A line break right after a separator does not end the pipeline:
      # `echo psql |` and `oc exec -i pod -- sh` on the next line are one.
      else (if $piped or $sep == "\n" then . else .pipe = null end) end
    | .copt = false | .cskip = false | .hs = false | .hstr = [] | .sh = false | .rw = false;
  # Opens an unquoted substitution: $(...), `...` or a process substitution
  # <(...) or >(...). The current segment and its state are kept aside on
  # .stack, and the text inside starts segments of its own, as a shell runs
  # it as a command of its own. .segn records where its segments start in
  # .segs, for close_sub.
  #   $kind - "sub" for $(...) and <(...), "tick" for `...`.
  def open_sub($kind):
    .stack += [{kind: $kind, segn: (.segs | length), cur, copt, cskip, hs, hstr, sh, rw, pipe}]
    | .cur = [] | .copt = false | .cskip = false | .hs = false | .hstr = [] | .sh = false | .rw = false
    | .pipe = null;
  # Closes the innermost substitution: its last segment ends, and the segment
  # around it goes on with the words after it. So in
  # `oc port-forward $(oc get pod -o name) 5432` the port is still a word of
  # the oc port-forward segment, and in `oc exec $(...) -- psql` psql is
  # still in the command part of the oc exec. When the shell runs its output
  # (see substitution_use), the echo or printf text it prints is split as
  # commands, and for "command" its words also replace the placeholder, as
  # the shell's word splitting puts them there (`$(echo oc) delete ns web`
  # runs `oc delete ns web`).
  def close_sub:
    finish_segment(")")
    | .stack[-1] as $o
    | .stack |= .[:-1]
    | (if $depth >= 6 then null else $o.cur | substitution_use($o.sh) end) as $use
    | (if $use == null then null else .segs[$o.segn:] | printed_text end) as $text
    # (ADR 0036) A process substitution that an oc apply, oc create or oc
    # replace reads as its file (`oc apply -f <(echo '...')`) holds an
    # inline manifest: the echo or printf text it prints becomes a
    # synthetic "manifest" segment (see synthetic_marker).
    | (if $depth >= 6 or ($o.cur[-1] // "") != "/dev/fd/63" then null else $o.cur[:-1] | manifest_oc end) as $manifest
    | (if $manifest == null then .
       else (.segs[$o.segn:] | printed_text) as $m
         | if $m == null then . else .nested += [$m | synthetic_segment("manifest"; $manifest.verb)] end
       end)
    # (ADR 0036) A process substitution that a bare `<` redirects to the
    # command's stdin (`oc exec -i pod -- sh < <(echo '...')`, `oc run -i
    # ... < <(echo ...)`) gives it the echo or printf text it prints, as a
    # here-string would, so finish_segment reads that text as the stdin of
    # oc exec, oc rsh, oc debug, oc run, oc apply, oc create, oc replace,
    # or xargs (a shell reads it as a script, see substitution_use). Its
    # /dev/fd/63 is then a redirection's target, not an argument, so it
    # leaves the words once substitution_use has seen it: `xargs oc <
    # <(echo delete ns web)` runs `oc delete ns web`.
    | (if $depth >= 6 or $o.stdin != true then null else .segs[$o.segn:] | printed_text end) as $stdin_text
    | . + ($o | del(.kind, .segn, .stdin))
    | (if $o.stdin == true and .cur[-1] == "/dev/fd/63" then .cur |= .[:-1] else . end)
    | (if $stdin_text == null then . else .hstr += [$stdin_text] end)
    | if $text == null then .
      else .nested += [$text | script_segments]
        | if $use == "command" then
            [$text | blank_words] as $words
            # After a runner's own words (`timeout 30 $(echo oc) ...`) the
            # placeholder stays in front of the output's words, because that
            # place is read loosely (see is_command_place) and the
            # placeholder may stand for a runner's argument: the checks that
            # count an expansion as not checkable (holds_expansion) still see
            # it there.
            | .cur = (if .cur[:-1] | is_bare_command_place then .cur[:-1] else .cur end) + $words
            | .sh = (.sh or any($words[]; is_shell_word))
            | .rw = (.rw or any($words[]; basename | IN(reparse_words[])))
          else . end
      end;
  # Closes what is still open at the end of a chunk, so that no segment kept
  # aside is lost: an unbalanced text is checked, not dropped.
  def close_all:
    if (.stack | length) == 0 then .
    elif .stack[-1].kind == "group" then .stack |= .[:-1] | close_all
    else close_sub | close_all end;
  # Lexes one chunk. $pipe is the piped echo or printf text that the chunk
  # before passes on, when the two were cut apart inside one pipeline; the
  # result carries the same for the next chunk.
  #   .hstr: the here-strings of the current segment, for finish_segment;
  #   .sh, .rw: whether an earlier word of the current segment is a shell, or
  #   one of reparse_words. They are flags, set as each word is read, so that
  #   a word costs the same however long its segment is (looking back over
  #   the segment for every word made a long segment cost quadratic time);
  #   .dl, .ps: whether the token just read is a word that ends in `$`, or a
  #   bare < or > redirection, so that a `(` right after it opens a $(...) or
  #   a process substitution rather than a subshell;
  #   .lt, .psin (ADR 0036): whether the token just read is a bare `<`, and
  #   whether it followed another one, so that the process substitution of
  #   `< <(...)` is read as the command's stdin (see close_sub);
  #   .gpipe, .braces (ADR 0036): the text that the echo and printf commands
  #   of a subshell group `( ... )` or brace group `{ ...; }` print, set
  #   when the group has just closed, so that a `|` right after it (also
  #   after a redirection) passes it on as piped text, and the starts in
  #   .segs of the open brace groups;
  #   .stack: the open substitutions and subshell groups, innermost last
  #   (see open_sub). An unquoted $(...) leaves its `$` in the word before
  #   it, a `...` leaves the word "`" and a process substitution the path
  #   /dev/fd/63 that bash passes, so the checks see that a shell fills in
  #   text there (see holds_expansion). A subshell group `( ... )` only ends
  #   segments, as before.
  def lex_chunk($pipe):
    reduce scan(token_re) as $t
      ({segs: [], cur: [], nested: [], pipe: $pipe, copt: false, cskip: false, hs: false, hstr: [],
        sh: false, rw: false, dl: false, ps: false, lt: false, psin: false, rd: false, gpipe: null, braces: [], stack: []};
       .dl as $dl | .ps as $ps | .lt as $lt | .psin as $psin | .rd as $rd | .gpipe as $gpipe
       | .dl = false | .ps = false | .lt = false | .psin = false | .rd = false | .gpipe = null
       | ((.stack | last // {}) | .kind) as $open
       | if $t[2] == "(" then
           if $dl then open_sub("sub")
           elif $ps then .cur += ["/dev/fd/63"] | open_sub("sub") | (if $psin then .stack[-1].stdin = true else . end)
           else finish_segment("(") | .stack += [{kind: "group", segn: (.segs | length)}] end
         elif $t[2] == ")" then
           if $open == "sub" then close_sub
           elif $open == "group" then
             finish_segment(")") | .stack[-1].segn as $sn | .stack |= .[:-1] | .gpipe = (.segs[$sn:] | printed_text)
           else finish_segment(")") end
         elif $t[2] == "`" then
           if $open == "tick" then close_sub else .cur += ["`"] | open_sub("tick") end
         elif $t[2] != null then
           finish_segment($t[2]) | (if $gpipe != null and ($t[2] == "|" or $t[2] == "|&") then .pipe = [$gpipe] else . end)
         elif $t[1] != null then
           .hs = ($t[1] | test("<<<$")) | .ps = ($t[1] | test("^[<>]$"))
           | .psin = ($lt and $t[1] == "<") | .lt = ($t[1] == "<") | .rd = true | .gpipe = $gpipe
         elif $t[3] != null then (
         [$t[3] | scan(part_re)] as $parts
         | ($parts | map(.[0] // (.[1] | values | dq_unescape) // .[2] // .[3] // (.[4] | values | ansi_c_text))
            | join("")) as $text
         # (ADR 0036) A `{` in the command name's place opens a brace group
         # and a `}` there closes it; what the group's echo and printf
         # commands print is then what a `|` right after it passes on, as
         # for a subshell group (see .gpipe), also after the target word of
         # a redirection (`( ... ) 2>/dev/null | sh`).
         | (if $rd then .gpipe = $gpipe
            elif (.cur | length) > 0 then .
            elif $text == "{" then .braces += [.segs | length]
            elif $text == "}" and (.braces | length) > 0 then
              .braces[-1] as $sn | .braces |= .[:-1] | .gpipe = (.segs[$sn:] | printed_text)
            else . end)
         # (ADR 0035) One double-quoted $(...) or `...` in the command name's
         # place is the echo or printf output it runs (see
         # quoted_command_text). The cheap tests on the word come first, so
         # that the earlier words are looked at only for such a word. As in
         # close_sub, the word's own text stays in front of that output after
         # a runner's own words (see is_bare_command_place). The output
         # stands only in the segment's words and in the shell and reparse
         # flags: a -c string, a here-string and the text that eval, ssh or
         # watch run again are read as written below, because lexing the
         # written `$(...)` reads both the commands the substitution runs
         # and its output in the command name's place, while the output
         # alone would drop the commands (`eval "$(echo x; oc delete ns
         # web)"` runs both).
         | (if $depth < 6 and ($parts | length) == 1 and $parts[0][1] != null
               and ($parts[0][1] | test("^(?:\\$\\([\\s\\S]*\\)|`[^`]*`)$")) and (.cur | is_command_place)
            then first(quoted_command_text($parts[0][1])) // null
            else null end) as $printed
         | (if $printed == null then [$text]
            elif .cur | is_bare_command_place then [$printed]
            else [$text, $printed] end) as $words
         | .sh as $in_shell
         # .copt: a -c was seen and its command string is still to come;
         # .cskip: the next word is the value of a shell option.
         | (if .copt and .cskip then {cmd: false, copt: true, cskip: false}
            elif .copt and ($text | test("^[-+]")) then
              {cmd: false, copt: true, cskip: ($text | IN(shell_value_options[]))}
            elif .copt then {cmd: true, copt: false, cskip: false}
            else {cmd: false,
                  copt: (($text | test("^-[A-Za-z]*c$")) or $text == "--command"
                         or ($in_shell and ($text | test("^-[A-Za-z]*c[A-Za-z]*$")))),
                  cskip: false} end) as $c
         | ($c.cmd or (.hs and $in_shell) or .rw) as $reparse
         | (if .hs then .hstr += [$text] else . end)
         | .copt = $c.copt | .cskip = $c.cskip | .hs = false
         | .cur += $words
         | .sh = ($in_shell or any($words[]; is_shell_word))
         | .rw = (.rw or any($words[]; basename | IN(reparse_words[])))
         | if $depth >= 6 then .
           elif $reparse then .nested += [$text | select(test("\\S")) | script_segments]
           else .nested += [
             $parts[] | .[1] | values | dq_unescape
             | scan("\\$\\(([\\s\\S]*)\\)|`([^`]*)`") | (.[0] // .[1])
             | select(test("\\S"))
             | script_segments
           ]
           end)
           | .dl = ($t[3] | endswith("$"))
         else . end)
    | close_all
    | (if (.cur | length) == 0 then .pipe else null end) as $carry
    | finish_segment("")
    | {segs: (.segs + .nested), pipe: $carry};
  reduce (gsub("\\\\\\n"; "") | chunks) as $chunk ({segs: [], pipe: null};
    # A chunk that reads piped text from the one before is lexed even without
    # a trigger word of its own (`... psql' |` then ` bash`).
    if $all or .pipe != null or ($chunk | may_matter) then
      .pipe as $p | ($chunk | lex_chunk($p)) as $r | .segs += $r.segs | .pipe = $r.pipe
    # A skipped piece of a pipeline passes the piped text on (see split_pipeline).
    elif ($chunk | test("\\|&?[ \\t]*$")) then .
    else .pipe = null end)
  | .segs;

# lex with the fast path for chunks that cannot matter (see lex/2).
#   $depth - the nesting level, 0 for a whole command.
def lex($depth): lex($depth; false);

# ------------------------------------------------------------------ check 1

# kubectl run as the command: every word before it is an assignment, or one
# of them is a runner word. One pass keeps both facts about the words read so
# far, so a long segment costs linear time.
def check_kubectl:
  first(foreach .[] as $w ({assign: true, runner: false, hit: false};
      .hit = (($w | basename) == "kubectl" and (.assign or .runner))
      | .assign = (.assign and ($w | is_assignment))
      | .runner = (.runner or ($w | IN(runner_words[])));
      select(.hit))
      | msg(1; "this command runs kubectl. Verseghy repositories use oc, never kubectl. "
               + "Run the same command with oc instead, for example `oc get pods -n <namespace>`."
               + text_only_hint));

# Whether a word turns oc's boolean --force on: `--force` alone, or `--force=`
# with a value that Go's strconv.ParseBool reads as true (pflag parses bool
# flags with it).
def truthy_force:
  . == "--force" or (startswith("--force=") and (.[8:] | IN("1", "t", "T", "TRUE", "true", "True")));

# The --grace-period values of a segment, in order (`--grace-period=N` or
# `--grace-period N`); pflag keeps the last one.
def grace_periods:
  . as $s
  | range(0; $s | length) as $i
  | if ($s[$i] | startswith("--grace-period=")) then $s[$i][15:]
    elif $s[$i] == "--grace-period" then ($s[$i + 1] // empty)
    else empty end;

# Force deletion. `oc delete` and `oc replace` document --force as
# "immediately remove resources from API and bypass graceful deletion", and
# kubectl turns --force without a grace period into --grace-period=0
# (k8s.io/kubectl pkg/cmd/delete), so any true --force on them is blocked,
# whatever the grace period. For any other command, a true --force with a zero
# grace period is blocked, as layer 1 does.
def check_force_delete:
  . as $s
  | select(any($s[]; truthy_force))
  | ([$s | grace_periods] | last // null) as $grace
  | select(any($s | oc_verbs; IN("delete", "replace"))
           or ($grace != null and ($grace | test("^[+-]?0+$"))))
  | msg(1; "`--force` force-deletes: on `oc delete` and `oc replace` it removes the resource at once and bypasses graceful deletion "
           + "(kubectl sets the grace period to 0 when --force has none), and with `--grace-period=0` it does so on any command. "
           + "Force-deleting pods is never allowed. Delete the pod normally (`oc delete pod <name>`) and let it terminate; "
           + "if it does not, stop and ask the maintainer.");

# The resource types an agent never deletes: PersistentVolumeClaims,
# PersistentVolumes, Argo CD Applications and ApplicationSets (ADR 0024
# section 4), and namespaces and OpenShift projects (ADR 0028), whose deletion
# deletes every object in them, their claims included. The input is a type
# without its group (`projects` of `projects.project.openshift.io`), in lower
# case.
def protected_kind:
  test("^(pvcs?|persistentvolumeclaims?|pvs?|persistentvolumes?|apps?|applications?|appsets?|applicationsets?|ns|namespaces?|projects?)$");

# The protected kinds, for the block reasons.
def protected_kinds_text:
  "a PersistentVolumeClaim, PersistentVolume, Argo CD Application or ApplicationSet, namespace or project";

# Whether a resource argument (`pvc`, `pods,pvc`, `persistentvolumeclaim.v1`,
# `pvc/data`) names a protected kind in its type part. An empty type part
# (`/x`, `,`, the /dev/fd/63 of `oc delete <(...)`) names none: jq splits ""
# into no pieces, so the `// ""` keeps protected_kind from getting null,
# which would stop the hook with an error instead of a verdict.
def names_protected_kind:
  any(split(",")[] | (split("/")[0] // "") | ascii_downcase | (split(".")[0] // ""); protected_kind);

# Every `oc <verb>` in a segment, as {before, words, args}: the words before
# oc, the words after it, and the positional arguments the verb reads, once
# for each way oc may read them (see oc_verb_indices and oc_verb_words). The
# verb is found first, so only an oc with that verb reads all its arguments.
#   $verb - the oc verb, for example "delete".
def oc_verb_args($verb):
  . as $s
  | range(0; $s | length) as $i
  | select(($s[$i] | basename) == "oc")
  | $s[$i + 1:] as $w
  | $w | oc_verb_indices as $k
  | select($w[$k] == $verb)
  | $w | oc_verb_words($k)
  | {before: $s[:$i], words: $w, args: positionals(oc_value_options)};

# The resource arguments that name a protected kind. Only resource types are
# matched, as kubectl reads them: the first argument after the verb
# (`TYPE[,TYPE...]` or `TYPE/NAME`) and the type part of every later
# `TYPE/NAME` argument. Other words are resource names, so
# `oc delete deployment app` is not an Application.
def protected_resource_args:
  to_entries[]
  | select((.key == 0 or (.value | contains("/"))) and (.value | names_protected_kind))
  | .value;

def protected_delete_advice:
  "Deleting one can destroy data: deleting an Application deletes every resource it manages, "
  + "and deleting a namespace or project deletes everything in it, its PersistentVolumeClaims included, so agents never do it. "
  + "A human does it after checking that the volume's reclaim policy is Retain and that a verified backup exists. Stop and ask the maintainer.";

# oc delete of a protected kind.
def check_oc_delete:
  first(oc_verb_args("delete") | .args | protected_resource_args) as $arg
  | msg(1; "`oc delete` names '\($arg)', \(protected_kinds_text). " + protected_delete_advice);

# oc delete with --raw, in any position (ADR 0035): kubectl then sends an
# HTTP DELETE to the URI it names ("Raw URI to DELETE to the server",
# `oc delete --help`, oc 4.22), and the URI can name any object, a namespace
# or a claim included (`/api/v1/namespaces/web`), without the resource types
# check_oc_delete reads. The hook does not read the URI, so every one is
# blocked; `oc get --raw` only reads and stays allowed.
def check_oc_delete_raw:
  first(oc_verb_args("delete") | .words[] | select(. == "--raw" or startswith("--raw=")))
  | msg(1; "`oc delete --raw` sends a DELETE request to the URI it names, which can name any object "
           + "(\(protected_kinds_text) included), and this hook does not read the URI, so agents never use it. "
           + protected_delete_advice);

# Whether a word holds text that the shell or a runner fills in when it
# runs: a variable, a $(...) or `...` (unquoted, the lexer leaves a `$` or a
# "`" word in its place), the `{}` of xargs -I{}, parallel and find -exec, or
# the replace string of the runner (see runner_feed).
#   $replace - the runner's replace string, or null.
def holds_expansion($replace): test("[$`]|\\{\\}") or ($replace != null and contains($replace));

# Whether an `oc delete` in a segment gets the resources it deletes from the
# output of another command, which the hook cannot read:
#   - it runs under xargs or parallel, which append their input words, and
#     its first resource argument has the TYPE/NAME form, so kubectl accepts
#     the appended TYPE/NAME words too (`xargs oc delete pod/web-0`). With a
#     type written out on its own (`xargs oc delete pod`), kubectl refuses
#     appended TYPE/NAME words, as it does for $(...) below;
#   - it has no resource argument and no -f or -k, so they came from a
#     runner such as xargs (`oc get pvc -o name | xargs oc delete`);
#   - the type part of a word that kubectl reads a type from (the first
#     resource argument and, after a TYPE/NAME one, every other) holds an
#     expansion: `oc delete $(oc get pvc -o name)`, oc delete `oc get pvc
#     -o name`, `oc delete "$r"` in a loop over such output,
#     `xargs -I X oc delete X`;
#   - its -f or --filename reads the output: `-f -` or /dev/stdin from a
#     pipe, the /dev/fd/63 a process substitution `<(...)` leaves, a value
#     that holds an expansion, or no value.
# `oc delete pod "$name"` fixes the type in the text, so it is not generated.
# A file named in the text (`-f pvcs.yaml`) is not visible to the hook.
def generated_delete:
  any(oc_verb_args("delete");
    .args as $args
    | runner_feed(.before; ["oc"] + .words) as $feed
    | ($feed != null and $feed.replace == null and (($args[0] // "") | contains("/")))
      or (($args | length) == 0 and (.words | any(.[]; is_filename_option) | not))
      or any($args | to_entries[] | select(.key == 0 or ($args[0] | contains("/")));
             .value | (split("/")[0] // "") | holds_expansion($feed.replace))
      or any(.words as $w | range(0; $w | length) as $i
             | if $w[$i] == "-f" or $w[$i] == "--filename" then ($w[$i + 1] // "")
               elif ($w[$i] | startswith("--filename=")) then $w[$i][11:]
               elif ($w[$i] | test("^-f.")) then $w[$i][2:] | sub("^="; "")
               else empty end;
             . == "" or . == "-" or startswith("/dev/") or startswith("/proc/") or holds_expansion($feed.replace)));

# oc delete whose resources another command of the same command line
# produces, when that command is an `oc get` of a protected kind:
# `oc delete $(oc get pvc -o name)`, `oc get pvc -o name | xargs oc delete`,
# `oc get pvc -o yaml | oc delete -f -`, `for r in $(oc get pv -o name); do
# oc delete "$r"; done`. The producing oc get is a segment of its own, so the
# segments of the whole command are checked together. The input is the array
# of all segments.
def check_generated_delete:
  . as $segs
  | first($segs[] | oc_verb_args("get") | .args | protected_resource_args) as $kind
  | first($segs[] | select(generated_delete))
  | msg(1; "`oc delete` here gets the resources it deletes from the output of another command ($(...), a variable, xargs or -f -), "
           + "and this command runs `oc get \($kind)`, \(protected_kinds_text), "
           + "so it may delete one. " + protected_delete_advice
           + " If the delete is meant for other resources, run the oc get on its own first, "
           + "then delete those resources with their type written out (`oc delete pod <name>`).");

# argocd's options that take a separate value, so their value is not read as
# a subcommand: the global options of argocd
# (argoproj/argo-cd@17b98d9b6:cmd/argocd/commands/root.go) and the value
# options of the subcommands the checks below look for: `argocd app delete`
# and `argocd app confirm-deletion` (-N, --app-namespace;
# cmd/argocd/commands/app.go), and `argocd app delete-resource` and `argocd
# app patch-resource` (--patch and --patch-type are patch-resource's own;
# cmd/argocd/commands/app_resources.go), and (ADR 0036) `argocd app patch`
# (--patch and --type, NewApplicationPatchCommand in app.go); `argocd appset
# delete` has only the boolean -y (cmd/argocd/commands/applicationset.go).
# The list serves every argocd command line, so a boolean option of any
# subcommand must never be listed: the subcommand after it would be taken
# for its value. No argocd subcommand defines --patch, --patch-type or
# --type as a boolean (--type is a string in `app patch`, `repo add` and
# `repocreds add`).
def argocd_value_options:
  ["--config", "--server", "--server-crt", "--client-crt", "--client-crt-key", "--auth-token",
   "--grpc-web-root-path", "--logformat", "--loglevel", "--header", "-H", "--port-forward-namespace",
   "--http-retry-max", "--argocd-context", "--server-name", "--controller-name",
   "--redis-haproxy-name", "--redis-name", "--repo-server-name", "--redis-compress", "--kube-context",
   "--propagation-policy", "-p", "--selector", "-l", "--app-namespace", "-N",
   "--resource-name", "--kind", "--group", "--namespace", "--project", "--patch", "--patch-type",
   "--type"];

# Every argocd subcommand of $subs that a segment runs, as a stream of
# {command: "<parent> <subcommand>", words: <every word after argocd>}.
# The subcommands are positional words, so options before the parent (`app`,
# `appset`) and after the subcommand cannot hide them: an option missing
# from argocd_value_options there only adds one more positional word.
# Between them, such an option may take the word after it as its value
# (`argocd app --new-option v delete x`; cobra's stripFlags reads an unknown
# `--option` without `=value` that way), so when the first positional word
# after the parent directly follows an option without `=value`, the next
# positional word counts as the subcommand too. A boolean option looks the
# same, so this errs towards blocking. argocd defines no alias for any word
# the checks look for, and cobra matches command names exactly (no prefix
# matching unless a program turns it on). The program counts by its name and
# by the names of its release downloads (see is_argocd_name).
#   $subs - the subcommands to find, as {parent: [subcommands]}.
def argocd_subcommands($subs):
  . as $s
  | range(0; $s | length) as $i
  | select($s[$i] | basename | is_argocd_name)
  | $s[$i + 1:] as $w
  | [$w | positional_indices(argocd_value_options)] as $p
  | range(0; ($p | length) - 1) as $j
  | $w[$p[$j]] as $parent
  | ($subs[$parent] // empty) as $listed
  | $p[$j + 1] as $k
  | $p[$j + 2] as $m
  | (if ($w[$k] | IN($listed[])) then $w[$k]
     elif ($w[$k - 1] | startswith("-") and (contains("=") | not))
          and $m != null and ($w[$m] | IN($listed[])) then $w[$m]
     else empty end) as $sub
  | {command: "\($parent) \($sub)", words: $w};

# The argocd subcommands that are blocked with any options and arguments, as
# {parent: [subcommands]}, with what each one does, for the block reason
# (argoproj/argo-cd@17b98d9b6):
#   - `app delete` (ADR 0028) deletes an Application, and with its default
#     --cascade every resource the Application manages;
#   - `app delete-resource` (D69) deletes resources an Application manages,
#     of any kind its --kind names, a PersistentVolumeClaim included;
#   - `appset delete` (D69) deletes ApplicationSets, and with them all their
#     Applications ("Are you sure you want to delete ... and all its
#     Applications?");
#   - `app confirm-deletion` (D105, ADR 0035) gets the Application, sets its
#     argocd.argoproj.io/deletion-approved annotation to the current time and
#     updates it (NewApplicationConfirmDeletionCommand in app.go). That
#     releases what a Prune=confirm or Delete=confirm sync option holds back
#     until it is confirmed: the prunes and deletions of the resources that
#     carry it (docs/user-guide/sync-options.md).
def argocd_blocked_subcommands: {app: ["delete", "delete-resource", "confirm-deletion"], appset: ["delete"]};

# Advice for a block of Argo CD's deletion approval: `argocd app
# confirm-deletion` and the annotation it writes (see check_deletion_approval).
def deletion_approval_advice:
  "A held deletion is approved by a human, after checking what Argo CD would then delete and that a verified backup exists. "
  + "Stop and ask the maintainer.";

# The block reason for one blocked argocd subcommand.
#   $command - "app delete", "app delete-resource", "app confirm-deletion" or
#              "appset delete".
def argocd_blocked_reason($command):
  if $command == "app delete" then
    "deletes an Argo CD Application through the Argo CD API, as `oc delete application` does. " + protected_delete_advice
  elif $command == "app delete-resource" then
    "deletes resources that an Argo CD Application manages through the Argo CD API, of any kind, "
    + "a PersistentVolumeClaim included, as `oc delete` does, and the hook does not tell one kind from another. "
    + protected_delete_advice
  elif $command == "app confirm-deletion" then
    "sets the argocd.argoproj.io/deletion-approved annotation on an Argo CD Application, which releases the prunes and deletions "
    + "that a Prune=confirm or Delete=confirm sync option holds back until a human confirms them, so agents never run it. "
    + deletion_approval_advice
  else
    "deletes an Argo CD ApplicationSet through the Argo CD API, and with it all its Applications, "
    + "as `oc delete applicationset` does. " + protected_delete_advice
  end;

# A blocked argocd subcommand with any options (ADR 0028, D69, D105): `argocd
# app delete`, `argocd app delete-resource`, `argocd app confirm-deletion` or
# `argocd appset delete` (see argocd_blocked_subcommands and, for how they
# are found, argocd_subcommands).
def check_argocd_blocked:
  first(argocd_subcommands(argocd_blocked_subcommands) | .command) as $command
  | msg(1; "`argocd \($command)` \(argocd_blocked_reason($command))");

# A word of a patch or an annotation as the program that reads it decodes
# it, for the matches of patch_hazard and names_deletion_approval, in lower
# case. kubectl reads a patch as YAML, of which JSON is a part (oc 4.22
# applies `-p` text in either), and both decoders turn an escape in a quoted
# string into its character: JSON's \uXXXX, and YAML's \xXX, \uXXXX and
# \UXXXXXXXX. So `"fin\u0061lizers"` is finalizers and
# `"argocd.argoproj.io/deletion-\x61pproved"` is the annotation's key. Only
# escapes of ASCII characters are decoded, because every word matched is
# ASCII. JSON's `\/` is a `/` (RFC 8259 section 7, ADR 0035): the API
# server's JSON decoder reads it so in a patch that reaches it as written,
# as Argo CD sends patch-resource's --patch (`"path":"\/metadata"` replaces
# the whole metadata) and kubectl a patch that starts with `{`. kubectl's
# YAML decoder (go-yaml v2), which reads every other `-p` patch, refuses
# it, so there it only errs towards blocking. YAML's escaped line break
# inside a double-quoted string (a `\` before a line break: LF, CR LF, CR,
# or the NEL, LS or PS that go-yaml v2 reads as one too; the next line's
# indentation dropped) joins a word across lines, so it is removed with that
# indentation (ADR 0035). The lexer removes a `\` and an LF as a shell line
# continuation before this, also inside single quotes, where the shell keeps
# them, and leaves the next line's indentation in the word, which the
# callers drop (see without_spaces and without_blanks); so this matters for
# the other line breaks and for one that a $'...' part decodes
# (`$'final\\\n  izers'`). Splitting at each backslash keeps the cost linear
# in the length of the word (a gsub would be quadratic, see `chunks`). An
# escaped backslash (`\\u0061`) is decoded as if it were not escaped, which
# errs towards blocking: no word the checks match holds a backslash, so
# decoding more escapes than the decoder does can only add a match.
def decoded_text:
  (if contains("\\") then
     split("\\") as $p
     | $p[0] + ([$p[1:][]
         | if test("^u00[0-7][0-9A-Fa-f]") then (.[3:5] | digits_value(16) | codepoint_text) + .[5:]
           elif test("^x[0-7][0-9A-Fa-f]") then (.[1:3] | digits_value(16) | codepoint_text) + .[3:]
           elif test("^U000000[0-7][0-9A-Fa-f]") then (.[7:9] | digits_value(16) | codepoint_text) + .[9:]
           elif test("^(?:\\r\\n?|[\\n\u0085\u2028\u2029])") then sub("^(?:\\r\\n?|[\\n\u0085\u2028\u2029])[ \\t]*"; "")
           elif startswith("/") then .
           else "\\" + . end] | join(""))
   else . end)
  | ascii_downcase;

# A text with every blank (space, tab, CR, LF) removed, so that a key that a
# YAML escaped line break folds across lines (`final` and `izers` with the
# next line's indentation between them, see decoded_text) is matched whole.
def without_blanks: [blank_words] | join("");

# A text with its spaces and tabs removed and its line breaks kept (ADR
# 0035), for patch_hazard. A key that an escaped line break folds across
# lines is matched whole (`final` and `  izers`), and so is a block YAML
# value in which the lexer has left the next line's indentation after
# removing the escaped line break as a shell line continuation (see
# decoded_text; `path: "/meta    data"`), while the line breaks that end a
# value and start the next key stay. without_blanks would join those lines
# too (`replacepath:`), and no decoder joins words across a line break that
# is not escaped. Plain string splits keep the cost linear.
def without_spaces: split(" ") | join("") | split("\t") | join("");

# YAML node properties that may stand before a value and leave it the same
# string: a tag (`!!str`, `!<tag:yaml.org,2002:str>`) and an anchor (`&a`),
# and a comment, which runs to the end of its line, after which the value
# starts on the next line (`path: # all of it`, then `  /metadata`) (ADR
# 0035). kubectl decodes every patch that does not start with `{` with
# go-yaml v2 (k8s.io/apimachinery util/yaml ToJSON, sigs.k8s.io/yaml
# YAMLToJSON), which accepts them there. A `#` right after the colon of a
# JSON key cannot occur in valid JSON, so it never hides a JSON value.
# go-yaml needs a blank after a tag or an anchor, but patch_hazard also
# matches the patch with its spaces removed (see without_spaces), which
# glues a tag to its value (`path:!!str"/metadata"`), so no blank is
# required here; a tag glued to a value in the text as written
# (`path: !!str/metadata`) is a tag with an empty value for go-yaml, which
# is the path "" of the whole object, so that counts too. A tag or an
# anchor never holds `!`, `&`, `#` or a quote here, so each property starts
# at one of those characters and the pattern has one way to match, which
# keeps it linear.
def yaml_properties_re: "(?:(?:!(?:<[^>]*>|[^\\s,{}\\[\\]!&#\"']*)|&[^\\s,{}\\[\\]!&#\"']+)\\s*|#[^\\n]*\\n\\s*)*";

# A patch, in decoded_text's lower case, that replaces or removes a whole
# object or its whole metadata, which drops its finalizers without naming
# them (ADR 0035):
#   - a JSON patch operation whose path is "" (the whole object) or
#     "/metadata", in JSON or YAML (`"path":"/metadata"`, `path: /metadata`),
#     also after a YAML tag, anchor or comment (see yaml_properties_re);
#     the API server accepts a `replace` there that gives no UID, because it
#     keeps the existing one (k8s.io/apiserver pkg/registry/rest, BeforeUpdate:
#     "Use the existing UID if none is provided");
#   - a strategic merge `$patch: replace` or `$patch: delete` directive, which
#     replaces the map it stands in with the patch or empties it, and
#     `$retainKeys`, which keeps only the keys it lists (k8s.io/apimachinery
#     pkg/util/strategicpatch, handleDirectiveInMergeMap and
#     applyRetainKeysDirective). The hook cannot tell which map a directive
#     stands in, so each one counts, wherever it is;
#   - a `path` or `$patch` key whose YAML value is a block scalar (`|`, `>`)
#     or an alias (`*a`), which go-yaml also reads as a plain string there:
#     the hook does not read those values, so each one counts, which errs
#     towards blocking.
def whole_metadata_patch_re:
  "(?:^|[^a-z0-9_])path[\"']?\\s*:\\s*" + yaml_properties_re
  + "(?:\"(?:/metadata)?\"|'(?:/metadata)?'|/metadata(?=[\\s,}\\]]|$)|[|>*])"
  + "|\\$patch[\"']?\\s*:\\s*" + yaml_properties_re + "(?:[\"']?(?:replace|delete)|[|>*])"
  + "|\\$retainkeys";

# A `path` key whose value is only a YAML tag (`path: !!str`, also after an
# anchor), with nothing after it on its line but a comment, or in a flow
# mapping only a blank before the `,` or `}` (`{path: !!str }`): go-yaml
# reads it as the empty string, the path "" of the whole object (ADR 0035).
# A tag whose value starts on the next line (`path: !!str` then
# `  /metadata`) also counts, which errs towards blocking. It is matched
# only on a patch as written (see patch_hazard): with the spaces removed, a
# tag and the value after it are glued together and look like a tag alone.
def tag_only_path_re:
  "(?:^|[^a-z0-9_])path[\"']?\\s*:[ \\t]*(?:&[^\\s,{}\\[\\]!&#\"']+[ \\t]+)?"
  + "!!?(?:<[^>]*>|[^\\s,{}\\[\\]!&#\"']*)(?:[ \\t]*(?:#[^\\n]*)?(?=[\\r\\n]|$)|[ \\t]+(?=[,}\\]]))";

# What a word of a patch does that agents never do, or nothing (ADR 0028,
# D105, ADR 0035). The word is compared in lower case:
#   - it names finalizers. Without them a resource goes without the
#     protection they hold (kubernetes.io/pvc-protection, Argo CD's resources
#     finalizer). A merge patch that sets the list, a JSON patch `remove` and
#     a strategic merge $deleteFromPrimitiveList each remove some, and the
#     hook cannot tell all of them from an addition, so it matches every
#     patch that names finalizers, additions included;
#   - it sets a PV's reclaim policy to Delete, which deletes the volume with
#     its claim, or (D69) to the deprecated Recycle, which wipes the volume's
#     data (`rm -rf /thevolume/*`) when its claim is released: it names
#     persistentVolumeReclaimPolicy and, in the same word, Delete, Recycle or
#     a shell expansion, whose value cannot be checked. Setting Retain stays
#     allowed. Other values are not matched;
#   - it replaces or removes a whole object or its whole metadata, and with
#     it the finalizers, without naming them (see whole_metadata_patch_re).
# The word is read as the patch decoder reads it (see decoded_text), so an
# escape such as fin\u0061lizers counts too, both as it is and with its
# spaces and tabs removed (see without_spaces): a key folded across lines
# counts only without them, and the whole-metadata rule needs the blank or
# line break that ends a YAML value (`path: /metadata` before the next key),
# which the second reading keeps. A tag with no value counts only as written
# (see tag_only_path_re).
#   $reclaim - whether the reclaim-policy match applies. It does for oc patch
#              and argocd app patch-resource, which can reach a
#              PersistentVolume; `argocd app patch` gets only the finalizer
#              match (D110, ADR 0036): the names and the whole-object or
#              whole-metadata operations, which ADR 0035 counts as finalizer
#              changes. It patches only an Application, which has no reclaim
#              policy.
def patch_hazard($reclaim):
  decoded_text as $written
  | ($written, ($written | without_spaces))
  | if contains("finalizers") then "changes finalizers"
    elif $reclaim and contains("persistentvolumereclaimpolicy") and test("delete|recycle|[$`]") then
      "sets a PersistentVolume's reclaim policy to Delete or Recycle (or to a value only the shell knows)"
    elif test(whole_metadata_patch_re) or (. == $written and test(tag_only_path_re)) then
      "replaces or removes a whole object or its whole metadata (a JSON patch on the path \"\" or \"/metadata\", "
      + "or a strategic merge `$patch: replace`, `$patch: delete` or `$retainKeys`), and with it the finalizers"
    else empty end;

# The whole patch rule, for oc patch and argocd app patch-resource.
def patch_hazard: patch_hazard(true);

# Why a finalizer or reclaim-policy patch is blocked, and what is allowed.
def patch_advice:
  "Without its finalizers a resource is deleted without the protection they hold, "
  + "a PersistentVolume whose reclaim policy is Delete is deleted with its claim, "
  + "and one whose policy is Recycle has its data wiped when its claim is released, so agents never do any of these. "
  + "Setting the reclaim policy to Retain is allowed (`oc patch pv <name> -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Retain\"}}'`). "
  + "For anything else, stop and ask the maintainer." + text_only_hint;

# An `oc patch` that can destroy data (ADR 0028, ADR 0035; see
# patch_hazard). Every word after `oc` is read, because kubectl takes the
# patch from -p, --patch, -p=... or a joined -p..., and a resource name
# rarely holds these words. A patch from --patch-file, or from another
# command's output, is not visible to the hook.
def check_oc_patch:
  first(oc_verb_args("patch") | .words[] | patch_hazard) as $what
  | msg(1; "this `oc patch` \($what). " + patch_advice);

# An `argocd app patch-resource` that can destroy data (D105, ADR 0035): the
# rule of check_oc_patch, read the same way, because patch-resource sends
# its --patch, in any of its three --patch-type formats, to a resource that
# an Application manages, a PersistentVolumeClaim among them
# (NewApplicationPatchResourceCommand in app_resources.go). Every word after
# argocd is read, so the patch counts wherever it stands (`--patch X`,
# `--patch=X`). It is found like the blocked subcommands (see
# argocd_subcommands); a patch-resource whose patch is harmless, and the
# argocd subcommands that only read (`app get`, `app get-resource`, `app
# resources`), stay allowed.
def check_argocd_patch_resource:
  first(argocd_subcommands({app: ["patch-resource"]}) | .words[] | patch_hazard) as $what
  | msg(1; "this `argocd app patch-resource` \($what). It sends the patch through the Argo CD API to a resource that an Application manages, "
           + "and the same patch through `oc patch` is blocked too. " + patch_advice);

# An `argocd app patch` that changes an Application's own finalizers (D110,
# ADR 0036): the finalizer match of the patch rule (see patch_hazard), read
# the same way, in every word after argocd, so the patch counts wherever
# --patch stands and for both --type values (json, merge). The Argo CD
# server applies the patch to the Application and keeps the finalizers of
# the result (server/application/application.go, updateApp: `app.Finalizers
# = newApp.Finalizers`), and they decide what a later deletion of the
# Application takes along: with Argo CD's resources finalizer, every
# resource it manages. An addition counts too, as it does for oc patch. It
# is found like the blocked subcommands (see argocd_subcommands); a patch of
# anything else (`--patch '{"spec":...}'`) stays allowed.
def check_argocd_app_patch:
  first(argocd_subcommands({app: ["patch"]}) | .words[] | patch_hazard(false)) as $what
  | msg(1; "this `argocd app patch` \($what) of an Argo CD Application. An Application's finalizers decide what deleting it takes along: "
           + "with Argo CD's resources finalizer, every resource it manages, its PersistentVolumeClaims included, so agents never change them, "
           + "additions included, and the same patch through `oc patch` is blocked too. Stop and ask the maintainer." + text_only_hint);

# Whether a word names Argo CD's deletion-approved annotation (D105). The key
# is argocd.argoproj.io/deletion-approved; only its last part is matched, in
# lower case, so that a JSON patch path, which writes the / of the key as ~1
# (/metadata/annotations/argocd.argoproj.io~1deletion-approved), counts too.
# The word is read as a patch decoder reads it (see decoded_text), with its
# blanks removed (see without_blanks), so `deletion\u002dapproved` and a key
# folded across lines count as well.
def names_deletion_approval: decoded_text | without_blanks | contains("deletion-approved");

# The runner words after which the next word is surely the program that runs,
# with the later words as its arguments: each reads no positional word of its
# own before the command, and stops reading its own options at the first
# word that is not one (getopt's "+" in sudo, env, ionice, setsid, strace,
# xargs and procps's watch; coreutils' nice; bash's exec, command, builtin,
# eval and time; nohup, zsh's noglob and the shell keywords). A runner that
# permuted its arguments would still run the word right after it, the first
# one that is not its option, so this does not rely on the "+". Left out of
# runner_words, which errs the other way (see is_command_place): ssh and
# rsh, whose first positional word is a host, after which OpenSSH reads
# options again, so `ssh oc -n argocd app patch ...` runs argocd on the host
# oc (rsh is oc's verb too); debug, oc's verb; chroot, flock and timeout,
# whose first positional word is a directory, a lock file or a duration
# (timeout counts with a duration, see is_sure_command_place); stdbuf, which
# needs an option; doas, which is left out until its ports are checked; and
# find's -exec words, which follow find's own words.
def sure_runner_words:
  runner_words - ["ssh", "rsh", "debug", "chroot", "flock", "timeout", "stdbuf", "doas",
                  "-exec", "-execdir", "-ok", "-okdir"];

# Whether a word is a duration as timeout reads it, in its plain form (`30`,
# `1.5m`).
def is_duration: test("^[0-9]+(?:\\.[0-9]+)?[smhd]?$");

# Whether the next word after a list of words is surely the program that the
# segment runs: each word is an assignment, a word of sure_runner_words, or
# `timeout` with a plain duration right after it, or that duration
# (`timeout 30 oc get ...`). This errs towards "not sure": a runner's own
# options (`sudo -u root`, `timeout -s KILL 30`) make it false, because the
# hook does not know which of them take a value, and the word after such an
# option may be its value (`env -u oc -v argocd ...` unsets a variable named
# oc and runs argocd).
def is_sure_command_place:
  . as $b
  | all(range(0; $b | length);
        . as $j
        | ($b[$j] | is_assignment or IN(sure_runner_words[]))
          or ($b[$j] == "timeout" and (($b[$j + 1] // "") | is_duration))
          or ($j > 0 and $b[$j - 1] == "timeout" and ($b[$j] | is_duration)));

# Whether the `oc` at index $oc of a segment surely runs and only reads, so
# that an `argocd` word that is the value of one of its options is surely
# not a program (see check_deletion_approval): the words before it put it
# in the program's place for sure (see is_sure_command_place), and every
# verb that oc may run (see oc_verb_indices) is get or describe. Both are
# needed: with a runner's options before it, the `oc` may be one of their
# values (`sudo -u oc -n argocd app patch ...`), and a verb that runs a
# program, such as `oc run ... -- sudo -n argocd app patch ...`, runs
# argocd in a pod. Each verb reading must be get or describe, because a
# word that one reading takes for the verb may be an option's value in the
# other (`oc --field-manager get run ...` runs `oc run`). No verb at all
# leaves oc printing its help, but counts as not sure all the same.
#   $oc - the index of the `oc` word in the segment.
def oc_surely_reads($oc):
  . as $s
  | ($s[:$oc] | is_sure_command_place)
    and ([$s[$oc + 1:] | oc_verb_indices as $k | $s[$oc + 1 + $k]] as $v
         | ($v | length) > 0 and all($v[]; IN("get", "describe")));

# Argo CD's deletion approval set by other means than `argocd app
# confirm-deletion` (D105, "the annotation spelling"; ADR 0035). Argo CD
# documents the manual way: apply the annotation
# argocd.argoproj.io/deletion-approved to the Application
# (docs/user-guide/sync-options.md). So it blocks:
#   - an `oc annotate` or `oc patch` that names it in any word after `oc`,
#     whatever the resource and whatever the value, its removal (`KEY-`)
#     included, as the patch rule matches finalizer additions too;
#   - any argocd command that names it. `argocd app patch` and `argocd app
#     patch-resource` write it with their --patch, and `argocd app create
#     --upsert` merges its --annotations into an existing Application
#     (server/application/application.go, updateApp with merge). The hook
#     does not tell these from an argocd command that only prints such a
#     key, nor from a command in which `argocd` is only an argument
#     (`grep -rn argocd docs/deletion-approved.md`), which errs towards
#     blocking; the maintainer confirmed this reading as built (D109). It
#     does not look for the subcommand either, because two options that
#     argocd_value_options lacks (`argocd app --repo <url> --path <dir>
#     create`) would hide it from argocd_subcommands. `argocd app set` has
#     no option that sets an annotation. Every `argocd` word counts, after
#     any wrapper and under argocd's download names too (see
#     is_argocd_name), except the value of an option of the segment's first
#     oc when that oc surely runs and only reads (see oc_surely_reads): in
#     `oc get application x -n argocd -o jsonpath=...`, `argocd` is
#     upstream Argo CD's default namespace (or a context named so), not a
#     program.
# Read-only commands stay allowed: `oc get` or `oc describe` of the
# annotation, also in a namespace or context named argocd, and an
# `oc annotate` of any other key, Argo CD's own included
# (`argocd.argoproj.io/refresh=hard`).
def check_deletion_approval:
  . as $s
  | (first(range(0; $s | length) as $j | select(($s[$j] | basename) == "oc") | $j) // null) as $oc
  # Computed once per segment: it does not depend on the argocd word.
  | ($oc != null and ($s | oc_surely_reads($oc))) as $oc_reads
  | first(
      ((oc_verb_args("annotate") | {how: "oc annotate", words}),
       (oc_verb_args("patch") | {how: "oc patch", words}),
       (range(0; $s | length) as $i | select($s[$i] | basename | is_argocd_name)
        | select(($oc_reads and $oc < $i - 1 and ($s[$i - 1] | IN(oc_value_options[]))) | not)
        | {how: "argocd", words: $s[$i + 1:]}))
      | select(any(.words[]; names_deletion_approval)) | .how) as $how
  | msg(1; "this `\($how)` command names Argo CD's argocd.argoproj.io/deletion-approved annotation, the one `argocd app confirm-deletion` writes. "
           + "On an Application it releases the prunes and deletions that a Prune=confirm or Delete=confirm sync option holds back, "
           + "whoever sets it, so agents never set or remove it. "
           + deletion_approval_advice + text_only_hint);

# The annotation in an inline manifest (D110, ADR 0036): a manifest that oc
# apply, oc create or oc replace reads from the command text, as a heredoc
# (also through cat), a here-string, piped echo or printf text, or the echo
# or printf output of a process substitution (see the synthetic "manifest"
# segments of strip_heredocs and lex). Argo CD documents applying the
# annotation as the manual approval, and an apply, create or replace of an
# Application with it sets it as an annotate does. The manifest is read as
# check_deletion_approval reads a word (see names_deletion_approval), so a
# YAML or JSON escape or a key folded across lines counts, and so does a
# manifest that only mentions the key, a `--dry-run=server` one included,
# which errs towards blocking. A manifest in a file, in another command's
# output or in an editor is not read (ADR 0036, "Negative and risks").
def check_manifest_deletion_approval:
  first(synthetic_text("manifest") | select(.text | names_deletion_approval) | .verb) as $verb
  | msg(1; "this `oc \($verb)` reads a manifest from the command text that names Argo CD's argocd.argoproj.io/deletion-approved annotation, "
           + "the one `argocd app confirm-deletion` writes. Applied to an Application, it releases the prunes and deletions "
           + "that a Prune=confirm or Delete=confirm sync option holds back, so agents never apply such a manifest. "
           + "The hook does not tell a manifest that sets the key from one that only mentions it, a --dry-run one included. "
           + deletion_approval_advice + manifest_text_hint);

def check_db_wipe:
  . as $s
  | first(range(0; $s | length) as $i
      | select($s[$i] == "db")
      | $s[$i + 1:] as $rest
      | (($rest | first_positional(["-e", "--environment"])) // "") as $sub
      | select(($sub | IN("reset", "down", "truncate"))
               or ($sub == "seed"
                   and any($rest[]; . == "--reset" or startswith("--reset=") or test("^-[A-Za-z]*r[A-Za-z]*$"))))
      | (if $sub == "seed" then "db seed --reset" else "db \($sub)" end) as $what
      | msg(1; "`\($what)` wipes or rolls back database data. Loco's db reset, db down, db truncate and db seed --reset are blocked "
               + "in every form: cargo loco, the app binary backend3-cli, and inside oc exec, oc rsh or oc debug. "
               + "To change the schema, add a new migration and run `cargo loco db migrate` against the local database. "
               + "For a clean local database, restart the throwaway compose database, whose data lives in a tmpfs "
               + "(`podman-compose down && podman-compose up -d postgres`, which recreates backend3_test, backend3_development and backend3_e2e, then `cargo loco db migrate`)."
               + text_only_hint));

def check_sea_orm_wipe:
  . as $s
  | first(range(0; $s | length) as $i
      | select(($s[$i] | basename) == "sea-orm-cli")
      | $s[$i + 1:] as $rest
      | [limit(2; $rest | positional_indices(["-d", "--migration-dir", "-s", "--database-schema", "-u", "--database-url"]))
         | $rest[.]] as $p
      | select(($p[0] // "") == "migrate" and (($p[1] // "") | IN("fresh", "refresh", "reset", "down")))
      | msg(1; "`sea-orm-cli migrate \($p[1])` drops tables or rolls back migrations. "
               + "Add a new forward migration instead and apply it with `cargo loco db migrate` against the local database."));

# ------------------------------------------------------------------ check 2

# Whether one host value is local: localhost, 127.0.0.1, ::1, a Unix socket
# directory (a path) or empty (the default socket). A value that still holds
# a shell expansion cannot be checked and counts as not local.
def local_host:
  ascii_downcase
  | gsub("%2f"; "/") | gsub("%3a"; ":") | gsub("%5b"; "[") | gsub("%5d"; "]")
  | gsub("^['\"]|['\"]$"; "")
  | if . == "" or startswith("/") then true
    elif test("[$`]") then false
    else (if startswith("[") then ((capture("^\\[(?<h>[^\\]]*)\\]") | .h) // .)
          elif (split(":") | length) > 2 then .
          else sub(":[0-9]*$"; "") end)
         | IN("localhost", "127.0.0.1", "::1")
    end;

# Targets of a host list such as PGHOST or -h ("h1,h2").
#   $from - where the value came from, for the message.
def host_list_targets($from): split(",")[] | {from: $from, host: ., local: local_host};

# Targets of a connection URI: every host of its authority part, every
# ?host= / &hostaddr= query value, and a ?service= value, which cannot be
# checked. A value that is not a URI cannot be checked
# and counts as not local; an empty value means "unset" and yields nothing.
#   $from - where the value came from, for the message.
def uri_targets($from):
  . as $v
  | ((capture("^[A-Za-z][A-Za-z0-9+.-]*://(?<auth>[^/?#]*)(?<rest>[\\s\\S]*)$")) // null) as $m
  | if $m == null then
      (if $v == "" then empty else {from: $from, host: $v, local: false} end)
    else
      ( ( ($m.auth | sub("^.*@"; "") | split(",") | if length == 0 then [""] else . end | .[]),
          ($m.rest | scan("[?&]host(?:addr)?=([^&#]*)") | .[0]) )
        | {from: $from, host: ., local: local_host} ),
      # A service name points at a pg_service.conf entry, which cannot be read.
      ($m.rest | scan("[?&]service=([^&#]*)") | .[0] | {from: $from, host: "service \(.)", local: false})
    end;

# The variables check 2 reads, as they reach one command: from assignments in
# the command text ("inline": `VAR=value cmd`, `export VAR=...`, `env VAR=...`,
# earlier in the same command or in an earlier segment, the latest one
# winning) or else from the hook's own environment. $vars holds the inline
# values of one segment (see `verdict`).

# The targets of one variable's value.
#   $name - the variable; $from - where the value came from, for the message.
def var_value_targets($name; $from):
  if $name | endswith("_URL") then uri_targets($from)
  # A service name points at an entry of a pg_service.conf file, which the
  # hook cannot read.
  elif $name == "PGSERVICE" then (select(. != "") | {from: $from, host: "service \(.)", local: false})
  else host_list_targets($from) end;

# Targets of the inline values among $names.
def inline_var_targets($vars; $names):
  $names[] as $n | ($vars[$n] // empty) | var_value_targets($n; "an inline \($n)= assignment");

# Targets of the environment's value of $name, unless an inline value
# replaces it for this command.
def env_var_targets($vars; $name):
  if $vars[$name] != null then empty
  else ($ENV[$name] // empty) | var_value_targets($name; "the environment's \($name)") end;

# Targets of -h / --host options (psql, pg_restore, mysql).
def host_option_targets:
  . as $a
  | range(0; $a | length) as $i
  | $a[$i] as $w
  | if $w == "-h" or $w == "--host" then ($a[$i + 1] // empty) | host_list_targets("the \($w) option")
    elif ($w | startswith("--host=")) then $w[7:] | host_list_targets("the --host option")
    elif ($w | test("^-h.")) then $w[2:] | host_list_targets("the -h option")
    else empty end;

# A -d / --dbname value (psql, pg_restore) that is a shell expansion cannot be
# checked. URIs and host= keys inside it are read by connection_string_targets.
def dbname_option_targets:
  . as $a
  | range(0; $a | length) as $i
  | $a[$i] as $w
  | (if $w == "-d" or $w == "--dbname" then ($a[$i + 1] // empty)
     elif ($w | startswith("--dbname=")) then $w[9:]
     elif ($w | test("^-d.")) then $w[2:]
     else empty end)
  | select(test("[$`]"))
  | {from: "the -d/--dbname option", host: ., local: false};

# Targets of sea-orm-cli's -u / --database-url option.
def database_url_option_targets:
  . as $a
  | range(0; $a | length) as $i
  | $a[$i] as $w
  | if $w == "-u" or $w == "--database-url" then ($a[$i + 1] // empty) | uri_targets("the \($w) option")
    elif ($w | startswith("--database-url=")) then $w[15:] | uri_targets("the --database-url option")
    elif ($w | test("^-u.")) then $w[2:] | uri_targets("the -u option")
    else empty end;

# Targets written into the arguments: connection URIs (in -d, --dbname, -u,
# --database-url or positional) and host= / hostaddr= / service= connection
# keys. libpq allows spaces around the = of a key/value connection string.
def connection_string_targets:
  .[]
  | (scan("(?:postgres|postgresql|mysql|mariadb)://[^\\s'\"]*") | uri_targets("a connection URI")),
    (scan("(?:^|[\\s?&=])host(?:addr)?\\s*=\\s*('[^']*'|[^\\s&']*)") | .[0] | host_list_targets("a host= connection key")),
    (scan("(?:^|[\\s?&])service\\s*=\\s*('[^']*'|[^\\s&']*)") | .[0]
     | {from: "a service= connection key", host: "service \(.)", local: false});

# Every database-touching command in a segment, as {kind, name, args}.
# `cargo run`, `cargo r` and `cargo loco-tool` count, because `cargo loco` is
# an alias of `cargo run --` (.cargo/config.toml), `loco-tool` is the same
# alias there and `r` is cargo's built-in alias of `run`, so all of them run
# the same backend3-cli; `cargo t` is cargo's built-in alias of `cargo test`.
# The maintainer accepted this reading (D69, ADR 0028 section 4).
# MariaDB's client `mariadb` (ADR 0028) takes the same host options and
# reads the same MYSQL_HOST as mysql, so it is checked as kind "mysql".
def db_commands:
  . as $s
  | range(0; $s | length) as $i
  | ($s[$i] | basename) as $b
  | if ($b | IN("psql", "pg_restore", "mysql", "sea-orm-cli")) then {kind: $b, name: $b, args: $s[$i + 1:]}
    elif $b == "mariadb" then {kind: "mysql", name: $b, args: $s[$i + 1:]}
    elif $b == "cargo" then
      (($s[$i + 1:] | cargo_subcommand) // "") as $sub
      | if ($sub | IN("loco", "loco-tool", "test", "t", "nextest", "run", "r"))
        then {kind: "cargo", name: "cargo \($sub)", args: $s[$i + 1:]}
        else empty end
    else empty end;

# Every Redis client in a segment, as {kind, name, args} like db_commands:
# a program of redis_client_names (redis-cli since D105, ADR 0035; the
# others since D110, ADR 0036). Each one sends any Redis command, FLUSHALL
# included, so in the cluster it is blocked whatever its options and
# arguments, `redis-cli ping` and a benchmark too. None is one of
# db_commands, because those also get the target check on the agent's own
# host, and the answers to D105 and D110 keep a Redis client there allowed:
# the clusters' Redis answers on this host only through a port-forward,
# which check_port_forward blocks.
def redis_clients:
  . as $s
  | range(0; $s | length) as $i
  | ($s[$i] | basename) as $b
  | select($b | IN(redis_client_names[]))
  | {kind: "redis", name: $b, args: $s[$i + 1:]};

# The Redis client (see redis_client_names) that a text names, or empty
# (D110, ADR 0036): for an inline manifest, an `oc run --overrides` JSON,
# which set a pod's command in fields the hook does not parse (`"command":
# ["redis-cli", "FLUSHALL"]`, `args: [sh, -c, "redis-cli flushall"]`), and
# the text that oc run passes to its pod, whatever program reads it
# (`subprocess.run(["redis-cli", "FLUSHALL"])`). A name counts as a word:
# not after a letter, digit, `_`, `.` or `-`, and not before a letter,
# digit, `_` or `-`, so `/usr/local/bin/redis-cli` and `"redis-cli"` count,
# and `my-redis-cli` and a pod `redis-cli-0` do not. Each of three readings
# of the text counts:
#   - the text as a patch decoder reads it (see decoded_text, which also
#     lower-cases it), so a YAML or JSON escape counts;
#   - the same with every blank removed (see without_blanks), because a
#     YAML escaped line break in a double-quoted scalar folds a name across
#     lines, and the lexer leaves the next line's indentation inside it
#     (`"redis-\`, then `   cli"`, see decoded_text);
#   - the text with its quotes and backslashes removed (see unquoted_texts),
#     in lower case, because a shell in the pod removes them from a script
#     that a manifest or the JSON holds (`"redis-c'l'i FLUSHALL"`,
#     `redis-c\"l\"i`), as the hook reads a `sh -c` string.
# A text that only mentions a client (a ConfigMap, a file name such as
# `redis-cli.log`) counts too, which errs towards blocking.
def redis_client_in_text:
  first((decoded_text | (., without_blanks)), (first(unquoted_texts) | ascii_downcase)
        | capture("(?:^|[^a-z0-9_.-])(?<c>" + (redis_client_names | join("|")) + ")(?![a-z0-9_-])")
        | .c);

# Every database target of one command (sources 1 to 4 of ADR 0024 section 4,
# check 2). Explicit sources are always checked: every inline variable the
# command kind reads, every option and every connection string, even where the
# program would prefer another one, so a local value can never hide a remote
# one. The environment's value of a variable is checked unless the command
# does not use it:
#   - an inline value of the same variable replaces it;
#   - cargo commands (and sea-orm-cli without -u) connect to DATABASE_URL;
#     sqlx falls back to PGHOSTADDR or PGHOST only when that URI names no
#     host, or when no URI is known (the configuration then decides, and
#     layer 3 checks it, so a remote PGHOST is refused to be safe);
#   - psql and pg_restore use PGHOST unless -h, a host= key or a URI host is
#     given, but libpq connects to PGHOSTADDR even then, and a PGSERVICE entry
#     cannot be checked at all; they do not read DATABASE_URL, which is
#     checked only when nothing explicit is present;
#   - mysql uses MYSQL_HOST unless -h or a URI host is given; the PostgreSQL
#     variables are checked only when nothing explicit is present.
#   $c    - the command, from db_commands;
#   $vars - the inline values of the watched variables, by name.
def db_targets($c; $vars):
  $c.kind as $k
  | [ if $k == "cargo" then empty
      elif $k == "sea-orm-cli" then ($c.args | database_url_option_targets)
      else ($c.args | host_option_targets, dbname_option_targets, connection_string_targets) end ] as $options
  | [ inline_var_targets($vars;
        ["DATABASE_URL", "PGHOST", "PGHOSTADDR"]
        + (if $k == "mysql" then ["MYSQL_HOST"] else [] end)
        + (if $k == "psql" or $k == "pg_restore" then ["PGSERVICE"] else [] end)) ] as $inline
  | ((($options | length) + ($inline | length)) > 0) as $explicit
  | $options[], $inline[],
    if $k == "cargo" or $k == "sea-orm-cli" then
      # The URI the command connects to: -u, else DATABASE_URL.
      (if ($options | length) > 0 then $options
       elif $vars.DATABASE_URL != null then [$vars.DATABASE_URL | uri_targets("")]
       else [env_var_targets($vars; "DATABASE_URL")] end) as $uri
      | (if ($options | length) == 0 then $uri[] else empty end
         | select($vars.DATABASE_URL == null)),
        (if any($uri[]; .host != "") then empty
         else env_var_targets($vars; "PGHOSTADDR"), env_var_targets($vars; "PGHOST") end)
    elif $k == "mysql" then
      (if any($options[]; .host != "") then empty else env_var_targets($vars; "MYSQL_HOST") end),
      (if $explicit then empty
       else env_var_targets($vars; "DATABASE_URL"), env_var_targets($vars; "PGHOSTADDR"),
            env_var_targets($vars; "PGHOST") end)
    else
      (if any($options[]; .host != "") then empty else env_var_targets($vars; "PGHOST") end),
      env_var_targets($vars; "PGHOSTADDR"),
      env_var_targets($vars; "PGSERVICE"),
      (if $explicit then empty else env_var_targets($vars; "DATABASE_URL") end)
    end;

def local_db_advice:
  " Agents only touch local databases: localhost, 127.0.0.1, ::1 or a Unix socket. "
  + "Use the throwaway compose database (`podman-compose up -d postgres`, "
  + "postgres://loco:loco@localhost:5432/backend3_development); any other database is operated by a human.";

# Whether a word of an in-cluster command part can name the program that
# runs. A path under /etc, /var, /run, /proc, /sys, /dev or /usr/share counts
# only in a bin or sbin directory: those trees hold data, configuration and
# sockets, such as MySQL's data directory /var/lib/mysql, which `ls` or `du`
# name as files. Every other word counts, so a client run by any other path
# (`/opt/bitnami/postgresql/bin/psql`, `./psql`) is still found.
def may_name_program:
  (test("^/(?:etc|var|run|proc|sys|dev|usr/share)/") | not) or test("/s?bin/[^/]*$");

# A database command, or (D105) a Redis client (see redis_clients), run
# through oc exec, oc rsh or oc debug. The words are
# stripped of quotes and backslashes and cut at blanks, so an oc inside a
# quoted command string (`parallel 'oc exec {} -- psql'`) and a client written
# in quotes inside a script (`sh -c '"psql" ...'`) count too. When a word holds
# a `$'` or a `$"`, the segment is read a second time with that word's
# $'...' and $"..." parts read as bash reads them (see unquoted_texts), and
# either reading counts (ADR 0035). Only the command
# part of the first oc exec, rsh or debug counts (see in_cluster_command), in
# each way oc may read its verb and the words that verb parses (see
# oc_verb_indices and oc_verb_words, ADR 0035), so a pod, container or
# namespace named like a client does not; that command part runs to the end
# of the segment, so it holds every later one. After a
# shell word in the command part, the words are also cut at shell
# separators, so a client inside a `sh -c` script counts after a `;` without
# a space (`sh -c 'cd tmp;psql'`). Elsewhere they are not, because ( ) | < >
# in a search pattern or a message are text: `grep -E "oc (exec|rsh) .* psql"`
# names no verb, and `rg "oc exec [^ ]+ -- (psql|mysql)"` no client.
#
# (D110, ADR 0036) A Redis client is also blocked as the command of a pod
# that the agent starts: in the command part of `oc run` and of `oc create
# job|cronjob|deployment|deploymentconfig` (see in_cluster_command and
# in_cluster_parts), and in the text that oc run passes to its pod (the
# synthetic "pod-stdin" segments of strip_heredocs and lex, read here as
# `oc run stdin -- sh -c '<text>'`, see in_cluster_script). Only the Redis
# clients count there: the answer names them, and a database command in
# such a pod keeps the target check that check_db_target gives every
# command word (`oc run ... -- psql -h postgres` is blocked there). Each
# family takes the first oc that has one of its verbs: the command part of
# that oc runs to the end of the segment, so it holds every later one, and
# an oc run before an oc exec cannot keep the exec's part from counting
# whole (`oc run x --image=y -- oc exec pod -- psql`).
#   $s - the words of a segment.
def check_in_cluster:
  # The readings of the first oc in the word list $f that has one of
  # $verbs, as a list of {verb, part} (see in_cluster_parts), or [] when no
  # oc has one. An oc create of any other type has no part, so the search
  # goes on to the next oc.
  #   $verbs - the oc verbs of one family.
  def readings($verbs):
    . as $f
    | first(range(0; $f | length) as $i
        | select(($f[$i] | basename) == "oc")
        | $f[$i + 1:] as $rest
        | [$rest | oc_verb_indices | select($rest[.] | IN($verbs[])) as $v
           | $rest | oc_verb_words($v) | in_cluster_parts($rest[$v])]
        | select(length > 0)) // [];
  ((synthetic_text("pod-stdin") | .text | in_cluster_script("run")) // .) as $s
  | first(
      (([$s[] | first(unquoted_texts) | blank_words],
        (select(any($s[]; test("\\$['\"]"))) | [$s[] | last(unquoted_texts) | blank_words]))
       | (readings(["exec", "rsh", "debug"])[] | .pod = false),
         (readings(["run", "create"])[] | .pod = true)),
      # The pod-start verbs also read the words as the lexer gives them,
      # with only their command part cut at blanks: their options often
      # hold a value with blanks (`--schedule '*/5 * * * *'`, `--env 'A=b
      # c'`), which the blank-split readings above cut into words, so the
      # option's value would take a word of its value's and the rest would
      # pass for the subcommand or the pod's name.
      ($s | readings(["run", "create"])[] | .pod = true | .part |= [.[] | first(unquoted_texts) | blank_words])
      | .verb as $verb | .pod as $pod | .part
      | (first(to_entries[] | select(.value | is_shell_word) | .key) // null) as $shell
      | (if $shell == null then .
         else .[:$shell + 1] + [.[$shell + 1:][] | splits("[;&|()`<>]+") | select(length > 0)] end)
      | first(map(select(may_name_program)) | ((if $pod then empty else db_commands end), redis_clients))
      | {verb: $verb, name, kind, pod: $pod}) as $c
  | if $c.pod then
      msg(2; "`\($c.name)` runs in the cluster as the command of the pod that `oc \($c.verb)` starts, so it reaches the Redis there "
             + "with any command, FLUSHALL included, whatever its options say. Agents never run a Redis client in the cluster, "
             + "in a pod they start included (`redis-cli ping` and a benchmark too); stop and ask the maintainer. "
             + "A Redis client on this host is not blocked."
             + text_only_hint)
    elif $c.kind == "redis" then
      msg(2; "`\($c.name)` runs inside the cluster through `oc \($c.verb)`, so it reaches the Redis there with any command, FLUSHALL included, "
             + "whatever its options say. Agents never run a Redis client in the cluster, `redis-cli ping` included; stop and ask the maintainer. "
             + "A Redis client on this host is not blocked."
             + text_only_hint)
    else
      msg(2; "`\($c.name)` runs inside the cluster through `oc \($c.verb)`, so its database is not local, whatever its options say. "
             + "Agents never run database commands in the cluster; stop and ask the maintainer."
             + text_only_hint)
    end;

# The values of the --overrides options in a list of words (`--overrides X`
# or `--overrides=X`), the JSON that `oc run` merges into the pod it makes.
def overrides_values:
  . as $a
  | range(0; $a | length) as $i
  | if $a[$i] == "--overrides" then ($a[$i + 1] // empty)
    elif ($a[$i] | startswith("--overrides=")) then $a[$i][12:]
    else empty end;

# A Redis client set as a pod's command or arguments in the --overrides JSON
# of `oc run` (D110, ADR 0036; `--overrides='{"spec":{"containers":[{...,
# "command":["redis-cli","FLUSHALL"]}]}}'`), found by its name in the JSON
# (see redis_client_in_text). Every word of the oc run counts, options
# before the verb included, as oc reads them (see oc_verb_args).
def check_run_overrides:
  first(oc_verb_args("run") | .words | overrides_values | redis_client_in_text) as $name
  | msg(2; "this `oc run` names `\($name)`, a Redis client, in its --overrides, which sets the command of the pod it starts, "
           + "so the client would reach the Redis in the cluster with any command, FLUSHALL included. "
           + "Agents never run a Redis client in the cluster; stop and ask the maintainer. A Redis client on this host is not blocked.");

# A Redis client named in the text that `oc run` passes to the pod it starts
# (D110, ADR 0036): a heredoc, a here-string, piped echo or printf text or
# `< <(echo ...)` (see the synthetic "pod-stdin" segments of strip_heredocs
# and lex), and a heredoc whose cat output fills in its --overrides (`oc run
# ... --overrides="$(cat <<'EOF'`, whose receiving line names oc run). The
# text is found by name (see redis_client_in_text), whatever program the pod
# runs, so a list (`["redis-cli", "FLUSHALL"]` in a Python script) or JSON
# without blanks counts too, besides the shell words that check_in_cluster
# reads in it.
def check_pod_stdin_redis_client:
  first(synthetic_text("pod-stdin") | .text | redis_client_in_text) as $name
  | msg(2; "the text that this `oc run` passes to the pod it starts names `\($name)`, a Redis client, which would reach the Redis in the cluster "
           + "with any command, FLUSHALL included. The hook does not tell a text that runs it from one that only mentions it. "
           + "Agents never run a Redis client in the cluster, in a pod they start included; stop and ask the maintainer. "
           + "A Redis client on this host is not blocked." + manifest_text_hint);

# A Redis client in an inline manifest (D110, ADR 0036): a manifest that oc
# apply, oc create or oc replace reads from the command text (see
# check_manifest_deletion_approval for the forms), found by its name (see
# redis_client_in_text), so a Pod, Job or Deployment that runs one is
# blocked, and so is a manifest that only mentions one, a `--dry-run=server`
# one included, which errs towards blocking.
def check_manifest_redis_client:
  first(synthetic_text("manifest") | {verb, name: (.text | redis_client_in_text)}) as $c
  | msg(2; "this `oc \($c.verb)` reads a manifest from the command text that names `\($c.name)`, a Redis client, "
           + "so a pod it describes would run the client in the cluster, where it reaches the Redis with any command, FLUSHALL included. "
           + "The hook does not tell a manifest that runs it from one that only mentions it, a --dry-run one included. "
           + "Agents never run a Redis client in the cluster; stop and ask the maintainer. A Redis client on this host is not blocked."
           + manifest_text_hint);

# Whether the remote part of an `oc port-forward` port spec
# ([LOCAL_PORT:]REMOTE_PORT) reaches a database: PostgreSQL's 5432, MySQL's
# and MariaDB's 3306, MySQL's X protocol port 33060, Redis's 6379 and Redis
# Sentinel's 26379 (D69), or a named port that contains postgres, mariadb,
# sql, redis or sentinel (so `postgres`, `postgresql`, `pgsql`, `mysql`,
# `mysqlx`, `redis`, `tcp-redis` and `tcp-sentinel` count) or is db.
# Sentinel counts because its SENTINEL SET, REMOVE and FAILOVER commands
# reconfigure the Redis servers it monitors (Redis docs, "High availability
# with Redis Sentinel"). A number may carry a leading `+` (ADR 0035): for a
# Service, kubectl reads the remote port with Go's strconv.Atoi, which
# accepts the sign, and forwards the plain number
# (translateServicePortToTargetPort, k8s.io/kubectl pkg/cmd/portforward), so
# `15432:+5432` forwards PostgreSQL.
def database_port:
  if test("^\\+?[0-9]+$") then ltrimstr("+") | tonumber | IN(5432, 3306, 33060, 6379, 26379)
  else ascii_downcase | test("postgres|mariadb|sql|redis|sentinel|^db$") end;

# An `oc port-forward` to a database port (ADR 0028; Redis since D69). oc
# listens on localhost by default (--address), so the database in the cluster
# then answers on localhost, and every client this check counts as local would
# reach it. No Redis client is among the listed commands (see db_commands),
# and a redis-cli on the agent's own host is not checked (see redis_clients),
# so a forwarded Redis would be reached with no check at all; the
# port-forward is the one point where the hook sees it. A port spec counts
# when its remote part is a database port (see database_port), or holds a
# shell expansion or the replace string of xargs or parallel, which cannot be
# checked (see holds_expansion); and when xargs or parallel append their
# input words, the ports cannot be seen at all.
# The first positional word after the verb is the target (TYPE/NAME or a
# pod); the rest are port specs. An empty port spec has an empty remote
# part, which is no database port. A database served on another port is not
# recognised.
def check_port_forward:
  first(oc_verb_args("port-forward")
    | runner_feed(.before; ["oc"] + .words) as $feed
    | if $feed != null and $feed.replace == null then "the ports that xargs or parallel append"
      else first(.args[1:][] | select(split(":") | last // "" | holds_expansion($feed.replace) or database_port)) | "'\(.)'"
      end) as $port
  | msg(2; "`oc port-forward` forwards \($port), "
           + "which is a PostgreSQL, MySQL, MariaDB or Redis port or cannot be checked. A port-forward makes the database or Redis in the cluster answer on localhost, "
           + "where this check would take a database for a local one and does not check a Redis client on this host at all. "
           + "Agents never connect to a database or Redis in the cluster; stop and ask the maintainer."
           + text_only_hint);

#   $vars - the inline values of the watched variables, by name.
def check_db_target($vars):
  first(db_commands as $c
    | [db_targets($c; $vars)] | map(select(.local | not)) | .[0] // empty
    | msg(2; "`\($c.name)` would use the database target '\(.host)' (from \(.from)), which is not local or cannot be checked."
             + local_db_advice));

#   $vars - the inline values of the watched variables, by name.
def check_import_source($vars):
  first(db_commands | select(.kind == "cargo") as $c
    | [inline_var_targets($vars; ["IMPORT_SOURCE_DATABASE_URL"]),
       env_var_targets($vars; "IMPORT_SOURCE_DATABASE_URL")]
    | map(select(.local | not)) | .[0] // empty
    | msg(2; "`\($c.name)` would read the legacy import source '\(.host)' (from \(.from)), which is not local or cannot be checked. "
             + "Outside the cut-over the importer reads only a local throwaway MySQL database (data-migration.md section 4.1); "
             + "the production import is run by the operator."));

# ------------------------------------------------------------------ verdict

# Pairs every segment with the inline values of the watched variables in
# scope for it: assignments anywhere in that segment or in an earlier one
# (nested command strings come after the text they came from), the latest per
# name. A fixed number of names per segment keeps the work linear in the
# number of segments, however many assignments the command holds.
def scoped_segments:
  reduce .[] as $s ({vars: {}, out: []};
    .vars += ([$s[] | capture("^(?<name>DATABASE_URL|PGHOST|PGHOSTADDR|PGSERVICE|MYSQL_HOST|IMPORT_SOURCE_DATABASE_URL)=(?<value>[\\s\\S]*)$")]
              | map({(.name): .value}) | add // {})
    | .out += [{seg: $s, vars: .vars}])
  | .out;

def verdict:
  (.tool_input.command // "" | tostring) as $cmd
  | if (.tool_name // "Bash") != "Bash" then "allow"
    elif ($cmd | may_matter | not) then "allow"
    else
      # lex cuts a long command into chunks and skips those that cannot
      # matter, which keeps long commands cheap.
      [$cmd | gsub("\\\\\\n"; "") | heredoc_texts | lex(0)[]] as $segs
      | (first(($segs | scoped_segments[] | .vars as $vars | .seg
                | (check_kubectl, check_force_delete, check_oc_delete, check_oc_delete_raw, check_argocd_blocked, check_oc_patch,
                   check_argocd_patch_resource, check_argocd_app_patch, check_deletion_approval,
                   check_manifest_deletion_approval, check_db_wipe, check_sea_orm_wipe,
                   check_in_cluster, check_run_overrides, check_pod_stdin_redis_client, check_manifest_redis_client, check_port_forward,
                   check_db_target($vars), check_import_source($vars))),
               ($segs | check_generated_delete)) // null) as $why
      | if $why == null then "allow" else "block\n\($why)" end
    end;

verdict
JQ

# jq reads the hook input from this script's stdin. A parse error or an empty
# input fails closed. The program goes in as a file that a process
# substitution fills, not as an argument: Linux limits one argument to 128
# KiB (MAX_ARG_STRLEN), and the commented program is longer than that (ADR
# 0036), so `jq -r "$GUARD_JQ"` would fail with "Argument list too long" and
# block every command. printf is a bash builtin, so no exec limit applies
# to it, and the substitution is a pipe, so unlike a here-string of this
# size it needs no temporary file. A /dev/fd path that cannot be opened
# makes jq fail, which blocks too.
if ! result="$(jq -r -f <(printf '%s' "$GUARD_JQ") 2>&1)"; then
  block "Blocked by .claude/hooks/guard-bash.sh: could not read the hook input as JSON, so the command was blocked to be safe. jq said: ${result}"
fi

case "$result" in
  allow)
    exit 0
    ;;
  block$'\n'*)
    block "${result#block$'\n'}"
    ;;
  *)
    block "Blocked by .claude/hooks/guard-bash.sh: the policy returned no verdict (output: '${result}'), so the command was blocked to be safe. Run \"bash scripts/tests/test-guard-bash.sh\" from the repository root to find the fault."
    ;;
esac
