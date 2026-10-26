# 0034. Test hashes and throwaway keys, refused in production

- Status: Accepted (2026-10-25)
- Decision questions: none: records the maintainer's answer of 2026-10-25 to the decision-register item D75 (research: DECISIONS R15), which settles whether public-safety rule 1 allows the test hashes and throwaway keys that the first authentication slices commit. It amends rule 1 of the public-safety rules in [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §1 and in [ADR 0001](0001-record-architecture-decisions.md) §8.

## Context

Public-safety rule 1 is stated in three places:

| Where | What it says |
|---|---|
| [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §1, "Public-safety rules" | "no credentials, password hashes, tokens, keys, or values from any secret" |
| [ADR 0001](0001-record-architecture-decisions.md) §8, for the design docs | "No credentials, password hashes, tokens, keys, or any value taken from a secret." |
| `CLAUDE.md`, "Public-safety rules", for everything else | the same rule, with one exception: "The compose database's `loco`/`loco` are throwaway local values." |

Taken literally, the rule also forbids values that protect nothing. The first authentication slices commit four kinds of them:

| Value | Where | What it is | Slice |
|---|---|---|---|
| `DUMMY_PASSWORD_HASH` | `src/domain/password_hash.rs` | An Argon2id hash of a random value that was discarded when the hash was made. Sign-in verifies it when no account matches, so an unknown address costs what a wrong password costs ([ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §4). | `auth-users` (Verseghy/website_backend3#210) |
| `ARGON2I_OF_PASSWORD`, `ARGON2D_OF_PASSWORD` | `tests/models/users.rs` | Argon2i and Argon2d hashes of the invented test password "any password", which pin that the classifier refuses both variants. | `auth-users` |
| `auth.jwt.secret` of development, test and e2e | `config/development.yaml`, `config/test.yaml`, `config/e2e.yaml` | Each is the base64 of a sentence that names its environment and says that the key is public and throwaway; the comment above each key gives the sentence. | `auth-login-api` (this ADR's PR) |
| `CLI_JWT_SECRET` | `tests/support/mod.rs` | The same kind of value, for the child processes of the CLI tests that load `config/production.yaml`. | `auth-login-api` |

A third slice, `totp-crypto`, adds a published throwaway TOTP encryption key for development, test and e2e in the same way, and makes production refuse it at boot.

None of them is a credential. The test password is invented and written next to its hashes. The dummy hash's input no longer exists. A throwaway key signs only tokens for local databases, which hold invented accounts. Fixed values keep the configurations self-contained: development, test and e2e start with no secret to set, and a request test reads the key it verifies tokens with from the configuration. Fixed hashes are fixed test inputs.

The one real risk is a throwaway key copied into a deployment. `config/production.yaml` reads the JWT secret with `get_env(name="JWT_SECRET")` and no default ([ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §11), so production does not start without it (`tests/cli/session_secret.rs`). A throwaway value reaches a deployment only when an operator sets the variable to it. [ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §1 requires a base64 value of at least 64 random bytes from the shared `backend-secrets` ExternalSecret, which "never appears in `config/*.yaml` or in the image"; the deployment keys live in 1Password (research: DECISIONS R8). Neither ADR 0006 nor ADR 0023 asks backend3 to compare a configured key with the published ones.

The maintainer answered on 2026-10-25 (research: DECISIONS R15):

- D75, "Public-safety rule 1 forbids credentials, password hashes, tokens and keys. Does it allow committed hashes of invented test passwords? And does it allow fixed throwaway keys for development, test and e2e whose text says they are public?": "Allow them, and make production refuse the published keys". The register gave that option's consequence: the rule gains a written exception, like `loco`/`loco`'s, plus "a production boot check that refuses the known throwaway values. It can ship with D92's secret boot check." The register's context for D75 names this slice's fixed development, test and e2e JWT secrets, with no `get_env` override, among the values it asks about.
- D92, "Should two code-level guards be added? (1) Bring back the JWT secret boot check [...] (2) Refuse `doctor --config` in production in the CLI pre-check": "Both, in one small hardening slice before k8s-backend3-deploy".

Both answers took the recommended option.

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0024 §1, public-safety rule 1: "no credentials, password hashes, tokens, keys, or values from any secret" | The rule keeps its text and gains the exceptions of section 2. | 2 |
| ADR 0001 §8, rule 1: "No credentials, password hashes, tokens, keys, or any value taken from a secret." | The same exceptions. A design doc names such a value by its constant or setting; it has no reason to copy it. | 2 |
| Neither ADR asks production to refuse a committed key. | Production refuses every published key. | 3 |

Rules 2 to 4 of both sections, and everything else in ADR 0024 §1 and ADR 0001 §8, hold as written. `CLAUDE.md` states the exceptions in its rule 1 and links here.

**ADR 0006 §1 is read, not amended.** Its "Algorithm and secret" row says that the secret "never appears in `config/*.yaml` or in the image". That sentence is about the deployment's secret, the value from `backend-secrets`, and it still holds for that value. The throwaway keys of section 2 item 3 are not that secret: they sit in `config/development.yaml`, `config/test.yaml` and `config/e2e.yaml`, and so in the image, whose runtime stage copies all of `config/`. Section 3 makes production refuse them. This is a reading under ADR 0001 §5, where the maintainer's most recent answer comes first: D75 names these keys. ADR 0006 keeps its Status.

### 2. What may be committed

Rule 1 still forbids every credential, password hash, token and key, and every value taken from a secret. Beside the compose database's `loco`/`loco`, these values are allowed:

1. **A hash of an invented password**, in a test. The password was invented for the test and is never a password that a person or an account uses. The comment next to the hash names the password or says that it is invented.
2. **A hash of a discarded random value**, such as `DUMMY_PASSWORD_HASH`. The comment says that the input was discarded when the hash was made and that the value is no credential.
3. **A fixed throwaway key for development, test or e2e**: a key in `config/development.yaml`, `config/test.yaml` or `config/e2e.yaml`, or a test constant. The key's own text says that it is public and throwaway: it is the base64 of a sentence that says so. It is made for this repository, never copied from a secret store, and never set on a deployment.

These stay forbidden: any value from 1Password or from the `backend-secrets` ExternalSecret; a hash of a password that a person uses; a key or token of any deployment, staging included; a token or cookie signed with such a key.

### 3. Production refuses the published keys

- **What.** A key is published when this repository holds its value: every key that section 2 item 3 allows. backend3 refuses to start with the production configuration (every process that loads `config/production.yaml`) when a configured key equals a published key. The error names the variable, for example `JWT_SECRET`, and says to set a key from the secret store.
- **Which keys.** Every key that has a published throwaway value:
  - the TOTP encryption key, whose published value is `PUBLISHED_LOCAL_KEY` in `src/domain/totp_secret.rs`, the key of the three local configurations. `totp-crypto` refuses it itself, in its own PR, with `Settings::refuse_published_keys` in `src/settings.rs`;
  - the JWT secret, whose published values are the `auth.jwt.secret` of the three local configurations and `CLI_JWT_SECRET`. `secret-guards` adds them to the same refusal;
  - any later key whose slice publishes a throwaway value.
- **When.** At boot, before the server listens or a task runs. `totp-crypto` runs the refusal in `App::load_config`, so every command stops before it connects. D92's JWT secret check (base64 of at least 64 bytes, ADR 0006 §1) sits beside it. A deployment with a published key then fails at once and names the key to replace.
- **The production tests.** A test that starts the production configuration uses a key that no committed file holds, generated when the test runs. `totp-crypto`'s production tests already do so for the TOTP key (`tests/support::cli_totp_key`). `CLI_JWT_SECRET` is published, so `secret-guards`, which adds the JWT secret to the refusal, replaces it.
- **Who.** A slice that publishes a throwaway key makes production refuse it in the same PR. `totp-crypto` does so for the TOTP encryption key: it adds the refusal itself, and its error names `TOTP_ENCRYPTION_KEY` and says to set the key from the secret store (the `backend-secrets` key `backend3-totp-key`). The JWT secrets of this ADR's PR are the one exception: the `secret-guards` slice adds them to `totp-crypto`'s refusal rather than adding a second one, because D92 gave the JWT secret's boot check to that slice and the production CLI tests must first move off `CLI_JWT_SECRET`. The same slice adds D92's two guards: the JWT secret boot check and the refusal of `doctor --config` in production. `secret-guards` merges before `k8s-backend3-deploy`, the first slice that runs backend3 with the production configuration.

## Consequences

**Positive**

- The committed hashes and keys of `auth-users` and `auth-login-api` follow rule 1 as amended, and the merged `auth-users` needs no fix-up.
- Development, test and e2e keep starting with no secret to set, and the committed hashes stay fixed test inputs.
- A throwaway key copied into a deployment stops the boot and names the key, instead of signing real sessions or encrypting real secrets with a key that anyone can read.

**Negative**

- Rule 1 now has exceptions, so a reviewer checks the comment or the encoded sentence of every committed hash or key, not only its presence.
- The refused list must grow with every new throwaway key. The configuration test under "What must be true in code and tests" catches a key that a slice forgets to add.
- Until `secret-guards` merges, what keeps a published JWT secret out of a deployment is the header of `config/production.yaml`, which names the `backend-secrets` key that `JWT_SECRET` comes from, and the 1Password items (research: DECISIONS R8). No deployment runs the production configuration before then (section 3).

**Follow-up work**

- In this ADR's PR (`auth-login-api`): `CLAUDE.md` rule 1 states the exceptions and links here; the comments of the throwaway JWT secrets, `CLI_JWT_SECRET` and the test hashes cite this ADR; the Status lines of ADR 0024 and ADR 0001; both ADR indexes and the overview's ADR list.
- `totp-crypto`, the next slice: its throwaway TOTP key follows section 2 item 3 and cites this ADR. The same PR adds the refusal of section 3 and refuses that key at boot (`Settings::refuse_published_keys`, run by `App::load_config`), with its CLI test and production tests on a key generated at run time. `secret-guards` does not refuse the TOTP key again.
- `secret-guards`: the published JWT secrets join `totp-crypto`'s refusal, `CLI_JWT_SECRET` is replaced in the production tests, and the configuration test below is added, together with D92's JWT secret boot check and its refusal of `doctor --config` in production.

**What must be true in code and tests**

- For each key in the refusal, from the PR that adds it: a CLI test starts the production configuration with that key and asserts that it refuses to start and names the variable, and a test with a key generated at run time starts.
- From `secret-guards` on: a test reads every committed configuration other than `config/production.yaml`, and every key constant under `tests/`, and fails when one holds a key value that the refusal does not list.
- A mutation of the refusal, such as an empty list or a comparison that never matches, fails these tests.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Allow them, with a written exception | decision register D75 | Not chosen: it leaves the one real risk, a throwaway key copied into a deployment, to an operator rule. |
| Forbid them | decision register D75 | Not chosen: tests would generate hashes and keys at run time, or configs would read `get_env` with generated defaults. The merged `auth-users` would need a fix-up PR, `auth-login-api` and `totp-crypto` would change before merge, and the tests would lose fixed values for no protection gained. |

## References

- research: DECISIONS R15 (D75, D92), the answers of 2026-10-25; research: DECISIONS R8 (the deployment keys in 1Password)
- research: decision register D75 and D92 (the questions, their context and the options)
- Verseghy/website_backend3#210 (`auth-users`, which committed the hashes)
- Amended: [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §1, rule 1; [ADR 0001](0001-record-architecture-decisions.md) §8, rule 1
- Read, not amended: [ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §1, "Algorithm and secret" (section 1)
- Related: [ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §4; [ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §11; [ADR 0001](0001-record-architecture-decisions.md) §5 (precedence)
