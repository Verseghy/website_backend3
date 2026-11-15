//! The base of the absolute URLs that backend3 mints: an absolute `http` or
//! `https` URL without a trailing slash (public-api.md §2.3).
//!
//! The public API builds every media URL as `{base}/{disk}/{stored path}`
//! by plain concatenation, so the configured text itself must already be
//! the exact prefix of every URL: a trailing slash would double the `/`, a
//! query or fragment would end up in the middle of the URL, and a spelling
//! that a URL parser normalises (an upper-case host, a default port, a
//! space) would mint URLs that differ from backend2's byte for byte. So
//! [`BaseUrl::parse`] accepts only the canonical form that the `url` crate
//! (the WHATWG URL parser, the same one sqlx and reqwest use) would write
//! itself, and says what to write instead.
//!
//! `media.public_base_url` is the first such base (ADR 0015 §3); the public
//! site address of the media library and the sitemap will be the second.
//! Both are read once at boot and passed down as values (ADR 0008).

use core::fmt;

use url::Url;

/// An absolute `http` or `https` URL with a host, without credentials, a
/// query, a fragment or a trailing slash, in its canonical spelling.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BaseUrl(String);

/// Why a configured text is not a [`BaseUrl`]. The messages say what to
/// write instead; the caller adds the setting key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BaseUrlError {
    /// The text is empty.
    #[error("is empty; set an absolute URL such as https://example.org/storage")]
    Empty,
    /// The text is not an absolute URL (a relative path, a bare host name).
    #[error(
        "is not an absolute URL; write the scheme and host, such as https://example.org/storage"
    )]
    NotAbsolute,
    /// The scheme is not `http` or `https`.
    #[error("uses the scheme `{0}`; only http and https URLs can be served to browsers")]
    Scheme(String),
    /// The URL names a user name or password, which no public URL may carry.
    #[error("holds a user name or password; remove them")]
    Credentials,
    /// The URL has a query or a fragment, which would land in the middle of
    /// every URL built on it.
    #[error("has a query or a fragment; remove everything from the first `?` or `#`")]
    QueryOrFragment,
    /// The URL ends with a slash, which would double the `/` before every
    /// path built on it.
    #[error("ends with a slash; remove the trailing `/`")]
    TrailingSlash,
    /// The URL is valid, but not written as the URL parser writes it, so the
    /// URLs built on it would not be byte-identical to the canonical ones.
    #[error("is not in canonical form; write it as {canonical}")]
    NotCanonical {
        /// The canonical spelling, without a trailing slash.
        canonical: String,
    },
}

