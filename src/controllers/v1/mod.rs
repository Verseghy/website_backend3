//! The public read API under `/api/v1` (docs/design/public-api.md).
//!
//! [`routes`] is the one builder of every `/api/v1` route; `App::routes` adds
//! it once. It merges the routes of the modules below, then puts
//! [`public_layers`] on each of them:
//!
//! - **CORS** (ADR 0014, DEC-18): `Access-Control-Allow-Origin: *` on every
//!   response, errors included, and never credentials. Loco's global `cors`
//!   middleware stays off, so `/admin` and `/api/admin` send no CORS header.
//! - **Panics** (public-api.md §8): a panicking handler is answered with the
//!   public `500` body inside the CORS layer, so it carries the CORS header
//!   too. Loco's global `catch_panic` middleware would answer it outside the
//!   route layers, without that header.
//!
//! Methods (public-api.md §2.1): a `GET` route answers `HEAD` too, as axum
//! does. Any other method gets axum's `405` with `Allow: GET,HEAD` and an
//! empty body, except `OPTIONS`: the `CorsLayer` answers every `OPTIONS`
//! request to a route itself as a CORS preflight, which allows `GET` and
//! `HEAD` and no request header. That is the settled answer, not a gap to
//! close with a `405`: the maintainer chose the preflight on 2026-10-25
//! (public-api.md §2.1 and open item O-10), so ADR 0014's `CorsLayer` stays.
//! The layer is on the routes only, so an unknown `/api/v1` path gets the
//! fallback's JSON `404` (below) for `OPTIONS` too.
//!
//! Loco and axum facts that shape [`routes`]:
//!
//! - `Routes::layer` wraps only the handlers that exist when it is called,
//!   so the layers go on after the last merge.
//! - `Routes::merge` drops the merged group's prefix, so the modules here
//!   write their paths in full, as their `#[utoipa::path]` does
//!   (docs/design/openapi-spike.md §3, rule 6), and a unit test checks them.
//! - axum's `MethodRouter::layer` also wraps the method router's fallback,
//!   which answers `405` and `OPTIONS`, so those answers get the layers too.
//!
//! Two answers come from Loco's middleware, outside the layers: an unknown
//! path, for every method, reaches the router fallback
//! (`crate::controllers::not_found`), which sets the CORS header itself from
//! [`ALLOW_ORIGIN`]; and a `304` from Loco's global `etag` middleware, which
//! builds a new response that keeps only the validator and freshness headers
//! of the `200`, so the slice that brings the `ETag` of public-api.md §9 must
//! add the CORS header to it.
//!
//! Two errors are built by the framework and carry no `Cache-Control`:
//! axum's `405` and Loco's `500` for a `loco_rs::Error` that a handler
//! returns with `?`. public-api.md §9 gives every `4xx` and `5xx` `no-store`,
//! so the slice that brings §9's caching headers must add `no-store` to
//! them too. backend3's own errors already carry it: the `Error` body, the
//! panic answer of [`answer_panic`] and the fallback's `404`.
//!
//! - [`openapi`]: the OpenAPI document and `GET /api/v1/openapi.json`
//!   (ADR 0005 §4).
//!
//! Each public endpoint slice (ADR 0025 §3, class (c)) merges its module's
//! routes in [`routes`] and adds its operation to the document.

use std::any::Any;

use axum::{
    http::{HeaderName, Method},
    response::{IntoResponse, Response},
};
use loco_rs::controller::Routes;
use tower::{
    ServiceBuilder,
    layer::util::{Identity, Stack},
};
use tower_http::{
    catch_panic::CatchPanicLayer,
    cors::{Any as AnyOrigin, CorsLayer},
};

use crate::dtos::v1::error::{Error, ErrorCode};

pub mod openapi;

/// The `Access-Control-Allow-Origin` value of every `/api/v1` response
/// (ADR 0014). [`cors_layer`] sends it through `Any`; the router fallback
/// sets it from this constant.
pub const ALLOW_ORIGIN: &str = "*";

