//! The caching headers of every `/api/v1` answer (public-api.md §9, DEC-12).
//!
//! There is no server-side response cache: every request reads PostgreSQL,
//! so an admin edit is visible on the next request (ADR 0016). Clients may
//! keep a copy of an answer but must revalidate it, which costs them a `304`
//! without a body when nothing changed. The rules of §8 and §9, and where
//! each one is kept:
//!
//! | Answer | `Cache-Control` | `ETag` | Built by |
//! |---|---|---|---|
//! | `200` | `no-cache` | weak, of the JSON body | [`cacheable_json`], [`cacheable_json_bytes`] |
//! | `200` of a preview | `no-store` | none | [`preview_json`] |
//! | `304` | `no-cache`, copied | the same, copied | Loco's `etag` middleware, then [`NotModifiedCors`] |
//! | `4xx`, `5xx` | `no-store` | none | the `Error` body, and [`error_headers`] for the framework's own |
//!
//! **Revalidation.** Loco's global `etag` middleware, enabled explicitly in
//! every `config/*.yaml`, compares the request's `If-None-Match` with the
//! `ETag` of the finished answer and, when they are equal, sends a new `304`
//! with an empty body that keeps only `ETag`, `Cache-Control`, `Vary`,
//! `Expires`, `Date` and `Content-Location` (loco-rs 1.2.0
//! `src/controller/middleware/etag.rs`). It never computes an `ETag`, so the
//! helpers here put [`weak_etag`] of the body on every cacheable answer. The
//! handler has already run, so a `304` saves bandwidth, not the database
//! query (§9).
//!
//! Loco compares the two header values byte for byte. A client that sends
//! back the tag it was given, as browsers do, gets the `304`. A list of tags,
//! `*`, or the tag without its `W/` gets the full `200`: RFC 9110 §13.1.2
//! would allow a `304` there too, so this answer is longer than needed but
//! never wrong. The OpenAPI document (`super::openapi`) promises the `304`
//! only for the tag sent back as received, so a later Loco that matches more
//! stays within the published contract.
//!
//! Three things the framework leaves to backend3:
//!
//! 1. **CORS on the `304`.** The CORS layer of `/api/v1` sits on the routes,
//!    inside Loco's middleware stack, and the new `304` drops its
//!    `Access-Control-Allow-Origin`. [`NotModifiedCors`], placed right outside
//!    the `etag` middleware by `App::middlewares`, adds it back to every `304`
//!    under `/api/v1` (ADR 0014: every `/api/v1` answer carries it).
//! 2. **`Vary` on the `304`.** The compression middleware, outside the `etag`
//!    one, adds `Vary: accept-encoding` only to a body large enough to
//!    compress, never to the empty `304`. RFC 9110 §15.4.5 asks a `304` to
//!    carry the `Vary` that its `200` would carry, so the helpers set it on
//!    every cacheable `200` themselves, and Loco copies it into the `304`.
//!    The compression middleware sees it there and adds no second one.
//! 3. **`no-store` on the framework's errors.** axum's `405` and Loco's `500`
//!    for a `loco_rs::Error` that a handler returns with `?` carry no
//!    `Cache-Control`. [`error_headers`], one of the layers of every
//!    `/api/v1` route (`super::public_layers`), gives every `4xx` and `5xx`
//!    `no-store` and removes any `ETag`, so the rule of §8 holds whoever
//!    built the error.

use axum::{
    body::Body,
    extract::Request,
    http::{
        HeaderValue, StatusCode,
        header::{ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_TYPE, ETAG, VARY},
    },
    middleware::{self, Next},
    response::Response,
};
use loco_rs::{Result, app::AppContext, controller::middleware::MiddlewareLayer};
use serde::Serialize;

use super::ALLOW_ORIGIN;
use crate::domain::{
    api_path::{ApiFamily, api_family},
    etag::weak_etag,
};

/// `Cache-Control` of a cacheable `200`: a client may keep the answer but
/// revalidates it before every use (public-api.md §9).
pub const NO_CACHE: HeaderValue = HeaderValue::from_static("no-cache");

