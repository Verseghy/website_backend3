//! The request's part of a sign-in or account activity entry (ADR 0011 §5):
//! [`ClientContext`] reads the request id and the client address that Loco's
//! middlewares resolved, and hands them to the model method that records the
//! event as a [`RequestContext`] (ADR 0008 rule 3: models never read the
//! request).
//!
//! - **The request id** is Loco's `LocoRequestId`, which its `request_id`
//!   middleware (on in every environment) puts into the request's
//!   extensions. backend3 drops any `x-request-id` header a client sends
//!   before that middleware runs (`crate::controllers::request_id`;
//!   research: DECISIONS R23 (D119)), so the id is always a UUID v4 that
//!   Loco made, and it is the one in the request's log lines and the
//!   response's `x-request-id` header.
//! - **The client address** is the one that Loco's `remote_ip` middleware
//!   resolved (`RemoteIP`): with the source of ADR 0023 §2, the rightmost
//!   `X-Forwarded-For` value, which the router appends and a client cannot
//!   choose, and the socket address for a request without that header (an
//!   in-cluster caller). Where the configuration leaves the middleware off,
//!   Loco resolves no address, and the entry holds none: backend3 never
//!   reads an address that the configured source does not give. An IPv4
//!   address that reaches backend3 in its IPv6 form (`::ffff:192.0.2.1`, as
//!   a dual-stack socket reports it) is stored as the IPv4 address, so one
//!   client has one address in the log.

use std::{convert::Infallible, net::IpAddr};

use axum::{extract::FromRequestParts, http::request::Parts};
use loco_rs::controller::middleware::{remote_ip::RemoteIP, request_id::LocoRequestId};

use crate::models::activity_log::RequestContext;

/// The request id and the client address of the request, for an account
/// activity entry; extract it in a handler and pass the inner
/// [`RequestContext`] down.
///
/// It never refuses a request: a request without either part gets `None`
/// for it, and the entry leaves the column empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientContext(pub RequestContext);

impl<S> FromRequestParts<S> for ClientContext
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    /// Reads the request id from the extensions and resolves the client
    /// address through Loco's `RemoteIP` extractor.
    ///
    /// - `parts`: the request's head.
    /// - `state`: the router state; `RemoteIP` reads nothing from it.
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = parts
            .extensions
            .get::<LocoRequestId>()
            .map(|id| id.get().to_owned());
        let remote = RemoteIP::from_request_parts(parts, state).await?;
        Ok(Self(RequestContext {
            request_id,
            ip: client_address(remote),
        }))
    }
}

