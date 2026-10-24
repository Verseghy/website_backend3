#!/usr/bin/env bash
# guard-bash.sh: the PreToolUse hook that screens every Bash command an agent
# runs in this repository (layer 2 of the guardrails, ADR 0024 section 4).
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
#     - oc delete naming a PVC, PV or Argo CD Application in any spelling
#       (ApplicationSets too, because deleting one deletes its Applications);
#     - Loco's data-wiping subcommands `db reset`, `db down`, `db truncate`
#       and `db seed --reset` / `-r` of any binary: cargo loco, the app binary
#       backend3-cli, or inside oc exec / oc rsh / oc debug;
#     - sea-orm-cli `migrate fresh|refresh|reset|down`.
#   check 2, the database target of database-touching commands (cargo loco,
#     cargo run, cargo test, cargo nextest, psql, pg_restore, mysql,
#     sea-orm-cli): it blocks when the target host is not local (localhost,
#     127.0.0.1, ::1 or a Unix socket) or cannot be checked, and for cargo
#     commands it checks the importer source IMPORT_SOURCE_DATABASE_URL the
#     same way. Any listed command run through oc exec, oc rsh or oc debug
#     (also as a heredoc, here-string or piped script) runs in the cluster and
#     is always blocked.
#
#   Layer 3, the load_config guard and the CLI pre-check in the Rust code, is
#   the one that cannot be bypassed from a shell. This hook is a second line of
#   defence against agent mistakes, not a security boundary: commands that come
#   from files or scripts are not visible to it.
#
# Contract with Claude Code (verified against the "Hooks reference" of the
# Claude Code docs, 2026-10-24)
#   - stdin is one JSON object; the Bash command is at .tool_input.command and
#     the tool name at .tool_name;
#   - exit 0 lets the command run;
#   - exit 2 blocks it, and stderr is shown to the agent as the reason, so every
#     message says what was refused and what to do instead;
#   - any other exit code is a NON-blocking error and the command would still
#     run. That is why every failure path below exits 2 (fail closed): a
#     missing jq, unreadable input, or an unexpected error in this script.
#   .claude/settings.json runs it as "$CLAUDE_PROJECT_DIR"/.claude/hooks/guard-bash.sh
#   (shell form; Claude Code exports CLAUDE_PROJECT_DIR to the hook process).
#
# Design notes
#   - It runs before every Bash call, so it must be fast: it starts one jq
#     process, and jq returns "allow" at once for any command that contains
#     neither a trigger word (kubectl, oc, db, cargo, psql, ...) nor one of the
#     variables check 2 reads. Typical commands take a few tens of
#     milliseconds; long ones are cut into chunks first (see `chunks`), and
#     each segment sees only the assignments in scope for it, so the work
#     grows linearly with the length of the command.
#   - The policy is a jq program embedded below, so the hook depends only on
#     bash and jq. jq's regular expressions do the tokenising in C.
#   - Shell parsing is approximated, deliberately erring towards blocking. A
#     word is parsed again as a command only where a shell would run it (the
#     command string of -c, also after `--` or other options; the arguments of
#     eval, ssh and watch; a here-string read by a shell; echo or printf
#     output piped into a shell; and $(...) inside double quotes), and a
#     heredoc body counts as a script only when a shell reads it (the
#     receiver is matched with quotes and backslashes removed, and a variable
#     as the receiver counts as a shell), so commit messages and files that
#     merely mention a command are not blocked. The cases it handles are
#     pinned by scripts/tests/test-guard-bash.sh, which must pass before this
#     file changes (it runs in the CI "Test" job).
#   - Where ADR 0024 leaves room, it is stricter, and each point needs the
#     maintainer's acceptance or an amending ADR (ADR 0001 section 6):
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
#         deletes its Applications.
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
  "Blocked by .claude/hooks/guard-bash.sh (ADR 0024 section 4, check \($check)): \($text)\n"
  + "The guardrails are described in docs/design/adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md, section 4.";

# Advice for a block that may be a false positive caused by text that only
# mentions a command, for example in a commit message.
def text_only_hint:
  " If the words only appear as text (a commit message, a PR body, a search), "
  + "write the text to a file and pass the file (git commit -F <file>, gh pr create --body-file <file>), "
  + "or use the Grep tool for searches.";

