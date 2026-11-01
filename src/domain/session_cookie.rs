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
//!
//! Logout clears the cookie with [`cleared_session_cookie`] (ADR 0006 §2):
//! the same name and attributes with an empty value and `Max-Age=0`, which
//! tells the browser to delete it at once. Both builders format the same
//! name and attribute list, because a browser deletes a cookie only when the
//! deleting `Set-Cookie` names the same cookie (name, host and `Path`), and
//! refuses a `__Host-` cookie, the deleting one included, without `Secure`
//! and `Path=/`.

/// The name of the admin session cookie (ADR 0006 §1). `auth.jwt.location`
/// in every `config/<env>.yaml` reads the token from this cookie first.
pub const SESSION_COOKIE_NAME: &str = "__Host-admin_session";

/// The `Set-Cookie` attributes of the session cookie except `Max-Age`, the
/// one attribute that setting and clearing write differently. Both
/// [`session_cookie`] and [`cleared_session_cookie`] format it in, so they
/// cannot drift apart.
const COOKIE_ATTRIBUTES: &str = "HttpOnly; Secure; SameSite=Strict; Path=/";

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
        "{SESSION_COOKIE_NAME}={token}; {COOKIE_ATTRIBUTES}; Max-Age={max_age_secs}"
    ))
}

/// Builds the `Set-Cookie` header value that deletes the session cookie in
/// the browser: `__Host-admin_session=; HttpOnly; Secure; SameSite=Strict;
/// Path=/; Max-Age=0` (ADR 0006 §2, "Logout").
///
/// The browser stores it in place of the session cookie, because it names
/// the same cookie: the same name, host and `Path` (RFC 6265 §5.3 step 11),
/// so the empty value replaces the token at once. `Max-Age=0` gives it the
/// earliest representable expiry (RFC 6265 §5.2.2: a `Max-Age` of zero or
/// less), and the browser evicts every expired cookie (RFC 6265 §5.3, after
/// step 12), so the cookie is gone; RFC 6265bis keeps both rules. The name
/// and attributes are those of [`session_cookie`], so the browser matches
/// the cookie it holds and accepts the `__Host-` name. Like
/// [`session_cookie`], it sends no `Expires`.
///
/// It takes no argument: the value is the same for every logout, so it
/// cannot fail.
#[must_use]
pub fn cleared_session_cookie() -> String {
    format!("{SESSION_COOKIE_NAME}=; {COOKIE_ATTRIBUTES}; Max-Age=0")
}

/// The cookie that a `Set-Cookie` value names, and its attributes: the name
/// before the first `=`, and every `;`-separated attribute after the value,
/// trimmed. `None` for a value without a `=` in its first part.
///
/// - `set_cookie`: a `Set-Cookie` header value.
///
/// A test helper only: it reads the values this module writes, not every
/// header a server could send.
#[cfg(test)]
fn name_and_attributes(set_cookie: &str) -> Option<(&str, Vec<&str>)> {
    let mut parts = set_cookie.split(';');
    let (name, _value) = parts.next()?.split_once('=')?;
    Some((name, parts.map(str::trim).collect()))
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
    fn the_cleared_cookie_has_an_empty_value_and_max_age_zero() {
        assert_eq!(
            cleared_session_cookie(),
            "__Host-admin_session=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"
        );
        let cleared = cleared_session_cookie();
        let (pair, _) = cleared.split_once(';').expect("attributes");
        assert_eq!(pair, format!("{SESSION_COOKIE_NAME}="));
    }

    #[test]
    fn the_cleared_cookie_names_the_cookie_that_sign_in_sets() {
        // A browser deletes only the cookie whose name, host and Path the
        // deleting `Set-Cookie` repeats, and takes a `__Host-` one only with
        // `Secure` and `Path=/` (the module documentation).
        let set = session_cookie("aaa.bbb.ccc", 7200).expect("valid");
        let cleared = cleared_session_cookie();
        let (set_name, mut set_attributes) = name_and_attributes(&set).expect("set");
        let (cleared_name, mut cleared_attributes) =
            name_and_attributes(&cleared).expect("cleared");
        assert_eq!(set_name, cleared_name);
        assert_eq!(set_attributes.pop(), Some("Max-Age=7200"));
        assert_eq!(cleared_attributes.pop(), Some("Max-Age=0"));
        assert_eq!(set_attributes, cleared_attributes);
        for required in ["Secure", "Path=/"] {
            assert!(cleared_attributes.contains(&required), "{required}");
        }
        assert!(
            !cleared_attributes
                .iter()
                .any(|attribute| attribute.starts_with("Domain")),
            "a __Host- cookie has no Domain"
        );
    }

    #[test]
    fn name_and_attributes_reads_the_values_this_module_writes() {
        assert_eq!(
            name_and_attributes("n=v; A; B=1"),
            Some(("n", vec!["A", "B=1"]))
        );
        assert_eq!(name_and_attributes("n="), Some(("n", vec![])));
        assert_eq!(name_and_attributes("no pair; A"), None);
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

        #[test]
        fn the_cleared_cookie_deletes_every_session_cookie_sign_in_can_set(
            token in JWT_LIKE,
            max_age in 1..=u64::MAX,
        ) {
            // Whatever token and lifetime sign-in used, logout's value names
            // the same cookie with the same attributes and differs only in
            // the empty value and `Max-Age=0`.
            let set = session_cookie(&token, max_age).expect("a JWT-shaped token is valid");
            let cleared = cleared_session_cookie();
            let (set_name, mut set_attributes) = name_and_attributes(&set).expect("set");
            let (cleared_name, mut cleared_attributes) =
                name_and_attributes(&cleared).expect("cleared");
            prop_assert_eq!(set_name, cleared_name);
            let expected_age = format!("Max-Age={max_age}");
            prop_assert_eq!(set_attributes.pop(), Some(expected_age.as_str()));
            prop_assert_eq!(cleared_attributes.pop(), Some("Max-Age=0"));
            prop_assert_eq!(set_attributes, cleared_attributes);
            let empty_value = format!("{SESSION_COOKIE_NAME}=;");
            prop_assert!(cleared.starts_with(&empty_value));
        }
    }
}
