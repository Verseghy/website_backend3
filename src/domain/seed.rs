//! The development, test and E2E seed: the one invented account and the
//! invented labels that `cargo loco db seed` creates, and the environments in
//! which seeding may run at all.
//!
//! Why it exists: the E2E suite signs in through the admin's sign-in screen,
//! and a developer who runs `cargo loco start` beside the Vite dev server
//! needs an account to sign in with. backend3 mounts no registration route
//! (ADR 0006 §4), so `Hooks::seed` (`src/app.rs`) creates this account
//! through the users model.
//!
//! The E2E suite also needs rows to list: the label list's tests page
//! through, sort and order labels (parity SHR-02, SHR-03; ADR 0020 §4). So
//! the seed also creates the invented labels of [`SEED_LABELS`] through the
//! labels model, in the same environments (recipe 1 step 11 of the project
//! skill: an E2E fixture comes from the seed, through the models). Each of
//! them also passes the rules of a label save
//! ([`super::labels::LabelInput::check`]), so the admin can edit a seeded
//! label like any other.
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

use super::{authz::Role, database_target::LOCAL_ONLY_ENVIRONMENTS};

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
    /// The roles the account holds (`user_roles`, ADR 0007 §3), written in
    /// the same transaction as the account.
    pub roles: &'static [Role],
}

/// The invented account of the development, test and E2E seed.
///
/// Its password is public and throwaway, as its text says, and at least 12
/// characters long, so the forced change of short passwords (ADR 0006 §6)
/// never applies to it. It holds the `admin` role, so it holds every
/// permission and can reach every admin screen that the E2E suite and a
/// developer open (ADR 0007 §2). Production never holds this account
/// ([`check_environment`]).
pub const SEED_ACCOUNT: SeedAccount = SeedAccount {
    name: "Próba Adminisztrátor",
    email: "seed-admin@example.org",
    password: "public-throwaway-seed-password",
    roles: &[Role::Admin],
};

/// A label that the seed creates (`labels`, parity LBL-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedLabel {
    /// The label's name, `labels.name`.
    pub name: &'static str,
    /// The label's colour, `labels.color`: `#rrggbb` in upper-case hex
    /// digits.
    pub color: &'static str,
}

/// One entry of [`SEED_LABELS`], written as a call so that rustfmt keeps
/// each label on one line.
///
/// - `name`: the label's name.
/// - `color`: its `#rrggbb` colour.
const fn label(name: &'static str, color: &'static str) -> SeedLabel {
    SeedLabel { name, color }
}

