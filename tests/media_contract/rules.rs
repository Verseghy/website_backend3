//! The contract tables of ADR 0015 §1 and ADR 0013 §7 as pure functions.
//!
//! The suite derives every expected header from these rules and the fixture
//! table, never from the code of a server under test, so a server cannot
//! pass by sharing a mistake with the suite. The fallback router keeps its
//! own copy of the rules in `src/`; that duplication is the point.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// ADR 0013 §7: the policy on every `/storage` response except a PDF.
pub const SANDBOX_CSP: &str =
    "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox";

/// ADR 0015 §1: entity images, whose names are unique per upload.
pub const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// ADR 0015 §1: everything else, because `uploads/` names can be replaced.
pub const ONE_HOUR: &str = "public, max-age=3600";

/// The folders whose files get [`IMMUTABLE`].
const IMMUTABLE_FOLDERS: [&str; 3] = ["posts_images/", "authors_images/", "colleagues_images/"];

/// The types that are shown inline (ADR 0013 §7, Download).
const INLINE_TYPES: [&str; 5] = [
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "application/pdf",
];

/// Bytes that stay literal in an encoded path: RFC 3986 unreserved
/// characters and the segment separator. Everything else, including every
/// byte of a non-ASCII character, is percent-encoded, as browsers do for the
/// accented names in content links.
const PATH: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// The `Cache-Control` a file under `/storage` must carry.
///
/// - `path`: the file's path below the storage root, such as
///   `posts_images/index/<hex>.jpg`, without a leading slash.
pub fn cache_control(path: &str) -> &'static str {
    if IMMUTABLE_FOLDERS
        .iter()
        .any(|folder| path.starts_with(folder))
    {
        IMMUTABLE
    } else {
        ONE_HOUR
    }
}

/// The media type of a `Content-Type` value: the part before any parameter,
/// trimmed and lower-cased, so `text/xml; charset=utf-8` and `text/xml` agree.
///
/// - `content_type`: the header value as sent.
pub fn media_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Whether a response must carry [`SANDBOX_CSP`]: every response except one
/// whose own type is `application/pdf` (ADR 0013 §7, Sandbox).
///
/// - `content_type`: the response's `Content-Type`, or `None` when it sends
///   none (a 204 preflight, a 405 without a body).
pub fn needs_sandbox(content_type: Option<&str>) -> bool {
    content_type.is_none_or(|value| media_type(value) != "application/pdf")
}

/// Whether a `200` or `206` for this file must carry
/// `Content-Disposition: attachment` (ADR 0013 §7, Download).
///
/// - `content_type`: the file's media type.
/// - `path`: the file's path below the storage root; only `sitemap.xml` at
///   the root is exempt, a `sitemap.xml` anywhere else is not.
pub fn needs_attachment(content_type: &str, path: &str) -> bool {
    path != "sitemap.xml" && !INLINE_TYPES.contains(&media_type(content_type).as_str())
}

/// The request target of a path below the storage root: `/storage/` plus
/// the path, percent-encoded as [`PATH`] says.
///
/// - `path`: the path below the storage root, in the exact bytes the volume
///   holds (UTF-8, as stored).
pub fn storage_target(path: &str) -> String {
    format!("/storage/{}", utf8_percent_encode(path, PATH))
}