/// `Cache-Control` of a preview and of every error: nothing may keep it
/// (public-api.md §8, §9).
pub const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");

/// `Content-Type` of every JSON answer. JSON has no charset parameter
/// (RFC 8259 §11), which is also how axum's and Loco's JSON answers send it.
const APPLICATION_JSON: HeaderValue = HeaderValue::from_static("application/json");

/// `Vary` of a cacheable `200`, the value the compression middleware writes.
const ACCEPT_ENCODING: HeaderValue = HeaderValue::from_static("accept-encoding");

/// A cacheable `200` with `body` serialised as JSON (public-api.md §9).
///
/// - `body`: the response DTO.
///
/// # Errors
///
/// Returns the serialisation error as a `loco_rs::Error`, Loco's `500`. The
/// DTOs of `crate::dtos::v1` have string keys only, so this does not happen
/// in practice.
pub fn cacheable_json<T: Serialize>(body: &T) -> Result<Response> {
    cacheable_json_bytes(serde_json::to_vec(body)?)
}

/// A cacheable `200` with a body that is already JSON: `application/json`,
/// `Cache-Control: no-cache`, the weak `ETag` of exactly these bytes and
/// `Vary: accept-encoding`.
///
/// - `body`: the serialised JSON, as it is sent before compression.
///
/// # Errors
///
/// Returns `loco_rs::Error::InvalidHeaderValue` if the tag were not a valid
/// header value. [`weak_etag`] writes visible ASCII only, so this does not
/// happen; the error is passed on rather than unwrapped, so that a change
/// there cannot become a panic inside a request.
pub fn cacheable_json_bytes(body: Vec<u8>) -> Result<Response> {
    let etag = HeaderValue::try_from(weak_etag(&body))?;
    let mut response = Response::new(Body::from(body));
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, APPLICATION_JSON);
    headers.insert(CACHE_CONTROL, NO_CACHE);
    headers.insert(ETAG, etag);
    headers.insert(VARY, ACCEPT_ENCODING);
    Ok(response)
}

/// A `200` of a preview (`/posts/{id}?token=`, public-api.md §7.3, §9):
/// `body` as JSON with `Cache-Control: no-store` and no `ETag`, because
/// unpublished content must not stay in any cache. Without an `ETag`, Loco's
/// middleware never answers it with a `304`.
///
/// - `body`: the response DTO.
///
/// # Errors
///
/// Returns the serialisation error as a `loco_rs::Error`, as
/// [`cacheable_json`] does.
pub fn preview_json<T: Serialize>(body: &T) -> Result<Response> {
    let mut response = Response::new(Body::from(serde_json::to_vec(body)?));
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, APPLICATION_JSON);
    headers.insert(CACHE_CONTROL, NO_STORE);
    Ok(response)
}

/// Gives every `4xx` and `5xx` answer of an `/api/v1` route
/// `Cache-Control: no-store` and removes any `ETag` (public-api.md §8, §9);
/// every other answer passes unchanged.
///
/// - `response`: the answer of the route: the handler's, Loco's `500` for a
///   `loco_rs::Error`, or axum's `405`.
///
/// `super::public_layers` runs it as a `tower` response mapper, so it is a
/// plain function.
#[must_use]
pub fn error_headers(mut response: Response) -> Response {
    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        let headers = response.headers_mut();
        headers.insert(CACHE_CONTROL, NO_STORE);
        headers.remove(ETAG);
    }
    response
}

/// A Loco middleware that adds `Access-Control-Allow-Origin` back to every
/// `304` that Loco's `etag` middleware answers under `/api/v1` (the module
/// documentation says why).
///
/// `App::middlewares` puts it right after `etag` in Loco's stack, which makes
/// it the next layer outside it: Loco applies the stack in order and each
/// entry wraps the router built so far. A `304` under `/api/v1` comes only
/// from that middleware, for a route whose `200` carried the header; an
/// unknown `/api/v1` path gets the fallback's `404`, which sets the header
/// itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct NotModifiedCors;

