# 0039. `doctor --config` refused in production

- Status: Accepted (2026-10-25)
- Decision questions: none: records the maintainer's answer of 2026-10-25 to the decision-register item D92 (research: DECISIONS R15), which settles two code-level guards. Its second guard makes the CLI pre-check of [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §4, layer 3, refuse one more command, so this ADR amends that bullet. D23 set the precedent that a guard stricter than its ADR is recorded in an amending ADR ([ADR 0028](0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md)).

## Context

[ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §4, layer 3, gives `backend3-cli` a pre-check that inspects its arguments before Loco parses them: "When the environment resolves to `production` (from `-e` or `--environment`, else from `loco_rs::environment::resolve_from_env()`), it refuses `db reset`, `db truncate` and `db seed --reset`, which wipe data." `cargo loco` runs the same binary, so the pre-check covers agents and operators alike. One more command does harm in production, by printing rather than by deleting:

| Fact | Value | Source |
|---|---|---|
| `doctor --config` prints the whole configuration | For `doctor --config` (`-c`), `cli::main` loads the configuration through `Hooks::load_config`, builds the app context, which connects to the database, and prints `Config`'s `Display`: the whole rendered configuration as YAML, `settings` included, and then the environment's name. | loco-rs 1.2.0 `src/cli.rs` (`cli::main`, `Commands::Doctor`), `src/config/mod.rs` (`impl Display for Config`) |
| What that configuration holds in production | `database.uri` and `queue.uri`, with the password of the `website_backend3` role (`DATABASE_URL`); `auth.jwt.secret`, the admin session secret (`JWT_SECRET`); and `settings.admin_auth.totp_encryption_key` (`TOTP_ENCRYPTION_KEY`). `config/production.yaml` renders each from the environment. | `config/production.yaml`; [ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §11 |
| `doctor --production` selects production | `doctor`'s deprecated `--production` (`-p`) flag makes the environment `production` after `-e` and `LOCO_ENV` have been read, whatever they say, with a deprecation warning. | loco-rs 1.2.0 `src/cli.rs` (`cli::main`) |
| `doctor` without `--config` | It runs Loco's checks (the database and queue connections, the production settings, the installed `sea-orm-cli`, the Loco version) and prints one line of result for each, with no configuration value. | loco-rs 1.2.0 `src/doctor.rs` |
| The other `--config` | `middleware --config` prints the settings of the enabled HTTP middlewares, `server.middlewares`. backend3's configurations hold no secret there. | loco-rs 1.2.0 `src/cli.rs` (`Commands::Middleware`); `config/production.yaml` |
| Where secret values never appear | "No secret value appears in Git, in the image, in CI or in logs." The output of a command goes wherever the operator's terminal sends it: its scrollback, a Job's log, a message pasted when asking for help. | [ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §11, "Where values never appear" |
| Until this ADR | The header of `config/production.yaml` asked operators not to run `backend3-cli doctor --config` there. No production deployment runs the configuration yet: `k8s-backend3-deploy` is the first, and D92's slice merges before it ([ADR 0034](0034-test-hashes-and-throwaway-keys-refused-in-production.md) §3, "Who"). | `config/production.yaml`; ADR 0034 §3 |

The maintainer answered on 2026-10-25, with the recommended option (research: DECISIONS R15). The register's question, the answer, and the consequence the register gave for that option (research: decision register D92):

- **D92**, "Should two code-level guards be added? (1) Bring back the JWT secret boot check that auth-login-api removed: expiration 7200, token locations cookie then Bearer, and a base64 secret of at least 64 bytes. (2) Refuse `doctor --config` in production in the CLI pre-check, because it prints the whole configuration, including DATABASE_URL and the TOTP key.": "Both, in one small hardening slice before k8s-backend3-deploy". Its consequence: "Misconfiguration fails at boot with the key named, and the one command that prints every secret is refused. The CLI part needs an amending ADR for ADR 0024 §4."

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0024 §4, layer 3, "The CLI pre-check": "it refuses `db reset`, `db truncate` and `db seed --reset`, which wipe data" | It also refuses `doctor --config`, which prints every secret of the configuration. | 2 |
| The same bullet: the environment resolves "from `-e` or `--environment`, else from `loco_rs::environment::resolve_from_env()`" | For `doctor`, `--production` resolves it to `production` too, as Loco does. | 2 |
| ADR 0024, Consequences, "What must be true in code and tests" | One item is added. | Consequences, "What must be true in code and tests" |

Everything else in ADR 0024 holds as written, the other two layers included. ADR 0024's "Negative and risks" item "The code-level guard covers only databases" now has one exception, this refusal.

D92's first guard, the JWT secret boot check, amends no decision: [ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §1 fixes the secret's format, the lifetime and the token locations, and ADR 0034 §3 ("When") places the check beside the published-key refusal in `App::load_config`. Section 3 says where it is built, so that both halves of the answer are found in one place.

### 2. The pre-check refuses `doctor --config` in production

- **What.** `backend3-cli` refuses `doctor` with `--config` or `-c` when the environment resolves to `production`. A cluster of short flags counts (`doctor -cp`), and so does `--config` or `--production` with an inline value, which clap refuses anyway: the check errs on the side of refusing, as it does for `db seed --reset`.
- **Which environment.** As for the other refused commands: the `-e` or `--environment` option, else `LOCO_ENV`, `RAILS_ENV` or `NODE_ENV`; any `production` among repeated `-e` options counts. `doctor --production` (`-p`) resolves to `production` whatever those say, because loco-rs 1.2.0 switches to production after reading them.
- **When and how.** In `src/bin/main.rs`, before Loco parses the arguments, through the pure `src/domain/cli_guard.rs` (ADR 0008). Nothing has been loaded or connected, so the message holds no configuration value. It exits with code 2, like the other refusals, names where `production` came from, says that the command prints the database URL, the session secret and the TOTP key, and says to run `doctor` without `--config`.
- **What stays allowed.** `doctor` without `--config` in production, which checks the configuration and the connections without printing them, and `doctor --config` outside production, where the configurations hold only the throwaway values of ADR 0034 §2. `middleware --config` stays allowed; it prints no secret.
- **The operator rule.** The header of `config/production.yaml` points at the refusal instead of asking operators not to run the command.

### 3. The JWT secret boot check (no amendment)

`App::load_config` runs `settings::check_session_jwt` in every environment, next to the typed settings: `auth.jwt` must exist, its secret must be base64 of at least 64 bytes (`src/domain/configured_key.rs`, the decoder that the TOTP key uses too), its expiration must be 7,200 s and its locations must be the cookie `__Host-admin_session`, then `Bearer`, and nothing else (`src/domain/session_jwt.rs`). The error names the key and, for the secret, the variable `JWT_SECRET`, and never contains the value. In production the decoded secret then goes to `Settings::refuse_published_keys`, which refuses the published JWT secrets of ADR 0034 §3 as well as the TOTP key.

## Consequences

**Positive**

- The one command that prints every production secret cannot run in production, by mistake or by an agent, in either of its forms.
- The operator rule in `config/production.yaml` becomes a guard that tests pin.
- A bad or published session secret stops every command at boot with the key named, instead of a `500` at the first sign-in.

**Negative**

- An operator who needs to see a rendered production value reads it from where it comes from, the Deployment's manifest or the secret store, not from `doctor --config`.
- The pre-check knows a little more of Loco's CLI grammar (`doctor`'s two flags). A Loco bump that adds an option to `doctor`, or another command that prints the configuration, needs the pre-check and this ADR reviewed; the Loco bump PRs already re-read the CLI (ADR 0024 §3).

**Follow-up work**

- In this ADR's PR (`secret-guards`): the refusal, its unit and CLI tests, the header of `config/production.yaml`, `CLAUDE.md`'s guardrail layer 3, the Status line of ADR 0024, both ADR indexes, and D92's JWT secret boot check with the published JWT secrets of ADR 0034 §3.

**What must be true in code and tests**

ADR 0024's list, under its Consequences, gains the first item; the others are this ADR's own.

- With `LOCO_ENV=production`, `backend3-cli doctor --config` (and `-c`, and `doctor --config --production` with `LOCO_ENV=development`) exits with code 2 before it connects and prints nothing on stdout, while `doctor` alone does connect.
- The pre-check's unit tests and properties cover every spelling, the environment sources and the commands that stay allowed.
- A test runs `doctor --config` in the `test` environment and asserts that it prints the session secret, the TOTP key and the database password, so that a Loco release that stops printing them shows that this refusal can be revisited.
- The JWT secret boot check of section 3 refuses each unusable secret, a changed lifetime and a changed token location, naming the key and never the value, and production refuses each published JWT secret (ADR 0034 §3 and its "What must be true in code and tests").

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Only the JWT boot check | decision register D92 | Not chosen: `doctor --config` would stay an operator rule. |
| Only the `doctor --config` refusal | decision register D92 | Not chosen: a bad JWT secret would still appear only at the first sign-in. |
| Neither | decision register D92 | Not chosen: operator rules only. |

## References

- research: DECISIONS R15 (D92), the answer of 2026-10-25; research: decision register D92 (the question, its context and the options)
- loco-rs 1.2.0 (crates.io): `src/cli.rs` (`cli::main`, `Commands::Doctor`, `Commands::Middleware`), `src/config/mod.rs` (`impl Display for Config`), `src/doctor.rs`
- Amended: [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §4, layer 3, "The CLI pre-check", and its Consequences, "What must be true in code and tests"
- Related: [ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §1; [ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §11; [ADR 0028](0028-guardrails-as-built-wider-checks-and-a-fail-closed-hook.md) (the D23 precedent); [ADR 0034](0034-test-hashes-and-throwaway-keys-refused-in-production.md) §3
