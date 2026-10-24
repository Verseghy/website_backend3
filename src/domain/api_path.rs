//! Which API a request path belongs to, for answering unknown paths.
//!
//! backend3 serves two JSON APIs on one host (ADR 0014): the admin API under
//! `/api/admin` and the public read API under `/api/v1`. Each has its own
//! error envelope (ADR 0021 §4 and docs/design/public-api.md §8). A request for a path
//! that no route matches must still get the envelope of the API it was aimed
//! at, with status `404`, and must never fall back to the admin app's
//! `index.html` (ADR 0004 §3). The router's fallback calls [`api_family`] to
//! choose the envelope.
//!
//! The check runs on the raw request path, the same bytes the router matched
//! against, and is case-sensitive like the routes themselves. `/api/v1x` is
//! not part of `/api/v1`.

/// The prefix of the admin JSON API (ADR 0014).
pub const ADMIN_API_PREFIX: &str = "/api/admin";
/// The prefix of the public read API (ADR 0014, public-api.md).
pub const PUBLIC_API_PREFIX: &str = "/api/v1";

/// One of backend3's two JSON APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiFamily {
    /// `/api/admin` and everything below it.
    Admin,
    /// `/api/v1` and everything below it.
    V1,
}

/// Returns the API that `path` belongs to, or `None` for a path outside both.
///
/// - `path`: the request path without the query string, as the router saw it.
///
/// A path belongs to an API when it equals the prefix or continues it with a
/// `/`.
#[must_use]
pub fn api_family(path: &str) -> Option<ApiFamily> {
    if is_under(path, ADMIN_API_PREFIX) {
        Some(ApiFamily::Admin)
    } else if is_under(path, PUBLIC_API_PREFIX) {
        Some(ApiFamily::V1)
    } else {
        None
    }
}

/// Whether `path` is `prefix` itself or a path below it.
fn is_under(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn admin_paths() {
        assert_eq!(api_family("/api/admin"), Some(ApiFamily::Admin));
        assert_eq!(api_family("/api/admin/"), Some(ApiFamily::Admin));
        assert_eq!(api_family("/api/admin/posts/12"), Some(ApiFamily::Admin));
    }

    #[test]
    fn public_paths() {
        assert_eq!(api_family("/api/v1"), Some(ApiFamily::V1));
        assert_eq!(api_family("/api/v1/"), Some(ApiFamily::V1));
        assert_eq!(api_family("/api/v1/does-not-exist"), Some(ApiFamily::V1));
    }

    #[test]
    fn other_paths() {
        for path in [
            "",
            "/",
            "/api",
            "/api/",
            "/api/v1x",
            "/api/v2/posts",
            "/api/adminx",
            "/API/v1",
            "/admin",
            "/admin/api/v1",
            "api/v1",
            "/_ping",
        ] {
            assert_eq!(api_family(path), None, "{path}");
        }
    }

    proptest! {
        #[test]
        fn never_panics(path in any::<String>()) {
            let _ = api_family(&path);
        }

        #[test]
        fn everything_below_a_prefix_belongs_to_it(rest in "(/[^/]{0,10}){0,5}") {
            let admin = format!("{ADMIN_API_PREFIX}{rest}");
            let public = format!("{PUBLIC_API_PREFIX}{rest}");
            prop_assert_eq!(api_family(&admin), Some(ApiFamily::Admin));
            prop_assert_eq!(api_family(&public), Some(ApiFamily::V1));
        }

        #[test]
        fn a_prefix_continued_without_a_slash_is_not_an_api(
            next in "[^/]",
            rest in "[^ ]{0,20}",
        ) {
            let admin = format!("{ADMIN_API_PREFIX}{next}{rest}");
            let public = format!("{PUBLIC_API_PREFIX}{next}{rest}");
            prop_assert_eq!(api_family(&admin), None);
            prop_assert_eq!(api_family(&public), None);
        }

        #[test]
        fn paths_not_starting_with_api_are_never_an_api(path in "[^/].*|/[^a].*") {
            prop_assert_eq!(api_family(&path), None);
        }
    }
}
