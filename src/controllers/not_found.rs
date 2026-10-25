//! The answer for paths that no route matches.
//!
//! `App::before_routes` installs [`fallback`] as the router's fallback, so it
//! runs inside Loco's middleware stack like every route. It is the only place
//! that answers an unknown path: the `/api/v1` router adds no fallback of its
//! own. It answers in the format of the API the path belongs to, decided by
//! the pure [`crate::domain::api_path::api_family`]:
//!
//! - `/api/admin/...`: the admin error envelope with `not_found` (ADR 0021 §4)
//!   and `Cache-Control: no-store`, as every `/api/admin` response
//!   (ADR 0023 §6).
//! - `/api/v1/...`: the public `Error` body `{ error, description }` with
//!   `not_found` (public-api.md §6.15, §8, DEC-11), `Cache-Control:
//!   no-store` and `Access-Control-Allow-Origin: *`, because every
//!   `/api/v1` response carries the CORS header, errors included
//!   (ADR 0014). The fallback runs outside the layers that
//!   `crate::controllers::v1::routes` puts on the public routes, so it adds
//!   the CORS header itself.
//! - backend1's retired REST routes (`/api/posts/getPost` and the rest of
//!   [`crate::domain::api_path::RETIRED_API_PREFIXES`]): the same public body
//!   and `no-store`, but no CORS header, which ADR 0014 gives `/api/v1` only.
//! - anything else: a short plain-text `404`.
//!
//! Loco's own `fallback` middleware cannot do this: it is a development
//! welcome page, and ADR 0004 §3 disables it in every environment. Without
//! this handler an unknown path would get axum's empty `404`.

use axum::{
    Json,
    http::{
        HeaderValue, StatusCode, Uri,
        header::{ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};

use crate::{
    controllers::v1::ALLOW_ORIGIN,
    domain::api_path::{ApiFamily, api_family},
    dtos::{
        admin::error::{ErrorCode, ErrorEnvelope},
        v1::error::{Error as PublicError, ErrorCode as PublicErrorCode},
    },
};

/// `Cache-Control` of every error response: errors are never cached.
const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");

/// The router fallback: a `404` in the format of the API the path belongs to.
///
/// - `uri`: the request URI; only its path is used.
pub async fn fallback(uri: Uri) -> Response {
    match api_family(uri.path()) {
        Some(ApiFamily::Admin) => admin_not_found(),
        Some(ApiFamily::V1) => public_not_found(),
        Some(ApiFamily::Retired) => retired_not_found(),
        None => plain_not_found(),
    }
}

/// The admin API `404`: `{ "error": "not_found", "description": ...,
/// "field": null, "params": {} }`.
#[must_use]
pub fn admin_not_found() -> Response {
    let mut response = (
        StatusCode::NOT_FOUND,
        Json(ErrorEnvelope::new(ErrorCode::NotFound)),
    )
        .into_response();
    response.headers_mut().insert(CACHE_CONTROL, NO_STORE);
    response
}

/// The public API `404`: `{ "error": "not_found", "description": "Resource
/// was not found" }` with `no-store`, and the CORS header of every `/api/v1`
/// response.
#[must_use]
pub fn public_not_found() -> Response {
    let mut response = PublicError::new(PublicErrorCode::NotFound).into_response();
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static(ALLOW_ORIGIN),
    );
    response
}

/// The `404` of backend1's retired REST routes: the public API's body and
/// `no-store` ([`PublicError`]'s own response), without the CORS header.
#[must_use]
pub fn retired_not_found() -> Response {
    PublicError::new(PublicErrorCode::NotFound).into_response()
}

/// The `404` for a path outside every API and outside `/admin`.
fn plain_not_found() -> Response {
    let mut response = (
        StatusCode::NOT_FOUND,
        [(CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Not Found\n",
    )
        .into_response();
    response.headers_mut().insert(CACHE_CONTROL, NO_STORE);
    response
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use serde_json::{Value, json};

    use super::*;

    async fn body_json(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[tokio::test]
    async fn admin_paths_get_the_admin_envelope() {
        let response = fallback(Uri::from_static("/api/admin/nope")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        assert!(
            response
                .headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );
        assert_eq!(
            body_json(response).await,
            json!({
                "error": "not_found",
                "description": "Resource was not found",
                "field": null,
                "params": {}
            })
        );
    }

    #[tokio::test]
    async fn public_paths_get_the_public_envelope() {
        let response = fallback(Uri::from_static("/api/v1/nope?x=1")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            body_json(response).await,
            json!({ "error": "not_found", "description": "Resource was not found" })
        );
    }

    #[tokio::test]
    async fn retired_paths_get_the_public_envelope_without_cors() {
        for path in ["/api/posts/getPost?id=1", "/api/user"] {
            let response = fallback(Uri::try_from(path).expect("a URI")).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(response.headers()[CACHE_CONTROL], "no-store", "{path}");
            assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
            assert!(
                response
                    .headers()
                    .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                    .is_none(),
                "{path}"
            );
            assert_eq!(
                body_json(response).await,
                json!({ "error": "not_found", "description": "Resource was not found" }),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn other_paths_get_plain_text() {
        for path in ["/nope", "/api/v2/posts"] {
            let response = fallback(Uri::try_from(path).expect("a URI")).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(
                response.headers()[CONTENT_TYPE],
                "text/plain; charset=utf-8",
                "{path}"
            );
            assert_eq!(response.headers()[CACHE_CONTROL], "no-store", "{path}");
        }
    }
}
