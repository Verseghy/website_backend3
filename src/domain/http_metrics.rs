//! The label values and latency buckets of backend3's HTTP request metrics
//! (ADR 0023 §9).
//!
//! Every distinct combination of label values is a time series that the
//! process keeps in memory until it exits, and the edge node has very little
//! to spare (ADR 0023, "Platform"). So no label value may come from the
//! client unchecked: a scanner that sends a thousand random paths or invented
//! methods must not create a thousand series. The rules here keep every label
//! inside a set that the code, not the client, bounds:
//!
//! - `route` is the route *template* the router matched (`/admin/{*path}`),
//!   never the request path. A request that no route matched is counted
//!   under [`UNMATCHED_ROUTE`]. Templates always start with `/`, so that
//!   value cannot collide with one.
//! - `method` is one of the nine methods of RFC 9110 and RFC 5789, compared
//!   case-sensitively as HTTP does; any other method is [`OTHER_METHOD`].
//! - `status` is the numeric status code, which the HTTP stack already
//!   limits to 100-999, or [`CLIENT_CLOSED_STATUS`] for a request that
//!   ended without a response.

/// The `route` label of a request that matched no route (the router's
/// fallback answered it).
pub const UNMATCHED_ROUTE: &str = "unmatched";

/// The `method` label of a request whose method is not one of
/// [`STANDARD_METHODS`].
pub const OTHER_METHOD: &str = "other";

/// The request methods that keep their own `method` label: the eight of
/// RFC 9110 §9 and `PATCH` of RFC 5789, in their registered (upper-case)
/// spelling.
pub const STANDARD_METHODS: [&str; 9] = [
    "GET", "HEAD", "POST", "PUT", "DELETE", "CONNECT", "OPTIONS", "TRACE", "PATCH",
];

/// The `status` label of a request that never got a response, because the
/// client, or the router in front of backend3 at its timeout, closed the
/// connection first. 499 is nginx's code for this; HTTP defines none, and
/// backend3 never sends it.
pub const CLIENT_CLOSED_STATUS: u16 = 499;

/// Upper bounds, in seconds, of the request-latency histogram buckets.
///
/// Prometheus' default buckets (5 ms to 10 s), extended to 120 s: uploads
/// with image optimisation can take longer than 10 s, and 120 s is the
/// longest request the admin API's Route lets through. Every request above
/// the last bound lands in the implicit `+Inf` bucket.
pub const LATENCY_BUCKETS_SECONDS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];

/// Returns the `method` label for a request method.
///
/// - `method`: the method as the request carried it, for example `GET`.
///
/// A standard method keeps its name; anything else, including a lower-case
/// spelling such as `get` (HTTP methods are case-sensitive, so that is an
/// extension method), becomes [`OTHER_METHOD`].
#[must_use]
pub fn method_label(method: &str) -> &'static str {
    STANDARD_METHODS
        .into_iter()
        .find(|standard| *standard == method)
        .unwrap_or(OTHER_METHOD)
}

/// Returns the `route` label for a request.
///
/// - `matched_template`: the route template the router matched, or `None`
///   when no route matched and the fallback answered.
#[must_use]
pub fn route_label(matched_template: Option<&str>) -> &str {
    matched_template.unwrap_or(UNMATCHED_ROUTE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn standard_methods_keep_their_name() {
        for method in STANDARD_METHODS {
            assert_eq!(method_label(method), method);
        }
    }

    #[test]
    fn other_methods_share_one_label() {
        for method in ["", "get", "Get", "BREW", "PROPFIND", "GET ", " GET", "GETX"] {
            assert_eq!(method_label(method), OTHER_METHOD, "{method:?}");
        }
    }

    #[test]
    fn a_matched_template_is_the_route() {
        assert_eq!(route_label(Some("/_ping")), "/_ping");
        assert_eq!(route_label(Some("/admin/{*path}")), "/admin/{*path}");
    }

    #[test]
    fn no_match_is_the_unmatched_route() {
        assert_eq!(route_label(None), UNMATCHED_ROUTE);
        assert!(!UNMATCHED_ROUTE.starts_with('/'));
    }

    #[test]
    fn buckets_are_positive_finite_and_strictly_increasing() {
        assert!(
            LATENCY_BUCKETS_SECONDS
                .iter()
                .all(|b| b.is_finite() && *b > 0.0)
        );
        assert!(LATENCY_BUCKETS_SECONDS.windows(2).all(|w| w[0] < w[1]));
        assert!((LATENCY_BUCKETS_SECONDS[13] - 120.0).abs() < f64::EPSILON);
    }

    proptest! {
        #[test]
        fn the_method_label_set_is_closed(method in any::<String>()) {
            let label = method_label(&method);
            prop_assert!(label == OTHER_METHOD || STANDARD_METHODS.contains(&label));
            // A method keeps its own label only when it is exactly that
            // standard method.
            prop_assert_eq!(label != OTHER_METHOD, STANDARD_METHODS.contains(&method.as_str()));
        }

        #[test]
        fn a_template_never_becomes_the_unmatched_route(rest in "[^ ]{0,40}") {
            let template = format!("/{rest}");
            prop_assert_eq!(route_label(Some(&template)), template.as_str());
            prop_assert_ne!(route_label(Some(&template)), UNMATCHED_ROUTE);
        }
    }
}
