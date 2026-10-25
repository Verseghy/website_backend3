#!/usr/bin/env bash
# guard-bash.sh: the PreToolUse hook that screens every Bash command an agent
# runs in this repository (layer 2 of the guardrails, ADR 0024 section 4 as
# amended by ADR 0028).
#
# Why it exists
#   Layer 1, the deny rules in .claude/settings.json, matches command text as
#   written. It misses the same command behind a wrapper (`sh -c '...'`), with
#   another flag order, or as the app binary instead of `cargo loco`. This hook
#   reads the whole command, splits it the way a shell would, and blocks:
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
#       delete`, `oc get pvc -o yaml | oc delete -f -`;
#     - `argocd app delete`, `argocd app delete-resource` and `argocd appset
#       delete`, with any options (ADR 0028; the last two since D69);
#     - an `oc patch` whose patch names finalizers, or names a PV's
#       persistentVolumeReclaimPolicy with Delete, Recycle (since D69) or a
#       value only the shell knows (ADR 0028);
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
#     same way. Any listed command run through oc exec, oc rsh or oc debug
#     (also as a heredoc, a here-string or an echo or printf script piped into
#     them, through any further pipeline stages) runs in the cluster and is
#     always blocked. Only the command part counts there: the words after `--`
#     for exec and debug; for rsh the words after the pod, the program its
#     --shell names, and with -f every word. So a pod, container or namespace
#     named like a client (`oc exec deploy/mysql -- df -h`) is not blocked by
#     its name, nor is a data path such as /var/lib/mysql. A pod looked up
#     with an unquoted $(...) or `...` does not end the command part
#     (`oc exec $(oc get pod -o name) -- psql` is blocked). It also blocks an
#     `oc port-forward` to a PostgreSQL, MySQL, MariaDB or (since D69) Redis
#     port (ADR 0028), because the forwarded database then answers on
#     localhost, which this check would otherwise take for a local one. No
#     Redis client is a listed command, so a forwarded Redis would answer on
#     localhost with no check at all.
#
#   Layer 3, the load_config guard and the CLI pre-check in the Rust code, is
#   the one that cannot be bypassed from a shell. This hook is a second line of
#   defence against agent mistakes, not a security boundary: commands that come
#   from files or scripts are not visible to it (`oc exec -i pod -- sh <
#   script.sh`, `oc delete -f list.txt`, `oc patch --patch-file p.json`), and
#   neither is text that only a shell run computes, because the hook reads the
#   command and does not run it: a variable's value (`P=psql; oc exec pod --
#   $P`), printf formats and echo -e escapes, or decoded output (`base64 -d`).
#   Nor does it block what no answer has put in its scope yet (ADR 0028,
#   "Negative and risks"): a Redis client run in the cluster, `argocd app
#   patch-resource` and `argocd app confirm-deletion`.
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
#     bash and jq. jq's regular expressions do the tokenising in C.
#   - Shell parsing is approximated, deliberately erring towards blocking. A
#     word is parsed again as a command only where a shell would run it (the
#     command string of -c, also after `--` or other options; the arguments of
#     eval, ssh and watch; a here-string read by a shell; echo or printf
#     output piped into a shell or into oc exec, oc rsh or oc debug; and
#     $(...) inside double quotes), and a
#     heredoc body counts as a script only when a shell reads it (the
#     receiver is matched with quotes and backslashes removed, and a variable
#     as the receiver counts as a shell), so commit messages and files that
#     merely mention a command are not blocked. The cases it handles are
#     pinned by scripts/tests/test-guard-bash.sh, which must pass before this
#     file changes (it runs in the CI "Test" job).
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
  "Blocked by .claude/hooks/guard-bash.sh (ADR 0024 section 4 as amended by ADR 0028, check \($check)): \($text)\n"
  + "The guardrails are described in docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md, section 4, "
  + "and docs/design/adr/0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md.";

# Advice for a block that may be a false positive caused by text that only
# mentions a command, for example in a commit message.
def text_only_hint:
  " If the words only appear as text (a commit message, a PR body, a search), "
  + "write the text to a file and pass the file (git commit -F <file>, gh pr create --body-file <file>), "
  + "or use the Grep tool for searches.";