/// The invented labels of the development, test and E2E seed, in the order
/// the seed writes them, so their ids grow down the list.
///
/// The E2E suite pages through and sorts the label list over them
/// (`admin/e2e/labels.spec.ts`; `admin/e2e/seed-labels.ts` holds the same
/// values, and `tests/seed/mod.rs` checks that the two agree), so the list
/// is shaped for those checks:
///
/// - 30 labels: more than one page of the default 25 (parity SHR-02), and
///   no more than the largest page of 100, so one page shows them all;
/// - five colours, each shared by six labels, so a sort by colour has ties
///   that only the id breaks (SHR-03);
/// - ADR 0020's ten words, whose Hungarian order (`hu_icu`) no other
///   collation gives, and 20 numbered „Próbacímke” labels.
///
/// Every label also passes the rules of a label save as written
/// ([`super::labels::LabelInput::check`]: a trimmed name of 1 to
/// [`super::labels::NAME_MAX_CHARS`] characters, a `#rrggbb` colour), so
/// an edit that changes nothing saves it unchanged instead of answering
/// `422` (parity LBL-02, SHR-13).
///
/// Every name is invented and belongs to no person or real label.
pub const SEED_LABELS: &[SeedLabel] = &[
    label("ábra", "#1E88E5"),
    label("alma", "#43A047"),
    label("cukor", "#E53935"),
    label("csiga", "#FDD835"),
    label("olló", "#8E24AA"),
    label("óra", "#1E88E5"),
    label("öröm", "#43A047"),
    label("őz", "#E53935"),
    label("zúgó", "#FDD835"),
    label("zsák", "#8E24AA"),
    label("Próbacímke 01", "#1E88E5"),
    label("Próbacímke 02", "#43A047"),
    label("Próbacímke 03", "#E53935"),
    label("Próbacímke 04", "#FDD835"),
    label("Próbacímke 05", "#8E24AA"),
    label("Próbacímke 06", "#1E88E5"),
    label("Próbacímke 07", "#43A047"),
    label("Próbacímke 08", "#E53935"),
    label("Próbacímke 09", "#FDD835"),
    label("Próbacímke 10", "#8E24AA"),
    label("Próbacímke 11", "#1E88E5"),
    label("Próbacímke 12", "#43A047"),
    label("Próbacímke 13", "#E53935"),
    label("Próbacímke 14", "#FDD835"),
    label("Próbacímke 15", "#8E24AA"),
    label("Próbacímke 16", "#1E88E5"),
    label("Próbacímke 17", "#43A047"),
    label("Próbacímke 18", "#E53935"),
    label("Próbacímke 19", "#FDD835"),
    label("Próbacímke 20", "#8E24AA"),
];

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

    #[test]
    fn the_seed_account_is_an_admin_with_every_permission() {
        // The E2E suite and a local developer reach every admin screen with
        // it, whatever permission a screen needs (ADR 0007 §2).
        assert_eq!(SEED_ACCOUNT.roles, [Role::Admin]);
        assert!(crate::domain::authz::is_admin(
            SEED_ACCOUNT.roles.iter().copied()
        ));
        assert_eq!(
            crate::domain::authz::effective_permissions(SEED_ACCOUNT.roles.iter().copied(), [])
                .len(),
            crate::domain::authz::Permission::ALL.len()
        );
    }

    /// Whether `color` is `#rrggbb` in upper-case hex digits, a colour the
    /// `labels` table's CHECK accepts written one way only, so that two
    /// seed labels share a colour exactly when their text is equal.
    fn is_upper_hex_colour(color: &str) -> bool {
        color.len() == 7
            && color.starts_with('#')
            && color[1..]
                .chars()
                .all(|digit| digit.is_ascii_digit() || ('A'..='F').contains(&digit))
    }

    #[test]
    fn the_seed_labels_are_storable_and_named_once() {
        let mut names = std::collections::BTreeSet::new();
        for label in SEED_LABELS {
            assert_eq!(label.name.trim(), label.name, "{label:?}");
            assert!(!label.name.is_empty(), "{label:?}");
            assert!(is_upper_hex_colour(label.color), "{label:?}");
            // The live labels' unique index is on lower(name) (labels
            // migration), so no two names may differ only in case.
            assert!(names.insert(label.name.to_lowercase()), "{label:?}");
        }
    }

    #[test]
    fn every_seed_label_passes_the_label_save_rules_unchanged() {
        // The admin saves a seeded label through the same rules as any other
        // (`LabelInput::check`, parity LBL-02, SHR-13). A seed label that
        // broke them would answer `422` to an edit that changes nothing, and
        // one that the check rewrites (trims) would change on that save.
        for label in SEED_LABELS {
            let checked = crate::domain::labels::LabelInput::check(label.name, label.color);
            assert_eq!(
                checked.as_ref().map(|input| (input.name(), input.color())),
                Ok((label.name, label.color)),
                "{label:?}"
            );
        }
    }

    #[test]
    fn the_seed_labels_fill_more_than_one_default_page_and_fit_on_the_largest() {
        // The list contract's sizes (domain::list_query, parity SHR-02).
        let default_page = usize::from(crate::domain::list_query::DEFAULT_PER_PAGE);
        let largest_page = crate::domain::list_query::PAGE_SIZES
            .into_iter()
            .map(usize::from)
            .max()
            .expect("page sizes");
        assert!(SEED_LABELS.len() > default_page, "{}", SEED_LABELS.len());
        assert!(SEED_LABELS.len() <= largest_page, "{}", SEED_LABELS.len());
    }

    #[test]
    fn every_seed_colour_is_shared_so_a_colour_sort_has_ties() {
        // SHR-03: a sort by colour has ties, which only the id breaks.
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        for label in SEED_LABELS {
            *counts.entry(label.color).or_default() += 1;
        }
        assert!(counts.len() > 1, "{counts:?}");
        assert!(counts.values().all(|&count| count >= 2), "{counts:?}");
    }

    #[test]
    fn the_seed_labels_hold_adr_0020s_ten_words() {
        // ADR 0020, "Ordering of the same ten words": the order only `hu`
        // gives, which the E2E suite checks on the label list.
        for word in [
            "ábra", "alma", "cukor", "csiga", "olló", "óra", "öröm", "őz", "zúgó", "zsák",
        ] {
            assert!(SEED_LABELS.iter().any(|label| label.name == word), "{word}");
        }
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
