# 0040. Local seed account, and every `db seed` refused in production

- Status: Accepted (2026-11-01)
- Decision questions: none: records the maintainer's answer of 2026-10-25 to the decision-register item D10 (research: DECISIONS R9), the pre-import production invariant, and the answer of 2026-11-01 to the third register's item D120 (research: DECISIONS R24), which allows the seed account's published password. One of the guards that the answer to D10 accepted refuses `db seed` in production, so the CLI pre-check of [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §4, layer 3, refuses more than that bullet lists, and the seed creates accounts in a way that [ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §4 does not list. This ADR amends both. D23 set the precedent that a guard stricter than its ADR is recorded in an amending ADR ([ADR 0028](0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md)). The answer to D120 adds a fourth item to the values that public-safety rule 1 allows, so this ADR also amends [ADR 0034](0034-test-hashes-and-throwaway-keys-refused-in-production.md) §2.

## Context

The admin's sign-in slice puts every admin page but the sign-in screen behind a session. Its browser tests, and a developer who runs the admin against a local backend3, then need an account to sign in with, and backend3 has no registration route.

| Fact | Value | Source |
|---|---|---|
| Who creates accounts | "Accounts are created only by `edit users` holders ([ADR 0007](0007-authorization-four-roles-enforced-on-every-operation.md)) and by the account-creation task of parity item AUTH-09 ([parity.md](../parity.md)), which an operator runs on a fresh development or staging database. In production the first accounts come only from the import." Neither is available to a fresh E2E database: an `edit users` holder needs an account first, and the AUTH-09 task is a later slice. | ADR 0006 §4, "Starter routes are not mounted" |
| What Loco's `db seed` does | `db seed` calls `Hooks::seed` with the `--from` directory (default `src/fixtures`). `--reset` (`-r`) first resets the database. `--dump` (`-d`) and `--dump-tables` do not seed: they write the rows of every table, or of the named ones, to files under the `--from` directory. | loco-rs 1.2.0 `src/cli.rs` (`DbCommands::Seed`), `src/boot.rs` (`run_db`, `RunDbCommand::Seed`) |
| What the pre-check refuses | "When the environment resolves to `production` (...), it refuses `db reset`, `db truncate` and `db seed --reset`, which wipe data." | ADR 0024 §4, layer 3, "The CLI pre-check" |
| The import needs empty targets | The import task refuses to start when "any table the import writes already holds a row". `users` is one of those tables. | [data-migration.md](../data-migration.md) §4.2 |
| Which environments are local | `App::load_config` refuses a database that is not local in `development`, `test` and `e2e`. Production and staging both run `LOCO_ENV=production`. | ADR 0024 §4, layer 3; `src/domain/database_target.rs` (`LOCAL_ONLY_ENVIRONMENTS`) |
| What public-safety rule 1 allows | Beside the compose database's `loco`/`loco`: a hash of an invented password, in a test; a hash of a discarded random value; and a fixed throwaway key for development, test or e2e whose own text says that it is public and throwaway. None of them is a password in plain text, though the comment next to the first may name the invented password that it hashes. | [ADR 0034](0034-test-hashes-and-throwaway-keys-refused-in-production.md) §2; `CLAUDE.md`, "Public-safety rules" |

The maintainer answered on 2026-10-25 with the recommended option (research: DECISIONS R9). The register's question and the answer (research: decision register D10):

- **D10**, "Do you accept that nothing writes a row into users, activity_log, revisions or any other import or seed target in production before the cut-over import, so that no production account exists before the window?": "Accept the invariant as stated". The register listed the guards that the answer accepts, the first of them "`db seed` is refused in production (admin-sign-in)", and gave this consequence: "The slices implement the guards above. Production stays empty, with no admin access, until the window."