# ------------------------------------------------------------------ helpers

def basename: sub("^.*/"; "");

def is_assignment: test("^[A-Za-z_][A-Za-z0-9_]*=");

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
# global options of `oc options` (oc 4.22) and the value options of delete,
# replace, exec, rsh, debug and port-forward. A boolean option must never be
# listed: the word after it would be skipped, and a pod taken for its value
# would let the next word, the command, pass for the pod in check_in_cluster.
# That is why patch's -p is not listed: it is the boolean --previous of
# `oc logs -p`. check_oc_patch reads every word of an oc patch instead.
def oc_value_options:
  ["-n", "--namespace", "-l", "--selector", "-f", "--filename", "-o", "--output",
   "--field-selector", "--context", "--cluster", "--kubeconfig", "--kuberc", "--server", "-s",
   "--token", "--user", "--as", "--as-group", "--as-uid", "--as-user-extra", "-k", "--kustomize",
   "--timeout", "--grace-period", "--request-timeout", "--template", "--raw",
   "--certificate-authority", "--client-certificate", "--client-key", "--cache-dir",
   "--tls-server-name", "--loglevel", "-v", "--v", "--log-flush-frequency",
   "--profile", "--profile-output", "-c", "--container", "--shell",
   "--pod-running-timeout", "--image", "--image-stream", "--node-name", "--to-namespace",
   "--as-user", "--address"];

# The first `oc exec`, `oc rsh` or `oc debug` in a list of words, as
# {verb, i} with the index i of its `oc`, or null. Global options may stand
# between oc and the verb.
def in_cluster_oc:
  . as $s
  | first(range(0; $s | length) as $i
      | select(($s[$i] | basename) == "oc")
      | ($s[$i + 1:] | first_positional(oc_value_options)) as $verb
      | select($verb | IN("exec", "rsh", "debug"))
      | {verb: $verb, i: $i}) // null;

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
# inside the cluster. The input is the words after the verb. Only these words
# can start a program there, so a pod, container or namespace named like a
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
#   $verb - exec, rsh or debug.
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
    elif $dash != null then $a[$dash + 1:]
    elif $t != null then $a[$t + 1:]
    else [] end;

# The synthetic segment for text that oc exec, oc rsh or oc debug reads on
# stdin, which runs in the cluster: `oc <verb> stdin -- sh -c '<text>'`. The
# word "stdin" stands in for the target, so that the command part (see
# in_cluster_command) holds `sh -c '<text>'` for every verb, rsh included.
#   $verb - exec, rsh or debug.
def in_cluster_script($verb): ["oc", $verb, "stdin", "--", "sh", "-c", .];

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
# (see lex_chunk).
# Quotes and backslashes are removed from each word the way the shell joins
# them. A quoted string that the shell would run as a command (after `-c`,
# `--command`, `eval`, `ssh` or `watch`, and every $(...) or `...` inside
# double quotes) is split again into segments of its own, recursively.

# One token per match. Capture groups, in order: [comment, redirection,
# separator, word]. Quotes and escapes stay inside the word for word_text.
def token_re:
  "(#[^\\n]*)"
  + "|((?:[0-9]+|&)?[<>]+&?[0-9-]*)"
  + "|(&&|\\|\\||;;|\\|&|[;&|()`\\n])"
  + "|((?:'[^']*'|\"(?:\\\\[\\s\\S]|[^\"\\\\])*\"|\\\\[\\s\\S]|[^\\s'\"\\\\;&|()<>`#])"
  + "(?:'[^']*'|\"(?:\\\\[\\s\\S]|[^\"\\\\])*\"|\\\\[\\s\\S]|[^\\s'\"\\\\;&|()<>`])*)";

# The parts of one word. Capture groups: [single-quoted content,
# double-quoted content, escaped character, plain text].
def part_re: "'([^']*)'|\"((?:\\\\[\\s\\S]|[^\"\\\\])*)\"|\\\\([\\s\\S])|([^'\"\\\\]+)";

