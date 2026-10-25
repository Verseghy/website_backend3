# 0028. Guardrails as built, wider checks and a fail-closed hook

- Status: Accepted (2026-10-25)
- Decision questions: none: amends [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §4, which records Q37, and carries the updated recipe list of its §2, which records Q43a. It records the maintainer's answers of 2026-10-25 to D23, D30 and, for ADR 0024 §2, D66 of the decision register (research: DECISIONS R9), and to D69 of the second decision register (research: DECISIONS R15).

## Context

[ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §4 sets three layers of guardrails against destructive agent commands. It fixes the layer-1 settings block verbatim and describes what the layer-2 hook, `.claude/hooks/guard-bash.sh`, checks. Under [ADR 0001](0001-record-architecture-decisions.md) §6, neither the block nor a rule beyond the ADR's text may change without a maintainer's answer. Agents collected five findings about ADR 0024 for that answer:

| Finding | What it says | Source |
|---|---|---|
| The hook is stricter than §4 | §4 says check 2 takes the target host from the first source present, and check 1 blocks `--force` together with `--grace-period=0`. The hook built in the bootstrap checks every target source; reads `PGHOSTADDR`; treats `PGSERVICE` and `service=` as not checkable; blocks a true `--force` on `oc delete` or `oc replace` without `--grace-period=0`; covers ApplicationSets; and checks heredocs, here-strings and piped scripts. | `Verseghy/website_backend3#195` item 2; `Verseghy/website_backend3#186` |
| Two more rules from the #197 fix | An `oc delete` fed by another command's output counts as naming a protected kind when the same command line runs an `oc get` of one. Text that `oc exec`, `oc rsh` or `oc debug` reads on stdin counts as a script that runs in the cluster, even when the program in the pod is not a shell. | `Verseghy/website_backend3#195` item 11; `Verseghy/website_backend3#203` |
| Candidates for both repositories | Block namespace and project deletion, `argocd app delete`, an `oc patch` that removes finalizers or sets a reclaim policy to Delete, `oc port-forward` to cluster databases, and the `mariadb` client. Add `"onFailure": "block"` to the hook entry. Optionally use the exec form (`args`). The settings block is fixed verbatim, so each change needs an amendment first. | `Verseghy/website_backend3#195` item 8, from the website_k8s follow-ups (`Verseghy/website_k8s#54`) |
| The project skill has five recipes | `.claude/skills/backend3/SKILL.md` gained recipe 5, adding a Rust dependency, in #200. ADR 0024 §2 lists four. | `Verseghy/website_backend3#195` item 10; `Verseghy/website_backend3#200` |
| Guardrails beyond the candidates | (a) Block an `oc patch` that sets the deprecated `Recycle` reclaim policy. (b) Block `argocd app delete-resource`, `argocd appset delete`, and port-forwards to Redis or other databases. (c) Confirm that the hook may treat `cargo run`, `cargo r`, `cargo loco-tool` and `cargo t` as spellings of ADR 0024's cargo commands, as it has since the bootstrap. | `Verseghy/website_backend3#195` item 12 |

The facts behind the candidates:

| Fact | Value | Source |
|---|---|---|
| A failing hook fails open by default | When a hook cannot start, exits with a code other than 0 or 2, or reaches its timeout, Claude Code still runs the tool call. `"onFailure": "block"` on a `command` hook makes such a failure do what exit code 2 does, which on `PreToolUse` blocks the tool call. The field has no effect on background hooks (`async`, `asyncRewake`). It needs Claude Code v2.1.295 or later. | Claude Code docs, "Hooks reference", "Block the action when a hook fails"; Claude Code changelog, 2.1.295 |
| The exec form | When `args` is present, Claude Code spawns `command` directly with `args` as its argument vector, with no shell. It substitutes `${CLAUDE_PROJECT_DIR}` into `command` and into each `args` element as a plain string, and still exports `CLAUDE_PROJECT_DIR` to the process. `command` is the executable's name or path; the docs' own examples include a project script as `command` with `"args": []`. `args` arrived in v2.1.139. | "Hooks reference", "Exec form and shell form" and the `MessageDisplay` example; changelog, 2.1.139 |
| Deleting a namespace or project | Deleting a namespace deletes every object in it, its PersistentVolumeClaims included. An OpenShift project is another representation of a namespace, so deleting a project deletes its namespace. | Kubernetes docs, "Namespaces"; `openshift/api`: `project/v1/types.go` (`Project`) |
| `argocd app delete` | It deletes an Application through the Argo CD API, and with its default `--cascade` every resource the Application manages, as `oc delete application` does. argocd defines no alias for `app` or `delete`. | `argoproj/argo-cd@17b98d9b6:cmd/argocd/commands/app.go` (`NewApplicationDeleteCommand`), `cmd/argocd/commands/root.go` |
| `argocd app delete-resource` and `argocd appset delete` | `app delete-resource` deletes resources that an Application manages, of the kind its required `--kind` names, a PersistentVolumeClaim included, through the Argo CD API. `appset delete` deletes ApplicationSets, and with them their Applications; its prompt asks "Are you sure you want to delete '...' and all its Applications?". argocd defines no alias for `appset`, `delete` or `delete-resource`. | `argoproj/argo-cd@17b98d9b6:cmd/argocd/commands/app_resources.go` (`NewApplicationDeleteResourceCommand`), `cmd/argocd/commands/applicationset.go` (`NewAppSetCommand`, `NewApplicationSetDeleteCommand`) |
| Finalizers and reclaim policies | A finalizer holds back a deletion; `kubernetes.io/pvc-protection`, for example, keeps a claim that a pod still uses. Without its finalizers a resource is deleted at once. A PersistentVolume whose reclaim policy is `Delete` is deleted with its storage when its claim is deleted. `Retain` keeps it. The deprecated `Recycle` wipes the volume's data and keeps the volume. | Kubernetes docs, "Finalizers" and "Persistent Volumes" (Reclaiming) |
| A port-forward looks local | `oc port-forward` listens on `localhost` by default (`--address=[localhost]`). A database in the cluster then answers on localhost, and check 2 counts localhost as local, so `psql -h localhost` would reach it. | oc 4.22, `oc port-forward --help` |
| Redis | Redis listens on 6379 by default, and Redis Sentinel on 26379; Sentinel's `SENTINEL SET`, `SENTINEL REMOVE` and `SENTINEL FAILOVER` change the Redis servers it monitors. The clusters run PostgreSQL, MySQL and backend2's Redis, which listens on 6379. | Redis docs, "High availability with Redis Sentinel"; `Verseghy/website_k8s@f0286dc`: `verseghy-website/postgres/deployment.yaml`, `verseghy-website/mysql/deployment.yaml`, `verseghy-website/backend2-redis/service.yaml` |
| Deny-rule wildcards | A `*` in a Bash rule matches any text. The space before a trailing `*` is part of the rule: `Bash(ls *)` does not match `lsof`. So `Bash(argocd app delete *)` does not match `argocd app delete-resource`. | Claude Code docs, "Configure permissions" ("Wildcard patterns") |
| The MariaDB client | `mariadb` is MariaDB's command-line client. It takes `-h` and `--host` like `mysql` and reads its default host from `MYSQL_HOST`. | MariaDB docs, "MariaDB Environment Variables" |
| website_k8s | Its committed `.claude/settings.json` has ADR 0024's deny rules without `Bash(cargo loco db reset *)`, and the repository has no Cargo project. It has no ADRs of its own; ADR 0024's "Scope of the guardrails" governs its guardrails. | `Verseghy/website_k8s@5b8ed44:.claude/settings.json` |

The maintainer answered on 2026-10-25, on the decision register (research: DECISIONS R9):

- **D23**, "guard-bash.sh is stricter than ADR 0024 §4", asked "Do you accept the hook's stricter checks? It checks every target source, reads PGHOSTADDR, treats PGSERVICE as uncheckable, blocks a true --force on oc delete/replace, covers ApplicationSets, and checks heredocs and piped scripts. If so, should an amending ADR record them?" The answer: "Accept and record in an amending ADR".
- **D30**, "ADR 0024 amendment for both repositories' guardrails", asked whether one website_backend3 ADR should amend ADR 0024 §4 for both repositories, applied in website_k8s by a human-reviewed PR, with the candidates above. The answer: "Adopt all candidates in one amending ADR".
- **D66**, "ADR tables that lag behind the code after #200", asked "Do you accept #200's additions as within the existing decisions, and should ADR 0022 §3's Toolchain row and ADR 0024 §2's list of project-skill recipes be updated in a docs PR?" The answer: "Accept, and update both tables in a docs PR". This ADR carries the ADR 0024 §2 part; the ADR 0022 §3 part is not recorded here.

On the same day the maintainer answered a second decision register (research: DECISIONS R15):

- **D69**, "Guard-hook rules nobody has answered yet (#195 items 11 and 12)", asked "Do you accept four guard-hook rules? (1) The two #203 rules from #195 item 11, which ADR 0028 records as part of D23's answer. (2) Blocking an `oc patch` that sets the deprecated Recycle reclaim policy (item 12a). (3) Blocking `argocd app delete-resource`, `argocd appset delete`, and port-forwards to Redis or other databases (item 12b). (4) Treating `cargo run`, `cargo r`, `cargo loco-tool` and `cargo t` as spellings of ADR 0024's cargo commands (item 12c)." The answer, of 2026-10-25: "Accept all four in ADR 0028".

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0024 §4, the layer-1 settings block | Replaced by the block of section 3: more deny rules, and the hook entry in exec form with `"onFailure": "block"`. | 3 |
| ADR 0024 §4, layer 2, check 1 | The stricter checks the hook already has are accepted as built (section 4). Namespaces, projects, `argocd app delete`, `argocd app delete-resource`, `argocd appset delete` and two kinds of `oc patch` are added (section 5). | 4, 5 |
| ADR 0024 §4, layer 2, check 2 | Every target source counts, not only the first, with `PGHOSTADDR` and `PGSERVICE`, and the cargo aliases count like the listed cargo commands (section 4). `mariadb` joins the listed commands, and a port-forward to a PostgreSQL, MySQL, MariaDB or Redis port is blocked (section 5). | 4, 5 |
| ADR 0024 §4, the paragraph on `scripts/tests/test-guard-bash.sh` | The tests also cover sections 3 to 5 (section 6). | 6 |
| ADR 0024 §4, "Scope of the guardrails", the website_k8s item | website_k8s takes the same amendment, through a human-reviewed PR, and leaves out the cargo deny rule (section 7). | 7 |

Section 2 changes no decision. D66 accepted the fifth recipe as within ADR 0024's existing decision. ADR 0001 §6 keeps the body of an amended ADR as it was, so once this ADR amends ADR 0024, ADR 0024 §2's list can no longer be brought up to date in place. This ADR therefore carries the list. Two more recipes joined the skill after D66 was asked: recipe 6, adding a task, with `Verseghy/website_backend3#208`, and recipe 7, using the current time, with `#207`. Each one writes down how to follow a decision already taken, ADR 0016 §4 and ADR 0010 point 7. Section 2 lists them too, so that the list matches the skill. D66's answer covers only the fifth recipe.

Everything else in ADR 0024 holds as written, layer 3 included. In particular, an agent still stops and asks the maintainer when a guardrail blocks legitimate work, and the hook still fails closed when `jq` is missing or its input cannot be read.

### 2. The project skill's recipes

ADR 0024 §2's row for `.claude/skills/backend3/` reads, brought up to date: the project's own skill; its `SKILL.md` has `name: backend3` and recipes for the repeated tasks: adding a vertical slice (ADR 0012), adding an entity under the safety net (ADR 0011), adding a public endpoint with its fixtures (ADR 0019), adding an importer step (ADR 0017), adding a Rust dependency (one `[workspace.dependencies]` entry with its reason, inherited with `workspace = true` and checked by `scripts/ci/check-workspace-deps.sh`, as `AGENTS.md` describes), adding a task (ADR 0016 §4 and its Consequences: its work runs under the shared advisory lock, and every task has a lock-held test), and using the current time (ADR 0010 point 7: "today" and the other calendar questions come from an injected clock, the app clock in the `AppContext`).

### 3. The layer-1 settings block

**Layer 1: deny rules and the hook entry in the committed `.claude/settings.json`**

```json
{
  "permissions": {
    "deny": [
      "Bash(cargo loco db reset *)",
      "Bash(oc delete pvc *)",
      "Bash(oc delete pvc/*)",
      "Bash(oc delete persistentvolumeclaim *)",
      "Bash(oc delete pv *)",
      "Bash(oc delete pv/*)",
      "Bash(oc delete persistentvolume *)",
      "Bash(oc delete application *)",
      "Bash(oc delete applications *)",
      "Bash(oc delete applications.argoproj.io *)",
      "Bash(oc delete namespace *)",
      "Bash(oc delete namespace/*)",
      "Bash(oc delete namespaces *)",
      "Bash(oc delete ns *)",
      "Bash(oc delete ns/*)",
      "Bash(oc delete project *)",
      "Bash(oc delete project/*)",
      "Bash(oc delete projects *)",
      "Bash(argocd app delete *)",
      "Bash(argocd app delete-resource *)",
      "Bash(argocd appset delete *)",
      "Bash(oc patch *finalizers*)",
      "Bash(oc patch *persistentVolumeReclaimPolicy*Delete*)",
      "Bash(oc patch *persistentVolumeReclaimPolicy*Recycle*)",
      "Bash(* --force --grace-period=0*)",
      "Bash(* --grace-period=0 --force*)",
      "Bash(kubectl *)"
    ]
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PROJECT_DIR}/.claude/hooks/guard-bash.sh",
            "args": [],
            "onFailure": "block"
          }
        ]
      }
    ]
  }
}
```

- **The deny rules** gain the forms of the section 5 checks that a rule on the command text can match: namespace and project deletion, `argocd app delete`, `argocd app delete-resource` and `argocd appset delete`, and the `oc patch` forms for finalizers and for a `Delete` or `Recycle` reclaim policy. A `*` in a rule matches any text, spaces included, but the space before a trailing `*` is part of the rule, so `Bash(argocd app delete *)` does not match `argocd app delete-resource`, which has a rule of its own. The port-forward and `mariadb` checks belong to check 2, which has no deny rules, as `psql` and `mysql` have none. The hook covers every other spelling.
- **`"onFailure": "block"`.** A hook that cannot start, crashes or reaches its timeout now blocks the command instead of letting it through. The hook still exits 2 on every failure it can detect itself, so it also fails closed where the field does not apply.
- **The exec form.** `args` is present, so no shell runs, and `${CLAUDE_PROJECT_DIR}` is substituted as a plain string; a project path with spaces or quotes needs no quoting. The script itself is `command`, with `"args": []`, not `bash` with the script in `args`: a client that predates `args` runs `command` in shell form, which still starts the hook, while `bash` alone would read the hook's JSON input as a script.
- **The minimum client.** Every agent host runs Claude Code v2.1.295 or later. An older client ignores `onFailure`, and one older than v2.1.139 also ignores `args`.

### 4. The stricter checks, accepted as built

These are the points where the hook was stricter than ADR 0024 §4's text. They are now part of the decision: the first five rows by the answer to D23, the two rows from #203 (`Verseghy/website_backend3#195` item 11) and the cargo row (item 12(c)) by the answer to D69 of 2026-10-25, "Accept all four in ADR 0028".

| ADR 0024 §4 said | The hook does | Why |
|---|---|---|
| Check 2 takes the target host from the first source present. | It checks every explicit source that is present: inline variables, options and connection strings. An environment variable is checked unless the command does not use it (an inline value of the same variable replaces it, for example). | psql prefers `-h` over `PGHOST`, and a local inline `PGHOST` must not hide a remote `DATABASE_URL`. |
| The sources are `DATABASE_URL` and `PGHOST` (for `mysql`, `MYSQL_HOST`). | It also reads `PGHOSTADDR`. A `PGSERVICE` variable or a `service=` key counts as a target that cannot be checked, and is blocked. | libpq and sqlx connect to `PGHOSTADDR`; a `pg_service.conf` entry cannot be read from the command. |
| Check 1 blocks `--force` together with `--grace-period=0`. | It also blocks a true `--force` (every value Go's `strconv.ParseBool` reads as true) on `oc delete` and `oc replace`, whatever the grace period. | kubectl turns `--force` without a grace period into `--grace-period=0`. |
| `oc delete` of a PVC, PV or Argo CD Application. | ApplicationSets count too. | Deleting an ApplicationSet deletes its Applications. |
| `sh -c` wrappers and other spellings. | It also checks heredocs read by a shell, here-strings, and `echo` or `printf` output piped into a shell, also through further pipeline stages and across a line break after `|`. A heredoc that only data reads, such as a commit message, is skipped. | These run the same commands as `sh -c`. |
| (from #203) | An `oc delete` whose resources come from another command's output (`$(...)`, a variable, `xargs`, `-f -` or a process substitution) counts as naming a protected kind when the same command line runs an `oc get` of one. | The hook cannot read that output. |
| (from #203) | Text that `oc exec`, `oc rsh` or `oc debug` reads on stdin (a heredoc, a here-string, piped `echo` or `printf` output) counts as a script that runs in the cluster, even when the program in the pod is not a shell. Under `xargs` the text becomes the oc command's arguments instead, as xargs passes it. | The program in the pod may hand its input to a shell; the hook cannot see what it does. |
| The listed commands of check 2 are `cargo loco`, `cargo test` and `cargo nextest`. | Since the bootstrap, it also counts `cargo run`, `cargo r`, `cargo loco-tool` and `cargo t`, like `cargo loco` and `cargo test`. | They start the same programs: ADR 0024 §4 itself says that `cargo loco` is the alias `run --`, `.cargo/config.toml` defines `loco-tool` as the same alias, and `r` and `t` are cargo's built-in aliases of `run` and `test`. |

The hook header in `.claude/hooks/guard-bash.sh` lists these points and the paths the hook cannot see. A new point like them needs the maintainer's answer or an amending ADR first (ADR 0001 §6).

### 5. The new checks

These are D30's candidates and, where marked, the additions that the answer to D69 accepted (`Verseghy/website_backend3#195` item 12(a) and (b)).

Check 1 also blocks, in every spelling and wrapper the hook already handles:

1. **Namespace and project deletion.** `oc delete` of a namespace (`namespace`, `namespaces`, `ns`) or an OpenShift project (`project`, `projects`, `projects.project.openshift.io`), as a type, in a type list or as `TYPE/NAME`, counts like a PVC. So does an `oc delete` fed by an `oc get` of one (section 4).
2. **The argocd commands that delete:** `argocd app delete`, and (D69) `argocd app delete-resource` and `argocd appset delete`, each with any options and any arguments, `--cascade=false` and every `--kind` included. `delete-resource` is blocked for every kind, because the hook does not tell a PVC from a pod there; `appset delete` deletes the ApplicationSet's Applications too. The hook finds the parent (`app`, `appset`) and the subcommand as positional words, after skipping the options it knows to take a value. An option it does not know cannot hide them when it stands before the parent or after the subcommand: its value only adds one more positional word. Between the parent and the subcommand, such an option may take the next word as its value, so the hook then also takes the positional word after that one as the subcommand.
3. **An `oc patch` that removes finalizers or sets a reclaim policy to Delete or Recycle**, the D30 candidate and, for `Recycle`, D69:
   - **Removing finalizers.** A merge patch that sets the list, a JSON patch `remove` and a strategic merge `$deleteFromPrimitiveList` each remove finalizers, and the hook cannot tell every one of them from an addition. So it matches every patch that names `finalizers`, additions included.
   - **A reclaim policy of Delete or Recycle.** The patch names `persistentVolumeReclaimPolicy` together with `Delete`, with the deprecated `Recycle`, which wipes the volume's data when its claim is released, or with a shell expansion, whose value cannot be checked (as with `PGSERVICE` in section 4). `Retain` and every other value are not matched.

   Both are matched without regard to case. A patch read from `--patch-file`, or from another command's output, is not visible to the hook.

Check 2 also:

4. **Blocks `oc port-forward` to a database port:** a port spec whose remote part is 5432 (PostgreSQL), 3306 (MySQL and MariaDB), 33060 (the MySQL X protocol), or (D69) 6379 (Redis) or 26379 (Redis Sentinel); a named port that contains `postgres`, `mariadb` or `sql` (so `postgresql`, `pgsql`, `mysql` and `mysqlx` count), or (D69) `redis` or `sentinel` (so `tcp-redis` and `tcp-sentinel` count), or is `db`; a port that holds a shell expansion or the replace string of `xargs` or `parallel`; and ports that `xargs` or `parallel` append, which cannot be seen. The local port does not matter, and neither does the name of the pod or service: a port-forward to any other port, for example a web service or a metrics exporter, is allowed, also on a Redis service. D69's "Redis or other databases" is read as the database engines the clusters run, PostgreSQL, MySQL and Redis: with Redis, the check knows the ports of each of them.
5. **Checks `mariadb` like `mysql`:** its target comes from `-h`, `--host`, a connection URI or `MYSQL_HOST`, and it is blocked when it is not local, or when it runs in the cluster through `oc exec`, `oc rsh` or `oc debug`.

A shell runs an unquoted `$(...)`, `` `...` `` or `<(...)` as a command of its own and then goes on with the command around it. The hook reads it the same way, so a pod looked up that way does not hide the words after it: `oc port-forward $(oc get pod -l app=postgres -o name) 5432` and `oc exec $(oc get pod -o name) -- psql` are blocked. This follows the shell; it adds no rule.

Each block reason names ADR 0024 §4 and this ADR, says what was refused and what to do instead, as before.

### 6. Tests

`scripts/tests/test-guard-bash.sh` keeps every case ADR 0024 §4 names, and also covers:

- block and allow cases for each check of section 5: every spelling of the namespace and project types; each of `argocd app delete`, `app delete-resource` and `appset delete` with known and unknown options before, between and after the subcommands, each value option the hook skips next to an unknown option, argocd's boolean global options before each subcommand, and the argocd subcommands that only read; each patch type for finalizers and for the `Delete` and `Recycle` reclaim policies; `Retain` and other harmless patches; database and non-database ports, the Redis and Sentinel ports among them, each part of the named-port rule, empty port specs; `xargs`; and `mariadb` with each target source and in the cluster;
- the cargo aliases of section 4: `cargo r`, `cargo loco-tool` and `cargo t` with a remote and a local target;
- targets and options given by an unquoted `$(...)`, `` `...` `` or `<(...)`, in front of the words that decide the verdict;
- text that only mentions those commands, which stays allowed: `git commit -m`, a `git commit -F -` heredoc, `gh pr create --body`, `gh pr comment --body` and `grep`;
- that `.claude/settings.json` equals the block of section 3, read from this ADR;
- that the Bash hook entry is one command hook in exec form, with the script as `command` and `"args": []`, with `"onFailure": "block"`, and is not a background hook;
- that the entry runs the hook the way Claude Code runs the exec form, with `${CLAUDE_PROJECT_DIR}` substituted as a plain string and no shell, from the repository and from a project path that holds a space.

### 7. website_k8s

D30 makes this one ADR govern both repositories, as ADR 0024's "Scope of the guardrails" already does. A website_k8s PR that a human reviews (ADR 0012) applies it there:

- its `.claude/settings.json` is the block of section 3 without `Bash(cargo loco db reset *)`. That rule is left out on purpose: website_k8s has no Cargo project. This was already the case before this ADR, and this ADR records it as intended;
- its hook gets the checks of sections 4 and 5. Check 2 leaves out the cargo command names, as ADR 0024 says, and keeps `psql`, `pg_restore`, `mysql`, `mariadb`, `sea-orm-cli` and the port-forward check;
- its hook tests cover the same cases as section 6, without the cargo ones.

Until that PR merges, website_k8s keeps the guardrails it has.

## Consequences

**Positive**

- ADR 0024 and the hook agree again, so the final review can read the code against the ADRs, and the website_k8s copy has one written target instead of drifting.
- A hook that cannot start, crashes or stalls now blocks the command. A wrong path, a missing executable bit or a timeout no longer turns the guardrail off.
- The data-loss class ADR 0024 targets (PVCs, PVs, Applications) is closed on seven more paths: a namespace or project deletion, `argocd app delete`, `argocd app delete-resource`, `argocd appset delete`, a finalizer patch, a Delete reclaim-policy patch and a Recycle reclaim-policy patch.
- A port-forward can no longer expose a cluster database on localhost, where check 2 would take it for a local one. Nor can it expose the clusters' Redis there, which check 2 does not cover at all: no Redis client is one of its listed commands.
- Each rule the hook had beyond ADR 0024's text, the #203 rules and the cargo aliases included, now rests on a maintainer's answer.
- The exec form needs no shell quoting of the project path.

**Negative and risks**

- Every agent host needs Claude Code v2.1.295 or later. An older client ignores `onFailure`, so for it a hook that cannot start or that times out fails open again, as before this ADR.
- The patch check blocks every change to finalizers, additions included. Adding a finalizer is then a task for a human.
- An `argocd app` or `argocd appset` command with an option the hook does not know right before its subcommand is blocked when the next positional word is a deleting subcommand, for example `argocd app --new-option get delete` for an Application named `delete`.
- `argocd app delete-resource` is blocked for every kind, so deleting a pod through Argo CD, to restart it for example, is a task for a human or for `oc delete pod`.
- The port-forward check knows databases by their ports. A database served on another port is not recognised, and a non-database service on 5432, 3306, 33060, 6379 or 26379, or on a named port that contains one of the database words, is blocked.
- A database engine that the clusters do not run today, MongoDB for example, is not recognised by its ports. Adding its ports needs an amending ADR, which can cite D69's answer (ADR 0001 §6).
- Three paths of the same data-loss class are outside D69's answer, so the hook does not block them: a Redis client run in the cluster through `oc exec`, `oc rsh` or `oc debug`, because the in-cluster rule of check 2 covers only its listed commands; `argocd app patch-resource` with a patch that names finalizers or sets a Delete or Recycle reclaim policy, because the patch check reads only `oc patch`; and `argocd app confirm-deletion`, which approves the deletions that a `Prune=confirm` or `Delete=confirm` sync option holds back. Blocking any of them needs another maintainer's answer or an amending ADR first (ADR 0001 §6).
- The patch and port-forward checks read the command text: a patch from a file and a port taken from another command's output are not visible, like the other file-based paths of ADR 0024 §4.
- website_k8s stays on its older guardrails until its human-reviewed PR merges.

**Follow-up work**

- This PR: `.claude/settings.json`, the hook and its tests, the "Guardrails" section of `CLAUDE.md`, ADR 0024's Status line, and both ADR indexes.
- A website_k8s PR that a human reviews: section 7.

**What must be true in code and tests**

- `.claude/settings.json` equals the block of section 3, and the hook tests check it.
- The hook blocks every case of sections 4 and 5, allows the harmless cases and texts that only mention the commands, and its tests pass in CI.
- Every failure path of the hook still exits 2.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Accept the stricter checks as an editorial reading, with no ADR | decision register D23, option 2 | The ADR text and the hook would keep disagreeing, and the website_k8s copy would drift without a written target. |
| Relax the hook to the ADR's literal rules | decision register D23, option 3 | It reopens the bypasses the bootstrap closed, for example a local inline `PGHOST` that hides a remote `DATABASE_URL`. |
| Adopt the deny coverage and `onFailure` only, and keep the shell form | decision register D30, option 2 | The maintainer chose all candidates. The shell form also needs the project path quoted. |
| Defer | decision register D30, option 3 | A hook crash or timeout would still let a blocked command through, and the listed destructive commands would stay unguarded. |
| Accept the fifth recipe as an addition, with no ADR change | decision register D66, option 2 | ADR 0024 §2's list would keep lagging behind the skill. |
| Accept item 11 and (c) only | second decision register D69, option 2 | A Recycle patch, `argocd app delete-resource` or `argocd appset delete`, and a Redis port-forward would stay possible from an agent shell. |
| Drop item 11 from this ADR and relax the hook | second decision register D69, option 3 | The stdin and generated-delete gaps that #197 found would reopen. |
| Exec form with `bash` as `command` and the script in `args` | not offered; considered while writing this ADR | A client that predates `args` would start `bash` with the hook's JSON input as its script instead of the hook. |
| Block every `oc port-forward` | not offered; considered while writing this ADR | It would also block port-forwards to web services, Argo CD and metrics, which agents use to inspect the clusters. |

## References

- research: DECISIONS R9 (the decision register's answers of 2026-10-25: D23, D30, D66); research: DECISIONS R15 (the second decision register's answers of 2026-10-25: D69); research: DECISIONS 37, 43a
- `Verseghy/website_backend3#195` items 2, 8, 10, 11 and 12; `Verseghy/website_backend3#186`, `#197`, `#200`, `#203`, `#207`, `#208`; `Verseghy/website_k8s#54`
- `website_backend3@ad496da`: `.claude/settings.json`, `.claude/hooks/guard-bash.sh`, `scripts/tests/test-guard-bash.sh`, `.claude/skills/backend3/SKILL.md` (five recipes, the Context table), `.cargo/config.toml`; `website_backend3@c28ebd2:.claude/skills/backend3/SKILL.md` (seven recipes, section 2); `Verseghy/website_k8s@5b8ed44:.claude/settings.json`; `Verseghy/website_k8s@f0286dc`: `verseghy-website/postgres/deployment.yaml`, `verseghy-website/mysql/deployment.yaml`, `verseghy-website/backend2-redis/service.yaml`
- Claude Code docs: "Hooks reference" ("Exec form and shell form", "Block the action when a hook fails", exit code 2 on `PreToolUse`) and "Configure permissions" ("Wildcard patterns"); Claude Code changelog, 2.1.139 (`args`) and 2.1.295 (`onFailure`)
- Kubernetes docs: "Namespaces", "Finalizers", "Persistent Volumes" (Reclaiming); `openshift/api`: `project/v1/types.go`; oc 4.22: `oc port-forward --help`, `oc patch --help`
- `argoproj/argo-cd@17b98d9b6`: `cmd/argocd/commands/app.go` (also `NewApplicationConfirmDeletionCommand`), `cmd/argocd/commands/app_resources.go` (also `NewApplicationPatchResourceCommand`), `cmd/argocd/commands/applicationset.go`, `cmd/argocd/commands/root.go`, `docs/user-guide/sync-options.md` ("Resource Pruning With Confirmation", "Resource Deletion With Confirmation")
- MariaDB docs: "MariaDB Environment Variables"; Redis docs: "High availability with Redis Sentinel"
- Related ADRs: [0001](0001-record-architecture-decisions.md) §5, §6; [0010](0010-all-instants-stored-as-timestamptz.md) point 7; [0012](0012-delivery-review-commits-and-licensing.md); [0016](0016-no-redis-in-memory-cache-postgres-queue-cronjob-tasks.md) §4 and Consequences; [0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §2, §4