Building the seed account put its password, in plain text, in three committed files (section 2), and none of the values that rule 1 allows is such a password. Under [ADR 0001](0001-record-architecture-decisions.md) §6 that needed the maintainer's answer before this ADR merged. The third decision register asked about it as D120, and the maintainer answered on 2026-11-01 with the recommended option (research: DECISIONS R24). Where the question names the password, the quote leaves it out, marked "(...)": a design doc names such a value by its constant, `SEED_ACCOUNT.password`, and does not copy it (ADR 0034 §1). The item below gives the register's question, the answer, and the consequence the register gave for that option (research: third decision register D120):

- **D120**, "admin-sign-in commits the seed account's invented password (...) in plain text, in src/domain/seed.rs (and so in every image), in admin/e2e/seed-account.ts and in AGENTS.md. Public-safety rule 1's exceptions (ADR 0034 §2) do not name such a password. Allow it in an amending ADR, or have the E2E job generate the password at run time?": "Allow it, in an amending ADR". Its consequence: "ADR 0040 (in this PR, not yet merged) also amends ADR 0034 §2 with a fourth item: the password of the invented local seed account, whose text says it is public and throwaway, in src/domain/seed.rs, admin/e2e/seed-account.ts and AGENTS.md. The exception holds only while production refuses every seed (ADR 0040 §3-§4). ADR 0040's header names D120, and CLAUDE.md rule 1, AGENTS.md ("The seed account"), ADR 0034's Status line, both ADR indexes and the overview's ADR paragraph are updated to match. No code changes: the two refusals do for this password what D75's production key refusal does for the keys. Local sign-in still takes one `db seed`, and the stacked wave-6 slices need no change."

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0006 §4, "Starter routes are not mounted": "Accounts are created only by `edit users` holders (...) and by the account-creation task of parity item AUTH-09" | Accounts are also created by `Hooks::seed`, in development, test and e2e only: one invented account. "In production the first accounts come only from the import" holds as written. | 2 |
| ADR 0024 §4, layer 3, "The CLI pre-check": "it refuses `db reset`, `db truncate` and `db seed --reset`, which wipe data" | It refuses `db reset`, `db truncate` and every `db seed`, in any form. | 3 |
| ADR 0024 §4, layer 3: the code-level guards | `Hooks::seed` refuses every environment but development, test and e2e. | 4 |
| ADR 0024, Consequences, "Negative and risks": "the CLI pre-check refuses it, with `db truncate` and `db seed --reset`, in `production`" | It refuses it with `db truncate` and every `db seed`. | 3 |
| ADR 0024, Consequences, "What must be true in code and tests" | Two items are added. | Consequences |
| ADR 0034 §2, "What may be committed": the three values allowed "beside the compose database's `loco`/`loco`" | A fourth item: the seed account's password, in three files, only while production refuses every seed (D120). | 5 |

Everything else in ADR 0006, ADR 0024 and ADR 0034 holds as written.

### 2. The seed account

- **What.** `Hooks::seed` (`src/app.rs`), which `cargo loco db seed` and `backend3-cli db seed` run, creates one account, `SEED_ACCOUNT` in `src/domain/seed.rs`, through `users::Model::seed_account`. The account is invented: its name belongs to no person, and its address is in `example.org`, the domain reserved for examples (RFC 2606). It is active, has no two-factor and needs no password change, so the sign-in gives it a full session.
- **The password.** The browser tests type the password into the sign-in screen, so it is in the repository: in `src/domain/seed.rs`, in `admin/e2e/seed-account.ts` for the tests, which a Rust test keeps equal, and in `AGENTS.md` ("The seed account") for a developer who signs in locally. Its own text says that it is public and throwaway. Public-safety rule 1 allows it in those three files (section 5). No hash of it is committed: the seed hashes it with Argon2id when it runs, with a fresh salt.
- **Idempotent.** When an account already has the address, in whatever state, the seed leaves it exactly as it is, so seeding twice gives the result of seeding once. To start over, the database is recreated.
- **Fixture files.** backend3 seeds no fixture files from `--from`: its entities derive no serde, so Loco's YAML loader cannot read them, and seeds are built through the models.

### 3. The pre-check refuses every `db seed` in production

