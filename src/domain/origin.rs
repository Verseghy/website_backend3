//! The `Origin` check of cookie-authenticated admin writes (ADR 0006 §3, as
//! amended by ADR 0044).
//!
//! The admin session cookie is `SameSite=Strict`, which keeps it off
//! cross-site requests. Every host under `verseghy-gimnazium.net` is the same
//! site as the admin, though (ADR 0014), so a page on a sibling host, such as
//! the public site at `https://verseghy-gimnazium.net`, can still make a
//! browser send a write that carries the cookie. Browsers attach the
//! `Origin` header to every request that is not a `GET` or `HEAD` (the Fetch
//! standard), so backend3 refuses a cookie-authenticated write unless that
//! header names the admin's own origin. Against sibling hosts this check is
//! the control itself, not a second layer, so it fails closed: a write that
//! carries no `Origin` header is refused as well (the maintainer's answer of
//! 2026-10-25, research: DECISIONS R9 (D08)).
//!
//! Three pure parts, which the admin route table's guard combines before it
//! opens a session (`controllers::admin::origin`):
//!
//! - [`AdminOrigin`], the configured origin (`settings.admin_auth.origin`),
//!   checked at boot to be written exactly as a browser serialises an
//!   origin, so that the comparison can be byte for byte;
//! - [`is_checked`], which requests the check applies to: a write (every
//!   method but the safe ones, [`is_write_method`]) that carries one of the
//!   [`CREDENTIAL_COOKIES`];
//! - [`check_origin`], the comparison of the request's `Origin` headers with
//!   the admin origin.
//!
//! A request that carries a Bearer token and no credential cookie is exempt,
//! because browsers never attach a Bearer token on their own (ADR 0006 §3).
//! One that carries both is checked (ADR 0044 §1 narrows §3's exemption so):
//! Loco reads the token from the cookie first (`auth.jwt.location`), so it
//! is cookie-authenticated. Whether a
//! request carries a credential cookie is read by the caller with Loco's own
//! cookie reader, so that the check sees every cookie that Loco or the
//! two-factor step would take a token from.

use core::fmt;

use url::Url;

use super::session_cookie::{MFA_COOKIE_NAME, SESSION_COOKIE_NAME};

/// The cookies that authenticate a browser to the admin API: the session
/// cookie, from which Loco reads the token (ADR 0006 §1), and the cookie of
/// the two-factor step, which `POST /auth/totp` reads itself (ADR 0006 §4).
/// A write that carries either is checked.
pub const CREDENTIAL_COOKIES: [&str; 2] = [SESSION_COOKIE_NAME, MFA_COOKIE_NAME];

/// The methods that RFC 9110 §9.2.1 defines as safe. Every other method,
/// `POST`, `PUT`, `PATCH` and `DELETE` above all, is a write for the check.
/// Method names are case-sensitive (RFC 9110 §9.1), so `post` is not `POST`
/// and not safe either.
const SAFE_METHODS: [&str; 4] = ["GET", "HEAD", "OPTIONS", "TRACE"];

/// The admin UI's origin: the only value that the `Origin` header of a
/// cookie-authenticated write may carry.
///
/// It holds an `http` or `https` origin exactly as browsers serialise it
/// (the WHATWG URL standard's ASCII serialisation of an origin): the scheme,
/// `://`, the host in lower case (an internationalised name in its
/// `xn--` form, an IPv6 address in brackets), and a `:port` only when it is
/// not the scheme's default; no path, no trailing slash. Production's is
/// `https://backend3.microshift.verseghy-gimnazium.net` (ADR 0014).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdminOrigin(String);

