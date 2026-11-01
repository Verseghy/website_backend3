//! The admin session's JWT configuration, `auth.jwt`, as ADR 0006 §1 fixes
//! it, checked at boot so that a misconfiguration stops every command with
//! the key named instead of failing at the first sign-in.
//!
//! ADR 0006 §1 fixes three values, and Loco reads all three from `auth.jwt`
//! (loco-rs 1.2.0 `config::JWT`):
//!
//! - **the secret** (`auth.jwt.secret`): base64 of at least 64 random bytes,
//!   decoded as every configured key is ([`decode_configured_key`],
//!   ADR 0023 §11). Loco decodes it only when it signs or checks a token, so
//!   without this check a value that is not base64 would show up as a `500`
//!   at the first correct sign-in, and a short one would never show up;
//! - **the lifetime** (`auth.jwt.expiration`): [`SESSION_LIFETIME_SECS`],
//!   two hours, which is also the session cookie's `Max-Age`;
//! - **where the token is read** (`auth.jwt.location`): the session cookie
//!   [`SESSION_COOKIE_NAME`] first, then the `Bearer` header of scripts, and
//!   nothing else. A query location would put the token in access logs.
//!
//! The local environments commit public throwaway secrets, which ADR 0034
//! §2 item 3 allows; [`PUBLISHED_JWT_SECRETS`] lists every one this
//! repository has ever held, and production refuses each of them at boot
//! (ADR 0034 §3, `Settings::refuse_published_keys` in `src/settings.rs`).
//!
//! Everything here is pure (ADR 0008 rule 1). The caller, `src/settings.rs`,
//! translates Loco's configuration types into the plain values these
//! functions take.

use std::fmt;

use super::{
    configured_key::{KeyError, decode_configured_key},
    session_cookie::SESSION_COOKIE_NAME,
};

/// The lifetime of an admin session token in seconds: two hours
/// (ADR 0006 §1). `auth.jwt.expiration` must be exactly this.
pub const SESSION_LIFETIME_SECS: u64 = 7200;

/// One place Loco may read the token from: a plain copy of loco-rs 1.2.0's
/// `config::JWTLocation`, so that this module needs no framework type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenLocation {
    /// The `Authorization: Bearer` header.
    Bearer,
    /// A cookie.
    Cookie {
        /// The cookie's name.
        name: String,
    },
    /// A query parameter.
    Query {
        /// The parameter's name.
        name: String,
    },
}

impl fmt::Display for TokenLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bearer => formatter.write_str("the Bearer header"),
            Self::Cookie { name } => write!(formatter, "the cookie {name}"),
            Self::Query { name } => write!(formatter, "the query parameter {name}"),
        }
    }
}

/// `auth.jwt.expiration` is not [`SESSION_LIFETIME_SECS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "is {found} seconds; the admin session lasts {SESSION_LIFETIME_SECS} seconds, two hours, \
     and the session cookie's Max-Age is this value (ADR 0006 §1)"
)]
pub struct ExpirationError {
    /// The configured lifetime.
    pub found: u64,
}

/// `auth.jwt.location` is not the session cookie followed by `Bearer`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "reads the token from {found}; it must list the cookie {SESSION_COOKIE_NAME} first, then \
     Bearer, and nothing else (ADR 0006 §1)"
)]
pub struct LocationsError {
    /// The configured locations, as a sentence part.
    pub found: String,
}

/// Checks `auth.jwt.expiration`.
///
/// - `expiration`: the configured token lifetime in seconds.
///
/// # Errors
///
/// [`ExpirationError`] unless it is [`SESSION_LIFETIME_SECS`]. A longer
/// lifetime would outlive the idle limit of ADR 0006 §1; a shorter one would
/// end sessions that the sliding refresh is meant to keep.
pub const fn check_expiration(expiration: u64) -> Result<(), ExpirationError> {
    if expiration == SESSION_LIFETIME_SECS {
        Ok(())
    } else {
        Err(ExpirationError { found: expiration })
    }
}