- **What.** When the environment resolves to `production`, as ADR 0024 §4 defines it, `backend3-cli` refuses `db seed` with any options: plain, `--reset` or `-r`, `--from`, `--dump` or `-d`, and `--dump-tables`. `db reset` and `db truncate` stay refused, and `db down` stays allowed.
- **Why the seed.** It would create an account with a published password in production, before the import, and leave `users` holding a row, which the import then refuses (data-migration.md §4.2). The invariant that D10 accepted forbids both.
- **Why `--dump` and `--dump-tables`.** They share the subcommand and its options. They write every table, password hashes included, to files inside the container. A backup follows the operator runbook's procedure, never a file in a pod.
- **When and how.** In `src/bin/main.rs`, before Loco parses the arguments, through the pure `src/domain/cli_guard.rs` (ADR 0008), as for the other refused commands. It exits with code 2 before anything loads or connects. The message names where `production` came from, says that backend3 never seeds production and why, and names the backup procedure for the `--dump` forms.

### 4. `Hooks::seed` refuses every other environment

- **What.** Before it touches the database, `Hooks::seed` runs `domain::seed::check_environment`. It allows exactly the environments whose database `App::load_config` already requires to be local, development, test and e2e, and refuses every other name, `production` and `staging` included, by exact comparison.
- **Why a second wall.** The pre-check sees only the command line of `backend3-cli`. The hook also stops a seed that reaches it another way, such as Loco's testing helper `loco_rs::testing::db::seed`, or an environment with a name other than `production`. Tying the list to the local-only environments means the seed can only reach a database that the database-target guard has already checked to be local.
- **The message** names the environment, the environments that may be seeded, the reason and the fix (set `LOCO_ENV` to one of them).

### 5. Public-safety rule 1 allows the seed account's password

- **What.** The list of [ADR 0034](0034-test-hashes-and-throwaway-keys-refused-in-production.md) §2, "What may be committed", gains a fourth item after its three:
  4. **The password of the invented local seed account**, `SEED_ACCOUNT.password` in `src/domain/seed.rs`, and its copies in `admin/e2e/seed-account.ts` and in `AGENTS.md` ("The seed account"). The account is the one of section 2, and the password's own text says that it is public and throwaway. It is made for this repository, never copied from a secret store, and never a password that a person uses.
- **Where.** In those three files only. Anywhere else the password is named by its constant, as a design doc names every value of ADR 0034 §2 (ADR 0034 §1); this ADR quotes D120 without it. None of the three files is a design doc, so what [ADR 0001](0001-record-architecture-decisions.md) §8 lets a design doc hold does not change.
- **The condition.** The exception holds only while production refuses every seed: the pre-check of section 3 refuses every `db seed` in production, and `Hooks::seed` refuses every environment but development, test and e2e (section 4). A change that would let a seed reach production or staging would end the exception, so it contradicts this ADR and needs the maintainer's answer first (ADR 0001 §6).
- **Why.** It follows the answer to D75 (ADR 0034): allow a value that protects nothing, and make production refuse it. The password protects nothing: it belongs to an invented account at an `example.org` address that only local databases hold. The two refusals do for this password what ADR 0034 §3's refusal of the published keys does for the keys, so the password needs no code change of its own.
- **What stays forbidden.** Everything that ADR 0034 §2 forbids: any value from 1Password or from the `backend-secrets` ExternalSecret, a hash of a password that a person uses, a key or token of any deployment, staging included, and a token or cookie signed with such a key.

Rule 1 of ADR 0024 §1 takes its exceptions from ADR 0034 §2 (ADR 0034 §1), so it covers this item without being amended itself. `CLAUDE.md` states the item in its rule 1.

## Consequences

**Positive**

- The browser tests sign in the way an editor does, through the sign-in screen, against an account that exists in every fresh E2E database.
- A developer gets an account on a local database with one command.
- No production command, by an operator or an agent, can create the seed account, and none can block the cut-over import by writing into `users` through the seed. Production stays empty until the import, as D10 asks.
- No `db seed --dump` can write the production tables, password hashes included, into a pod.
- The seed account's password follows public-safety rule 1 as amended (section 5). A local seed or an E2E run sets no variable, and a kept local database signs in with the same password as a fresh one.

