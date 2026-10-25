#!/usr/bin/env bash
# test-generator-anchors.sh: the files that `cargo loco generate` edits keep
# the anchors and imports its injected lines need (AGENTS.md rule 1,
# "Generate, then edit").
#
# Why it exists
#   loco-gen 1.2.0 edits existing files through rrgen 0.6.0: it inserts one
#   line before or after the FIRST line of the file that matches an anchor
#   regex (rrgen src/lib.rs, `apply_injection`). Three things break that
#   silently, and the breakage only shows when someone generates the first
#   worker, task or migration of a slice:
#     - prose that repeats an anchor text above the real anchor (the injected
#       line then lands in a doc comment);
#     - a doc comment or attribute directly above a `before:` anchor (the
#       injected line separates it from its item, and `missing_docs` fails);
#     - a name the injected line uses but the file does not import (the
#       `BackgroundWorker` trait for `Worker::build`, the `tasks` module).
#   The one generator that edits Cargo.toml, `cargo loco generate deployment
#   lambda`, adds `lambda_http` after the first line matching
#   `\[dependencies\]`. Prose that names that table above the real header
#   (the comments of [workspace.dependencies], for example) would turn the
#   line into a workspace entry that no crate uses. Where it lands correctly,
#   it is still a requirement in a member table, which
#   scripts/ci/check-workspace-deps.sh reports with the fix
#   (.claude/skills/backend3/SKILL.md, "Add a Rust dependency").
#   This test simulates each injection of the loco-gen 1.2.0 templates on a
#   copy of the real file and checks where the line lands. The anchors are
#   copied from those templates (`templates/controller/api/controller.t`,
#   `worker/worker.t`, `task/task.t`, `migration/*.t`,
#   `deployment/lambda/lambda.t`); a Loco bump re-reads
#   them (ADR 0024 section 3). Negative controls prove that each check fails
#   on the shape it guards against.
#
# Usage
#   bash scripts/tests/test-generator-anchors.sh     (from the repository root)
#   It needs bash and awk, and no network. It exits non-zero on any failure.

set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
APP_RS="$ROOT/src/app.rs"
MIGRATION_LIB="$ROOT/migration/src/lib.rs"
CARGO_TOML="$ROOT/Cargo.toml"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

tests_run=0
failures=0

# Records one result as a TAP line.
#   $1 - "ok" or "fail"; $2 - the case name; $3 - details for a failure.
report() {
  tests_run=$((tests_run + 1))
  if [[ $1 == ok ]]; then
    printf 'ok %d - %s\n' "$tests_run" "$2"
  else
    failures=$((failures + 1))
    printf 'not ok %d - %s\n    # %s\n' "$tests_run" "$2" "$3"
  fi
}

# Prints the number of the first line of FILE that matches the extended
# regular expression REGEX, or nothing.
#   $1 - the file; $2 - the regex.
first_match() {
  awk -v re="$2" '$0 ~ re { print NR; exit }' "$1"
}

# Writes FILE with CONTENT inserted before or after its first line matching
# REGEX, as rrgen does, to OUT. Fails when nothing matches (rrgen errors too).
#   $1 - the file; $2 - "before" or "after"; $3 - the regex; $4 - the line;
#   $5 - the output file.
inject() {
  local file=$1 where=$2 regex=$3 content=$4 out=$5 line
  line=$(first_match "$file" "$regex")
  [[ -n $line ]] || return 1
  awk -v n="$line" -v where="$where" -v content="$content" '
    NR == n && where == "before" { print content }
    { print }
    NR == n && where == "after" { print content }
  ' "$file" >"$out"
}

# Prints line N of FILE.
#   $1 - the file; $2 - the line number.
line_at() {
  sed -n "${2}p" "$1"
}

# Prints the nearest line at or above line N of FILE that declares a
# function (contains "fn ").
#   $1 - the file; $2 - the line number.
enclosing_fn() {
  awk -v n="$2" 'NR <= n && /(^|[^A-Za-z_])fn / { last = $0 } NR == n { print last; exit }' "$1"
}

# --------------------------------------------------------------- the checks
# Each check prints nothing and returns 0 when the file is fine, or prints the
# reason and returns 1.

# `cargo loco generate controller` (and `scaffold`): after "AppRoutes::",
#   `            .add_route(controllers::<name>::routes())`.
check_routes_anchor() {
  local file=$1 line text
  line=$(first_match "$file" 'AppRoutes::')
  [[ -n $line ]] || { echo "no line matches 'AppRoutes::'"; return 1; }
  text=$(line_at "$file" "$line")
  if [[ ! $text =~ AppRoutes::with_default_routes\(\) ]] || [[ $text =~ \;[[:space:]]*$ ]]; then
    echo "the first 'AppRoutes::' line (line $line) is not the start of the route chain: $text"
    return 1
  fi
}