# ------------------------------------------------------------------ lexing
#
# The command is split into segments (simple commands) of words. Separators
# (; & && || | |& ( ) ` and newlines) end a segment. Redirections are dropped.
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
# oc rsh or oc debug feeds to a shell is kept as the -c script of a synthetic
# `oc <verb> -- sh -c '<body>'` line, so that check 2 sees the database
# commands in it next to the oc that runs them in the cluster.
def strip_heredocs:
  gsub("(?<pre>^[^\\n<]*)<<-?[ \\t]*(?<q>['\"]?)(?<d>[A-Za-z0-9_.-]+)\\k<q>(?<rest>[^\\n]*)\\n(?:(?<body>[\\s\\S]*?)\\n)?[ \\t]*\\k<d>[ \\t]*(?=\\n|$)";
    (.pre + " " + .rest) as $line
    | ($line | unquoted_text) as $plain
    | (.body // "") as $body
    | $line + "\n"
      + if ($plain | test(in_cluster_receiver_re)) then
          "oc \($plain | capture(in_cluster_receiver_re).verb) -- sh -c \($body | sq_quote)"
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

# jq's scan, match and gsub recompute every match's offset from the start of
# the string, so their cost grows with (matches x length). A long command is
# therefore cut into chunks before it is lexed: at newlines first, and a piece
# that is still long at ; then && then || then |. Every one of those is a
# segment separator, so cutting there outside quotes changes no segment.
# Commands up to 16 KiB are not cut.
def chunks:
  def refine($seps):
    if length <= 16384 or ($seps | length) == 0 then .
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
#     (`echo '...' | sh`), with printf's \n and \t turned into whitespace;
#   - and, in any other word, every $(...) or `...` inside double quotes.
#   $depth - the nesting level; nesting stops at 6 to bound the work.
def lex($depth):
  # Ends the current segment at separator $sep and resets the per-segment
  # state. When the segment runs a shell and an earlier stage of its pipeline
  # was an echo or printf, those arguments are split as commands.
  def finish_segment($sep):
    ($sep == "|" or $sep == "|&") as $piped
    | if (.cur | length) > 0 then
        (if $depth < 6 and .pipe != null and any(.cur[]; is_shell_word) then
           .nested += [.pipe[] | gsub("\\\\n"; "\n") | gsub("\\\\t"; "\t")
                       | select(test("\\S")) | lex($depth + 1)[]]
         else . end)
        | (first(.cur[] | select(test("^[A-Za-z_][A-Za-z0-9_]*=") | not)) // "" | sub("^.*/"; "")) as $command
        | .pipe = (if $piped then (if ($command | IN("echo", "printf")) then .cur[1:] else .pipe end) else null end)
        | .segs += [.cur] | .cur = []
      else (if $piped then . else .pipe = null end) end
    | .copt = false | .cskip = false | .hs = false;
  reduce (gsub("\\\\\\n"; "") | scan(token_re)) as $t
    ({segs: [], cur: [], nested: [], pipe: null, copt: false, cskip: false, hs: false};
     if $t[2] != null then finish_segment($t[2])
     elif $t[1] != null then .hs = ($t[1] | test("<<<$"))
     elif $t[3] != null then
       [$t[3] | scan(part_re)] as $parts
       | ($parts | map(.[0] // (.[1] | values | dq_unescape) // .[2] // .[3]) | join("")) as $text
       | (any(.cur[]; is_shell_word)) as $in_shell
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
       | ($c.cmd or (.hs and $in_shell) or any(.cur[]; sub("^.*/"; "") | IN(reparse_words[]))) as $reparse
       | .copt = $c.copt | .cskip = $c.cskip | .hs = false
       | .cur += [$text]
       | if $depth >= 6 then .
         elif $reparse then .nested += [$text | select(test("\\S")) | lex($depth + 1)[]]
         else .nested += [
           $parts[] | .[1] | values | dq_unescape
           | scan("\\$\\(([\\s\\S]*)\\)|`([^`]*)`") | (.[0] // .[1])
           | select(test("\\S"))
           | lex($depth + 1)[]
         ]
         end
     else . end)
  | finish_segment("")
  | .segs + .nested;

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

# oc options that take a separate value, so their value is not read as a
# resource or a verb (for example the namespace in `-n apps`): the global
# options of `oc options` (oc 4.22) and the value options of delete, replace,
# exec, rsh and debug.
def oc_value_options:
  ["-n", "--namespace", "-l", "--selector", "-f", "--filename", "-o", "--output",
   "--field-selector", "--context", "--cluster", "--kubeconfig", "--kuberc", "--server", "-s",
   "--token", "--user", "--as", "--as-group", "--as-uid", "--as-user-extra", "-k", "--kustomize",
   "--timeout", "--grace-period", "--request-timeout", "--template", "--raw",
   "--certificate-authority", "--client-certificate", "--client-key", "--cache-dir",
   "--tls-server-name", "--loglevel", "-v", "--v", "--log-flush-frequency",
   "--profile", "--profile-output", "-c", "--container", "--shell",
   "--pod-running-timeout", "--image", "--node-name", "--to-namespace"];

# Words after which kubectl is being run rather than mentioned.
def runner_words:
  ["sudo", "doas", "env", "exec", "command", "builtin", "nohup", "nice", "time",
   "timeout", "xargs", "watch", "stdbuf", "setsid", "strace", "ionice", "flock",
   "chroot", "ssh", "eval", "noglob", "rsh", "debug", "-exec", "-execdir", "-ok",
   "-okdir", "then", "do", "else", "elif", "if", "while", "until", "!", "{"];

# The first positional word after `cargo`: the cargo subcommand. `+toolchain`
# and global options are skipped.
def cargo_subcommand:
  reduce .[] as $w ({skip: false, sub: null};
    if .sub != null then .
    elif .skip then .skip = false
    elif ($w | startswith("+")) then .
    elif ($w | startswith("-")) then .skip = ($w | IN("--color", "--config", "-Z", "-C"))
    else .sub = $w end)
  | .sub;

# ------------------------------------------------------------------ check 1

def check_kubectl:
  . as $s
  | first(range(0; $s | length) as $i
      | select(($s[$i] | basename) == "kubectl")
      | select(($s[:$i] | all(.[]; is_assignment)) or ($s[:$i] | any(.[]; IN(runner_words[]))))
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
      | ($s[$i + 1:] | positionals(oc_value_options))[0] // "") // "";

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

def protected_kind:
  test("^(pvcs?|persistentvolumeclaims?|pvs?|persistentvolumes?|apps?|applications?|appsets?|applicationsets?)$");

# Whether a resource argument (`pvc`, `pods,pvc`, `persistentvolumeclaim.v1`,
# `pvc/data`) names a protected kind in its type part.
def names_protected_kind:
  any(split(",")[] | split("/")[0] | ascii_downcase | split(".")[0]; protected_kind);

# oc delete of a protected kind. Only resource types are matched, as kubectl
# reads them: the first argument after the verb (`TYPE[,TYPE...]` or
# `TYPE/NAME`) and the type part of every later `TYPE/NAME` argument. Other
# words are resource names, so `oc delete deployment app` is not an
# Application.
def check_oc_delete:
  . as $s
  | first(range(0; $s | length) as $i
      | select(($s[$i] | basename) == "oc")
      | ($s[$i + 1:] | positionals(oc_value_options)) as $p
      | select(($p[0] // "") == "delete")
      | ($p[1:] | to_entries[]) as $e
      | select(($e.key == 0 or ($e.value | contains("/"))) and ($e.value | names_protected_kind))
      | $e.value as $arg
      | msg(1; "`oc delete` names '\($arg)', a PersistentVolumeClaim, PersistentVolume or Argo CD Application (or ApplicationSet). "
               + "Deleting one can destroy data, and deleting an Application deletes every resource it manages, so agents never do it. "
               + "A human does it after checking that the volume's reclaim policy is Retain and that a verified backup exists. Stop and ask the maintainer."));

def check_db_wipe:
  . as $s
  | first(range(0; $s | length) as $i
      | select($s[$i] == "db")
      | $s[$i + 1:] as $rest
      | (($rest | positionals(["-e", "--environment"]))[0] // "") as $sub
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
      | ($s[$i + 1:] | positionals(["-d", "--migration-dir", "-s", "--database-schema", "-u", "--database-url"])) as $p
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
# `cargo run` and `cargo r` count, because `cargo loco` is an alias of
# `cargo run --` (.cargo/config.toml) and runs the same backend3-cli.
def db_commands:
  . as $s
  | range(0; $s | length) as $i
  | ($s[$i] | basename) as $b
  | if ($b | IN("psql", "pg_restore", "mysql", "sea-orm-cli")) then {kind: $b, name: $b, args: $s[$i + 1:]}
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

def check_in_cluster:
  [.[] | splits("\\s+") | select(length > 0)] as $f
  | first(range(0; $f | length) as $i
      | select(($f[$i] | basename) == "oc")
      | $f[$i + 1:] as $rest
      | (($rest | positionals(oc_value_options))[0] // "") as $verb
      | select($verb | IN("exec", "rsh", "debug"))
      | ($rest | first(db_commands)) as $c
      | msg(2; "`\($c.name)` runs inside the cluster through `oc \($verb)`, so its database is not local, whatever its options say. "
               + "Agents never run database commands in the cluster; stop and ask the maintainer."));

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

# A command can only be blocked if it contains one of these words, so every
# other command is allowed without being parsed.
def trigger_re: "kubectl|grace-period|\\boc\\b|\\bdb\\b|psql|pg_restore|mysql|sea-orm-cli|cargo";

# The variables check 2 reads.
def watched_var_re: "DATABASE_URL|PGHOST|PGSERVICE|MYSQL_HOST";

# Whether a piece of shell text can matter to any check: it holds a trigger
# word, or an assignment that check 2 reads. Quotes and backslashes are
# dropped first, so k'ube'ctl still counts. split/join is linear, unlike a
# gsub with one match per quote.
def may_matter:
  split("\"") | join("") | split("'") | join("") | split("\\") | join("")
  | test(trigger_re + "|" + watched_var_re);

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
      # Chunks that cannot matter are not lexed, which keeps long commands cheap.
      [$cmd | gsub("\\\\\\n"; "") | (if test("<<") then strip_heredocs else . end)
       | chunks | select(may_matter) | lex(0)[]] as $segs
      | (first($segs | scoped_segments[] | .vars as $vars | .seg
               | (check_kubectl, check_force_delete, check_oc_delete, check_db_wipe, check_sea_orm_wipe,
                  check_in_cluster, check_db_target($vars), check_import_source($vars))) // null) as $why
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
