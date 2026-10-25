//! The PostgreSQL advisory-lock key of a Loco task (ADR 0016 §4).
//!
//! PostgreSQL names an advisory lock by one `bigint`, so every task's lock
//! (`src/models/task_lock.rs`) is keyed by [`task_lock_key`] of its name:
//!
//! - **Stable**, so that a `CronJob` run of the old image and a manual run of
//!   the new one contend for the same key during a rollout: FNV-1a 64, a
//!   published algorithm with fixed constants, and not
//!   `std::hash::DefaultHasher`, which may change between Rust releases.
//! - **Namespaced**: the hashed input is [`KEY_NAMESPACE`] and the name, as
//!   every user of advisory locks in a database shares one key space.
//! - **Checked**: only snake case names, the form of every task the ADRs name
//!   and of `cargo loco generate task`, so a mistyped name cannot lock under
//!   a key no other run uses.

/// What the hashed input starts with, before the task name.
pub const KEY_NAMESPACE: &str = "backend3:task:";

/// The FNV-1a 64 offset basis (FNV specification, draft-eastlake-fnv).
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// The FNV-1a 64 prime (FNV specification, draft-eastlake-fnv).
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A task name that cannot be turned into a lock key. The `Display` text is
/// the message the operator or developer sees.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "the task name {name:?} is not snake_case: a task name starts with a lowercase ASCII \
     letter and holds only lowercase ASCII letters, digits and underscores; pass the name \
     the task registers in its `TaskInfo` (ADR 0016 §4)"
)]
pub struct NotSnakeCase {
    /// The refused name.
    pub name: String,
}

/// Returns the advisory-lock key of the task called `task_name`.
///
/// - `task_name`: the name the task registers in its `TaskInfo`, which is
///   also the name `backend3-cli task <name>` runs it by.
///
/// The key is the FNV-1a 64 hash of [`KEY_NAMESPACE`] followed by the name,
/// its 64 bits read as the signed `bigint` that `pg_try_advisory_xact_lock`
/// takes.
///
/// # Errors
///
/// Returns [`NotSnakeCase`] for a name that is empty or not snake case.
pub fn task_lock_key(task_name: &str) -> Result<i64, NotSnakeCase> {
    let mut characters = task_name.chars();
    let snake_case = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|rest| rest.is_ascii_lowercase() || rest.is_ascii_digit() || rest == '_');
    if !snake_case {
        return Err(NotSnakeCase {
            name: task_name.to_owned(),
        });
    }
    let hash = fnv1a_64(fnv1a_64(FNV_OFFSET_BASIS, KEY_NAMESPACE), task_name);
    // Reinterprets the 64 bits; no value is lost or clamped.
    Ok(hash.cast_signed())
}

/// Continues the FNV-1a 64 hash whose state is `state` over the bytes of
/// `input`; from [`FNV_OFFSET_BASIS`], it is the hash of `input`.
fn fnv1a_64(state: u64, input: &str) -> u64 {
    input.bytes().fold(state, |state, byte| {
        (state ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Snake-case names, as the proptests generate them.
    const NAME: &str = "[a-z][a-z0-9_]{0,63}";

    #[test]
    fn the_hash_is_fnv1a_64() {
        // The FNV-1a 64 test vectors of the FNV reference implementation.
        for (input, hash) in [
            ("", 0xcbf2_9ce4_8422_2325),
            ("a", 0xaf63_dc4c_8601_ec8c),
            ("foobar", 0x8594_4171_f739_67e8),
        ] {
            assert_eq!(fnv1a_64(FNV_OFFSET_BASIS, input), hash, "{input:?}");
        }
    }

    /// The keys of tasks the ADRs name (0013, 0017) and of a few other names.
    /// A failure means a key changed, so runs of the old and the new build
    /// would not exclude each other during a rollout. Never update a value
    /// to make this pass; restore the algorithm instead.
    #[test]
    fn the_keys_are_stable() {
        for (name, key) in [
            ("import_mysql", 6_921_639_353_034_420_907),
            ("content_normalize", -2_168_210_955_607_375_084),
            ("rich_text_manifest", -6_668_671_831_441_708_629),
            ("a", -8_039_884_016_689_039_502),
            ("x_9", 6_024_738_678_789_683_517),
        ] {
            assert_eq!(task_lock_key(name), Ok(key), "{name}");
        }
    }

    #[test]
    fn names_that_are_not_snake_case_are_refused_with_the_rule() {
        for name in ["a", "z9", "a__b", "v1_export"] {
            assert!(task_lock_key(name).is_ok(), "{name}");
        }
        for name in [
            "",
            "_import",
            "1import",
            "importMysql",
            "import-mysql",
            "import_mysql\n",
            "tárolás",
        ] {
            let message = task_lock_key(name).expect_err(name).to_string();
            let start = format!("the task name {name:?} is not snake_case: ");
            assert!(message.starts_with(&start), "{message}");
            assert!(
                message.ends_with("in its `TaskInfo` (ADR 0016 §4)"),
                "{message}"
            );
        }
    }

    proptest! {
        #[test]
        fn never_panics(name in any::<String>()) {
            let _ = task_lock_key(&name);
        }

        #[test]
        fn every_snake_case_name_has_a_key(name in NAME) {
            prop_assert!(task_lock_key(&name).is_ok());
        }

        /// FNV-1a passes its state through a bijection at every byte (an XOR,
        /// then a multiplication by an odd prime modulo 2^64), so names of
        /// the same length that differ in one character never share a key.
        #[test]
        fn names_that_differ_in_one_character_have_different_keys(
            name in NAME,
            position in any::<prop::sample::Index>(),
            replacement in "[a-z0-9_]",
        ) {
            // The names are ASCII, so a byte index is a character index.
            let index = position.index(name.len());
            let mut other = name.clone();
            other.replace_range(index..=index, &replacement);
            prop_assume!(other != name && task_lock_key(&other).is_ok());
            prop_assert_ne!(task_lock_key(&name), task_lock_key(&other));
        }

        #[test]
        fn a_character_outside_snake_case_is_refused(
            prefix in NAME,
            bad in "[^a-z0-9_]",
            suffix in "[a-z0-9_]{0,20}",
        ) {
            let name = format!("{prefix}{bad}{suffix}");
            prop_assert_eq!(task_lock_key(&name), Err(NotSnakeCase { name: name.clone() }));
        }
    }
}