impl NotModifiedCors {
    /// The name `cargo loco middleware` lists it under.
    pub const NAME: &'static str = "v1_not_modified_cors";
}

impl MiddlewareLayer for NotModifiedCors {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    /// Always on: it changes only a `304` under `/api/v1`, which exists only
    /// while the `etag` middleware is on.
    fn is_enabled(&self) -> bool {
        true
    }

    fn config(&self) -> serde_json::Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    fn apply(&self, app: axum::Router<AppContext>) -> Result<axum::Router<AppContext>> {
        Ok(app.layer(middleware::from_fn(add_cors_to_not_modified)))
    }
}

/// The body of [`NotModifiedCors`]: runs the rest of the stack, then sets
/// `Access-Control-Allow-Origin: *` on a `304` whose path is under `/api/v1`.
///
/// The path is read from the request before it is passed on, as the router
/// matched it (after `App::serve` trimmed a trailing slash). `insert`
/// replaces any value, so the answer carries the header exactly once even if
/// a later Loco keeps it on the `304`.
async fn add_cors_to_not_modified(request: Request, next: Next) -> Response {
    let public = api_family(request.uri().path()) == Some(ApiFamily::V1);
    let mut response = next.run(request).await;
    if public && response.status() == StatusCode::NOT_MODIFIED {
        response.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static(ALLOW_ORIGIN),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::to_bytes,
        http::{HeaderName, header::IF_NONE_MATCH},
        routing::get,
    };
    use serde_json::json;
    use tower::ServiceExt as _;

    use super::*;

    /// The values of header `name` in `response`, in order.
    fn values(response: &Response, name: &HeaderName) -> Vec<String> {
        response
            .headers()
            .get_all(name)
            .iter()
            .map(|value| value.to_str().expect("ASCII").to_owned())
            .collect()
    }

    /// The body of `response`.
    async fn body(response: Response) -> Vec<u8> {
        to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body")
            .to_vec()
    }

    #[tokio::test]
    async fn a_cacheable_answer_carries_no_cache_and_the_weak_tag_of_its_body() {
        let dto = json!({ "id": 600, "title": "Őszi kirándulás" });
        let response = cacheable_json(&dto).expect("serialises");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(values(&response, &CONTENT_TYPE), ["application/json"]);
        assert_eq!(values(&response, &CACHE_CONTROL), ["no-cache"]);
        assert_eq!(values(&response, &VARY), ["accept-encoding"]);
        let tag = values(&response, &ETAG);
        let bytes = body(response).await;
        // Non-ASCII text stays UTF-8, not `\u` escapes (public-api.md §2.1).
        assert_eq!(bytes, serde_json::to_vec(&dto).expect("serialises"));
        assert!(String::from_utf8_lossy(&bytes).contains("Őszi"));
        assert_eq!(tag, [weak_etag(&bytes)]);
    }

    #[tokio::test]
    async fn the_tag_follows_the_bytes() {
        let first = cacheable_json_bytes(b"[1]".to_vec()).expect("a response");
        let same = cacheable_json_bytes(b"[1]".to_vec()).expect("a response");
        let other = cacheable_json_bytes(b"[2]".to_vec()).expect("a response");
        assert_eq!(values(&first, &ETAG), values(&same, &ETAG));
        assert_ne!(values(&first, &ETAG), values(&other, &ETAG));
        assert_eq!(body(first).await, b"[1]");
    }

    #[tokio::test]
    async fn a_preview_carries_no_store_and_no_tag() {
        let response = preview_json(&json!({ "id": 7 })).expect("serialises");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(values(&response, &CONTENT_TYPE), ["application/json"]);
        assert_eq!(values(&response, &CACHE_CONTROL), ["no-store"]);
        assert_eq!(values(&response, &ETAG), Vec::<String>::new());
        assert_eq!(values(&response, &VARY), Vec::<String>::new());
        assert_eq!(body(response).await, br#"{"id":7}"#);
    }

    /// A response with `status`, `Cache-Control: no-cache` and an `ETag`, as
    /// a handler might build one by mistake.
    fn tagged(status: StatusCode) -> Response {
        let mut response = cacheable_json_bytes(b"{}".to_vec()).expect("a response");
        *response.status_mut() = status;
        response
    }

    #[test]
    fn every_error_gets_no_store_and_loses_its_tag() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::NOT_FOUND,
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::IM_A_TEAPOT,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let response = error_headers(tagged(status));
            assert_eq!(response.status(), status);
            assert_eq!(values(&response, &CACHE_CONTROL), ["no-store"], "{status}");
            assert!(values(&response, &ETAG).is_empty(), "{status}");
        }
        // An error without any caching header gets `no-store` too, as axum's
        // 405 does.
        let mut bare = Response::new(Body::empty());
        *bare.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        assert_eq!(values(&error_headers(bare), &CACHE_CONTROL), ["no-store"]);
    }

    #[test]
    fn other_answers_keep_their_headers() {
        for status in [
            StatusCode::OK,
            StatusCode::NO_CONTENT,
            StatusCode::NOT_MODIFIED,
            StatusCode::PERMANENT_REDIRECT,
        ] {
            let response = error_headers(tagged(status));
            assert_eq!(values(&response, &CACHE_CONTROL), ["no-cache"], "{status}");
            assert_eq!(values(&response, &ETAG).len(), 1, "{status}");
        }
    }

    /// Sends `GET path` with `If-None-Match` through [`add_cors_to_not_modified`]
    /// in front of a router whose every route answers `status` and an `ETag`,
    /// standing in for Loco's `etag` middleware, which answers the `304`.
    async fn through_middleware(path: &str, status: StatusCode) -> Response {
        let answer = move || async move {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = status;
            response
                .headers_mut()
                .insert(ETAG, HeaderValue::from_static("W/\"x\""));
            response
        };
        let router = Router::new()
            .route("/api/v1/openapi.json", get(answer))
            .route("/api/v1x", get(answer))
            .route("/api/admin/posts", get(answer))
            .route("/admin", get(answer))
            .layer(middleware::from_fn(add_cors_to_not_modified));
        router
            .oneshot(
                Request::get(path)
                    .header(IF_NONE_MATCH, "W/\"x\"")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("the router is infallible")
    }

    #[tokio::test]
    async fn a_public_304_gets_the_cors_header_once() {
        let response = through_middleware("/api/v1/openapi.json", StatusCode::NOT_MODIFIED).await;
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            values(&response, &ACCESS_CONTROL_ALLOW_ORIGIN),
            [ALLOW_ORIGIN]
        );
        assert_eq!(values(&response, &ETAG), ["W/\"x\""]);
    }

    #[tokio::test]
    async fn other_304s_and_other_statuses_are_left_alone() {
        // Outside /api/v1: the admin API and the admin UI never send CORS
        // headers (ADR 0014), and `/api/v1x` is not under `/api/v1`.
        for path in ["/api/admin/posts", "/admin", "/api/v1x"] {
            let response = through_middleware(path, StatusCode::NOT_MODIFIED).await;
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED, "{path}");
            assert!(
                values(&response, &ACCESS_CONTROL_ALLOW_ORIGIN).is_empty(),
                "{path}"
            );
        }
        // Under /api/v1, a status other than 304 keeps what the route layers
        // gave it; here, nothing.
        for status in [StatusCode::OK, StatusCode::NOT_FOUND] {
            let response = through_middleware("/api/v1/openapi.json", status).await;
            assert!(
                values(&response, &ACCESS_CONTROL_ALLOW_ORIGIN).is_empty(),
                "{status}"
            );
        }
    }

    #[test]
    fn the_middleware_is_named_enabled_and_has_no_settings() {
        let layer = NotModifiedCors;
        assert_eq!(layer.name(), "v1_not_modified_cors");
        assert!(layer.is_enabled());
        assert_eq!(layer.config().expect("serialises"), json!({}));
    }
}