/// Checks `auth.jwt.location`.
///
/// - `locations`: the configured locations in order; a single location is
///   a list of one. `None` when the key is absent, in which case loco-rs
///   1.2.0 reads only the Bearer header (`auth::jwt` extractor).
///
/// # Errors
///
/// [`LocationsError`] unless the list is exactly the cookie
/// [`SESSION_COOKIE_NAME`], then [`TokenLocation::Bearer`]. The message
/// lists what was configured; names of cookies and parameters are not
/// secrets.
pub fn check_locations(locations: Option<&[TokenLocation]>) -> Result<(), LocationsError> {
    let Some(locations) = locations else {
        return Err(LocationsError {
            found: "the Bearer header only, Loco's default when the key is not set".to_owned(),
        });
    };
    match locations {
        [TokenLocation::Cookie { name }, TokenLocation::Bearer] if name == SESSION_COOKIE_NAME => {
            Ok(())
        }
        [] => Err(LocationsError {
            found: "an empty list".to_owned(),
        }),
        _ => Err(LocationsError {
            found: locations
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", then "),
        }),
    }
}

/// A throwaway JWT secret that this public repository holds, or held: a
/// value that production refuses (ADR 0034 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedJwtSecret {
    /// Where the repository publishes it, for the refusal's message.
    pub source: &'static str,
    /// The secret's decoded bytes: a sentence that says it is public and
    /// throwaway (ADR 0034 §2 item 3). The configuration holds its base64.
    pub text: &'static [u8],
}

/// Every JWT secret this repository has published (ADR 0034 §3, "Which
/// keys"): the `auth.jwt.secret` of the three local configurations, and the
/// `CLI_JWT_SECRET` that `tests/support` held until the production tests
/// moved to a secret made at run time. That constant left the tree, but its
/// value stays in the public history, so production keeps refusing it.
///
/// A slice that commits another JWT secret adds it here in the same PR;
/// `tests/boot/published_keys.rs` fails on a committed key that production
/// does not refuse.
pub const PUBLISHED_JWT_SECRETS: [PublishedJwtSecret; 4] = [
    PublishedJwtSecret {
        source: "config/development.yaml",
        text: b"backend3 development JWT secret: public and throwaway, never valid on any \
              deployment.",
    },
    PublishedJwtSecret {
        source: "config/test.yaml",
        text: b"backend3 test JWT secret: public and throwaway, used only by the Rust test \
              suite.",
    },
    PublishedJwtSecret {
        source: "config/e2e.yaml",
        text: b"backend3 e2e JWT secret: public and throwaway, used only by the E2E job and its \
              server.",
    },
    PublishedJwtSecret {
        source: "tests/support/mod.rs (the former CLI_JWT_SECRET)",
        text: b"backend3 CLI test JWT secret: public and throwaway, only for child processes.",
    },
];

/// A checked JWT secret: the decoded bytes of a configured
/// `auth.jwt.secret`. `Debug` hides the bytes, and equality looks at every
/// byte of two secrets of the same length.
///
/// Loco keeps signing with its own copy of the configured text
/// (`ctx.config`); this value exists only so that the boot checks can
/// compare it.
#[derive(Clone)]
pub struct JwtSecret(Vec<u8>);

impl JwtSecret {
    /// Decodes and checks a configured JWT secret.
    ///
    /// - `encoded`: the configured `auth.jwt.secret`, which production reads
    ///   from `JWT_SECRET`.
    ///
    /// # Errors
    ///
    /// The [`KeyError`] of [`decode_configured_key`]: empty, not standard
    /// base64 with `=` padding on one line, or fewer than 64 decoded bytes.
    /// No message contains the value.
    pub fn from_base64(encoded: &str) -> Result<Self, KeyError> {
        decode_configured_key(encoded).map(Self)
    }

    /// Where this repository publishes this secret, if it is one of
    /// [`PUBLISHED_JWT_SECRETS`]; `None` for any other secret.
    ///
    /// The decoded bytes are compared, not the configured text. Canonical
    /// base64 has one spelling per value, so the two agree; comparing bytes
    /// keeps the answer right if the decoder ever accepts a second one. A
    /// secret that differs from a published one in any byte, or is a
    /// prefix of one, is another secret.
    #[must_use]
    pub fn published_in(&self) -> Option<&'static str> {
        PUBLISHED_JWT_SECRETS
            .iter()
            .find(|published| same_bytes(&self.0, published.text))
            .map(|published| published.source)
    }
}

impl fmt::Debug for JwtSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JwtSecret(<redacted>)")
    }
}

impl PartialEq for JwtSecret {
    fn eq(&self, other: &Self) -> bool {
        same_bytes(&self.0, &other.0)
    }
}

impl Eq for JwtSecret {}

