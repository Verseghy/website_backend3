//! Which typed e-mail addresses an account could have (ADR 0006 §7).
//!
//! `users.email` is a PostgreSQL `text` column, and `text` cannot hold the
//! character U+0000. A query that binds a string containing it does not just
//! find no row: PostgreSQL refuses the whole statement (SQLSTATE 22021,
//! "invalid byte sequence for encoding UTF8"). A lookup by a typed address
//! therefore asks [`can_be_stored`] first and treats any other address as one
//! that no account has, so sign-in answers it exactly like an unknown address
//! instead of with a server error. U+0000 is the only character that `text`
//! refuses, because a Rust `&str` is always valid UTF-8.
//!
//! The column has no length limit, so length is not a reason here; the rule
//! for the addresses an account may be given belongs to account creation.

/// Whether `address` could be the `users.email` of some account, that is,
/// whether PostgreSQL `text` can store it: it contains no U+0000.
///
/// - `address`: the address as typed, before any case folding.
#[must_use]
pub fn can_be_stored(address: &str) -> bool {
    !address.contains('\0')
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn only_an_address_with_u0000_cannot_be_stored() {
        for (address, expected) in [
            ("szerkeszto@example.test", true),
            ("Ékezetes.Cím@example.test", true),
            ("", true),
            (" \t\u{7f}\u{1}@example.test", true),
            ("a\0b@example.test", false),
            ("\0", false),
            ("szerkeszto@example.test\0", false),
        ] {
            assert_eq!(can_be_stored(address), expected, "{address:?}");
        }
    }

    proptest! {
        #[test]
        fn a_u0000_anywhere_is_the_only_refusal(
            address in "[^\u{0}]*",
            at in any::<prop::sample::Index>(),
        ) {
            prop_assert!(can_be_stored(&address));
            // Insert U+0000 at any character boundary, ends included.
            let boundaries: Vec<usize> = address
                .char_indices()
                .map(|(index, _)| index)
                .chain([address.len()])
                .collect();
            let mut with_nul = address.clone();
            with_nul.insert(boundaries[at.index(boundaries.len())], '\0');
            prop_assert!(!can_be_stored(&with_nul));
        }
    }
}
