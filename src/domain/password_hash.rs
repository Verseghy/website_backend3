//! Which kind of password hash a stored `users.password_hash` value is, and
//! the dummy hash for sign-ins that match no account (ADR 0006 §4).
//!
//! `users.password_hash` holds one of two formats (ADR 0006 §7):
//!
//! - an Argon2id PHC string such as `$argon2id$v=19$m=19456,t=2,p=1$...`,
//!   written by `loco_rs::hash::hash_password`;
//! - until its owner's first sign-in, the Laravel bcrypt string that the
//!   importer copied unchanged (`$2y$10$...`; `$2a$` and `$2b$` are the other
//!   spellings of the same algorithm). Sign-in verifies it once with bcrypt
//!   and replaces it with an Argon2id hash.
//!
//! [`classify`] picks the verifier from the value's prefix alone. It does not
//! check that the rest of the string is well formed: the verifier rejects a
//! malformed hash, so a second parser here would only duplicate it (the
//! users model says why that rejection may skip the Argon2 work). The
//! prefixes are compared case-sensitively, because both formats are written
//! in lower case and any other spelling was not written by either library.
//!
//! [`DUMMY_PASSWORD_HASH`] is what sign-in verifies when no account matches
//! the e-mail address, so an unknown address costs one Argon2id
//! verification, like a known one, and the response time does not reveal
//! which addresses have accounts (ADR 0006 §4, "Same answer for every
//! failure").
//!
//! [`legacy_password_must_change`] decides what a legacy password that has
//! just verified may open: ADR 0006 §6 lets one shorter than 12 characters
//! (`MIN_PASSWORD_CHARS`, counted by [`password_length`]) sign in only to a
//! `password_change` session while
//! `admin_auth.force_change_short_legacy_password` is on (§8). The length is
//! the one every password rule of ADR 0006 §6 counts: Unicode scalar values
//! after NFC normalisation, so `á` counts once however it was typed.

use unicode_normalization::UnicodeNormalization as _;

/// The kind of a stored password hash, decided by its prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PasswordHashKind {
    /// An Argon2id PHC string (`$argon2id$...`), the format backend3 writes.
    Argon2id,
    /// A legacy bcrypt string (`$2y$...`, `$2a$...` or `$2b$...`) from the
    /// import, replaced by Argon2id at its owner's first sign-in.
    Bcrypt,
    /// Anything else, including the empty string and the other Argon2
    /// variants (`$argon2i$`, `$argon2d$`), which backend3 never writes. Such
    /// a value never verifies.
    Unknown,
}

/// The prefix of an Argon2id PHC string.
pub const ARGON2ID_PREFIX: &str = "$argon2id$";

/// The prefixes of a bcrypt string. Laravel writes `$2y$`; `$2a$` and `$2b$`
/// are the same algorithm under the names other libraries use.
pub const BCRYPT_PREFIXES: [&str; 3] = ["$2y$", "$2a$", "$2b$"];

/// A constant Argon2id hash that sign-in verifies when no account matches,
/// so that every failed sign-in costs one password verification.
///
/// Its parameters (`m=19456,t=2,p=1`, version 19) are the argon2 crate's
/// defaults, which `loco_rs::hash::hash_password` uses for every real hash.
/// Argon2 verification runs with the parameters stored in the hash, so equal
/// parameters mean equal cost. The model tests compare these parameters
/// with a freshly hashed password, so a change of the defaults (for example
/// in a Loco bump) fails the tests until this value is regenerated, and they
/// time its verification against a real one.
///
/// It is the hash of a random value that was discarded when the hash was
/// made: it is no account's password and no credential, and the caller
/// treats its verification as a failure whatever the result.
pub const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$4f7sp9+gyHNdsULUGeMmWw$K3YoBekWANdK/na/AHC+hbYUUge5S9dN/cv84nh/JzM";

/// Returns the kind of the stored password hash `stored`.
///
/// - `stored`: a value of `users.password_hash`, exactly as stored.
#[must_use]
pub fn classify(stored: &str) -> PasswordHashKind {
    if stored.starts_with(ARGON2ID_PREFIX) {
        PasswordHashKind::Argon2id
    } else if BCRYPT_PREFIXES
        .iter()
        .any(|prefix| stored.starts_with(prefix))
    {
        PasswordHashKind::Bcrypt
    } else {
        PasswordHashKind::Unknown
    }
}

/// The fewest characters a password may have (ADR 0006 §6), counted by
/// [`password_length`]. A legacy password below it still verifies, but
/// [`legacy_password_must_change`] keeps it from opening a full session.
pub const MIN_PASSWORD_CHARS: usize = 12;

