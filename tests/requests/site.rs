//! Host-level routes of ADR 0014 and the health endpoint the CI wait loops
//! poll.

use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::support::with_server;

#[tokio::test]
async fn root_redirects_to_the_admin() {
    with_server(|request, _ctx| async move {
        let response = request.get("/").await;
        assert_eq!(response.status_code(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.header("location"), "/admin/");
    })
    .await;
}

#[tokio::test]
async fn robots_txt_disallows_everything() {
    with_server(|request, _ctx| async move {
        let response = request.get("/robots.txt").await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.text(), "User-agent: *\nDisallow: /\n");
    })
    .await;
}

#[tokio::test]
async fn ping_answers_ok() {
    with_server(|request, _ctx| async move {
        let response = request.get("/_ping").await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.json::<Value>(), json!({ "ok": true }));
    })
    .await;
}