# `cargo loco generate worker`: after "fn connect_workers",
#   `        queue.register(crate::workers::<name>::Worker::build(ctx)).await?;`,
#   which needs the parameters `ctx` and `queue` and the BackgroundWorker trait.
check_worker_anchor() {
  local file=$1 line text
  line=$(first_match "$file" 'fn connect_workers')
  [[ -n $line ]] || { echo "no line matches 'fn connect_workers'"; return 1; }
  text=$(line_at "$file" "$line")
  if [[ ! $text =~ ^[[:space:]]*async\ fn\ connect_workers\(ctx:\ \&AppContext,\ queue:\ \&Queue\)\ -\>\ Result\<\(\)\>\ \{$ ]]; then
    echo "the first 'fn connect_workers' line (line $line) is not the signature with ctx and queue ending in '{': $text"
    return 1
  fi
  if ! grep -Eq '^use loco_rs::bgworker::BackgroundWorker;$' "$file"; then
    echo "missing 'use loco_rs::bgworker::BackgroundWorker;': the injected Worker::build(ctx) calls a trait method"
    return 1
  fi
}

# `cargo loco generate task`: before "// tasks-inject",
#   `        tasks.register(tasks::<file>::<Task>);`, which needs the `tasks`
#   parameter and `use crate::tasks;`.
check_task_anchor() {
  local file=$1 line fn_line
  line=$(first_match "$file" '// tasks-inject')
  [[ -n $line ]] || { echo "no line matches '// tasks-inject'"; return 1; }
  fn_line=$(enclosing_fn "$file" "$line")
  if [[ ! $fn_line =~ fn\ register_tasks\(tasks:\ \&mut\ Tasks\) ]]; then
    echo "the first '// tasks-inject' line (line $line) is not inside fn register_tasks(tasks: &mut Tasks): $fn_line"
    return 1
  fi
  if ! grep -Eq '^use crate::tasks;$' "$file"; then
    echo "missing 'use crate::tasks;': the injected tasks::<file>::<Task> path names that module"
    return 1
  fi
}

# `cargo loco generate migration` and `generate model`: before
#   "inject-above", `            Box::new(<module>::Migration),`.
check_migration_list_anchor() {
  local file=$1 line text fn_line
  line=$(first_match "$file" 'inject-above')
  [[ -n $line ]] || { echo "no line matches 'inject-above'"; return 1; }
  text=$(line_at "$file" "$line")
  fn_line=$(enclosing_fn "$file" "$line")
  if [[ ! $text =~ ^[[:space:]]*//\ inject-above ]] || [[ ! $fn_line =~ fn\ migrations\(\) ]]; then
    echo "the first 'inject-above' line (line $line) is not the marker comment inside fn migrations(): $text"
    return 1
  fi
}

# The same generators: before "pub struct Migrator", `mod <module>;`. The
#   inserted line must not come between the struct's doc comment and the
#   struct, or `missing_docs` fails.
check_migration_mod_anchor() {
  local file=$1 out="$WORK/mod-injected.rs" struct_line before
  if ! inject "$file" before 'pub struct Migrator' 'mod m00000000_000000_probe;' "$out"; then
    echo "no line matches 'pub struct Migrator'"
    return 1
  fi
  struct_line=$(first_match "$out" '^pub struct Migrator;')
  [[ -n $struct_line ]] || { echo "no 'pub struct Migrator;' declaration"; return 1; }
  before=$(line_at "$out" "$((struct_line - 1))")
  if [[ ! $before =~ ^/// ]]; then
    echo "after the injection, line $((struct_line - 1)) above 'pub struct Migrator;' is '$before', not its doc comment"
    return 1
  fi
}

# `cargo loco generate deployment lambda`: after "\[dependencies\]" in
#   Cargo.toml, `lambda_http = { version = "1.3" }`. The first matching line
#   must be the package's own [dependencies] table header. (The anchor is
#   written as a bracket expression here because awk -v would turn `\[` into
#   a plain `[`.)
check_cargo_dependencies_anchor() {
  local file=$1 line text
  line=$(first_match "$file" '[[]dependencies[]]')
  [[ -n $line ]] || { echo "no line matches '[dependencies]'"; return 1; }
  text=$(line_at "$file" "$line")
  if [[ $text != "[dependencies]" ]]; then
    echo "the first line matching '[dependencies]' (line $line) is not the table header: $text"
    return 1
  fi
}

# Prints the nearest TOML table header at or above line N of FILE.
#   $1 - the file; $2 - the line number.
enclosing_table() {
  awk -v n="$2" 'NR <= n && /^\[/ { last = $0 } NR == n { print last; exit }' "$1"
}

# Runs one check and records the expected outcome.
#   $1 - "pass" or "fail"; $2 - the case name; $3 - the check; $4 - the file.
expect_check() {
  local expected=$1 name=$2 check=$3 file=$4 reason status=0
  reason=$("$check" "$file") || status=$?
  if [[ $expected == pass && $status -eq 0 ]] || [[ $expected == fail && $status -ne 0 ]]; then
    report ok "$name" ""
  else
    report fail "$name" "${reason:-the check passed, but it should have failed}"
  fi
}

# --------------------------------------------------------------- the real files

expect_check pass "src/app.rs: controller routes anchor" check_routes_anchor "$APP_RS"
expect_check pass "src/app.rs: worker anchor and BackgroundWorker import" check_worker_anchor "$APP_RS"
expect_check pass "src/app.rs: task anchor and tasks import" check_task_anchor "$APP_RS"
expect_check pass "migration/src/lib.rs: migration list anchor" check_migration_list_anchor "$MIGRATION_LIB"
expect_check pass "migration/src/lib.rs: mod anchor keeps the Migrator doc attached" check_migration_mod_anchor "$MIGRATION_LIB"
expect_check pass "Cargo.toml: the first '[dependencies]' line is the package's table header" check_cargo_dependencies_anchor "$CARGO_TOML"

# The injection itself, end to end on copies: each injected line lands next
# to its anchor.
inject "$APP_RS" after 'fn connect_workers' '        queue.register(crate::workers::probe::Worker::build(ctx)).await?;' "$WORK/app-worker.rs"
worker_line=$(first_match "$WORK/app-worker.rs" 'workers::probe::Worker::build')
if [[ $(enclosing_fn "$WORK/app-worker.rs" "$worker_line") =~ fn\ connect_workers ]]; then
  report ok "a generated worker registration lands in connect_workers" ""
else
  report fail "a generated worker registration lands in connect_workers" "line $worker_line"
fi
inject "$MIGRATION_LIB" before 'inject-above' '            Box::new(m00000000_000000_probe::Migration),' "$WORK/lib-list.rs"
list_line=$(first_match "$WORK/lib-list.rs" 'm00000000_000000_probe::Migration')
if [[ $(enclosing_fn "$WORK/lib-list.rs" "$list_line") =~ fn\ migrations\(\) ]]; then
  report ok "a generated migration lands in Migrator::migrations" ""
else
  report fail "a generated migration lands in Migrator::migrations" "line $list_line"
fi
inject "$CARGO_TOML" after '[[]dependencies[]]' 'lambda_http = { version = "1.3" }' "$WORK/cargo-lambda.toml"
lambda_line=$(first_match "$WORK/cargo-lambda.toml" '^lambda_http = ')
if [[ $(enclosing_table "$WORK/cargo-lambda.toml" "$lambda_line") == "[dependencies]" ]]; then
  report ok "the lambda generator's dependency lands in the package's [dependencies]" ""
else
  report fail "the lambda generator's dependency lands in the package's [dependencies]" \
    "line $lambda_line is under $(enclosing_table "$WORK/cargo-lambda.toml" "$lambda_line")"
fi

# Files the generators append a `mod` line to must exist.
for appended in src/controllers/mod.rs src/models/mod.rs src/tasks/mod.rs src/workers/mod.rs \
  tests/models/mod.rs tests/requests/mod.rs tests/tasks/mod.rs tests/workers/mod.rs; do
  if [[ -f "$ROOT/$appended" ]]; then
    report ok "$appended exists for the generators to append to" ""
  else
    report fail "$appended exists for the generators to append to" "create it with a module doc comment"
  fi
done

# --------------------------------------------------------------- negative controls

# Prose that repeats an anchor above the real one.
# shellcheck disable=SC2016 # the backticks are literal Rust doc text
{ echo '//! New files are registered above the `inject-above` marker.'; cat "$MIGRATION_LIB"; } >"$WORK/prose-anchor.rs"
expect_check fail "control: an 'inject-above' in the module docs is caught" check_migration_list_anchor "$WORK/prose-anchor.rs"
{ echo '// See fn connect_workers below.'; cat "$APP_RS"; } >"$WORK/prose-worker.rs"
expect_check fail "control: a 'fn connect_workers' in a comment above is caught" check_worker_anchor "$WORK/prose-worker.rs"
# shellcheck disable=SC2016 # the backticks are literal TOML comment text
awk '{ print } !done && /^\[workspace\.dependencies\]$/ { print "# backend3 adds `net` in its [dependencies]."; done = 1 }' \
  "$CARGO_TOML" >"$WORK/prose-cargo.toml"
expect_check fail "control: a '[dependencies]' in a comment above the header is caught" check_cargo_dependencies_anchor "$WORK/prose-cargo.toml"

# Loco's own template shape, a doc comment directly above the struct.
grep -v 'pub struct Migrator`,$' "$MIGRATION_LIB" >"$WORK/doc-above.rs"
expect_check fail "control: a doc comment directly above 'pub struct Migrator' is caught" check_migration_mod_anchor "$WORK/doc-above.rs"

# Missing imports.
grep -Ev '^use loco_rs::bgworker::BackgroundWorker;$' "$APP_RS" >"$WORK/no-trait.rs"
expect_check fail "control: a missing BackgroundWorker import is caught" check_worker_anchor "$WORK/no-trait.rs"
grep -Ev '^use crate::tasks;$' "$APP_RS" >"$WORK/no-tasks.rs"
expect_check fail "control: a missing tasks import is caught" check_task_anchor "$WORK/no-tasks.rs"

echo "1..$tests_run"
if ((failures > 0)); then
  echo "# $failures of $tests_run test(s) failed: $(basename "$0")"
  exit 1
fi
echo "# all $tests_run tests passed: $(basename "$0")"