/// Why a configured text is not an [`AdminOrigin`]. The messages say what to
/// write instead; the caller adds the setting key and, in production, the
/// environment variable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdminOriginError {
    /// The text is empty.
    #[error(
        "is empty; set the origin the admin is served from, such as https://backend3.example.org"
    )]
    Empty,
    /// The text is not an absolute URL (a bare host name, a path).
    #[error(
        "is not an absolute URL; write the scheme and the host, such as https://backend3.example.org"
    )]
    NotAbsolute,
    /// The scheme is not `http` or `https`.
    #[error("uses the scheme `{0}`; the admin is served over http or https only")]
    Scheme(String),
    /// The URL names a user name or a password, which an origin never holds.
    #[error("holds a user name or a password; remove them")]
    Credentials,
    /// The URL has a path (a trailing slash, a dot segment such as `/.` and
    /// a backslash included), a query or a fragment: it is a URL on the
    /// origin, not the origin.
    #[error("is not an origin; write only the scheme, the host and the port, as {origin}")]
    NotAnOrigin {
        /// The origin of the URL, as browsers send it.
        origin: String,
    },
    /// The text names an origin, but not in the spelling browsers send (an
    /// upper-case letter, the scheme's default port, a surrounding space),
    /// so no `Origin` header would ever match it byte for byte.
    #[error("is not written as browsers send an origin; write it as {canonical}")]
    NotCanonical {
        /// The spelling browsers send.
        canonical: String,
    },
}

impl AdminOrigin {
    /// Checks `text` and keeps it as written.
    ///
    /// - `text`: the configured value, exactly as the configuration holds
    ///   it; nothing is trimmed, so a stray space is refused rather than
    ///   silently dropped.
    ///
    /// # Errors
    ///
    /// The first rule of [`AdminOriginError`] that `text` breaks, in the
    /// order of its variants.
    pub fn parse(text: &str) -> Result<Self, AdminOriginError> {
        if text.is_empty() {
            return Err(AdminOriginError::Empty);
        }
        let url = Url::parse(text).map_err(|_| AdminOriginError::NotAbsolute)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(AdminOriginError::Scheme(url.scheme().to_owned()));
        }
        // Both schemes are "special" in the WHATWG URL standard, so the
        // parser refuses them without a host, and their origin is a tuple
        // (never opaque): no host check is needed.
        if !url.username().is_empty() || url.password().is_some() {
            return Err(AdminOriginError::Credentials);
        }
        let origin = url.origin().ascii_serialization();
        // The parsed URL alone cannot tell an origin from a URL on it: the
        // parser writes an empty path as `/`, drops dot segments (`/.`,
        // `/..`, `/%2e`) and reads a backslash as a slash, so `https://a/`,
        // `https://a/.` and `https://a\` all have the path `/`. The text is
        // therefore read as well, as the parser reads it.
        if has_text_after_authority(text)
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AdminOriginError::NotAnOrigin { origin });
        }
        if origin != text {
            return Err(AdminOriginError::NotCanonical { canonical: origin });
        }
        Ok(Self(text.to_owned()))
    }

    /// The origin as configured, which is how browsers send it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AdminOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Whether `text`, which the URL parser has read as an `http` or `https`
/// URL, holds anything after its authority: a path (a lone `/`, a dot
/// segment or a backslash included), a query or a fragment.
///
/// - `text`: the configured value, as [`AdminOrigin::parse`] got it.
///
/// It follows the WHATWG URL parser for these two "special" schemes: ASCII
/// tabs and newlines are ignored wherever they stand, the scheme ends at
/// the first `:`, the run of `/` and `\` after it leads into the authority,
/// and the authority ends at the next `/`, `\`, `?` or `#`. Leading spaces
/// and control characters, which the parser also drops, hold no `:`, so
/// the first `:` of the text is the scheme's.
fn has_text_after_authority(text: &str) -> bool {
    let Some((_, after_scheme)) = text.split_once(':') else {
        return false;
    };
    after_scheme
        .chars()
        .filter(|character| !matches!(character, '\t' | '\n' | '\r'))
        .skip_while(|character| matches!(character, '/' | '\\'))
        .any(|character| matches!(character, '/' | '\\' | '?' | '#'))
}

/// Whether a request with `method` is a write for the check: every method
/// but the safe ones of RFC 9110 §9.2.1 (`GET`, `HEAD`, `OPTIONS`, `TRACE`).
///
/// - `method`: the request method, as the request line spells it.
///
/// For the methods an admin route can be declared with (`GET` and the four
/// of ADR 0006 §3), these are exactly `POST`, `PUT`, `PATCH` and `DELETE`;
/// any other method counts as a write too, so the check fails closed.
#[must_use]
pub fn is_write_method(method: &str) -> bool {
    !SAFE_METHODS.contains(&method)
}