/// The methods the public API answers (public-api.md §2.1).
pub const METHODS: [Method; 2] = [Method::GET, Method::HEAD];

/// The panic handler of [`public_layers`], as a function pointer so that the
/// layer type can be named.
pub type PanicHandler = fn(Box<dyn Any + Send + 'static>) -> Response;

/// The type of [`public_layers`].
pub type PublicLayers =
    ServiceBuilder<Stack<CatchPanicLayer<PanicHandler>, Stack<CorsLayer, Identity>>>;

/// Every route of `/api/v1`, with [`public_layers`] on each, for
/// `App::routes`.
#[must_use]
pub fn routes() -> Routes {
    Routes::new()
        .merge(openapi::routes())
        // Last: `Routes::layer` wraps only the handlers merged above.
        .layer(public_layers())
}

/// The layers of every `/api/v1` route: [`cors_layer`] outside, so that it
/// answers a preflight first and adds its header to every answer, then the
/// panic handler [`answer_panic`], so that its `500` passes through the CORS
/// layer.
#[must_use]
pub fn public_layers() -> PublicLayers {
    ServiceBuilder::new()
        .layer(cors_layer())
        .layer(CatchPanicLayer::custom(answer_panic as PanicHandler))
}

/// The CORS layer of `/api/v1` (ADR 0014).
///
/// - Any origin, which tower-http sends as `*` on every response, with or
///   without an `Origin` header. tower-http refuses to combine it with
///   credentials: `CorsLayer::layer` panics, so a change that allowed them
///   would stop the application from starting.
/// - [`METHODS`] in a preflight answer, and no request or exposed header:
///   the public site sends simple requests and reads only CORS-safelisted
///   headers.
/// - No `Vary` header. tower-http's default `Vary: origin,
///   access-control-request-method, access-control-request-headers` is for
///   answers that depend on those headers; this one never does.
// No `#[must_use]` here: `CorsLayer` is `#[must_use]` itself, and clippy
// rejects the attribute twice (`double_must_use`).
pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AnyOrigin)
        .allow_methods(METHODS)
        .vary(Vec::<HeaderName>::new())
}

/// The answer to a panicking `/api/v1` handler: the public `500` body
/// ([`ErrorCode::InternalServerError`], with no internal details) and
/// `no-store`. It logs the panic as Loco's own `catch_panic` handler does
/// (`server_panic` at `ERROR` level), so log searches find both.
///
/// - `payload`: what the handler panicked with. `CatchPanicLayer` hands it
///   over by value, so the signature takes it by value although only a
///   reference is read.
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the signature that CatchPanicLayer::custom calls"
)]
pub fn answer_panic(payload: Box<dyn Any + Send + 'static>) -> Response {
    tracing::error!(err.msg = panic_message(payload.as_ref()), "server_panic");
    Error::new(ErrorCode::InternalServerError).into_response()
}

