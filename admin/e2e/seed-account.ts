/**
 * The account that `LOCO_ENV=e2e cargo loco db seed` creates in the E2E
 * database: `SEED_ACCOUNT` of `src/domain/seed.rs`, which the E2E suite
 * types into the sign-in screen.
 *
 * The account is invented and its password is published on purpose: only the
 * local development, test and e2e databases ever hold it, because the seed
 * refuses every other environment and the CLI refuses `db seed` in
 * production. `tests/seed/mod.rs` fails when these lines differ from the Rust
 * constant, so keep each field on its own line, as written.
 */
export const SEED_ACCOUNT = {
  name: "Próba Adminisztrátor",
  email: "seed-admin@example.org",
  password: "public-throwaway-seed-password",
} as const
