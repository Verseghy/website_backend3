//! The entity tag of a public API answer (public-api.md §9, DEC-12).
//!
//! backend3 keeps no server-side response cache: every request reads
//! PostgreSQL, so an admin edit is visible on the next request (ADR 0016).
//! Browsers and the SSR pod may keep a copy of an answer, but `no-cache`
//! makes them revalidate it before every use. Loco's `etag` middleware
//! answers a revalidation whose `If-None-Match` equals the answer's `ETag`
//! with `304 Not Modified` and no body, but it never computes an `ETag`
//! itself. So backend3 puts one on every cacheable answer: [`weak_etag`] of
//! the serialised JSON body.
//!
//! The form of the tag, and why:
//!
//! - **Weak** (`W/"..."`, RFC 9110 §8.8.1 and §8.8.3). The compression
//!   middleware sends the same JSON gzip-compressed or not, depending on the
//!   request; a strong validator would have to differ per content coding.
//!   A weak one claims "the same content", which is exactly what a hash of
//!   the uncompressed body shows.
//! - **SHA-256 of the body, in lowercase hex.** The algorithm has a fixed,
//!   published definition, so every replica and every release gives the
//!   same tag for the same bytes, and two different bodies never share a
//!   tag in practice: a shared tag would let a client keep a stale copy.
//!   Rust's `DefaultHasher` has neither property; its algorithm may change
//!   between Rust releases. Lowercase hex uses only characters that RFC 9110
//!   allows inside an entity tag (`etagc`).
//! - **The body only.** The tag depends on nothing else: not on the request,
//!   the time, the process or the content coding.

use sha2::{Digest as _, Sha256};

/// The number of hex digits between the quotes of a tag: SHA-256 has 32
/// bytes.
pub const HASH_HEX_DIGITS: usize = 64;

/// The text before the hash: the weak indicator and the opening quote.
const PREFIX: &str = "W/\"";

/// The lowercase hex digits, indexed by their value.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Returns the weak entity tag of a response body, `W/"<hash>"`, where
/// `<hash>` is the SHA-256 of the body in lowercase hex.
///
/// - `body`: the bytes of the answer as the handler serialised them, before
///   any compression.
///
/// The result is always [`HASH_HEX_DIGITS`] + 4 ASCII characters long, so it
/// is a valid header value.
#[must_use]
pub fn weak_etag(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    let mut tag = String::with_capacity(PREFIX.len() + HASH_HEX_DIGITS + 1);
    tag.push_str(PREFIX);
    for byte in digest {
        tag.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        tag.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    tag.push('"');
    tag
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Whether `tag` has the form `W/"<64 lowercase hex digits>"`.
    fn is_weak_sha256_tag(tag: &str) -> bool {
        tag.strip_prefix(PREFIX)
            .and_then(|rest| rest.strip_suffix('"'))
            .is_some_and(|hash| {
                hash.len() == HASH_HEX_DIGITS
                    && hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
    }

    #[test]
    fn the_tag_is_the_sha256_of_the_body() {
        // FIPS 180-2's test vectors for SHA-256.
        assert_eq!(
            weak_etag(b""),
            r#"W/"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855""#
        );
        assert_eq!(
            weak_etag(b"abc"),
            r#"W/"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad""#
        );
    }

    #[test]
    fn a_json_body_gets_a_well_formed_weak_tag() {
        let tag = weak_etag(r#"{"items":[],"pageInfo":{"endCursor":null}}"#.as_bytes());
        assert!(is_weak_sha256_tag(&tag), "{tag}");
        assert_eq!(tag.len(), HASH_HEX_DIGITS + 4);
    }

    #[test]
    fn one_changed_byte_changes_the_tag() {
        assert_ne!(weak_etag(br#"{"id":1}"#), weak_etag(br#"{"id":2}"#));
        // Bytes, not characters: the same text in another normal form is
        // another body.
        assert_ne!(weak_etag("é".as_bytes()), weak_etag("e\u{301}".as_bytes()));
    }

    #[test]
    fn the_shape_checker_refuses_other_tags() {
        // Guards the helper the properties rely on.
        assert!(!is_weak_sha256_tag(&weak_etag(b"x")[2..]));
        assert!(!is_weak_sha256_tag(&weak_etag(b"x").to_uppercase()));
        assert!(!is_weak_sha256_tag(r#"W/"abc""#));
    }

    proptest! {
        #[test]
        fn every_body_gets_a_weak_sha256_tag(body in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let tag = weak_etag(&body);
            prop_assert!(is_weak_sha256_tag(&tag), "{}", tag);
            // RFC 9110 §8.8.3: every character is visible ASCII, which a
            // header value can carry as it is.
            prop_assert!(tag.bytes().all(|byte| (0x21..=0x7e).contains(&byte)));
        }

        #[test]
        fn the_tag_depends_only_on_the_body(body in proptest::collection::vec(any::<u8>(), 0..512)) {
            prop_assert_eq!(weak_etag(&body), weak_etag(&body.clone()));
        }

        #[test]
        fn different_bodies_get_different_tags(
            first in proptest::collection::vec(any::<u8>(), 0..256),
            second in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            prop_assume!(first != second);
            prop_assert_ne!(weak_etag(&first), weak_etag(&second));
        }
    }
}
