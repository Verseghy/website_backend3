//! Classification of `/admin` request paths and their `Cache-Control` policy
//! (ADR 0004 §3).
//!
//! backend3 serves the built admin single-page app under `/admin` with
//! browser-history routing. Every request under that prefix is one of two
//! things: a request for a file of the Vite build (`dist/`), or a client-side
//! route that the app shell (`index.html`) renders in the browser. The server
//! cannot ask the app which routes exist, so the decision is made from the
//! path alone:
//!
//! - **The last path segment contains a dot: a file request.** Client routes
//!   never put a dot in their last segment (ADR 0004 §3); the media library,
//!   for example, passes folder paths as a query parameter. A file request
//!   that matches no file is a `404`, never `index.html`, so a missing
//!   `app-1a2b.js` cannot come back as HTML with a confusing MIME error.
//! - **The first segment is `assets`: a build asset.** Vite writes its hashed
//!   output there, so a hit is cached as immutable for a year. The directory
//!   is reserved for build output: every path under it is a file request,
//!   whether or not its last segment has a dot.
//! - **Everything else: a client route**, answered with `index.html`.
//!
//! The classifier works on the percent-decoded path, as tower-http's
//! `ServeDir` does, so `app%2Ejs` and `app.js` are classified alike. A path
//! with a `..` segment, invalid UTF-8 after decoding, or a NUL byte is
//! rejected outright, before anything touches the file system.

use percent_encoding::percent_decode_str;

/// What a request below `/admin` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdminRequest {
    /// `/admin` or `/admin/`: the app shell, `dist/index.html`.
    Index,
    /// A client-side route such as `/admin/posts/12/show`: also
    /// `dist/index.html`, because the browser router renders it.
    ClientRoute,
    /// A file under `/admin/assets/`: Vite's content-hashed build output.
    Asset,
    /// Any other file request, such as `/admin/favicon.ico`.
    File,
    /// A path that must never reach the file system: a `..` segment, invalid
    /// UTF-8 after percent-decoding, a NUL byte, or no leading slash.
    Rejected,
}

impl AdminRequest {
    /// Whether the response body is `dist/index.html` rather than a file
    /// looked up from the path.
    #[must_use]
    pub const fn serves_index(self) -> bool {
        matches!(self, Self::Index | Self::ClientRoute)
    }
}

/// The name of the build-output directory inside `dist/` (Vite's default
/// `build.assetsDir`).
pub const ASSETS_DIR: &str = "assets";

/// Classifies the part of a request path that follows the `/admin` prefix.
///
/// - `rest`: the raw (still percent-encoded) path after `/admin`, for example
///   `""` for `/admin`, `"/"` for `/admin/`, or `"/assets/app-1a2b.js"`. It
///   must be empty or start with `/`; anything else is
///   [`AdminRequest::Rejected`], because the router never produces it.
///
/// Empty segments and `.` segments are ignored, as `ServeDir` ignores them.
#[must_use]
pub fn classify(rest: &str) -> AdminRequest {
    if !(rest.is_empty() || rest.starts_with('/')) {
        return AdminRequest::Rejected;
    }
    let Ok(decoded) = percent_decode_str(rest).decode_utf8() else {
        return AdminRequest::Rejected;
    };
    if decoded.contains('\0') {
        return AdminRequest::Rejected;
    }

    let segments: Vec<&str> = decoded
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.contains(&"..") {
        return AdminRequest::Rejected;
    }

    match (segments.first(), segments.last()) {
        (None, _) | (_, None) => AdminRequest::Index,
        (Some(&first), _) if first == ASSETS_DIR => AdminRequest::Asset,
        (_, Some(last)) if last.contains('.') => AdminRequest::File,
        _ => AdminRequest::ClientRoute,
    }
}

