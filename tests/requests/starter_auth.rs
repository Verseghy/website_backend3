//! Loco's starter auth routes are not mounted (ADR 0006 §4; parity AUTH-06,
//! self-registration retired by ADR 0019 §3).
//!
//! `loco new` 1.2.0 generates them under `/api/auth`: `register`,
//! `verify/{token}`, `login`, `forgot`, `reset`, `current`, `magic-link`,
//! `magic-link/{token}` and `resend-verification-mail`
//! (`loco@1.2.0:base_template/src/controllers/auth.rs`). backend3's own
//! sign-in routes live under `/api/admin/auth` and include none of the
//! mail-based ones, so every starter path answers the router fallback's
//! `404` there and under the starter's own prefix, and no request creates an
//! account.

use axum::http::{Method, StatusCode};
use backend3::models::users::Entity;
use loco_rs::prelude::{EntityTrait, PaginatorTrait};
use serde_json::{Value, json};

use crate::support::with_server;

/// The starter's routes that backend3 never mounts under any prefix, with
/// their methods. `login` is left out: backend3 adds its own
/// `/api/admin/auth/login` (ADR 0006 §4).
const NEVER_MOUNTED: &[(Method, &str)] = &[
    (Method::POST, "register"),
    (Method::GET, "verify/0b1c2d3e4f"),
    (Method::POST, "forgot"),
    (Method::POST, "reset"),
    (Method::GET, "current"),
    (Method::POST, "magic-link"),
    (Method::GET, "magic-link/0b1c2d3e4f"),
    (Method::POST, "resend-verification-mail"),
];

/// A body of the shape the starter's handlers read, so a mounted handler
/// would have had everything it needs.
fn starter_body() -> Value {
    json!({
        "name": "Próba Felhasználó",
        "email": "regisztracio@example.test",
        "password": "starter-route-password",
        "token": "0b1c2d3e4f",
    })
}

#[tokio::test]
async fn starter_paths_under_the_admin_api_answer_the_admin_json_404() {
    with_server(|request, ctx| async move {
        for (method, path) in NEVER_MOUNTED {
            let path = format!("/api/admin/auth/{path}");
            let response = request
                .method(method.clone(), &path)
                .json(&starter_body())
                .await;
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
        assert_eq!(Entity::find().count(&ctx.db).await.expect("count"), 0);
    })
    .await;
}

#[tokio::test]
async fn the_starter_prefix_is_not_mounted_at_all() {
    // `/api/auth` belongs to neither backend3 API, so the fallback answers
    // with its plain-text 404 (src/domain/api_path.rs).
    with_server(|request, ctx| async move {
        let paths = NEVER_MOUNTED
            .iter()
            .cloned()
            .chain([(Method::POST, "login")]);
        for (method, path) in paths {
            let path = format!("/api/auth/{path}");
            let response = request
                .method(method.clone(), &path)
                .json(&starter_body())
                .await;
            assert_eq!(
                response.status_code(),
                StatusCode::NOT_FOUND,
                "{method} {path}"
            );
            assert_eq!(response.text(), "Not Found\n", "{method} {path}");
            assert_eq!(
                response.header("cache-control"),
                "no-store",
                "{method} {path}"
            );
        }
        assert_eq!(Entity::find().count(&ctx.db).await.expect("count"), 0);
    })
    .await;
}