**Negative**

- The seed account's password is in the repository in plain text, and the binary of every environment carries it. Rule 1 allows it only because the two refusals keep the account out of every database but the local ones (section 5).
- Rule 1 has one more exception, so a reviewer also checks that the password appears in no file but the three of section 5, and that a change weakens neither refusal.
- An operator who wants a dump of production tables uses the runbook's backup procedure, not `db seed --dump`.
- Nothing that production needs can be built on `db seed`. Content that the cut-over seeds, the release-1 seed of [ADR 0019](0019-scope-parity-tracking-retired-features-release-1-additions.md), runs as a task of its own ([cutover.md](../cutover.md) §13 item 6, [data-migration.md](../data-migration.md) §13 item 7), so this refusal does not touch it.
- The pre-check knows one more part of Loco's CLI grammar. A Loco bump that adds a `db` subcommand or a `db seed` option needs the pre-check reviewed; the Loco bump PRs already re-read the CLI (ADR 0024 §3).

**Follow-up work**

- In this ADR's PR (`admin-sign-in`): the seed account, both refusals with their unit and CLI tests, `AGENTS.md` ("The seed account"), `CLAUDE.md`'s public-safety rule 1 and guardrail layer 3, the header of `config/production.yaml`, the Status lines of ADR 0006, ADR 0024 and ADR 0034, both ADR indexes and the overview's ADR paragraph.

**What must be true in code and tests**

ADR 0024's list, under its Consequences, gains the first two items; the others are this ADR's own.

- With `LOCO_ENV=production`, `backend3-cli db seed`, `db seed --from <dir>`, `db seed --dump` and `db seed --dump-tables <tables>` exit with code 2 before they connect, while an allowed command such as `db status` does connect.
- `Hooks::seed` refuses `production`, `staging` and every other name but development, test and e2e, and writes no row when it refuses.
- The pre-check's unit tests and properties cover every form of `db seed` and the environment sources; `check_environment` has unit tests and a property over arbitrary names.
- Seeding twice leaves one account; an existing account with the seed address, trashed or inactive, is left unchanged; the seed account signs in through `/api/admin/auth/login`; and the E2E suite's copy of the account equals `SEED_ACCOUNT`.
- `SEED_ACCOUNT` has an `example.org` address, and its password is 12 to 128 characters long and says that it is public (section 5).

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Allow a production admin account before the window | decision register D10 | Not chosen: the import refuses a non-empty `users` table, and the window would need a drop-and-recreate before the import, one more step that can go wrong. |
| Refuse only `db seed --reset` in production, as ADR 0024 §4 lists | ADR 0024 §4 | A plain `db seed` would create the seed account in production and break the invariant D10 accepted. |
| Generate the seed password when the E2E job runs, from a new setting | third decision register D120 | Not chosen: every local seed and E2E run would need one more variable, and the seed leaves an existing account as it is (section 2), so on a kept local database sign-in would fail unless the seed also reset that password or the database were recreated. It adds no protection: only local databases ever hold the account. |

## References

- research: DECISIONS R9 (D10, D23); research: decision register D10
- research: DECISIONS R24 (D120), the answer of 2026-11-01; research: third decision register D120 (the question, its context and the options)
- [data-migration.md](../data-migration.md) §4.2
- loco-rs 1.2.0 (crates.io): `src/cli.rs` (`DbCommands::Seed`), `src/boot.rs` (`run_db`), `src/testing/db.rs` (`seed`)
- RFC 2606, "Reserved Top Level DNS Names" (`example.org`)
- Amended: [ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §4; [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §4, layer 3, and its Consequences; [ADR 0034](0034-test-hashes-and-throwaway-keys-refused-in-production.md) §2
- Related ADRs: [0001](0001-record-architecture-decisions.md) (§6, §8), [0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md), [0028](0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md), [0039](0039-doctor-config-refused-in-production.md) (which also amends the CLI pre-check, with `doctor --config`)