/// `Cache-Control` of `index.html`, of root files and of client routes: the
/// browser may keep a copy but must revalidate it, so a reload after a
/// deploy fetches the new app shell (ADR 0004 §5).
pub const CACHE_REVALIDATE: &str = "no-cache";
/// `Cache-Control` of a found build asset. Asset names carry a content hash,
/// so a given URL never changes its content.
pub const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// `Cache-Control` of every error response. A `404` must never be cached for
/// long: during a rolling update a newer pod may serve the asset that an
/// older pod did not have (ADR 0004 §3).
pub const CACHE_NO_STORE: &str = "no-store";

/// Whether a status counts as a successful file response for caching: `200`,
/// `206` (a byte range) or `304` (a successful revalidation).
#[must_use]
pub const fn is_cacheable_success(status: u16) -> bool {
    matches!(status, 200 | 206 | 304)
}

/// The `Cache-Control` value for a response to a classified `/admin` request.
///
/// - `request`: the classification of the request path ([`classify`]).
/// - `status`: the HTTP status code of the response that is about to be sent.
///
/// Successful responses get [`CACHE_IMMUTABLE`] for assets and
/// [`CACHE_REVALIDATE`] for everything else. Every other status, and every
/// rejected path, gets [`CACHE_NO_STORE`], so the one-year lifetime is never
/// attached to an error. `405` responses come from the router, not from here,
/// and carry no `Cache-Control` at all.
#[must_use]
pub const fn cache_control(request: AdminRequest, status: u16) -> &'static str {
    if !is_cacheable_success(status) {
        return CACHE_NO_STORE;
    }
    match request {
        AdminRequest::Asset => CACHE_IMMUTABLE,
        AdminRequest::Index | AdminRequest::ClientRoute | AdminRequest::File => CACHE_REVALIDATE,
        AdminRequest::Rejected => CACHE_NO_STORE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn index_paths() {
        assert_eq!(classify(""), AdminRequest::Index);
        assert_eq!(classify("/"), AdminRequest::Index);
        assert_eq!(classify("//"), AdminRequest::Index);
        assert_eq!(classify("/./"), AdminRequest::Index);
    }

    #[test]
    fn client_routes() {
        assert_eq!(classify("/posts"), AdminRequest::ClientRoute);
        assert_eq!(classify("/posts/12/show"), AdminRequest::ClientRoute);
        assert_eq!(classify("/posts/12/show/"), AdminRequest::ClientRoute);
        assert_eq!(classify("/media"), AdminRequest::ClientRoute);
        // A dot in an earlier segment does not make a file request.
        assert_eq!(classify("/v1.2/posts"), AdminRequest::ClientRoute);
        // `assets` only counts as the first segment.
        assert_eq!(classify("/posts/assets"), AdminRequest::ClientRoute);
        assert_eq!(classify("/assetsx"), AdminRequest::ClientRoute);
    }

    #[test]
    fn assets_are_always_file_requests() {
        assert_eq!(classify("/assets/index-B7xKq2Lm.js"), AdminRequest::Asset);
        assert_eq!(classify("/assets/missing.css"), AdminRequest::Asset);
        assert_eq!(classify("/assets/fonts/inter.woff2"), AdminRequest::Asset);
        assert_eq!(classify("/assets/"), AdminRequest::Asset);
        assert_eq!(classify("/assets"), AdminRequest::Asset);
        assert_eq!(classify("/assets/no-extension"), AdminRequest::Asset);
        assert_eq!(classify("//assets//app.js"), AdminRequest::Asset);
    }

    #[test]
    fn root_and_nested_files() {
        assert_eq!(classify("/favicon.ico"), AdminRequest::File);
        assert_eq!(classify("/index.html"), AdminRequest::File);
        assert_eq!(classify("/icons/logo.svg"), AdminRequest::File);
        assert_eq!(classify("/posts/12/show.json"), AdminRequest::File);
        // A leading dot is still a dot: never an index.html fallback.
        assert_eq!(classify("/.env"), AdminRequest::File);
        assert_eq!(classify("/trailing."), AdminRequest::File);
    }

    #[test]
    fn percent_encoding_is_decoded_before_classifying() {
        assert_eq!(classify("/app%2Ejs"), AdminRequest::File);
        assert_eq!(classify("/%61ssets/app.js"), AdminRequest::Asset);
        assert_eq!(classify("/posts%2F12"), AdminRequest::ClientRoute);
        assert_eq!(classify("/caf%C3%A9"), AdminRequest::ClientRoute);
    }

    #[test]
    fn traversal_and_garbage_are_rejected() {
        assert_eq!(classify("/.."), AdminRequest::Rejected);
        assert_eq!(classify("/../Cargo.toml"), AdminRequest::Rejected);
        assert_eq!(classify("/assets/../../Cargo.toml"), AdminRequest::Rejected);
        assert_eq!(
            classify("/assets/%2e%2e/%2E%2E/Cargo.toml"),
            AdminRequest::Rejected
        );
        assert_eq!(classify("/..%2f..%2fCargo.toml"), AdminRequest::Rejected);
        assert_eq!(classify("/posts/../.."), AdminRequest::Rejected);
        assert_eq!(classify("/%FF"), AdminRequest::Rejected);
        assert_eq!(classify("/a%00b"), AdminRequest::Rejected);
        assert_eq!(classify("posts"), AdminRequest::Rejected);
        assert_eq!(classify("x/"), AdminRequest::Rejected);
    }

    #[test]
    fn serves_index_only_for_index_and_client_routes() {
        assert!(AdminRequest::Index.serves_index());
        assert!(AdminRequest::ClientRoute.serves_index());
        assert!(!AdminRequest::Asset.serves_index());
        assert!(!AdminRequest::File.serves_index());
        assert!(!AdminRequest::Rejected.serves_index());
    }

    #[test]
    fn cache_control_table() {
        // Rows of the ADR 0004 §3 routing table.
        assert_eq!(cache_control(AdminRequest::Index, 200), CACHE_REVALIDATE);
        assert_eq!(cache_control(AdminRequest::Index, 304), CACHE_REVALIDATE);
        assert_eq!(
            cache_control(AdminRequest::ClientRoute, 200),
            CACHE_REVALIDATE
        );
        assert_eq!(cache_control(AdminRequest::Asset, 200), CACHE_IMMUTABLE);
        assert_eq!(cache_control(AdminRequest::Asset, 206), CACHE_IMMUTABLE);
        assert_eq!(cache_control(AdminRequest::Asset, 304), CACHE_IMMUTABLE);
        assert_eq!(cache_control(AdminRequest::Asset, 404), CACHE_NO_STORE);
        assert_eq!(cache_control(AdminRequest::File, 200), CACHE_REVALIDATE);
        assert_eq!(cache_control(AdminRequest::File, 404), CACHE_NO_STORE);
        assert_eq!(cache_control(AdminRequest::Index, 404), CACHE_NO_STORE);
        assert_eq!(cache_control(AdminRequest::Asset, 416), CACHE_NO_STORE);
        assert_eq!(cache_control(AdminRequest::Asset, 500), CACHE_NO_STORE);
        assert_eq!(cache_control(AdminRequest::Rejected, 200), CACHE_NO_STORE);
        assert_eq!(cache_control(AdminRequest::Rejected, 404), CACHE_NO_STORE);
    }

    #[test]
    fn cacheable_success_statuses() {
        for status in [200, 206, 304] {
            assert!(is_cacheable_success(status), "{status}");
        }
        for status in [
            0, 100, 201, 204, 301, 302, 400, 403, 404, 405, 416, 500, 503,
        ] {
            assert!(!is_cacheable_success(status), "{status}");
        }
    }

    /// A path segment without `/`, `.` or `%`, so it cannot be empty, a dot
    /// segment, a file name or an escape.
    fn plain_segment() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_~-]{1,12}"
    }

    /// Percent-encodes every byte of `segment`, `a` included, as `%61`.
    fn encode_every_byte(segment: &str) -> String {
        use std::fmt::Write as _;
        segment.bytes().fold(String::new(), |mut encoded, byte| {
            let _ = write!(encoded, "%{byte:02X}");
            encoded
        })
    }

    fn any_request() -> impl Strategy<Value = AdminRequest> {
        prop_oneof![
            Just(AdminRequest::Index),
            Just(AdminRequest::ClientRoute),
            Just(AdminRequest::Asset),
            Just(AdminRequest::File),
            Just(AdminRequest::Rejected),
        ]
    }

    proptest! {
        #[test]
        fn never_panics(rest in any::<String>()) {
            let _ = classify(&rest);
        }

        #[test]
        fn dotless_paths_outside_assets_are_client_routes(
            segments in proptest::collection::vec(plain_segment(), 1..6),
            trailing_slash in any::<bool>(),
        ) {
            prop_assume!(segments[0] != ASSETS_DIR);
            let mut rest = format!("/{}", segments.join("/"));
            if trailing_slash {
                rest.push('/');
            }
            prop_assert_eq!(classify(&rest), AdminRequest::ClientRoute);
        }

        #[test]
        fn a_dot_in_the_last_segment_makes_a_file_request(
            segments in proptest::collection::vec(plain_segment(), 0..5),
            stem in plain_segment(),
            extension in "[a-z0-9]{1,5}",
        ) {
            prop_assume!(segments.first().is_none_or(|first| first != ASSETS_DIR));
            let mut parts = segments.clone();
            parts.push(format!("{stem}.{extension}"));
            let rest = format!("/{}", parts.join("/"));
            prop_assert_eq!(classify(&rest), AdminRequest::File);
        }

        #[test]
        fn everything_under_assets_is_an_asset(
            segments in proptest::collection::vec(plain_segment(), 0..5),
            file in proptest::option::of("[a-z0-9]{1,8}\\.[a-z0-9]{1,4}"),
        ) {
            let mut parts = vec![ASSETS_DIR.to_owned()];
            parts.extend(segments);
            parts.extend(file);
            let rest = format!("/{}", parts.join("/"));
            prop_assert_eq!(classify(&rest), AdminRequest::Asset);
        }

        #[test]
        fn any_dot_dot_segment_is_rejected(
            before in proptest::collection::vec(plain_segment(), 0..4),
            after in proptest::collection::vec(plain_segment(), 0..4),
            spelling in prop_oneof![Just(".."), Just("%2e%2e"), Just("%2E."), Just(".%2E")],
        ) {
            let mut parts = before;
            parts.push(spelling.to_owned());
            parts.extend(after);
            let rest = format!("/{}", parts.join("/"));
            prop_assert_eq!(classify(&rest), AdminRequest::Rejected);
        }

        #[test]
        fn percent_encoding_every_byte_does_not_change_the_class(
            segments in proptest::collection::vec("[a-zA-Z0-9_.~-]{1,12}", 0..6),
        ) {
            let rest = format!("/{}", segments.join("/"));
            let encoded: String = rest
                .split('/')
                .map(encode_every_byte)
                .collect::<Vec<_>>()
                .join("/");
            prop_assert_eq!(classify(&encoded), classify(&rest));
        }

        #[test]
        fn exactly_200_206_and_304_are_cacheable_successes(status in any::<u16>()) {
            let documented = [200, 206, 304];
            prop_assert_eq!(is_cacheable_success(status), documented.contains(&status));
        }

        #[test]
        fn no_error_or_informational_status_is_cacheable(
            status in prop_oneof![0u16..200, 400u16..=u16::MAX],
        ) {
            prop_assert!(!is_cacheable_success(status));
        }

        #[test]
        fn immutable_only_on_successful_asset_responses(
            request in any_request(),
            status in any::<u16>(),
        ) {
            let value = cache_control(request, status);
            if value == CACHE_IMMUTABLE {
                prop_assert_eq!(request, AdminRequest::Asset);
                prop_assert!(is_cacheable_success(status));
            }
            if !is_cacheable_success(status) {
                prop_assert_eq!(value, CACHE_NO_STORE);
            }
        }
    }
}
