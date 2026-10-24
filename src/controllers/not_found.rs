//! The answer for paths that no route matches.
//!
//! `App::before_routes` installs [`fallback`] as the router's fallback, so it
//! runs inside Loco's middleware stack like every route. It answers in the
//! format of the API the path belongs to, decided by the pure
//! [`crate::domain::api_path::api_family`]:
//!
//! - `/api/admin/...`: the admin error envelope with `not_found` (ADR 0021 §4)
//!   and `Cache-Control: no-store`, as every `/api/admin` response
//!   (ADR 0023 §6).
//! - `/api/v1/...`: Loco's `ErrorDetail` envelope `{ error, description }`
//!   with `not_found` (public-api.md §6.15, §8, DEC-11), `Cache-Control:
//!   no-store` and `Access-Control-Allow-Origin: *`, because every
//!   `/api/v1` response carries the CORS header, errors included
//!   (ADR 0014).
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
use loco_rs::controller::ErrorDetail;

use crate::{
    domain::api_path::{ApiFamily, api_family},
    dtos::admin::error::{ErrorCode, ErrorEnvelope},
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
/// was not found" }`, with the CORS header of every `/api/v1` response.
#[must_use]
pub fn public_not_found() -> Response {
    let mut response = (
        StatusCode::NOT_FOUND,
        Json(ErrorDetail::new("not_found", "Resource was not found")),
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, NO_STORE);
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
}

/// The `404` for a path outside both APIs and outside `/admin`.
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
        assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            body_json(response).await,
            json!({ "error": "not_found", "description": "Resource was not found" })
        );
    }

    #[tokio::test]
    async fn other_paths_get_plain_text() {
        let response = fallback(Uri::from_static("/nope")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
    }
}