/// Whether a comma-separated header value, such as
/// `Access-Control-Allow-Methods`, lists `token`, ignoring case and spaces.
///
/// - `value`: the header value.
/// - `token`: the token to look for.
pub fn lists(value: &str, token: &str) -> bool {
    value
        .split(',')
        .any(|item| item.trim().eq_ignore_ascii_case(token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use percent_encoding::percent_decode_str;
    use proptest::prelude::*;

    #[test]
    fn entity_folders_are_immutable_and_everything_else_one_hour() {
        for path in [
            "posts_images/index/0f1e2d3c4b5a69788796a5b4c3d2e1f0.jpg",
            "posts_images/a.JPG",
            "authors_images/a.png",
            "colleagues_images/a.jpg",
        ] {
            assert_eq!(cache_control(path), IMMUTABLE, "{path}");
        }
        for path in [
            "uploads/docs/a.pdf",
            "sitemap.xml",
            "posts_images_old/a.jpg",
            "uploads/posts_images/a.jpg",
        ] {
            assert_eq!(cache_control(path), ONE_HOUR, "{path}");
        }
    }

    #[test]
    fn media_type_drops_parameters_and_case() {
        assert_eq!(media_type("text/XML; charset=utf-8"), "text/xml");
        assert_eq!(media_type(" application/pdf "), "application/pdf");
        assert_eq!(media_type(""), "");
    }

    #[test]
    fn only_a_pdf_response_goes_without_the_sandbox() {
        assert!(!needs_sandbox(Some("application/pdf")));
        assert!(!needs_sandbox(Some("Application/PDF; x=y")));
        for value in [
            Some("text/html"),
            Some("image/jpeg"),
            Some("image/svg+xml"),
            None,
        ] {
            assert!(needs_sandbox(value), "{value:?}");
        }
    }

    #[test]
    fn raster_images_and_pdf_are_inline_and_the_rest_downloads() {
        for content_type in INLINE_TYPES {
            assert!(
                !needs_attachment(content_type, "uploads/x"),
                "{content_type}"
            );
        }
        for content_type in [
            "image/svg+xml",
            "text/html",
            "text/xml",
            "text/plain",
            "audio/mpeg",
            "application/octet-stream",
            "application/msword",
        ] {
            assert!(
                needs_attachment(content_type, "uploads/x"),
                "{content_type}"
            );
        }
    }

    #[test]
    fn only_the_root_sitemap_is_exempt() {
        assert!(!needs_attachment("text/xml", "sitemap.xml"));
        assert!(needs_attachment("text/xml", "uploads/docs/sitemap.xml"));
        assert!(needs_attachment("text/xml", "sitemap.xml.bak"));
    }

    #[test]
    fn targets_encode_spaces_brackets_and_accents() {
        assert_eq!(
            storage_target("uploads/docs/Felvételi (1) [PR].pdf"),
            "/storage/uploads/docs/Felv%C3%A9teli%20%281%29%20%5BPR%5D.pdf"
        );
        assert_eq!(
            storage_target("posts_images/index/a-b_c~d.JPG"),
            "/storage/posts_images/index/a-b_c~d.JPG"
        );
    }

    #[test]
    fn lists_compares_tokens_not_substrings() {
        assert!(lists("GET, HEAD, OPTIONS", "head"));
        assert!(lists("Range", "range"));
        assert!(!lists("GETTER", "GET"));
        assert!(!lists("", "GET"));
    }

    proptest! {
        #[test]
        fn a_target_decodes_back_to_the_stored_bytes(path in "[a-zA-Z0-9 ()\\[\\]#?%&+éáőűÁ/._~-]{0,40}") {
            let target = storage_target(&path);
            let encoded = target.strip_prefix("/storage/").expect("prefix");
            let decoded = percent_decode_str(encoded).decode_utf8().expect("UTF-8");
            prop_assert_eq!(decoded.as_ref(), path.as_str());
            prop_assert!(encoded.bytes().all(|byte| byte.is_ascii_graphic() && !b"?#[] ".contains(&byte)));
        }

        #[test]
        fn every_file_below_an_entity_folder_is_immutable(folder in 0usize..3, rest in "[a-z0-9/._]{0,30}") {
            let path = format!("{}{rest}", IMMUTABLE_FOLDERS[folder]);
            prop_assert_eq!(cache_control(&path), IMMUTABLE);
        }

        #[test]
        fn every_file_below_uploads_is_cached_one_hour(rest in "[a-zA-Z0-9/._ ]{0,30}") {
            prop_assert_eq!(cache_control(&format!("uploads/{rest}")), ONE_HOUR);
        }

        #[test]
        fn media_type_is_the_lower_cased_part_before_any_parameter(
            kind in "[A-Za-z]{1,12}/[A-Za-z0-9.+-]{1,24}",
            space in "[ \t]{0,3}",
            parameters in proptest::collection::vec("[a-z]{1,8}=[A-Za-z0-9-]{1,8}", 0..3),
        ) {
            let value = std::iter::once(format!("{space}{kind}{space}"))
                .chain(parameters)
                .collect::<Vec<_>>()
                .join(";");
            prop_assert_eq!(media_type(&value), kind.to_ascii_lowercase());
        }

        #[test]
        fn lists_finds_exactly_the_tokens_of_the_list(
            tokens in proptest::collection::vec("[A-Za-z-]{2,12}", 1..5),
            pick in any::<proptest::sample::Index>(),
            cut in any::<proptest::sample::Index>(),
            spacing in "[ \t]{0,2}",
            upper in any::<bool>(),
        ) {
            let value = tokens.join(&format!("{spacing},{spacing}"));
            let token = pick.get(&tokens);
            // Every listed token is found, whatever its case.
            let asked = if upper { token.to_ascii_uppercase() } else { token.to_ascii_lowercase() };
            prop_assert!(lists(&value, &asked));
            // A strict prefix or suffix of a token (both non-empty, as `at`
            // lies inside the token) is found only when it is itself one of
            // the tokens: tokens are compared, never searched.
            let at = 1 + cut.index(token.len() - 1);
            for part in [&token[..at], &token[at..]] {
                let listed = tokens.iter().any(|other| other.eq_ignore_ascii_case(part));
                prop_assert_eq!(lists(&value, part), listed, "{} in {}", part, value);
            }
        }

        #[test]
        fn inline_types_never_download_whatever_the_parameter_or_path(
            inline in 0usize..5,
            parameter in proptest::option::of("[a-z]{1,8}=[a-z0-9]{1,8}"),
            path in "[a-zA-Z0-9/._ -]{0,40}",
        ) {
            let content_type = match parameter {
                Some(parameter) => format!("{}; {parameter}", INLINE_TYPES[inline]),
                None => INLINE_TYPES[inline].to_owned(),
            };
            prop_assert!(!needs_attachment(&content_type, &path));
        }

        #[test]
        fn every_other_type_downloads_except_the_root_sitemap(
            kind in "[a-z]{1,12}/[a-z0-9.+-]{1,24}",
            path in "[a-zA-Z0-9/._ -]{0,40}",
        ) {
            prop_assume!(!INLINE_TYPES.contains(&kind.as_str()));
            prop_assert_eq!(needs_attachment(&kind, &path), path != "sitemap.xml");
            prop_assert!(!needs_attachment(&kind, "sitemap.xml"));
        }

        #[test]
        fn parameters_never_change_the_sandbox_rule(parameter in "[a-z]{1,8}=[a-z0-9]{1,8}") {
            let pdf = format!("application/pdf; {parameter}");
            let html = format!("text/html; {parameter}");
            prop_assert!(!needs_sandbox(Some(&pdf)));
            prop_assert!(needs_sandbox(Some(&html)));
        }
    }
}
