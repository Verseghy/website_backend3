//! Unknown paths under `/api/admin` and `/api/v1` answer the JSON `404` of
//! their API and never fall back to the admin app (ADR 0004 §3,
//! ADR 0021 §4, public-api.md §8).

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::{fixture, with_server};

#[tokio::test]
async fn unknown_admin_api_paths_get_the_admin_envelope() {
    with_server(|request, _ctx| async move {
        for (method, path) in [
            (Method::GET, "/api/admin"),
            (Method::GET, "/api/admin/"),
            (Method::GET, "/api/admin/posts/12/show"),
            (Method::POST, "/api/admin/posts"),
            (Method::DELETE, "/api/admin/posts/12"),
        ] {
            let response = request.method(method.clone(), path).await;
            assert_eq!(
                response.status_code(),
                StatusCode::NOT_FOUND,
                "{method} {path}"
            );
            assert_eq!(
                response.header("content-type"),
                "application/json",
                "{method} {path}"
            );
            assert_eq!(
                response.header("cache-control"),
                "no-store",
                "{method} {path}"
            );
            assert_ne!(response.as_bytes().as_ref(), fixture("index.html"));
            assert_eq!(
                response.json::<Value>(),
                json!({
                    "error": "not_found",
                    "description": "Resource was not found",
                    "field": null,
                    "params": {}
                }),
                "{method} {path}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn unknown_public_api_paths_get_the_public_envelope() {
    with_server(|request, _ctx| async move {
        for (method, path) in [
            (Method::GET, "/api/v1"),
            (Method::GET, "/api/v1/"),
            (Method::GET, "/api/v1/does-not-exist"),
            (Method::GET, "/api/v1/posts/12?token=x"),
            (Method::POST, "/api/v1/posts"),
        ] {
            let response = request.method(method.clone(), path).await;
            assert_eq!(
                response.status_code(),
                StatusCode::NOT_FOUND,
                "{method} {path}"
            );
            assert_eq!(
                response.header("content-type"),
                "application/json",
                "{method} {path}"
            );
            assert_eq!(
                response.header("cache-control"),
                "no-store",
                "{method} {path}"
            );
            assert_eq!(
                response.header("access-control-allow-origin"),
                "*",
                "{method} {path}"
            );
            assert!(
                response
                    .maybe_header("access-control-allow-credentials")
                    .is_none()
            );
            assert_eq!(
                response.json::<Value>(),
                json!({ "error": "not_found", "description": "Resource was not found" }),
                "{method} {path}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn other_unknown_paths_get_a_plain_404() {
    with_server(|request, _ctx| async move {
        for path in [
            "/api/v1x",
            "/api/v2/posts",
            "/api",
            "/nope",
            "/administrator",
        ] {
            let response = request.get(path).await;
            assert_eq!(response.status_code(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(response.text(), "Not Found\n", "{path}");
            assert_eq!(response.header("cache-control"), "no-store", "{path}");
        }
    })
    .await;
}