/// Whether two byte strings are equal. Every byte of equally long strings is
/// read, so the time does not depend on where they differ; only the boot
/// check compares secrets, once, but an early exit buys nothing either.
///
/// - `left`, `right`: the byte strings.
fn same_bytes(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |acc, (l, r)| acc | (l ^ r))
            == 0
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use proptest::prelude::*;

    use super::*;
    use crate::domain::configured_key::CONFIGURED_KEY_MIN_LEN;

    fn cookie(name: &str) -> TokenLocation {
        TokenLocation::Cookie {
            name: name.to_owned(),
        }
    }

    fn session_locations() -> Vec<TokenLocation> {
        vec![cookie(SESSION_COOKIE_NAME), TokenLocation::Bearer]
    }

    #[test]
    fn the_lifetime_is_exactly_two_hours() {
        assert_eq!(SESSION_LIFETIME_SECS, 2 * 60 * 60);
        assert_eq!(check_expiration(7200), Ok(()));
        for found in [0, 1, 3600, 7199, 7201, 43_200, 604_800, u64::MAX] {
            let error = check_expiration(found).expect_err("refused");
            assert_eq!(error, ExpirationError { found });
            let message = error.to_string();
            assert!(
                message.starts_with(&format!("is {found} seconds; the admin session lasts 7200")),
                "{message}"
            );
        }
    }

    #[test]
    fn the_cookie_then_bearer_is_the_only_accepted_list() {
        assert_eq!(check_locations(Some(&session_locations())), Ok(()));
        let query = TokenLocation::Query {
            name: "token".to_owned(),
        };
        let cases: [(Option<Vec<TokenLocation>>, &str); 8] = [
            (
                None,
                "the Bearer header only, Loco's default when the key is not set",
            ),
            (Some(vec![]), "an empty list"),
            (Some(vec![TokenLocation::Bearer]), "the Bearer header"),
            (
                Some(vec![cookie(SESSION_COOKIE_NAME)]),
                "the cookie __Host-admin_session",
            ),
            (
                Some(vec![TokenLocation::Bearer, cookie(SESSION_COOKIE_NAME)]),
                "the Bearer header, then the cookie __Host-admin_session",
            ),
            (
                Some(vec![cookie("admin_session"), TokenLocation::Bearer]),
                "the cookie admin_session, then the Bearer header",
            ),
            (
                Some(vec![
                    cookie(SESSION_COOKIE_NAME),
                    TokenLocation::Bearer,
                    query.clone(),
                ]),
                "the cookie __Host-admin_session, then the Bearer header, then the query \
                 parameter token",
            ),
            (
                Some(vec![query, TokenLocation::Bearer]),
                "the query parameter token, then the Bearer header",
            ),
        ];
        for (locations, found) in cases {
            let error = check_locations(locations.as_deref()).expect_err("refused");
            assert_eq!(error.found, found);
            let message = error.to_string();
            assert!(
                message.starts_with(&format!("reads the token from {found}; it must list")),
                "{message}"
            );
            assert!(
                message.contains("the cookie __Host-admin_session first, then Bearer"),
                "{message}"
            );
        }
    }

    #[test]
    fn every_published_secret_is_a_valid_secret_that_says_it_is_throwaway() {
        for published in PUBLISHED_JWT_SECRETS {
            let text = std::str::from_utf8(published.text).expect("readable");
            assert!(text.contains("public and throwaway"), "{text}");
            assert!(text.len() >= CONFIGURED_KEY_MIN_LEN, "{text}");
            let secret = JwtSecret::from_base64(&STANDARD.encode(published.text)).expect("valid");
            assert_eq!(secret.published_in(), Some(published.source));
        }
        // Four distinct values, from four sources.
        for (index, left) in PUBLISHED_JWT_SECRETS.iter().enumerate() {
            for right in &PUBLISHED_JWT_SECRETS[index + 1..] {
                assert_ne!(left.text, right.text);
                assert_ne!(left.source, right.source);
            }
        }
    }

    #[test]
    fn a_near_copy_of_a_published_secret_is_another_secret() {
        for published in PUBLISHED_JWT_SECRETS {
            let mut changed = published.text.to_vec();
            if let Some(last) = changed.last_mut() {
                *last ^= 1;
            }
            let mut longer = published.text.to_vec();
            longer.push(b'!');
            let prefix = published.text[..CONFIGURED_KEY_MIN_LEN].to_vec();
            for bytes in [changed, longer, prefix] {
                let secret = JwtSecret::from_base64(&STANDARD.encode(&bytes)).expect("valid");
                assert_eq!(secret.published_in(), None, "{published:?}");
            }
        }
    }

    #[test]
    fn a_bad_secret_is_refused_without_its_value() {
        let short = STANDARD.encode([0x5a_u8; 40]);
        for (value, problem) in [
            ("", "is empty"),
            ("a pasted password, not base64", "is not valid base64"),
            (short.as_str(), "decodes to 40 bytes"),
        ] {
            let message = JwtSecret::from_base64(value)
                .expect_err("refused")
                .to_string();
            assert!(message.contains(problem), "{value:?}: {message}");
            if !value.is_empty() {
                assert!(!message.contains(value), "{message}");
            }
        }
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let published = PUBLISHED_JWT_SECRETS[0];
        let encoded = STANDARD.encode(published.text);
        let printed = format!("{:?}", JwtSecret::from_base64(&encoded).expect("valid"));
        assert_eq!(printed, "JwtSecret(<redacted>)");
    }

    #[test]
    fn locations_print_as_sentence_parts() {
        assert_eq!(TokenLocation::Bearer.to_string(), "the Bearer header");
        assert_eq!(cookie("c").to_string(), "the cookie c");
        assert_eq!(
            TokenLocation::Query {
                name: "q".to_owned()
            }
            .to_string(),
            "the query parameter q"
        );
    }

    #[test]
    fn same_bytes_compares_whole_strings() {
        assert!(same_bytes(b"", b""));
        assert!(same_bytes(b"abc", b"abc"));
        assert!(!same_bytes(b"abc", b"abd"));
        assert!(!same_bytes(b"abc", b"ab"));
        assert!(!same_bytes(b"ab", b"abc"));
    }

    /// Any location. The two of the accepted list are drawn most often, so
    /// that every run also tries the lists next to it: the pair itself, the
    /// pair reversed, and the pair with a location added before, between or
    /// after. A random cookie name is almost never the session cookie's, so
    /// without that weight the property would only see lists that are
    /// refused anyway.
    fn location() -> impl Strategy<Value = TokenLocation> {
        prop_oneof![
            3 => Just(cookie(SESSION_COOKIE_NAME)),
            3 => Just(TokenLocation::Bearer),
            1 => "[A-Za-z_-]{0,24}".prop_map(|name| TokenLocation::Cookie { name }),
            1 => "[A-Za-z_-]{0,24}".prop_map(|name| TokenLocation::Query { name }),
            // The right name in the wrong place.
            1 => Just(TokenLocation::Query {
                name: SESSION_COOKIE_NAME.to_owned(),
            }),
        ]
    }

    proptest! {
        #[test]
        fn only_the_session_list_passes(
            locations in proptest::collection::vec(location(), 0..5),
        ) {
            let accepted = check_locations(Some(&locations)).is_ok();
            prop_assert_eq!(accepted, locations == session_locations());
        }

        #[test]
        fn a_location_added_to_the_session_list_is_refused(
            extra in location(),
            at in 0..=2_usize,
        ) {
            // "Nothing else" (ADR 0006 §1): a third location anywhere in
            // the list, the session cookie or Bearer again included.
            let mut locations = session_locations();
            locations.insert(at, extra);
            let error = check_locations(Some(&locations)).expect_err("refused");
            prop_assert!(error.found.contains(", then "), "{}", error.found);
        }

        #[test]
        fn only_two_hours_pass(expiration in any::<u64>()) {
            prop_assert_eq!(check_expiration(expiration).is_ok(), expiration == 7200);
        }

        #[test]
        fn any_secret_of_64_bytes_or_more_is_accepted_and_unpublished(
            bytes in proptest::collection::vec(any::<u8>(), 64..=160),
        ) {
            prop_assume!(PUBLISHED_JWT_SECRETS.iter().all(|p| p.text != bytes.as_slice()));
            let secret = JwtSecret::from_base64(&STANDARD.encode(&bytes)).expect("valid");
            prop_assert_eq!(secret.published_in(), None);
            prop_assert_eq!(&secret, &JwtSecret(bytes));
        }

        #[test]
        fn shorter_secrets_are_refused(bytes in proptest::collection::vec(any::<u8>(), 1..64)) {
            prop_assert_eq!(
                JwtSecret::from_base64(&STANDARD.encode(&bytes)),
                Err(KeyError::TooShort { len: bytes.len() })
            );
        }

        #[test]
        fn equality_is_byte_equality(
            left in proptest::collection::vec(any::<u8>(), 0..80),
            right in proptest::collection::vec(any::<u8>(), 0..80),
        ) {
            prop_assert_eq!(same_bytes(&left, &right), left == right);
            prop_assert!(same_bytes(&left, &left));
        }
    }
}
