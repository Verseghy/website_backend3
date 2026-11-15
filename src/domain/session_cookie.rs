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
//!
//! The sliding refresh (ADR 0006 §1) sets the same cookie again, with a new
//! token and the new token's lifetime as `Max-Age`. [`names_session_cookie`]
//! tells whether an answer already sets, replaces or deletes it, so that the
//! passive refresh never adds a second one beside a handler's own.

/// The name of the admin session cookie (ADR 0006 §1). `auth.jwt.location`
/// in every `config/<env>.yaml` reads the token from this cookie first.
pub const SESSION_COOKIE_NAME: &str = "__Host-admin_session";

/// The name of the cookie that carries the `mfa` token from the password
/// step of the sign-in to the TOTP step (ADR 0006 §4). It has the session
/// cookie's attributes with `Max-Age=300`, and `POST /api/admin/auth/totp`
/// reads it itself, so `auth.jwt.location` never names it. The `Origin`
/// check counts it as a credential cookie from the start
/// (`domain::origin::CREDENTIAL_COOKIES`, ADR 0044), so the TOTP step is
/// covered as soon as it exists.
pub const MFA_COOKIE_NAME: &str = "__Host-admin_mfa";

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

/// Whether a `Set-Cookie` header value sets, replaces or deletes the session
/// cookie: whether the name of its cookie is [`SESSION_COOKIE_NAME`].
///
/// - `set_cookie`: the header value's bytes, which need not be UTF-8.
///
/// The name is read as a browser reads it (RFC 6265 §5.2): the part before
/// the first `;`, up to its first `=`, without the spaces and tabs around
/// it. Cookie names are case-sensitive. A value without a `=` there sets no
/// cookie at all and names none.
///
/// The admin route table's guard asks it before it adds the passive
/// refresh's cookie to an answer (ADR 0006 §1): an answer whose handler
/// already set the session cookie (the logout's deletion, the explicit
/// refresh, later a password change) keeps that cookie alone, because a
/// browser keeps the last of two cookies with one name.
#[must_use]
pub fn names_session_cookie(set_cookie: &[u8]) -> bool {
    let pair = set_cookie
        .split(|&byte| byte == b';')
        .next()
        .unwrap_or_default();
    let Some(equals) = pair.iter().position(|&byte| byte == b'=') else {
        return false;
    };
    trim_space_and_tab(&pair[..equals]) == SESSION_COOKIE_NAME.as_bytes()
}

/// `bytes` without the spaces and horizontal tabs at either end: RFC 6265
/// §5.2's whitespace around a cookie name, and nothing wider.
fn trim_space_and_tab(bytes: &[u8]) -> &[u8] {
    let is_blank = |byte: &u8| *byte == b' ' || *byte == b'\t';
    let start = bytes
        .iter()
        .position(|byte| !is_blank(byte))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !is_blank(byte))
        .map_or(start, |last| last + 1);
    &bytes[start..end]
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
    fn both_cookie_names_carry_the_host_prefix() {
        // ADR 0006 §1, §4: the `__Host-` prefix keeps sibling hosts from
        // setting or overwriting either cookie.
        assert_eq!(SESSION_COOKIE_NAME, "__Host-admin_session");
        assert_eq!(MFA_COOKIE_NAME, "__Host-admin_mfa");
        assert_ne!(SESSION_COOKIE_NAME, MFA_COOKIE_NAME);
    }

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
    fn the_session_cookie_is_recognised_by_its_name_only() {
        // What this module writes: setting and deleting.
        let set = session_cookie("aaa.bbb.ccc", 7200).expect("valid");
        assert!(names_session_cookie(set.as_bytes()));
        assert!(names_session_cookie(cleared_session_cookie().as_bytes()));
        for (value, names) in [
            ("__Host-admin_session=x", true),
            (" __Host-admin_session\t=x; Path=/", true),
            ("__Host-admin_session=", true),
            // Another cookie, or the name in the wrong place.
            ("theme=dark; __Host-admin_session=x", false),
            ("__Host-admin_mfa=x; HttpOnly", false),
            ("__host-admin_session=x", false),
            ("__Host-admin_session_old=x", false),
            ("x=__Host-admin_session", false),
            // No name-value pair at all.
            ("__Host-admin_session", false),
            ("__Host-admin_session; Max-Age=0", false),
            ("", false),
        ] {
            assert_eq!(names_session_cookie(value.as_bytes()), names, "{value:?}");
        }
        // A value that is not UTF-8 is read as bytes.
        assert!(names_session_cookie(b"__Host-admin_session=\xff\xfe"));
        assert!(!names_session_cookie(b"\xff=x"));
    }

    #[test]
    fn trim_space_and_tab_trims_only_those() {
        assert_eq!(trim_space_and_tab(b" \t a b \t"), b"a b");
        assert_eq!(trim_space_and_tab(b"\n a"), b"\n a");
        assert_eq!(trim_space_and_tab(b"  "), b"");
        assert_eq!(trim_space_and_tab(b""), b"");
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

        #[test]
        fn a_set_cookie_names_the_session_cookie_exactly_when_its_name_is_it(
            name in prop_oneof![
                3 => Just(SESSION_COOKIE_NAME.to_owned()),
                1 => "[ \t]{0,2}__Host-admin_session[ \t]{0,2}",
                // Cookie-name characters: no `=`, `;` or blank.
                2 => "[!-:<>-~]{0,24}",
            ],
            value in "[!-:<-~]{0,30}",
            attributes in "(; [A-Za-z-]{1,8}(=[!-:<-~]{0,8})?){0,4}",
        ) {
            let header = format!("{name}={value}{attributes}");
            prop_assert_eq!(
                names_session_cookie(header.as_bytes()),
                name.trim_matches([' ', '\t']) == SESSION_COOKIE_NAME,
                "{}", header
            );
        }

        #[test]
        fn every_cookie_this_module_writes_names_the_session_cookie(
            token in JWT_LIKE,
            max_age in 1..=u64::MAX,
        ) {
            let set = session_cookie(&token, max_age).expect("valid");
            prop_assert!(names_session_cookie(set.as_bytes()));
        }
    }
}
