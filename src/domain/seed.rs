//! The development, test and E2E seed: the one invented account that
//! `cargo loco db seed` creates, and the environments in which seeding may
//! run at all.
//!
//! Why it exists: the E2E suite signs in through the admin's sign-in screen,
//! and a developer who runs `cargo loco start` beside the Vite dev server
//! needs an account to sign in with. backend3 mounts no registration route
//! (ADR 0006 §4), so `Hooks::seed` (`src/app.rs`) creates this account
//! through the users model.
//!
//! The account is invented. Its name and address belong to no person
//! (`example.org` is reserved for examples, RFC 2606), and its password,
//! [`SEED_ACCOUNT`]`.password`, is published here on purpose: the E2E suite
//! types it into the sign-in screen (`admin/e2e/seed-account.ts` holds the
//! same values, and `tests/seed/mod.rs` checks that the two agree). Like the
//! compose database's `loco`/`loco`, it protects nothing: only a local
//! database ever holds the account (below). No hash of it is committed:
//! `Hooks::seed` hashes it with Argon2id when it runs, with a fresh salt.
//!
//! Where seeding runs: only in the environments whose database
//! `App::load_config` already requires to be local, development, test and
//! e2e ([`LOCAL_ONLY_ENVIRONMENTS`], ADR 0024 §4 layer 3). Never in
//! production, nor in staging, which runs the production configuration:
//! production holds no account before the cut-over import, which refuses to
//! run once `users` holds a row (data-migration.md §4.2), and its first
//! accounts come only from that import (ADR 0006 §4). A production seed would
//! leave a known account there and block the import. The maintainer accepted
//! that pre-import invariant on 2026-10-25, and ADR 0040 records the seed
//! and both refusals (research: DECISIONS R9 (D10)). The CLI pre-check
//! ([`super::cli_guard`]) refuses `db seed` in production before anything
//! loads; [`check_environment`] is the second wall, which `Hooks::seed` runs
//! on every call, whatever started it and whatever the environment is called.

use super::database_target::LOCAL_ONLY_ENVIRONMENTS;

/// The environments in which `Hooks::seed` runs: exactly those whose
/// database must be local ([`LOCAL_ONLY_ENVIRONMENTS`]). Seeding is tied to
/// that list on purpose, so the seed can only ever reach a database that the
/// database-target guard has already checked to be local.
pub const SEED_ENVIRONMENTS: &[&str] = LOCAL_ONLY_ENVIRONMENTS;

/// An account that the seed creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedAccount {
    /// The display name, `users.name`.
    pub name: &'static str,
    /// The sign-in address, `users.email`.
    pub email: &'static str,
    /// The plain-text password, which the seed hashes when it runs.
    pub password: &'static str,
}

/// The invented account of the development, test and E2E seed.
///
/// Its password is public and throwaway, as its text says, and at least 12
/// characters long, so the forced change of short passwords (ADR 0006 §6)
/// never applies to it. Production never holds this account
/// ([`check_environment`]).
pub const SEED_ACCOUNT: SeedAccount = SeedAccount {
    name: "Próba Adminisztrátor",
    email: "seed-admin@example.org",
    password: "public-throwaway-seed-password",
};

/// Why `Hooks::seed` refused to run. The `Display` text is the message the
/// operator or developer sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedRefusal {
    /// The environment name the seed was asked to run in.
    pub environment: String,
}

impl core::fmt::Display for SeedRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "Refusing to seed: the environment is {:?}, and backend3 seeds only {}. The seed \
             creates the development and E2E account, whose password is published. Production \
             holds no account before the cut-over import, which refuses to run once `users` \
             holds a row, and its first accounts come only from that import (ADR 0040, \
             data-migration.md §4.2). To seed a local \
             database, run `db seed` with LOCO_ENV set to one of those environments.",
            self.environment,
            SEED_ENVIRONMENTS.join(", ")
        )
    }
}

impl core::error::Error for SeedRefusal {}

