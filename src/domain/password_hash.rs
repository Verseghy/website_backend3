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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

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
}