/// The message of a panic payload: the text of `panic!("...")`, formatted or
/// not, or a fixed text for a payload of any other type.
fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("no error details")
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{
            Request, StatusCode,
            header::{ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, VARY},
        },
        routing::get,
    };
    use tower::ServiceExt as _;

    use super::*;
    use crate::domain::api_path::{ApiFamily, api_family};

    /// Sends `GET /` to a route that answers with `handler`'s response
    /// behind [`public_layers`], put on the method router as `Routes::layer`
    /// does. The request tests cover the methods and the preflight through
    /// the real router; these cases need handlers no route has yet.
    async fn through_layers<F, R>(handler: F) -> (StatusCode, Vec<(String, String)>, Vec<u8>)
    where
        F: FnOnce() -> R + Clone + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        let router = Router::new().route(
            "/",
            get(move || async move { handler().into_response() }).layer(public_layers()),
        );
        let response = router
            .oneshot(Request::get("/").body(Body::empty()).expect("a request"))
            .await
            .expect("the router is infallible");
        let status = response.status();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_str().expect("ASCII").to_owned()))
            .collect();
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        (status, headers, body.to_vec())
    }

    /// The values of header `name`, in order.
    fn header<'a>(headers: &'a [(String, String)], name: &HeaderName) -> Vec<&'a str> {
        headers
            .iter()
            .filter(|(key, _)| key == name.as_str())
            .map(|(_, value)| value.as_str())
            .collect()
    }

    #[tokio::test]
    async fn a_handler_error_gets_cors_and_no_store() {
        let (status, headers, body) = through_layers(|| Error::new(ErrorCode::InvalidLimit)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(header(&headers, &ACCESS_CONTROL_ALLOW_ORIGIN), ["*"]);
        assert_eq!(header(&headers, &CACHE_CONTROL), ["no-store"]);
        assert_eq!(header(&headers, &VARY), [] as [&str; 0]);
        assert_eq!(
            body,
            br#"{"error":"invalid_limit","description":"limit must be an integer between 1 and 100"}"#
        );
    }

    #[tokio::test]
    async fn a_loco_500_gets_cors_and_the_public_body() {
        let (status, headers, body) = through_layers(|| -> loco_rs::Result<String> {
            Err(loco_rs::Error::InternalServerError)
        })
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(header(&headers, &ACCESS_CONTROL_ALLOW_ORIGIN), ["*"]);
        // Loco's own 500 has the bytes of the public `Error` body.
        assert_eq!(
            body,
            serde_json::to_vec(&Error::new(ErrorCode::InternalServerError)).expect("serializes")
        );
    }

    #[tokio::test]
    async fn a_panicking_handler_gets_the_public_500_with_cors() {
        let (status, headers, body) = through_layers(|| -> Response {
            panic!("a bug in a public handler");
        })
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(header(&headers, &ACCESS_CONTROL_ALLOW_ORIGIN), ["*"]);
        assert_eq!(header(&headers, &CACHE_CONTROL), ["no-store"]);
        assert_eq!(
            body,
            br#"{"error":"internal_server_error","description":"Internal Server Error"}"#
        );
    }

    #[tokio::test]
    async fn a_success_keeps_its_headers_and_gets_cors() {
        let (status, headers, body) =
            through_layers(|| ([(CACHE_CONTROL, "no-cache")], "ok")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(header(&headers, &CACHE_CONTROL), ["no-cache"]);
        assert_eq!(
            header(&headers, &ACCESS_CONTROL_ALLOW_ORIGIN),
            [ALLOW_ORIGIN]
        );
        assert_eq!(body, b"ok");
    }

    #[test]
    fn panic_messages_are_read_from_both_payload_types() {
        let formatted: Box<dyn Any + Send> = Box::new(format!("id {}", 7));
        let literal: Box<dyn Any + Send> = Box::new("plain");
        let other: Box<dyn Any + Send> = Box::new(42_u8);
        assert_eq!(panic_message(formatted.as_ref()), "id 7");
        assert_eq!(panic_message(literal.as_ref()), "plain");
        assert_eq!(panic_message(other.as_ref()), "no error details");
    }

    #[test]
    fn every_route_is_written_in_full_under_the_public_api() {
        let routes = routes();
        assert_eq!(routes.prefix, None);
        for handler in &routes.handlers {
            assert_eq!(
                api_family(&handler.uri),
                Some(ApiFamily::V1),
                "{}",
                handler.uri
            );
            assert!(!handler.actions.is_empty(), "{}", handler.uri);
        }
        assert!(
            routes
                .handlers
                .iter()
                .any(|handler| handler.uri == openapi::DOCUMENT_ROUTE
                    && handler.actions == [Method::GET])
        );
        assert_eq!(METHODS, [Method::GET, Method::HEAD]);
    }
}