/// The length of `password` as ADR 0006 §6 counts it: the number of Unicode
/// scalar values after NFC normalisation.
///
/// - `password`: the password as typed.
///
/// NFC composes a letter and its combining accents where Unicode has one
/// code point for them, so `á` typed as `a` and U+0301 counts once, as the
/// precomposed U+00E1 does. Bytes are never counted: `á` is two bytes in
/// UTF-8 and still one character.
#[must_use]
pub fn password_length(password: &str) -> usize {
    password.nfc().count()
}

/// Whether a legacy password that has just verified against its bcrypt hash
/// must be changed before its account may have a full `admin` session
/// (ADR 0006 §6, §8).
///
/// - `password`: the password as typed, which matched the stored bcrypt
///   hash.
/// - `force_change_short`: `settings.admin_auth.force_change_short_legacy_password`.
///
/// `true` exactly when the setting is on and the password is shorter than
/// [`MIN_PASSWORD_CHARS`] by [`password_length`]. Such a sign-in yields
/// only a `password_change` session (ADR 0006 §6); until that session
/// exists, the sign-in refuses it as it refuses every account that must
/// change its password, and keeps the bcrypt hash, so the rule still sees a
/// legacy password at the next sign-in (`users::Model::authenticate`).
#[must_use]
pub fn legacy_password_must_change(password: &str, force_change_short: bool) -> bool {
    force_change_short && password_length(password) < MIN_PASSWORD_CHARS
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_password_is_as_long_as_its_characters_after_nfc() {
        for (password, length) in [
            ("", 0),
            ("abc", 3),
            // Eleven and twelve characters, the edge of the rule.
            ("jelszo12345", 11),
            ("jelszo123456", 12),
            // Precomposed: 12 characters, 24 bytes.
            ("áááááááááááá", 12),
            // Decomposed (`a` and U+0301): 24 scalar values, 12 after NFC.
            (
                "a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}",
                12,
            ),
            ("árvíztűrő tükörfúrógép", 22),
            // Spaces and symbols count like letters.
            ("            ", 12),
            ("🔑🔑", 2),
        ] {
            assert_eq!(password_length(password), length, "{password:?}");
        }
        // Bytes would count the precomposed form twice.
        assert_eq!("áááááááááááá".len(), 24);
    }

    #[test]
    fn a_short_legacy_password_must_change_only_while_the_setting_is_on() {
        for (password, short) in [
            ("", true),
            ("jelszo12345", true),
            ("jelszo123456", false),
            ("ááááááááááá", true),
            ("áááááááááááá", false),
            // Eleven `á`, typed decomposed: 22 scalar values, 11 after NFC.
            (
                "a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}a\u{301}",
                true,
            ),
            ("helyes jelszó 2026", false),
        ] {
            assert_eq!(
                legacy_password_must_change(password, true),
                short,
                "{password:?}"
            );
            assert!(
                !legacy_password_must_change(password, false),
                "{password:?}"
            );
        }
        assert_eq!(MIN_PASSWORD_CHARS, 12);
    }

    #[test]
    fn the_classifier_table() {
        for (stored, expected) in [
            (
                "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA",
                PasswordHashKind::Argon2id,
            ),
            ("$argon2id$", PasswordHashKind::Argon2id),
            (
                "$2y$10$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234",
                PasswordHashKind::Bcrypt,
            ),
            ("$2a$12$x", PasswordHashKind::Bcrypt),
            ("$2b$04$x", PasswordHashKind::Bcrypt),
            ("$2y$", PasswordHashKind::Bcrypt),
            ("", PasswordHashKind::Unknown),
            ("$", PasswordHashKind::Unknown),
            (
                "$argon2i$v=19$m=4096,t=3,p=1$x$y",
                PasswordHashKind::Unknown,
            ),
            (
                "$argon2d$v=19$m=4096,t=3,p=1$x$y",
                PasswordHashKind::Unknown,
            ),
            ("$argon2id", PasswordHashKind::Unknown),
            ("$ARGON2ID$v=19$x", PasswordHashKind::Unknown),
            ("$2x$10$x", PasswordHashKind::Unknown),
            ("$2$10$x", PasswordHashKind::Unknown),
            ("$2Y$10$x", PasswordHashKind::Unknown),
            ("$2y", PasswordHashKind::Unknown),
            (" $2y$10$x", PasswordHashKind::Unknown),
            (" $argon2id$v=19$x", PasswordHashKind::Unknown),
            ("plain text password", PasswordHashKind::Unknown),
            // A hex digest, the shape of an unsalted MD5 or SHA value.
            (
                "0123456789abcdef0123456789abcdef",
                PasswordHashKind::Unknown,
            ),
        ] {
            assert_eq!(classify(stored), expected, "{stored:?}");
        }
    }

    /// Whether `segment` is `len` characters of the unpadded B64 alphabet
    /// that PHC strings use for the salt and the hash.
    fn is_phc_b64(segment: &str, len: usize) -> bool {
        segment.len() == len
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
    }

    #[test]
    fn the_dummy_hash_is_a_complete_argon2id_phc_string() {
        // `$argon2id$v=..$m=..,t=..,p=..$<salt>$<hash>`. A salt or hash that
        // does not decode would make the verifier refuse the string at once,
        // without the Argon2 work the dummy exists for. The argon2 crate's
        // defaults give a 16-byte salt and a 32-byte hash: 22 and 43
        // characters. The model tests compare the parameters with a real hash.
        assert_eq!(classify(DUMMY_PASSWORD_HASH), PasswordHashKind::Argon2id);
        let segments: Vec<&str> = DUMMY_PASSWORD_HASH.split('$').collect();
        assert_eq!(segments.len(), 6, "{DUMMY_PASSWORD_HASH}");
        assert!(is_phc_b64(segments[4], 22), "salt {}", segments[4]);
        assert!(is_phc_b64(segments[5], 43), "hash {}", segments[5]);
        assert!(!is_phc_b64("not!base64", 10) && !is_phc_b64("abc", 4));
    }

    proptest! {
        #[test]
        fn anything_after_the_argon2id_prefix_is_argon2id(rest in any::<String>()) {
            prop_assert_eq!(
                classify(&format!("{ARGON2ID_PREFIX}{rest}")),
                PasswordHashKind::Argon2id
            );
        }

        #[test]
        fn anything_after_a_bcrypt_prefix_is_bcrypt(
            prefix in proptest::sample::select(BCRYPT_PREFIXES.to_vec()),
            rest in any::<String>(),
        ) {
            prop_assert_eq!(classify(&format!("{prefix}{rest}")), PasswordHashKind::Bcrypt);
        }

        #[test]
        fn a_value_is_unknown_exactly_when_it_starts_with_no_known_prefix(
            // Arbitrary text, and near misses of the prefixes.
            stored in prop_oneof![any::<String>(), "\\$[a2$]?[a-z0-9$]{0,10}.*"],
        ) {
            let known = stored.starts_with(ARGON2ID_PREFIX)
                || BCRYPT_PREFIXES.iter().any(|prefix| stored.starts_with(prefix));
            prop_assert_eq!(classify(&stored) == PasswordHashKind::Unknown, !known);
        }

        #[test]
        fn a_known_kind_does_not_change_when_the_value_grows(
            stored in "\\$(argon2id|2y|2a|2b)\\$.{0,20}",
            suffix in any::<String>(),
        ) {
            let kind = classify(&stored);
            prop_assert_ne!(kind, PasswordHashKind::Unknown);
            prop_assert_eq!(classify(&format!("{stored}{suffix}")), kind);
        }
    }

    /// Text over Hungarian letters, with or without accents, digits and a
    /// space: the passwords whose accents may arrive composed or not.
    fn hungarian_text() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 áéíóöőúüűÁÉÍÓÖŐÚÜŰ]{0,40}"
    }

    proptest! {
        #[test]
        fn the_length_of_ascii_text_is_its_byte_count(password in "[ -~]{0,64}") {
            prop_assert_eq!(password_length(&password), password.len());
        }

        #[test]
        fn the_length_does_not_depend_on_how_the_accents_were_typed(
            password in hungarian_text(),
        ) {
            // NFD splits every accented letter into its base letter and a
            // combining accent; NFC joins them again, so the decomposed
            // spelling counts as the precomposed one does.
            let decomposed: String = password.nfd().collect();
            prop_assert_eq!(password_length(&decomposed), password_length(&password));
            prop_assert_eq!(password_length(&password), password.chars().count());
        }

        #[test]
        fn the_length_is_that_of_the_normalised_text(password in any::<String>()) {
            let normalised: String = password.nfc().collect();
            prop_assert_eq!(password_length(&password), normalised.chars().count());
            // Normalising again changes nothing (NFC is idempotent).
            prop_assert_eq!(password_length(&normalised), password_length(&password));
        }

        #[test]
        fn a_legacy_password_must_change_exactly_when_the_setting_is_on_and_it_is_short(
            password in any::<String>(),
            force_change_short in any::<bool>(),
        ) {
            prop_assert_eq!(
                legacy_password_must_change(&password, force_change_short),
                force_change_short && password_length(&password) < MIN_PASSWORD_CHARS
            );
            prop_assert!(!legacy_password_must_change(&password, false));
        }

        #[test]
        fn adding_characters_never_makes_a_long_enough_password_short(
            password in hungarian_text(),
            suffix in hungarian_text(),
        ) {
            // Over letters, digits and spaces, a longer password is never
            // shorter after NFC: no letter of this alphabet combines with
            // the start of the suffix.
            let longer = format!("{password}{suffix}");
            prop_assert!(password_length(&longer) >= password_length(&password));
            if !legacy_password_must_change(&password, true) {
                prop_assert!(!legacy_password_must_change(&longer, true));
            }
        }
    }
}