/// The address an entry records for `remote`, or `None` when Loco resolved
/// none.
///
/// - `remote`: what Loco's `RemoteIP` extractor resolved.
///
/// A forwarded and a socket address are both the client's as the configured
/// source sees it (ADR 0023 §2), so both are kept; an IPv4-mapped IPv6
/// address becomes its IPv4 address (the module documentation says why).
fn client_address(remote: RemoteIP) -> Option<IpAddr> {
    match remote {
        RemoteIP::Forwarded(ip) | RemoteIP::Socket(ip) => Some(ip.to_canonical()),
        RemoteIP::None => None,
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::{ConnectInfo, Request},
        middleware::from_fn,
        routing::get,
    };
    use loco_rs::controller::middleware::{
        remote_ip::ClientIpSource, request_id::request_id_middleware,
    };
    use tower::ServiceExt as _;

    use super::*;

    /// A handler that answers with what [`ClientContext`] extracted, as
    /// `<request id or ->|<address or ->`.
    async fn echo(ClientContext(context): ClientContext) -> String {
        format!(
            "{}|{}",
            context.request_id.as_deref().unwrap_or("-"),
            context
                .ip
                .map_or_else(|| "-".to_owned(), |ip| ip.to_string())
        )
    }

    /// Sends `request` to [`echo`] behind Loco's request id middleware and,
    /// when `source` is set, the `remote_ip` middleware with that source,
    /// as Loco applies them; returns the response's `x-request-id` and the
    /// two parts the handler saw.
    async fn extract(source: Option<ClientIpSource>, request: Request) -> (String, String, String) {
        let mut router = Router::new().route("/", get(echo));
        if let Some(source) = source {
            router = router.layer(source.into_extension());
        }
        let router = router.layer(from_fn(request_id_middleware));
        let response = router.oneshot(request).await.expect("infallible");
        let header = response
            .headers()
            .get("x-request-id")
            .expect("the request id header")
            .to_str()
            .expect("ASCII")
            .to_owned();
        let body = to_bytes(response.into_body(), 1024).await.expect("body");
        let body = String::from_utf8(body.to_vec()).expect("UTF-8");
        let (request_id, ip) = body.split_once('|').expect("two parts");
        (header, request_id.to_owned(), ip.to_owned())
    }

    fn request(forwarded_for: Option<&str>, socket: Option<&str>) -> Request {
        let mut builder = Request::get("/");
        if let Some(forwarded_for) = forwarded_for {
            builder = builder.header("x-forwarded-for", forwarded_for);
        }
        let mut request = builder.body(Body::empty()).expect("request");
        if let Some(socket) = socket {
            let socket: SocketAddr = socket.parse().expect("socket address");
            request.extensions_mut().insert(ConnectInfo(socket));
        }
        request
    }

    #[tokio::test]
    async fn the_request_id_is_the_one_loco_answers_with() {
        let (header, request_id, _) = extract(None, request(None, None)).await;
        assert_eq!(request_id, header);
        assert!(
            loco_rs::prelude::Uuid::parse_str(&request_id).is_ok(),
            "{request_id}"
        );
    }

    #[tokio::test]
    async fn without_the_remote_ip_middleware_no_address_is_resolved() {
        // Loco's `RemoteIP` resolves nothing when the middleware is off, and
        // the extractor reads no header or socket on its own.
        let (_, _, ip) = extract(
            None,
            request(Some("203.0.113.9"), Some("198.51.100.7:40000")),
        )
        .await;
        assert_eq!(ip, "-");
    }

    #[tokio::test]
    async fn the_rightmost_forwarded_address_is_the_client_s() {
        let source = || Some(ClientIpSource::RightmostXForwardedFor);
        for (case, forwarded_for, expected) in [
            ("one hop", "203.0.113.9", "203.0.113.9"),
            // A value the client sent comes first; the router appends its own.
            ("forged first", "192.0.2.66, 203.0.113.9", "203.0.113.9"),
            ("IPv6", "2001:db8::9", "2001:db8::9"),
            ("IPv4-mapped IPv6", "::ffff:203.0.113.9", "203.0.113.9"),
        ] {
            let (_, _, ip) = extract(
                source(),
                request(Some(forwarded_for), Some("198.51.100.7:40000")),
            )
            .await;
            assert_eq!(ip, expected, "{case}");
        }
    }

    #[tokio::test]
    async fn without_the_header_the_socket_address_is_the_client_s() {
        let source = || Some(ClientIpSource::RightmostXForwardedFor);
        let (_, _, ip) = extract(source(), request(None, Some("198.51.100.7:40000"))).await;
        assert_eq!(ip, "198.51.100.7");
        let (_, _, ip) =
            extract(source(), request(None, Some("[::ffff:198.51.100.7]:40000"))).await;
        assert_eq!(ip, "198.51.100.7", "IPv4-mapped socket address");
        // No header and no socket: nothing to resolve.
        let (_, _, ip) = extract(source(), request(None, None)).await;
        assert_eq!(ip, "-");
    }

    #[test]
    fn client_address_keeps_both_kinds_and_maps_ipv4_in_ipv6() {
        let v4: IpAddr = "192.0.2.1".parse().expect("IPv4");
        let v6: IpAddr = "2001:db8::1".parse().expect("IPv6");
        let mapped: IpAddr = "::ffff:192.0.2.1".parse().expect("IPv4-mapped");
        assert_eq!(client_address(RemoteIP::Forwarded(v4)), Some(v4));
        assert_eq!(client_address(RemoteIP::Socket(v4)), Some(v4));
        assert_eq!(client_address(RemoteIP::Forwarded(v6)), Some(v6));
        assert_eq!(client_address(RemoteIP::Socket(v6)), Some(v6));
        assert_eq!(client_address(RemoteIP::Forwarded(mapped)), Some(v4));
        assert_eq!(client_address(RemoteIP::Socket(mapped)), Some(v4));
        assert_eq!(client_address(RemoteIP::None), None);
    }
}