/// Whether the `Origin` check applies to a request: it is a write
/// ([`is_write_method`]) and carries a credential cookie
/// ([`CREDENTIAL_COOKIES`]).
///
/// - `method`: the request method.
/// - `carries_credential_cookie`: whether the request carries a cookie of
///   [`CREDENTIAL_COOKIES`], with any value, whatever else it carries (a
///   Bearer token included).
///
/// A read is never checked: it changes nothing, and without CORS headers a
/// page on another origin cannot read the answer (ADR 0014). A request
/// without a credential cookie, a script's Bearer request or a first
/// sign-in, carries no session that a page on another origin could borrow.
#[must_use]
pub fn is_checked(method: &str, carries_credential_cookie: bool) -> bool {
    carries_credential_cookie && is_write_method(method)
}

/// Why a checked request's `Origin` does not let it through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum OriginRefusal {
    /// The request carries no `Origin` header. Browsers send one with every
    /// write, so this is a client that does not, or one that strips it; the
    /// check fails closed (D08).
    #[error("the request carries no Origin header")]
    Missing,
    /// The request carries more than one `Origin` header, which no browser
    /// sends.
    #[error("the request carries more than one Origin header")]
    Repeated,
    /// The request's `Origin` header names another origin: another site, a
    /// sibling host, another scheme or port, or `null` (an opaque origin,
    /// such as a sandboxed frame's).
    #[error("the request's Origin header names another origin")]
    Foreign,
}

