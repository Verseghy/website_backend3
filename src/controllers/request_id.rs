//! Every request's id is one that the server makes (research: DECISIONS R23
//! (D119), "Server id for every request: drop the incoming header").
//!
//! Loco's `request_id` middleware, on in every environment, gives each
//! request an id: it puts it into the request's extensions
//! (`LocoRequestId`), into the request's log span and into the response's
//! `x-request-id` header. It keeps an id that the client sent in an
//! `x-request-id` header, cleaned to letters, digits, `_`, `-` and `@` and
//! cut to 255 characters, and makes a UUID v4 only when none came (loco-rs
//! 1.2.0 `src/controller/middleware/request_id.rs`). backend3 stores that id
//! in activity entries and revisions, which can never be changed (ADR 0011
//! §4, §5), so a kept header would put text of the caller's choice into
//! them for good, and would let a caller make unrelated entries look like
//! one request's.
//!
//! [`without_client_request_id`] therefore removes every `x-request-id`
//! header from the request before Loco's middlewares run, so Loco makes a
//! UUID v4 for every request. The id still does its two jobs: it groups the
//! entries of one request, and it links them to the request's log lines
//! and to the response header. A client or a proxy in front of backend3
//! cannot pass its own id through; none does (the admin UI sends no such
//! header, and the router in front of backend3 sets none).
//!
//! `App::after_routes` applies it to Loco's finished router, outside Loco's
//! middleware stack, inside the HTTP request metrics layer: a layer added
//! there wraps every route and the fallback after Loco's middlewares, so it
//! runs before them on every request (loco-rs 1.2.0 `src/boot.rs`,
//! `setup_routes`). The fallback `/storage` router is merged after it, so
//! the layer does not reach `/storage`, which runs outside Loco's
//! middlewares and makes no request id (`controllers::storage`).

use axum::{
    Router as AxumRouter,
    extract::Request,
    http::{HeaderMap, HeaderName},
    middleware::map_request,
};

/// The request header that Loco's `request_id` middleware reads and its
/// response carries (loco-rs 1.2.0 `X_REQUEST_ID`).
pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Wraps `router` in the layer that removes every `x-request-id` header from
/// each request, so that Loco's `request_id` middleware makes the id itself
/// (the module documentation says why).
///
/// - `router`: the finished application router, Loco's middleware stack
///   included; the layer goes outside it.
pub fn without_client_request_id(router: AxumRouter) -> AxumRouter {
    router.layer(map_request(drop_client_request_id))
}

/// The request with every `x-request-id` header removed.
///
/// - `request`: the incoming request.
async fn drop_client_request_id(mut request: Request) -> Request {
    remove_request_id(request.headers_mut());
    request
}

/// Removes every value of the `x-request-id` header from `headers`, also
/// when a client sent it more than once, and leaves every other header as
/// it was.
///
/// - `headers`: the request's headers.
fn remove_request_id(headers: &mut HeaderMap) {
    // `HeaderMap::remove` drops all values of the name, not only the first.
    headers.remove(REQUEST_ID_HEADER);
}

#[cfg(test)]
mod tests {
    use axum::{
        Extension,
        body::{Body, to_bytes},
        http::HeaderValue,
        middleware::from_fn,
        routing::get,
    };
    use loco_rs::controller::middleware::request_id::{LocoRequestId, request_id_middleware};
    use tower::ServiceExt as _;

    use super::*;

    /// Answers with the request id that Loco handed the handler.
    async fn echo(Extension(id): Extension<LocoRequestId>) -> String {
        id.get().to_owned()
    }

    /// A router with Loco's request id middleware, wrapped in
    /// [`without_client_request_id`] when `drop` is set, as
    /// `App::after_routes` wraps the application.
    fn router(drop: bool) -> AxumRouter {
        let router = AxumRouter::new()
            .route("/", get(echo))
            .layer(from_fn(request_id_middleware));
        if drop {
            without_client_request_id(router)
        } else {
            router
        }
    }

    /// Sends a request with the `x-request-id` values `sent` and returns the
    /// id the handler saw and the response's `x-request-id`.
    async fn ids(drop: bool, sent: &[&str]) -> (String, String) {
        let mut request = Request::get("/");
        for value in sent {
            request = request.header(REQUEST_ID_HEADER, *value);
        }
        let response = router(drop)
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("infallible");
        let header = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .expect("Loco answers with the id")
            .to_str()
            .expect("ASCII")
            .to_owned();
        let body = to_bytes(response.into_body(), 1024).await.expect("body");
        (String::from_utf8(body.to_vec()).expect("UTF-8"), header)
    }

    fn assert_server_made(id: &str) {
        let uuid = loco_rs::prelude::Uuid::parse_str(id).expect("a UUID");
        assert_eq!(uuid.get_version_num(), 4, "{id}");
    }

    #[tokio::test]
    async fn loco_alone_keeps_the_caller_s_id() {
        // The reason for the layer: without it, the caller's text is the id.
        assert_eq!(
            ids(false, &["kovacs_janos@verseghy"]).await,
            (
                "kovacs_janos@verseghy".to_owned(),
                "kovacs_janos@verseghy".to_owned()
            )
        );
    }

    #[tokio::test]
    async fn every_request_gets_an_id_the_server_made() {
        for sent in [
            &[][..],
            &["kovacs_janos@verseghy"],
            &["a", "b"],
            // The form of an id Loco would make: still not the caller's.
            &["0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d"],
        ] {
            let (seen, answered) = ids(true, sent).await;
            assert_eq!(seen, answered, "{sent:?}");
            assert_server_made(&seen);
            assert!(!sent.contains(&seen.as_str()), "{sent:?}");
        }
    }

    #[test]
    fn only_the_request_id_header_is_removed() {
        let mut headers = HeaderMap::new();
        headers.append(REQUEST_ID_HEADER, HeaderValue::from_static("first"));
        headers.append(REQUEST_ID_HEADER, HeaderValue::from_static("second"));
        // Header names are case-insensitive: any spelling is the same header.
        headers.append(
            HeaderName::from_bytes(b"X-Request-Id").expect("a header name"),
            HeaderValue::from_static("third"),
        );
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.9"));
        headers.insert("x-request-ids", HeaderValue::from_static("kept"));
        remove_request_id(&mut headers);
        assert_eq!(headers.get(REQUEST_ID_HEADER), None);
        assert_eq!(headers.len(), 2, "{headers:?}");
        assert_eq!(headers["x-forwarded-for"], "203.0.113.9");
        assert_eq!(headers["x-request-ids"], "kept");
    }
}