# Removes the backslashes that escape $ ` " \ inside double quotes.
def dq_unescape: gsub("\\\\(?<c>[$`\"\\\\])"; .c);

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

# The text with every quote and backslash removed: the words a shell would
# see, joined, which is close enough for the receiver tests above.
def unquoted_text: split("\"") | join("") | split("'") | join("") | split("\\") | join("");

# Quotes a text as one single-quoted shell word.
def sq_quote: "'" + gsub("'"; "'\\''") + "'";

# Removes heredoc bodies that are data, so that a commit message or a file
# written with `cat <<'EOF'` cannot trip check 1. A body is kept when a shell
# reads it, and only its $(...) and `...` are kept when the delimiter is
# unquoted (the shell expands those even in data). A body that oc exec,
# oc rsh or oc debug reads is kept as the -c script of a synthetic
# `oc <verb> stdin -- sh -c '<body>'` line (see in_cluster_script), so that
# check 2 sees the database commands in it next to the oc that runs them in
# the cluster. lex does the same for here-strings and piped echo or printf
# text.
def strip_heredocs:
  gsub("(?<pre>^[^\\n<]*)<<-?[ \\t]*(?<q>['\"]?)(?<d>[A-Za-z0-9_.-]+)\\k<q>(?<rest>[^\\n]*)\\n(?:(?<body>[\\s\\S]*?)\\n)?[ \\t]*\\k<d>[ \\t]*(?=\\n|$)";
    (.pre + " " + .rest) as $line
    | ($line | unquoted_text) as $plain
    | (.body // "") as $body
    | $line + "\n"
      + if ($plain | test(in_cluster_receiver_re)) then
          "oc \($plain | capture(in_cluster_receiver_re).verb) stdin -- sh -c \($body | sq_quote)"
        elif ($plain | test(shell_receiver_re)) or ($plain | test(var_receiver_re)) then $body
        elif .q == "" then ([$body | scan("\\$\\([\\s\\S]*?\\)|`[^`]*`")] | join("\n"))
        else "" end);

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
# other command is allowed without being parsed.
def trigger_re: "kubectl|grace-period|\\boc\\b|\\bdb\\b|psql|pg_restore|mysql|mariadb|sea-orm-cli|cargo|argocd";

# The variables check 2 reads.
def watched_var_re: "DATABASE_URL|PGHOST|PGSERVICE|MYSQL_HOST";

# Whether a piece of shell text can matter to any check: it holds a trigger
# word, or an assignment that check 2 reads. Quotes and backslashes are
# dropped first, so k'ube'ctl still counts. split/join is linear, unlike a
# gsub with one match per quote. lex skips the chunks for which this is false,
# because no check can block a segment without such a word.
def may_matter:
  split("\"") | join("") | split("'") | join("") | split("\\") | join("")
  | test(trigger_re + "|" + watched_var_re);

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
#   - and, in any other word, every $(...) or `...` inside double quotes.
# A long text is cut into chunks first (see `chunks`), and chunks that cannot
# matter (see may_matter) are not lexed.
#   $depth - the nesting level; nesting stops at 6 to bound the work.
def lex($depth):
  # Ends the current segment at separator $sep and resets the per-segment
  # state. When the segment reads text on stdin, from an echo or printf
  # earlier in its pipeline or from its here-strings, that text is handled as
  # described above.
  def finish_segment($sep):
    ($sep == "|" or $sep == "|&") as $piped
    | if (.cur | length) > 0 then
        # Only a segment that reads such text needs to find its oc.
        (if .pipe != null or (.hstr | length) > 0 then .cur | in_cluster_oc else null end) as $oc
        # printf's \n and \t, with plain string splits (a gsub is quadratic).
        | [(.pipe // [])[] | split("\\n") | join("\n") | split("\\t") | join("\t")] as $piped_in
        # echo joins its arguments with spaces; each here-string is a line.
        | ([($piped_in | join(" ")), .hstr[]] | map(select(test("\\S"))) | join("\n")) as $stdin
        | .cur as $cur
        | (if $depth >= 6 or $stdin == "" then .
           elif $oc != null then
             runner_feed($cur[:$oc.i]; $cur[$oc.i:]) as $feed
             | if $feed == null then
                 .nested += [$stdin | in_cluster_script($oc.verb)] + ($stdin | lex($depth + 1))
               elif $feed.replace != null then
                 .nested += [$stdin | split("\n")[] | select(test("\\S")) | sub("^[ \\t]+"; "") as $line
                             | $cur[:$oc.i] + [$cur[$oc.i:][] | split($feed.replace) | join($line)]]
               else .nested += [$cur + [$stdin | blank_words]] end
           elif .pipe != null and .sh then
             .nested += [$piped_in[] | select(test("\\S")) | lex($depth + 1)[]]
           else . end)
        | (first(.cur[] | select(is_assignment | not)) // "" | basename) as $command
        | .pipe = (if $piped then (if ($command | IN("echo", "printf")) then .cur[1:] else .pipe end) else null end)
        | .segs += [.cur] | .cur = []
      # A line break right after a separator does not end the pipeline:
      # `echo psql |` and `oc exec -i pod -- sh` on the next line are one.
      else (if $piped or $sep == "\n" then . else .pipe = null end) end
    | .copt = false | .cskip = false | .hs = false | .hstr = [] | .sh = false | .rw = false;
  # Opens an unquoted substitution: $(...), `...` or a process substitution
  # <(...) or >(...). The current segment and its state are kept aside on
  # .stack, and the text inside starts segments of its own, as a shell runs
  # it as a command of its own.
  #   $kind - "sub" for $(...) and <(...), "tick" for `...`.
  def open_sub($kind):
    .stack += [{kind: $kind, cur, copt, cskip, hs, hstr, sh, rw, pipe}]
    | .cur = [] | .copt = false | .cskip = false | .hs = false | .hstr = [] | .sh = false | .rw = false
    | .pipe = null;
  # Closes the innermost substitution: its last segment ends, and the segment
  # around it goes on with the words after it. So in
  # `oc port-forward $(oc get pod -o name) 5432` the port is still a word of
  # the oc port-forward segment, and in `oc exec $(...) -- psql` psql is
  # still in the command part of the oc exec.
  def close_sub:
    finish_segment(")") | .stack[-1] as $o | .stack |= .[:-1] | . + ($o | del(.kind));
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
  #   .stack: the open substitutions and subshell groups, innermost last
  #   (see open_sub). An unquoted $(...) leaves its `$` in the word before
  #   it, a `...` leaves the word "`" and a process substitution the path
  #   /dev/fd/63 that bash passes, so the checks see that a shell fills in
  #   text there (see holds_expansion). A subshell group `( ... )` only ends
  #   segments, as before.
  def lex_chunk($pipe):
    reduce scan(token_re) as $t
      ({segs: [], cur: [], nested: [], pipe: $pipe, copt: false, cskip: false, hs: false, hstr: [],
        sh: false, rw: false, dl: false, ps: false, stack: []};
       .dl as $dl | .ps as $ps | .dl = false | .ps = false
       | ((.stack | last // {}) | .kind) as $open
       | if $t[2] == "(" then
           if $dl then open_sub("sub")
           elif $ps then .cur += ["/dev/fd/63"] | open_sub("sub")
           else finish_segment("(") | .stack += [{kind: "group"}] end
         elif $t[2] == ")" then
           if $open == "sub" then close_sub
           elif $open == "group" then finish_segment(")") | .stack |= .[:-1]
           else finish_segment(")") end
         elif $t[2] == "`" then
           if $open == "tick" then close_sub else .cur += ["`"] | open_sub("tick") end
         elif $t[2] != null then finish_segment($t[2])
         elif $t[1] != null then .hs = ($t[1] | test("<<<$")) | .ps = ($t[1] | test("^[<>]$"))
         elif $t[3] != null then (
         [$t[3] | scan(part_re)] as $parts
         | ($parts | map(.[0] // (.[1] | values | dq_unescape) // .[2] // .[3]) | join("")) as $text
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
         | .cur += [$text]
         | .sh = ($in_shell or ($text | is_shell_word))
         | .rw = (.rw or ($text | basename | IN(reparse_words[])))
         | if $depth >= 6 then .
           elif $reparse then .nested += [$text | select(test("\\S")) | lex($depth + 1)[]]
           else .nested += [
             $parts[] | .[1] | values | dq_unescape
             | scan("\\$\\(([\\s\\S]*)\\)|`([^`]*)`") | (.[0] // .[1])
             | select(test("\\S"))
             | lex($depth + 1)[]
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
    if .pipe != null or ($chunk | may_matter) then
      .pipe as $p | ($chunk | lex_chunk($p)) as $r | .segs += $r.segs | .pipe = $r.pipe
    # A skipped piece of a pipeline passes the piped text on (see split_pipeline).
    elif ($chunk | test("\\|&?[ \\t]*$")) then .
    else .pipe = null end)
  | .segs;

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

# The verb of the first oc command in a segment, or "".
def oc_verb:
  . as $s
  | first(range(0; $s | length) as $i
      | select(($s[$i] | basename) == "oc")
      | ($s[$i + 1:] | first_positional(oc_value_options)) // "") // "";

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
  | select(($s | oc_verb | IN("delete", "replace"))
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
# `pvc/data`) names a protected kind in its type part.
def names_protected_kind:
  any(split(",")[] | split("/")[0] | ascii_downcase | split(".")[0]; protected_kind);

# Every `oc <verb>` in a segment, as {before, words, args}: the words before
# oc, the words after it, and the positional arguments after the verb. The
# verb is found first, so only an oc with that verb reads all its arguments.
#   $verb - the oc verb, for example "delete".
def oc_verb_args($verb):
  . as $s
  | range(0; $s | length) as $i
  | select(($s[$i] | basename) == "oc")
  | $s[$i + 1:] as $w
  | ($w | first_positional_index(oc_value_options)) as $k
  | select($k != null and $w[$k] == $verb)
  | {before: $s[:$i], words: $w, args: ($w[$k + 1:] | positionals(oc_value_options))};

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
             .value | split("/")[0] | holds_expansion($feed.replace))
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
# options of `argocd app delete` (cmd/argocd/commands/app.go) and of `argocd
# app delete-resource` (cmd/argocd/commands/app_resources.go); `argocd appset
# delete` has only the boolean -y (cmd/argocd/commands/applicationset.go). A
# boolean option must never be listed.
def argocd_value_options:
  ["--config", "--server", "--server-crt", "--client-crt", "--client-crt-key", "--auth-token",
   "--grpc-web-root-path", "--logformat", "--loglevel", "--header", "-H", "--port-forward-namespace",
   "--http-retry-max", "--argocd-context", "--server-name", "--controller-name",
   "--redis-haproxy-name", "--redis-name", "--repo-server-name", "--redis-compress", "--kube-context",
   "--propagation-policy", "-p", "--selector", "-l", "--app-namespace", "-N",
   "--resource-name", "--kind", "--group", "--namespace", "--project"];

# The argocd subcommands that delete, as {parent: [subcommands]}, with what
# each one deletes, for the block reason (argoproj/argo-cd@17b98d9b6):
#   - `app delete` (ADR 0028) deletes an Application, and with its default
#     --cascade every resource the Application manages;
#   - `app delete-resource` (D69) deletes resources an Application manages,
#     of any kind its --kind names, a PersistentVolumeClaim included;
#   - `appset delete` (D69) deletes ApplicationSets, and with them all their
#     Applications ("Are you sure you want to delete ... and all its
#     Applications?").
# argocd defines no alias for any of these words, and cobra matches command
# names exactly (no prefix matching unless a program turns it on).
def argocd_deleting_subcommands: {app: ["delete", "delete-resource"], appset: ["delete"]};

# The block reason for one deleting argocd subcommand.
#   $command - "app delete", "app delete-resource" or "appset delete".
def argocd_delete_what($command):
  if $command == "app delete" then
    "deletes an Argo CD Application through the Argo CD API, as `oc delete application` does"
  elif $command == "app delete-resource" then
    "deletes resources that an Argo CD Application manages through the Argo CD API, of any kind, "
    + "a PersistentVolumeClaim included, as `oc delete` does, and the hook does not tell one kind from another"
  else
    "deletes an Argo CD ApplicationSet through the Argo CD API, and with it all its Applications, "
    + "as `oc delete applicationset` does"
  end;

# A deleting argocd subcommand with any options (ADR 0028, D69): `argocd app
# delete`, `argocd app delete-resource` or `argocd appset delete` (see
# argocd_deleting_subcommands). The subcommands are positional words, so
# options before the parent (`app`, `appset`) and after the subcommand cannot
# hide them: an option missing from argocd_value_options there only adds one
# more positional word. Between them, such an option may take the word after
# it as its value (`argocd app --new-option v delete x`; cobra's stripFlags
# reads an unknown `--option` without `=value` that way), so when the first
# positional word after the parent directly follows an option without
# `=value`, the next positional word counts as the subcommand too. A boolean
# option looks the same, so this errs towards blocking.
def check_argocd_delete:
  . as $s
  | argocd_deleting_subcommands as $subs
  | first(range(0; $s | length) as $i
      | select(($s[$i] | basename) == "argocd")
      | $s[$i + 1:] as $w
      | [$w | positional_indices(argocd_value_options)] as $p
      | range(0; ($p | length) - 1) as $j
      | $w[$p[$j]] as $parent
      | ($subs[$parent] // empty) as $deleting
      | $p[$j + 1] as $k
      | $p[$j + 2] as $m
      | if ($w[$k] | IN($deleting[])) then "\($parent) \($w[$k])"
        elif ($w[$k - 1] | startswith("-") and (contains("=") | not))
             and $m != null and ($w[$m] | IN($deleting[])) then "\($parent) \($w[$m])"
        else empty end) as $command
  | msg(1; "`argocd \($command)` \(argocd_delete_what($command)). " + protected_delete_advice);

# An `oc patch` that can destroy data (ADR 0028):
#   - one that removes finalizers. Without them a resource goes without the
#     protection they hold (kubernetes.io/pvc-protection, Argo CD's resources
#     finalizer). A merge patch that sets the list, a JSON patch `remove` and
#     a strategic merge $deleteFromPrimitiveList each remove some, and the
#     hook cannot tell all of them from an addition, so it matches every
#     patch that names finalizers, additions included;
#   - one that sets a PV's reclaim policy to Delete, which deletes the volume
#     with its claim, or (D69) to the deprecated Recycle, which wipes the
#     volume's data (`rm -rf /thevolume/*`) when its claim is released: its
#     patch names persistentVolumeReclaimPolicy and, in the same word, Delete,
#     Recycle or a shell expansion, whose value cannot be checked. Setting
#     Retain stays allowed. Other values are not matched.
# Every word after `oc` is read, in lower case, because kubectl takes the
# patch from -p, --patch, -p=... or a joined -p..., and a resource name
# rarely holds these words. A patch from --patch-file, or from another
# command's output, is not visible to the hook.
def check_oc_patch:
  first(oc_verb_args("patch") | .words[] | ascii_downcase
    | if contains("finalizers") then "changes finalizers"
      elif contains("persistentvolumereclaimpolicy") and test("delete|recycle|[$`]") then
        "sets a PersistentVolume's reclaim policy to Delete or Recycle (or to a value only the shell knows)"
      else empty end) as $what
  | msg(1; "this `oc patch` \($what). Without its finalizers a resource is deleted without the protection they hold, "
           + "a PersistentVolume whose reclaim policy is Delete is deleted with its claim, "
           + "and one whose policy is Recycle has its data wiped when its claim is released, so agents never do any of these. "
           + "Setting the reclaim policy to Retain is allowed (`oc patch pv <name> -p '{\"spec\":{\"persistentVolumeReclaimPolicy\":\"Retain\"}}'`). "
           + "For anything else, stop and ask the maintainer." + text_only_hint);

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

# A database command run through oc exec, oc rsh or oc debug. The words are
# stripped of quotes and backslashes and cut at blanks, so an oc inside a
# quoted command string (`parallel 'oc exec {} -- psql'`) and a client written
# in quotes inside a script (`sh -c '"psql" ...'`) count too. Only the command
# part of the first oc exec, rsh or debug counts (see in_cluster_command), so
# a pod, container or namespace named like a client does not; that command
# part runs to the end of the segment, so it holds every later one. After a
# shell word in the command part, the words are also cut at shell
# separators, so a client inside a `sh -c` script counts after a `;` without
# a space (`sh -c 'cd tmp;psql'`). Elsewhere they are not, because ( ) | < >
# in a search pattern or a message are text: `grep -E "oc (exec|rsh) .* psql"`
# names no verb, and `rg "oc exec [^ ]+ -- (psql|mysql)"` no client.
def check_in_cluster:
  [.[] | unquoted_text | blank_words] as $f
  | first(range(0; $f | length) as $i
      | select(($f[$i] | basename) == "oc")
      | $f[$i + 1:] as $rest
      | ($rest | first_positional_index(oc_value_options)) as $v
      | select($v != null and ($rest[$v] | IN("exec", "rsh", "debug")))
      | {verb: $rest[$v], part: ($rest[$v + 1:] | in_cluster_command($rest[$v]))}) as $oc
  | (first($oc.part | to_entries[] | select(.value | is_shell_word) | .key) // null) as $shell
  | (if $shell == null then $oc.part
     else $oc.part[:$shell + 1]
          + [$oc.part[$shell + 1:][] | splits("[;&|()`<>]+") | select(length > 0)] end) as $words
  | first($words | map(select(may_name_program)) | db_commands) as $c
  | msg(2; "`\($c.name)` runs inside the cluster through `oc \($oc.verb)`, so its database is not local, whatever its options say. "
           + "Agents never run database commands in the cluster; stop and ask the maintainer."
           + text_only_hint);

# Whether the remote part of an `oc port-forward` port spec
# ([LOCAL_PORT:]REMOTE_PORT) reaches a database: PostgreSQL's 5432, MySQL's
# and MariaDB's 3306, MySQL's X protocol port 33060, Redis's 6379 and Redis
# Sentinel's 26379 (D69), or a named port that contains postgres, mariadb,
# sql, redis or sentinel (so `postgres`, `postgresql`, `pgsql`, `mysql`,
# `mysqlx`, `redis`, `tcp-redis` and `tcp-sentinel` count) or is db.
# Sentinel counts because its SENTINEL SET, REMOVE and FAILOVER commands
# reconfigure the Redis servers it monitors (Redis docs, "High availability
# with Redis Sentinel").
def database_port:
  if test("^[0-9]+$") then tonumber | IN(5432, 3306, 33060, 6379, 26379)
  else ascii_downcase | test("postgres|mariadb|sql|redis|sentinel|^db$") end;

# An `oc port-forward` to a database port (ADR 0028; Redis since D69). oc
# listens on localhost by default (--address), so the database in the cluster
# then answers on localhost, and every client this check counts as local would
# reach it. No Redis client is among the listed commands (see db_commands), so
# a forwarded Redis would be reached with no check at all; the port-forward is
# the one point where the hook sees it. A port spec counts when its remote
# part is a database port (see database_port), or holds a shell expansion or
# the replace string of xargs or parallel, which cannot be checked (see
# holds_expansion); and when xargs or parallel append their input words, the
# ports cannot be seen at all.
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
           + "where this check would take a database for a local one and does not check Redis clients at all. "
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
      [$cmd | gsub("\\\\\\n"; "") | (if test("<<") then strip_heredocs else . end) | lex(0)[]] as $segs
      | (first(($segs | scoped_segments[] | .vars as $vars | .seg
                | (check_kubectl, check_force_delete, check_oc_delete, check_argocd_delete, check_oc_patch,
                   check_db_wipe, check_sea_orm_wipe,
                   check_in_cluster, check_port_forward, check_db_target($vars), check_import_source($vars))),
               ($segs | check_generated_delete)) // null) as $why
      | if $why == null then "allow" else "block\n\($why)" end
    end;

verdict
JQ

# jq reads the hook input from this script's stdin. A parse error or an empty
# input fails closed.
if ! result="$(jq -r "$GUARD_JQ" 2>&1)"; then
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