/// Checks the `Origin` headers of a request that [`is_checked`] applies to.
///
/// - `origins`: the values of every `Origin` header of the request, in
///   order, as raw bytes.
/// - `admin`: the admin origin.
///
/// The request passes only with exactly one `Origin` header whose value is
/// byte for byte the admin origin. There is no fallback to `Referer` (D08)
/// and no normalisation: browsers send the serialisation that
/// [`AdminOrigin`] holds, so any other spelling comes from a client that is
/// not a browser at the admin.
///
/// # Errors
///
/// [`OriginRefusal::Missing`] without an `Origin` header,
/// [`OriginRefusal::Repeated`] with more than one, and
/// [`OriginRefusal::Foreign`] when the one header names another origin.
pub fn check_origin<'a>(
    origins: impl IntoIterator<Item = &'a [u8]>,
    admin: &AdminOrigin,
) -> Result<(), OriginRefusal> {
    let mut origins = origins.into_iter();
    let Some(origin) = origins.next() else {
        return Err(OriginRefusal::Missing);
    };
    if origins.next().is_some() {
        return Err(OriginRefusal::Repeated);
    }
    if origin == admin.as_str().as_bytes() {
        Ok(())
    } else {
        Err(OriginRefusal::Foreign)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Production's admin origin (ADR 0014).
    const PRODUCTION: &str = "https://backend3.microshift.verseghy-gimnazium.net";

    fn production() -> AdminOrigin {
        AdminOrigin::parse(PRODUCTION).expect("production's origin")
    }

    #[test]
    fn production_s_origin_and_the_local_ones_are_accepted_as_written() {
        assert_eq!(production().as_str(), PRODUCTION);
        assert_eq!(production().to_string(), PRODUCTION);
        for text in [
            "http://localhost",
            "http://localhost:5150",
            "http://127.0.0.1:5150",
            "http://[::1]:5150",
            "https://xn--rvztr-wqa0gx3bwi.example",
        ] {
            assert_eq!(
                AdminOrigin::parse(text).map(|origin| origin.as_str().to_owned()),
                Ok(text.to_owned())
            );
        }
    }

    #[test]
    fn each_rule_has_its_refusal() {
        let not_an_origin = |origin: &str| AdminOriginError::NotAnOrigin {
            origin: origin.to_owned(),
        };
        let not_canonical = |canonical: &str| AdminOriginError::NotCanonical {
            canonical: canonical.to_owned(),
        };
        for (text, expected) in [
            ("", AdminOriginError::Empty),
            ("backend3.example.org", AdminOriginError::NotAbsolute),
            ("/admin", AdminOriginError::NotAbsolute),
            ("https://", AdminOriginError::NotAbsolute),
            (
                "ftp://example.org",
                AdminOriginError::Scheme("ftp".to_owned()),
            ),
            (
                "file:///srv/admin",
                AdminOriginError::Scheme("file".to_owned()),
            ),
            (
                "https://user:secret@example.org",
                AdminOriginError::Credentials,
            ),
            ("https://user@example.org", AdminOriginError::Credentials),
            ("https://example.org/", not_an_origin("https://example.org")),
            (
                "https://example.org/admin",
                not_an_origin("https://example.org"),
            ),
            (
                "https://example.org?x=1",
                not_an_origin("https://example.org"),
            ),
            ("https://example.org?", not_an_origin("https://example.org")),
            (
                "https://example.org#top",
                not_an_origin("https://example.org"),
            ),
            (
                "http://localhost:5150/",
                not_an_origin("http://localhost:5150"),
            ),
            // The parser drops dot segments and reads a backslash as a
            // slash, so each of these has the path `/` once parsed; the
            // text still holds a path.
            (
                "https://example.org/.",
                not_an_origin("https://example.org"),
            ),
            (
                "https://example.org/..",
                not_an_origin("https://example.org"),
            ),
            (
                "https://example.org/%2e",
                not_an_origin("https://example.org"),
            ),
            (
                "https://example.org/./",
                not_an_origin("https://example.org"),
            ),
            (
                "https://example.org\\",
                not_an_origin("https://example.org"),
            ),
            (
                "https://example.org\\admin",
                not_an_origin("https://example.org"),
            ),
            (
                "https://example.org/\t",
                not_an_origin("https://example.org"),
            ),
            ("HTTPS://Example.org", not_canonical("https://example.org")),
            // Spellings of the authority alone that the parser also reads:
            // backslashes or no slashes after the scheme, a tab in the host.
            (
                "https:\\\\example.org",
                not_canonical("https://example.org"),
            ),
            ("https:example.org", not_canonical("https://example.org")),
            (
                "https://exa\tmple.org",
                not_canonical("https://example.org"),
            ),
            (
                "https://example.org:443",
                not_canonical("https://example.org"),
            ),
            ("http://localhost:80", not_canonical("http://localhost")),
            (" https://example.org", not_canonical("https://example.org")),
            ("https://example.org ", not_canonical("https://example.org")),
            (
                "https://árvíztűrő.example",
                not_canonical("https://xn--rvztr-wqa0gx3bwi.example"),
            ),
        ] {
            assert_eq!(AdminOrigin::parse(text), Err(expected), "{text:?}");
        }
    }

    #[test]
    fn the_text_after_the_authority_is_found_as_the_parser_reads_it() {
        for (text, after) in [
            ("https://example.org", false),
            ("http://[::1]:5150", false),
            ("https:example.org", false),
            ("https:\\\\example.org", false),
            ("https:/\t/example.org", false),
            ("https://exa\nmple.org", false),
            ("https", false),
            ("https://example.org/", true),
            ("https://example.org/.", true),
            ("https://example.org\\", true),
            ("https://example.org?", true),
            ("https://example.org#", true),
            ("https://example.org:443/x", true),
            ("https:example.org/", true),
            ("https://example.org/\n", true),
        ] {
            assert_eq!(has_text_after_authority(text), after, "{text:?}");
        }
    }

    #[test]
    fn messages_say_what_to_write() {
        assert_eq!(
            AdminOrigin::parse(&format!("{PRODUCTION}/"))
                .unwrap_err()
                .to_string(),
            format!(
                "is not an origin; write only the scheme, the host and the port, as {PRODUCTION}"
            )
        );
        assert_eq!(
            AdminOrigin::parse("https://EXAMPLE.org")
                .unwrap_err()
                .to_string(),
            "is not written as browsers send an origin; write it as https://example.org"
        );
        assert_eq!(
            AdminOriginError::Empty.to_string(),
            "is empty; set the origin the admin is served from, such as https://backend3.example.org"
        );
    }

    #[test]
    fn the_credential_cookies_are_the_session_and_the_two_factor_cookie() {
        assert_eq!(
            CREDENTIAL_COOKIES,
            ["__Host-admin_session", "__Host-admin_mfa"]
        );
    }

    #[test]
    fn the_four_write_methods_and_unknown_ones_are_writes() {
        for method in [
            "POST", "PUT", "PATCH", "DELETE", "PROPFIND", "post", "get", "",
        ] {
            assert!(is_write_method(method), "{method:?}");
        }
        for method in SAFE_METHODS {
            assert!(!is_write_method(method), "{method}");
        }
    }

    #[test]
    fn only_a_write_with_a_credential_cookie_is_checked() {
        for (method, cookie, checked) in [
            ("POST", true, true),
            ("PUT", true, true),
            ("PATCH", true, true),
            ("DELETE", true, true),
            ("POST", false, false),
            ("DELETE", false, false),
            ("GET", true, false),
            ("HEAD", true, false),
            ("OPTIONS", true, false),
            ("GET", false, false),
        ] {
            assert_eq!(is_checked(method, cookie), checked, "{method} {cookie}");
        }
    }

    /// [`check_origin`] over text values.
    fn check(origins: &[&str], admin: &AdminOrigin) -> Result<(), OriginRefusal> {
        check_origin(origins.iter().map(|origin| origin.as_bytes()), admin)
    }

    #[test]
    fn only_the_admin_origin_alone_passes() {
        let admin = production();
        assert_eq!(check(&[PRODUCTION], &admin), Ok(()));
        assert_eq!(check(&[], &admin), Err(OriginRefusal::Missing));
        assert_eq!(
            check(&[PRODUCTION, PRODUCTION], &admin),
            Err(OriginRefusal::Repeated)
        );
        assert_eq!(
            check(&[PRODUCTION, "https://verseghy-gimnazium.net"], &admin),
            Err(OriginRefusal::Repeated)
        );
        for foreign in [
            // The public site and the legacy host: the same site as the
            // admin, so their requests carry the cookie (ADR 0014).
            "https://verseghy-gimnazium.net",
            "https://backend.microshift.verseghy-gimnazium.net",
            "https://backend2.microshift.verseghy-gimnazium.net",
            // Another scheme or port of the same host.
            "http://backend3.microshift.verseghy-gimnazium.net",
            "https://backend3.microshift.verseghy-gimnazium.net:8443",
            // Spellings browsers never send.
            "https://backend3.microshift.verseghy-gimnazium.net:443",
            "https://backend3.microshift.verseghy-gimnazium.net/",
            "https://BACKEND3.microshift.verseghy-gimnazium.net",
            " https://backend3.microshift.verseghy-gimnazium.net",
            // RFC 6454's list form, an opaque origin, and nothing.
            "https://backend3.microshift.verseghy-gimnazium.net https://verseghy-gimnazium.net",
            "null",
            "",
            // A host that merely starts or ends like the admin's.
            "https://backend3.microshift.verseghy-gimnazium.net.example",
            "https://evil-backend3.microshift.verseghy-gimnazium.net",
        ] {
            assert_eq!(
                check(&[foreign], &admin),
                Err(OriginRefusal::Foreign),
                "{foreign:?}"
            );
        }
        // Bytes that are not text are compared as bytes, and refused.
        assert_eq!(
            check_origin([&b"https://backend3\xff"[..]], &admin),
            Err(OriginRefusal::Foreign)
        );
    }

    #[test]
    fn refusals_name_their_reason() {
        assert_eq!(
            OriginRefusal::Missing.to_string(),
            "the request carries no Origin header"
        );
        assert_eq!(
            OriginRefusal::Repeated.to_string(),
            "the request carries more than one Origin header"
        );
        assert_eq!(
            OriginRefusal::Foreign.to_string(),
            "the request's Origin header names another origin"
        );
    }

    /// An origin as browsers serialise it: `http` or `https`, a lower-case
    /// host of one to three labels, and a port that is not a default one.
    fn browser_origin() -> impl Strategy<Value = String> {
        (
            prop_oneof![Just("http"), Just("https")],
            "[a-z][a-z0-9-]{0,10}(\\.[a-z][a-z0-9-]{0,10}){0,2}",
            proptest::option::of(1024_u16..),
        )
            // A label that starts with `xn--` is an internationalised name
            // in Punycode, which a random label rarely is: the parser
            // refuses it, so such a host is no browser origin. The unit
            // tests cover a valid one.
            .prop_filter(
                "a label ends with a letter or digit and is no xn-- label",
                |(_, host, _)| {
                    host.split('.')
                        .all(|label| !label.ends_with('-') && !label.starts_with("xn--"))
                },
            )
            .prop_map(|(scheme, host, port)| {
                let port = port.map(|port| format!(":{port}")).unwrap_or_default();
                format!("{scheme}://{host}{port}")
            })
    }

    proptest! {
        #[test]
        fn a_browser_origin_is_kept_exactly(text in browser_origin()) {
            prop_assert_eq!(
                AdminOrigin::parse(&text).map(|origin| origin.as_str().to_owned()),
                Ok(text)
            );
        }

        #[test]
        fn anything_after_the_origin_is_refused(
            text in browser_origin(),
            rest in prop_oneof![
                "[/?#\\\\][!-~]{0,12}",
                // Dot segments, which the parser drops from the path, so
                // only the text shows them.
                "([/\\\\](\\.|\\.\\.|%2[eE]|\\.%2[eE]|%2[eE]\\.|%2[eE]%2[eE]))+[/\\\\]?",
            ],
        ) {
            prop_assert!(
                matches!(
                    AdminOrigin::parse(&format!("{text}{rest}")),
                    Err(AdminOriginError::NotAnOrigin { origin }) if origin == text
                ),
                "{}{}", text, rest
            );
        }

        #[test]
        fn whatever_is_accepted_is_its_own_browser_origin(text in "\\PC{0,40}") {
            if let Ok(origin) = AdminOrigin::parse(&text) {
                prop_assert_eq!(origin.as_str(), text.as_str());
                let url = Url::parse(&text).expect("parses");
                prop_assert_eq!(url.origin().ascii_serialization(), text);
            }
        }

        #[test]
        fn a_refused_spelling_names_an_accepted_one(text in "\\PC{0,40}") {
            if let Err(
                AdminOriginError::NotCanonical { canonical: suggested }
                | AdminOriginError::NotAnOrigin { origin: suggested },
            ) = AdminOrigin::parse(&text)
            {
                prop_assert_eq!(
                    AdminOrigin::parse(&suggested).map(|origin| origin.as_str().to_owned()),
                    Ok(suggested)
                );
            }
        }

        #[test]
        fn only_the_safe_methods_are_not_writes(method in "[A-Za-z-]{0,10}") {
            prop_assert_eq!(is_write_method(&method), !SAFE_METHODS.contains(&method.as_str()));
        }

        #[test]
        fn nothing_without_a_credential_cookie_is_checked(method in "[A-Za-z-]{0,10}") {
            prop_assert!(!is_checked(&method, false));
            prop_assert_eq!(is_checked(&method, true), is_write_method(&method));
        }

        #[test]
        fn a_request_passes_only_with_the_admin_origin_alone(
            admin in browser_origin(),
            origins in proptest::collection::vec(
                prop_oneof![
                    browser_origin().prop_map(String::into_bytes),
                    proptest::collection::vec(any::<u8>(), 0..40),
                ],
                0..3,
            ),
        ) {
            let parsed = AdminOrigin::parse(&admin).expect("a browser origin");
            let expected = match origins.as_slice() {
                [] => Err(OriginRefusal::Missing),
                [one] if one.as_slice() == admin.as_bytes() => Ok(()),
                [_] => Err(OriginRefusal::Foreign),
                _ => Err(OriginRefusal::Repeated),
            };
            prop_assert_eq!(check_origin(origins.iter().map(Vec::as_slice), &parsed), expected);
        }

        #[test]
        fn the_admin_origin_itself_always_passes_and_any_change_to_it_is_refused(
            admin in browser_origin(),
            index in any::<prop::sample::Index>(),
            replacement in any::<u8>(),
        ) {
            let parsed = AdminOrigin::parse(&admin).expect("a browser origin");
            prop_assert_eq!(check_origin([admin.as_bytes()], &parsed), Ok(()));
            let mut changed = admin.clone().into_bytes();
            let at = index.index(changed.len());
            prop_assume!(changed[at] != replacement);
            changed[at] = replacement;
            prop_assert_eq!(check_origin([changed.as_slice()], &parsed), Err(OriginRefusal::Foreign));
            // A prefix or an extension of it is another origin, too.
            prop_assert_eq!(
                check_origin([&admin.as_bytes()[..at]], &parsed),
                Err(OriginRefusal::Foreign)
            );
            let longer = format!("{admin}/");
            prop_assert_eq!(check_origin([longer.as_bytes()], &parsed), Err(OriginRefusal::Foreign));
        }
    }
}