/// Checks that seeding may run in `environment`.
///
/// - `environment`: the environment name as Loco resolved it, for example
///   `e2e` or `production`. The comparison is exact, as Loco's own is, so
///   `Production` or `e2e ` is an unknown environment and is refused.
///
/// # Errors
///
/// Returns a [`SeedRefusal`] for every environment that is not in
/// [`SEED_ENVIRONMENTS`].
pub fn check_environment(environment: &str) -> Result<(), SeedRefusal> {
    if SEED_ENVIRONMENTS.contains(&environment) {
        Ok(())
    } else {
        Err(SeedRefusal {
            environment: environment.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::domain::email_address;

    #[test]
    fn seeds_the_three_local_environments() {
        for environment in ["development", "test", "e2e"] {
            assert_eq!(check_environment(environment), Ok(()), "{environment}");
        }
    }

    #[test]
    fn refuses_production_staging_and_every_other_name() {
        for environment in [
            "production",
            "staging",
            "Production",
            "E2E",
            "e2e ",
            " test",
            "dev",
            "",
        ] {
            assert_eq!(
                check_environment(environment),
                Err(SeedRefusal {
                    environment: environment.to_owned()
                }),
                "{environment:?}"
            );
        }
    }

    #[test]
    fn the_seed_environments_are_the_local_only_ones() {
        assert_eq!(SEED_ENVIRONMENTS, ["development", "test", "e2e"]);
        assert!(!SEED_ENVIRONMENTS.contains(&"production"));
    }

    #[test]
    fn the_message_names_the_environment_the_reason_and_the_fix() {
        let message = check_environment("production")
            .expect_err("refused")
            .to_string();
        assert!(
            message.starts_with("Refusing to seed: the environment is \"production\""),
            "{message}"
        );
        assert!(
            message.contains("seeds only development, test, e2e"),
            "{message}"
        );
        assert!(message.contains("cut-over import"), "{message}");
        assert!(message.contains("ADR 0040"), "{message}");
        assert!(message.contains("LOCO_ENV"), "{message}");
        // The name is quoted, so an empty or padded name stays visible.
        let message = check_environment("").expect_err("refused").to_string();
        assert!(message.contains("environment is \"\","), "{message}");
    }

    #[test]
    fn the_seed_account_is_a_storable_invented_account() {
        assert_ne!(SEED_ACCOUNT.name.trim(), "");
        assert!(email_address::can_be_stored(SEED_ACCOUNT.email));
        assert!(
            SEED_ACCOUNT.email.ends_with("@example.org"),
            "an address of the RFC 2606 example domain, which belongs to no person"
        );
        // ADR 0006 §6: 12 to 128 characters, so no forced change applies.
        let length = SEED_ACCOUNT.password.chars().count();
        assert!((12..=128).contains(&length), "{length}");
        assert!(SEED_ACCOUNT.password.contains("public"));
    }

    proptest! {
        #[test]
        fn only_the_listed_names_are_seeded(environment in any::<String>()) {
            let allowed = SEED_ENVIRONMENTS.contains(&environment.as_str());
            match check_environment(&environment) {
                Ok(()) => prop_assert!(allowed),
                Err(refusal) => {
                    prop_assert!(!allowed);
                    prop_assert_eq!(refusal.environment, environment);
                }
            }
        }

        #[test]
        fn a_changed_letter_is_refused(
            index in 0usize..3,
            position in 0usize..11,
            letter in proptest::char::range('A', 'z'),
        ) {
            let name = SEED_ENVIRONMENTS[index];
            let position = position % name.len();
            let mut changed: Vec<char> = name.chars().collect();
            prop_assume!(changed[position] != letter);
            changed[position] = letter;
            let changed: String = changed.into_iter().collect();
            prop_assume!(!SEED_ENVIRONMENTS.contains(&changed.as_str()));
            prop_assert!(check_environment(&changed).is_err(), "{changed}");
        }
    }
}
