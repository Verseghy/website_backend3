//! The `Set-Cookie` value that carries the admin session (ADR 0006 §1).
//!
//! The browser sign-in hands the session token to the admin UI only in a
//! cookie that page scripts cannot read, because the admin renders rich HTML
//! and an injection is never fully ruled out (ADR 0006, Context). Every
//! attribute is fixed by ADR 0006 §1:
//!
//! - the `__Host-` name prefix: the browser refuses the cookie unless it is
//!   `Secure`, has `Path=/` and has no `Domain`, so a sibling host under the
//!   same site can neither set nor overwrite it;
//! - `HttpOnly`: page scripts cannot read it;
//! - `Secure`: it travels only over HTTPS (browsers treat `http://localhost`
//!   as secure too, which the local E2E run relies on);
//! - `SameSite=Strict`: browsers do not send it on cross-site requests
//!   (ADR 0006 §3);
//! - `Path=/`: required by the prefix; it reaches `/api/admin`;
//! - `Max-Age`: the token's lifetime, so the cookie and the token it carries
//!   expire together. No `Expires` is sent: every current browser prefers
//!   `Max-Age`, and an `Expires` date would need the clock here.
//!
//! The value is the JWT itself. It must consist of RFC 6265 §4.1.1
//! `cookie-octet`s, which every JWT does (base64url segments joined by
//! dots). The builder refuses any other value instead of quoting or
//! escaping it: a value that would need either is not a token Loco issued.

/// The name of the admin session cookie (ADR 0006 §1). `auth.jwt.location`
/// in every `config/<env>.yaml` reads the token from this cookie first.
pub const SESSION_COOKIE_NAME: &str = "__Host-admin_session";

/// Why a session cookie could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionCookieError {
    /// The token is the empty string.
    #[error("the session token is empty")]
    EmptyToken,
    /// The token contains a character outside the RFC 6265 `cookie-octet`
    /// set, such as a space, a `;` or a non-ASCII letter.
    #[error(
        "the session token contains {character:?} at byte {position}, which a cookie value \
         cannot carry (RFC 6265 §4.1.1)"
    )]
    InvalidCharacter {
        /// The first character that is not a `cookie-octet`.
        character: char,
        /// Its byte offset in the token.
        position: usize,
    },
    /// `Max-Age` is zero, which would delete the cookie instead of setting
    /// it.
    #[error("the session cookie's Max-Age must be at least 1 second; 0 deletes the cookie")]
    ZeroMaxAge,
}

/// Whether `character` is an RFC 6265 §4.1.1 `cookie-octet`: printable
/// US-ASCII except space, `"`, `,`, `;` and `\`.
fn is_cookie_octet(character: char) -> bool {
    matches!(character, '\u{21}' | '\u{23}'..='\u{2B}' | '\u{2D}'..='\u{3A}' | '\u{3C}'..='\u{5B}' | '\u{5D}'..='\u{7E}')
}