impl BaseUrl {
    /// Checks `text` and keeps it as written.
    ///
    /// - `text`: the configured value, exactly as the configuration holds
    ///   it; nothing is trimmed, so a stray space is refused rather than
    ///   silently dropped.
    ///
    /// # Errors
    ///
    /// The first rule of [`BaseUrlError`] that `text` breaks, in the order
    /// of its variants.
    pub fn parse(text: &str) -> Result<Self, BaseUrlError> {
        if text.is_empty() {
            return Err(BaseUrlError::Empty);
        }
        let url = Url::parse(text).map_err(|_| BaseUrlError::NotAbsolute)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(BaseUrlError::Scheme(url.scheme().to_owned()));
        }
        // Both schemes are "special" in the WHATWG URL standard, so the
        // parser refuses them without a host: no host check is needed.
        if !url.username().is_empty() || url.password().is_some() {
            return Err(BaseUrlError::Credentials);
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(BaseUrlError::QueryOrFragment);
        }
        if text.ends_with('/') {
            return Err(BaseUrlError::TrailingSlash);
        }
        // The parser writes an empty path as `/`, the one slash a base
        // without a path may leave out. A text that the parser normalises
        // into a trailing slash (`/a/.`) is suggested without it.
        let canonical = url.as_str().trim_end_matches('/');
        if canonical != text {
            return Err(BaseUrlError::NotCanonical {
                canonical: canonical.to_owned(),
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// The URL as configured, with no trailing slash.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_production_value_is_accepted_as_written() {
        let text = "https://backend.microshift.verseghy-gimnazium.net/storage";
        assert_eq!(
            BaseUrl::parse(text).map(|url| url.to_string()),
            Ok(text.to_owned())
        );
        for text in [
            "http://localhost:5150/storage",
            "https://example.org",
            "http://127.0.0.1:8080/a/b",
            "https://[::1]:8443/storage",
        ] {
            assert_eq!(
                BaseUrl::parse(text).map(|url| url.as_str().to_owned()),
                Ok(text.to_owned())
            );
        }
    }

    #[test]
    fn each_rule_has_its_refusal() {
        for (text, expected) in [
            ("", BaseUrlError::Empty),
            ("/storage", BaseUrlError::NotAbsolute),
            ("backend.example.org/storage", BaseUrlError::NotAbsolute),
            ("https://", BaseUrlError::NotAbsolute),
            (
                "ftp://example.org/storage",
                BaseUrlError::Scheme("ftp".to_owned()),
            ),
            ("file:///srv/media", BaseUrlError::Scheme("file".to_owned())),
            ("https://user:secret@example.org", BaseUrlError::Credentials),
            ("https://user@example.org", BaseUrlError::Credentials),
            (
                "https://example.org/storage?x=1",
                BaseUrlError::QueryOrFragment,
            ),
            (
                "https://example.org/storage#top",
                BaseUrlError::QueryOrFragment,
            ),
            ("https://example.org/storage/", BaseUrlError::TrailingSlash),
            ("https://example.org/", BaseUrlError::TrailingSlash),
            (
                "HTTPS://Example.org/storage",
                BaseUrlError::NotCanonical {
                    canonical: "https://example.org/storage".to_owned(),
                },
            ),
            (
                "https://example.org:443/storage",
                BaseUrlError::NotCanonical {
                    canonical: "https://example.org/storage".to_owned(),
                },
            ),
            (
                " https://example.org",
                BaseUrlError::NotCanonical {
                    canonical: "https://example.org".to_owned(),
                },
            ),
            (
                "https://example.org/a b",
                BaseUrlError::NotCanonical {
                    canonical: "https://example.org/a%20b".to_owned(),
                },
            ),
        ] {
            assert_eq!(BaseUrl::parse(text), Err(expected), "{text:?}");
        }
    }

    #[test]
    fn messages_say_what_to_write() {
        assert_eq!(
            BaseUrlError::TrailingSlash.to_string(),
            "ends with a slash; remove the trailing `/`"
        );
        assert_eq!(
            BaseUrl::parse("https://EXAMPLE.org")
                .unwrap_err()
                .to_string(),
            "is not in canonical form; write it as https://example.org"
        );
    }

    /// A canonical base: a lower-case host, an optional port that is not the
    /// scheme's default, and path segments of unreserved characters.
    fn canonical_base() -> impl Strategy<Value = String> {
        (
            prop_oneof![Just("http"), Just("https")],
            "[a-z][a-z0-9-]{0,10}(\\.[a-z][a-z0-9-]{0,10}){0,2}",
            proptest::option::of(1024_u16..),
            proptest::collection::vec("[A-Za-z0-9._~-]{1,8}", 0..4),
        )
            .prop_filter("a segment of dots is normalised away", |(_, _, _, path)| {
                path.iter().all(|segment| segment.chars().any(|c| c != '.'))
            })
            .prop_map(|(scheme, host, port, path)| {
                let port = port.map(|port| format!(":{port}")).unwrap_or_default();
                let path = path.iter().fold(String::new(), |mut path, segment| {
                    path.push('/');
                    path.push_str(segment);
                    path
                });
                format!("{scheme}://{host}{port}{path}")
            })
    }

    proptest! {
        #[test]
        fn a_canonical_base_is_kept_exactly(text in canonical_base()) {
            prop_assert_eq!(BaseUrl::parse(&text).map(|url| url.as_str().to_owned()), Ok(text));
        }

        #[test]
        fn a_trailing_slash_is_always_refused(text in canonical_base()) {
            prop_assert_eq!(BaseUrl::parse(&format!("{text}/")), Err(BaseUrlError::TrailingSlash));
        }

        #[test]
        fn whatever_is_accepted_is_an_absolute_web_url_without_a_trailing_slash(text in "\\PC{0,40}") {
            if let Ok(url) = BaseUrl::parse(&text) {
                prop_assert_eq!(url.as_str(), text.as_str());
                prop_assert!(!text.ends_with('/'));
                prop_assert!(text.starts_with("http://") || text.starts_with("https://"));
                let parsed = Url::parse(&text).expect("parses");
                prop_assert!(parsed.query().is_none() && parsed.fragment().is_none());
            }
        }

        #[test]
        fn a_refused_non_canonical_base_names_an_accepted_spelling(text in "\\PC{0,40}") {
            if let Err(BaseUrlError::NotCanonical { canonical }) = BaseUrl::parse(&text) {
                prop_assert_eq!(
                    BaseUrl::parse(&canonical).map(|url| url.as_str().to_owned()),
                    Ok(canonical)
                );
            }
        }
    }
}