/// Builds the `Set-Cookie` header value that stores the session `token`:
/// `__Host-admin_session=<token>; HttpOnly; Secure; SameSite=Strict; Path=/;
/// Max-Age=<max_age_secs>`.
///
/// - `token`: the signed session JWT.
/// - `max_age_secs`: how long the browser keeps the cookie, in seconds; the
///   caller passes the token's lifetime, `auth.jwt.expiration` (two hours,
///   ADR 0006 §1).
///
/// # Errors
///
/// [`SessionCookieError::EmptyToken`] for an empty token,
/// [`SessionCookieError::InvalidCharacter`] for the first character that a
/// cookie value cannot carry, and [`SessionCookieError::ZeroMaxAge`] for a
/// `max_age_secs` of 0.
pub fn session_cookie(token: &str, max_age_secs: u64) -> Result<String, SessionCookieError> {
    if token.is_empty() {
        return Err(SessionCookieError::EmptyToken);
    }
    if let Some((position, character)) = token
        .char_indices()
        .find(|&(_, character)| !is_cookie_octet(character))
    {
        return Err(SessionCookieError::InvalidCharacter {
            character,
            position,
        });
    }
    if max_age_secs == 0 {
        return Err(SessionCookieError::ZeroMaxAge);
    }
    Ok(format!(
        "{SESSION_COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; \
         Max-Age={max_age_secs}"
    ))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The shape of a JWT: three base64url segments joined by dots. Mixed
    /// with arbitrary strings below, which almost never are cookie values.
    const JWT_LIKE: &str = "[A-Za-z0-9_-]{1,40}\\.[A-Za-z0-9_-]{1,80}\\.[A-Za-z0-9_-]{1,90}";

    #[test]
    fn the_cookie_has_exactly_the_adr_attributes() {
        assert_eq!(
            session_cookie("aaa.bbb.ccc", 7200),
            Ok(
                "__Host-admin_session=aaa.bbb.ccc; HttpOnly; Secure; SameSite=Strict; Path=/; \
                Max-Age=7200"
                    .to_owned()
            )
        );
        assert_eq!(
            session_cookie("t", u64::MAX)
                .expect("valid")
                .rsplit_once("Max-Age=")
                .map(|(_, age)| age.to_owned()),
            Some(u64::MAX.to_string())
        );
    }

    #[test]
    fn an_empty_token_or_a_zero_max_age_is_refused() {
        assert_eq!(
            session_cookie("", 7200),
            Err(SessionCookieError::EmptyToken)
        );
        assert_eq!(
            session_cookie("a.b.c", 0),
            Err(SessionCookieError::ZeroMaxAge)
        );
    }

    #[test]
    fn characters_outside_the_cookie_octets_are_refused_at_their_position() {
        for (token, character, position) in [
            ("a b", ' ', 1),
            ("a\"b", '"', 1),
            ("a,b", ',', 1),
            ("a;b", ';', 1),
            ("a\\b", '\\', 1),
            ("ab\u{7f}", '\u{7f}', 2),
            ("\tab", '\t', 0),
            ("aé", 'é', 1),
            ("a=b; Domain=example.test", ';', 3),
        ] {
            assert_eq!(
                session_cookie(token, 7200),
                Err(SessionCookieError::InvalidCharacter {
                    character,
                    position
                }),
                "{token:?}"
            );
        }
        // The edges of the allowed ranges.
        assert!(session_cookie("!#+-:<[]~=", 7200).is_ok());
    }

    #[test]
    fn errors_say_what_to_fix() {
        let message = SessionCookieError::InvalidCharacter {
            character: ';',
            position: 3,
        }
        .to_string();
        assert!(
            message.contains("';' at byte 3") && message.contains("RFC 6265"),
            "{message}"
        );
        assert!(
            SessionCookieError::ZeroMaxAge
                .to_string()
                .contains("at least 1 second")
        );
    }

    proptest! {
        #[test]
        fn a_token_is_accepted_exactly_when_every_character_is_a_cookie_octet(
            token in prop_oneof![JWT_LIKE, any::<String>()],
            max_age in 1..=u64::MAX,
        ) {
            let first_bad = token.char_indices().find(|&(_, c)| !is_cookie_octet(c));
            match (session_cookie(&token, max_age), first_bad) {
                (Err(SessionCookieError::EmptyToken), _) => prop_assert!(token.is_empty()),
                (Err(SessionCookieError::InvalidCharacter { character, position }), Some(bad)) => {
                    prop_assert_eq!((position, character), bad);
                }
                (Ok(cookie), None) => prop_assert_eq!(
                    cookie,
                    format!(
                        "{SESSION_COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Strict; \
                         Path=/; Max-Age={max_age}"
                    )
                ),
                (outcome, first_bad) => {
                    prop_assert!(false, "{token:?}: {outcome:?} with first bad {first_bad:?}");
                }
            }
        }
    }
}
